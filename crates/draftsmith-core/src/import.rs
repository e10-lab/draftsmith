//! DXF·DWG 가져오기 (acadrust → 도면 모델).
//!
//! 모형 공간 객체를 읽고, 블록 참조(INSERT)는 블록 정의를 변환해 펼친다 (구현 노트 §5).
//! 가져오는 모양: 선·폴리선(LW·2D)·원·호·타원·한 줄 문자·여러 줄 문자(위치만)·블록 속성값.
//! 해치·치수 등 그 밖의 객체는 [`Drawing::skipped`] 에 개수만 남긴다.

use std::collections::HashMap;
use std::path::Path;

use acadrust::types::{Color, Vector3};
use acadrust::{CadDocument, DwgReader, DxfReader, EntityType};
use thiserror::Error;

use crate::drawing::{Drawing, Entity, Shape};
use crate::geom::{Affine, Vec2};

/// 블록 안 블록을 펼치는 최대 깊이. 정의가 서로를 부르는 깨진 도면에서 끝없이 돌지 않게.
pub const MAX_BLOCK_DEPTH: usize = 16;
/// 펼친 결과가 이 개수를 넘으면 더 펼치지 않는다 (거대한 배열 블록 등으로 메모리가 터지지 않게).
pub const MAX_ENTITIES: usize = 20_000_000;

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("파일을 열 수 없습니다: {0}")]
    Io(#[from] std::io::Error),
    #[error("도면을 읽지 못했습니다: {0}")]
    Cad(String),
}

/// 파일 머리말로 DWG 인지 본다 (`AC10..` 으로 시작). 아니면 DXF 로 읽는다.
fn is_dwg(path: &Path) -> std::io::Result<bool> {
    use std::io::Read;
    let mut head = [0u8; 4];
    let n = std::fs::File::open(path)?.read(&mut head)?;
    Ok(n == 4 && &head[..2] == b"AC" && head[2].is_ascii_digit())
}

/// DXF 또는 DWG 파일을 읽는다.
pub fn read_file(path: impl AsRef<Path>) -> Result<Drawing, ImportError> {
    let path = path.as_ref();
    let doc = if is_dwg(path)? {
        DwgReader::from_file(path)
            .and_then(|mut r| r.read())
            .map_err(|e| ImportError::Cad(e.to_string()))?
    } else {
        DxfReader::from_file(path)
            .and_then(|r| r.read())
            .map_err(|e| ImportError::Cad(e.to_string()))?
    };
    Ok(from_document(&doc))
}

fn v2(v: &Vector3) -> Vec2 {
    Vec2::new(v.x, v.y)
}

/// 객체 좌표계(OCS): 돌출 방향이 −Z 면 X 가 뒤집힌다 (대칭 복사한 블록·원에 흔함).
/// 기울어진 돌출 방향은 2D 도면에서 드물어 무시한다.
fn ocs(normal: &Vector3) -> Affine {
    if normal.z < 0.0 {
        Affine::scale(-1.0, 1.0)
    } else {
        Affine::IDENTITY
    }
}

/// 블록 안 객체가 블록 참조에게서 물려받는 것
#[derive(Clone, Copy)]
struct Inherit {
    /// 블록 참조의 레이어 — 블록 안 「0」 레이어 객체가 이 레이어로 그려진다
    layer: u32,
    /// 블록 참조의 색 — 블록 안 ByBlock 객체가 이 색으로 그려진다 (`None` 이면 그 레이어 색)
    color: Option<[u8; 3]>,
}

struct Converter<'a> {
    doc: &'a CadDocument,
    d: Drawing,
    layer_ids: HashMap<String, u32>,
    /// 지금 펼치고 있는 블록 이름들 (자기 자신을 부르는 정의 막기)
    stack: Vec<String>,
}

impl<'a> Converter<'a> {
    fn layer(&mut self, name: &str) -> u32 {
        if let Some(&i) = self.layer_ids.get(name) {
            return i;
        }
        let i = self.d.layer_index(name);
        self.layer_ids.insert(name.to_string(), i);
        i
    }

    fn skip(&mut self, kind: &str) {
        *self.d.skipped.entry(kind.to_string()).or_insert(0) += 1;
    }

    /// 객체의 레이어·색. 블록 안이면 「0」 레이어와 ByBlock 색을 블록 참조에게서 받는다.
    fn style(
        &mut self,
        layer: &str,
        color: &Color,
        inherit: Option<Inherit>,
    ) -> (u32, Option<[u8; 3]>) {
        let layer = match inherit {
            Some(h) if layer == "0" => h.layer,
            _ => self.layer(layer),
        };
        let color = match (color, inherit) {
            (Color::ByBlock, Some(h)) => h.color,
            (c, _) => c.rgb().map(|(r, g, b)| [r, g, b]),
        };
        (layer, color)
    }

    fn push(&mut self, layer: u32, color: Option<[u8; 3]>, shape: Shape, t: &Affine) {
        let shape = if *t == Affine::IDENTITY {
            shape
        } else {
            shape.transformed(t)
        };
        self.d.entities.push(Entity {
            layer,
            color,
            shape,
        });
    }

    /// 객체 하나를 `t` 로 변환해 넣는다. 블록 참조는 정의를 펼친다.
    fn entity(&mut self, e: &EntityType, t: &Affine, inherit: Option<Inherit>) {
        if self.d.entities.len() >= MAX_ENTITIES {
            self.skip("개수 한도 초과");
            return;
        }
        let common = e.common();
        if common.invisible {
            return;
        }
        let (layer, color) = self.style(&common.layer, &common.color, inherit);
        let shape = match e {
            EntityType::Line(x) => Shape::Line {
                a: v2(&x.start),
                b: v2(&x.end),
            },
            EntityType::LwPolyline(x) => {
                let s = Shape::Polyline {
                    points: x
                        .vertices
                        .iter()
                        .map(|v| (Vec2::new(v.location.x, v.location.y), v.bulge))
                        .collect(),
                    closed: x.is_closed,
                };
                return self.push(layer, color, s, &ocs(&x.normal).then(t));
            }
            EntityType::Polyline2D(x) => {
                let s = Shape::Polyline {
                    points: x
                        .vertices
                        .iter()
                        .map(|v| (v2(&v.location), v.bulge))
                        .collect(),
                    closed: x.is_closed(),
                };
                return self.push(layer, color, s, &ocs(&x.normal).then(t));
            }
            EntityType::Circle(x) => {
                let s = Shape::Circle {
                    center: v2(&x.center),
                    radius: x.radius,
                };
                return self.push(layer, color, s, &ocs(&x.normal).then(t));
            }
            EntityType::Arc(x) => {
                let s = Shape::Arc {
                    center: v2(&x.center),
                    radius: x.radius,
                    start: x.start_angle,
                    end: x.end_angle,
                };
                return self.push(layer, color, s, &ocs(&x.normal).then(t));
            }
            EntityType::Ellipse(x) => {
                // 타원은 세계 좌표. 짧은 축 = 돌출 방향 × 긴 축 × 비율
                let u = v2(&x.major_axis);
                let k = if x.normal.z < 0.0 { -1.0 } else { 1.0 };
                let v = Vec2::new(-u.y * k * x.minor_axis_ratio, u.x * k * x.minor_axis_ratio);
                Shape::Ellipse {
                    center: v2(&x.center),
                    u,
                    v,
                    start: x.start_parameter,
                    end: x.end_parameter,
                }
            }
            EntityType::Text(x) => {
                let s = Shape::Text {
                    at: v2(&x.insertion_point),
                    height: x.height,
                    rotation: x.rotation,
                    value: x.value.clone(),
                };
                return self.push(layer, color, s, &ocs(&x.normal).then(t));
            }
            EntityType::MText(x) => Shape::Text {
                at: v2(&x.insertion_point),
                height: x.height,
                rotation: x.rotation,
                value: x.value.clone(),
            },
            EntityType::Insert(x) => return self.insert(x, layer, color, t),
            other => {
                self.skip(other.as_entity().entity_type());
                return;
            }
        };
        self.push(layer, color, shape, t);
    }

    /// 블록 참조 펼치기:
    /// 블록 안 좌표 → (−기준점) → 축척 → 회전 → 삽입점(배열이면 칸마다) → 돌출 방향(OCS) → 바깥 변환 `t`
    fn insert(
        &mut self,
        x: &acadrust::entities::Insert,
        layer: u32,
        color: Option<[u8; 3]>,
        t: &Affine,
    ) {
        let doc = self.doc;
        let Some(block) = doc.block_records.get(&x.block_name) else {
            self.skip("블록 정의 없음");
            return;
        };
        if self.stack.len() >= MAX_BLOCK_DEPTH || self.stack.iter().any(|n| n == &x.block_name) {
            self.skip("블록 순환·깊이 초과");
            return;
        }
        let base = &block.base_point;
        let local = Affine::translate(-base.x, -base.y)
            .then(&Affine::scale(x.x_scale(), x.y_scale()))
            .then(&Affine::rotate(x.rotation));
        let outer = ocs(&x.normal).then(t);
        let points = if x.is_array() {
            x.array_points()
        } else {
            vec![x.insert_point]
        };
        if points.is_empty() {
            self.skip("배열 블록 한도 초과");
        }
        let inherit = Some(Inherit { layer, color });
        self.stack.push(x.block_name.clone());
        for p in points {
            self.d.inserts += 1;
            let place = local.then(&Affine::translate(p.x, p.y)).then(&outer);
            for e in doc.entities_in_block(&x.block_name) {
                self.entity(e, &place, inherit);
            }
        }
        self.stack.pop();

        // 속성값(ATTRIB)은 이미 세계 좌표 — 블록 참조의 바깥 변환만 건다
        for a in &x.attributes {
            if a.flags.invisible || a.common.invisible || a.value.is_empty() {
                continue;
            }
            let (al, ac) = self.style(&a.common.layer, &a.common.color, inherit);
            let s = Shape::Text {
                at: v2(&a.insertion_point),
                height: a.height,
                rotation: a.rotation,
                value: a.value.clone(),
            };
            self.push(al, ac, s, t);
        }
    }
}

/// acadrust 문서를 도면 모델로 바꾼다 (모형 공간, 블록 참조는 펼침).
pub fn from_document(doc: &CadDocument) -> Drawing {
    let mut c = Converter {
        doc,
        d: Drawing {
            code_page: doc.header.code_page.clone(),
            ..Default::default()
        },
        layer_ids: HashMap::new(),
        stack: Vec::new(),
    };
    for l in doc.layers.iter() {
        let i = c.layer(&l.name) as usize;
        let layer = &mut c.d.layers[i];
        if let Some((r, g, b)) = l.color.rgb() {
            layer.color = [r, g, b];
        }
        layer.visible = !l.flags.frozen && !l.is_off();
    }
    for e in doc.model_space_entities() {
        c.entity(e, &Affine::IDENTITY, None);
    }
    c.d
}

#[cfg(test)]
mod tests {
    use super::*;
    use acadrust::entities::{Arc, Circle, Insert, Line};
    use acadrust::tables::BlockRecord;

    /// 블록 정의를 만든다: 기준점 `base`, 안에 `items` 객체들
    fn define(doc: &mut CadDocument, name: &str, base: Vector3, items: Vec<EntityType>) {
        let mut br = BlockRecord::new(name);
        br.handle = doc.allocate_handle();
        br.base_point = base;
        let owner = br.handle;
        doc.block_records.add(br).unwrap();
        for mut e in items {
            e.common_mut().owner_handle = owner;
            doc.add_entity(e).unwrap();
        }
    }

    fn on(mut e: EntityType, layer: &str, color: Color) -> EntityType {
        e.common_mut().layer = layer.to_string();
        e.common_mut().color = color;
        e
    }

    fn close(a: Vec2, b: (f64, f64)) -> bool {
        (a.x - b.0).abs() < 1e-9 && (a.y - b.1).abs() < 1e-9
    }

    fn line_of(d: &Drawing, i: usize) -> (Vec2, Vec2) {
        match &d.entities[i].shape {
            Shape::Line { a, b } => (*a, *b),
            s => panic!("선이 아님: {s:?}"),
        }
    }

    #[test]
    fn block_definitions_are_not_drawn_in_model_space() {
        let mut doc = CadDocument::new();
        define(
            &mut doc,
            "B",
            Vector3::ZERO,
            vec![EntityType::Line(Line::from_coords(
                0.0, 0.0, 0.0, 1.0, 0.0, 0.0,
            ))],
        );
        let d = from_document(&doc);
        assert!(
            d.entities.is_empty(),
            "정의만 있고 참조가 없으면 아무것도 안 그린다"
        );
    }

    #[test]
    fn insert_applies_base_scale_rotation_position() {
        let mut doc = CadDocument::new();
        // 기준점 (1,0) — 블록 안 (1,0)→(2,0) 선은 기준점 기준 (0,0)→(1,0)
        define(
            &mut doc,
            "B",
            Vector3::new(1.0, 0.0, 0.0),
            vec![EntityType::Line(Line::from_coords(
                1.0, 0.0, 0.0, 2.0, 0.0, 0.0,
            ))],
        );
        let ins = Insert::new("B", Vector3::new(10.0, 5.0, 0.0))
            .with_scale(2.0, 2.0, 1.0)
            .with_rotation(std::f64::consts::FRAC_PI_2);
        doc.add_entity(EntityType::Insert(ins)).unwrap();
        let d = from_document(&doc);
        assert_eq!(d.entities.len(), 1);
        let (a, b) = line_of(&d, 0);
        // 길이 1 → 2 로 늘고 90° 돌아 위쪽으로
        assert!(
            close(a, (10.0, 5.0)) && close(b, (10.0, 7.0)),
            "{a:?} {b:?}"
        );
    }

    #[test]
    fn layer_zero_and_byblock_inherit_from_insert() {
        let mut doc = CadDocument::new();
        define(
            &mut doc,
            "B",
            Vector3::ZERO,
            vec![
                on(
                    EntityType::Line(Line::from_coords(0.0, 0.0, 0.0, 1.0, 0.0, 0.0)),
                    "0",
                    Color::ByBlock,
                ),
                on(
                    EntityType::Line(Line::from_coords(0.0, 0.0, 0.0, 0.0, 1.0, 0.0)),
                    "A-FIXED",
                    Color::Index(3),
                ),
            ],
        );
        let ins = on(
            EntityType::Insert(Insert::new("B", Vector3::ZERO)),
            "A-DOOR",
            Color::Index(1),
        );
        doc.add_entity(ins).unwrap();
        let d = from_document(&doc);
        let name = |e: &Entity| d.layers[e.layer as usize].name.clone();
        assert_eq!(name(&d.entities[0]), "A-DOOR");
        assert_eq!(
            d.entities[0].color,
            Some([255, 0, 0]),
            "ByBlock → 참조의 빨강"
        );
        assert_eq!(name(&d.entities[1]), "A-FIXED");
        assert_eq!(d.entities[1].color, Some([0, 255, 0]));
    }

    #[test]
    fn mirrored_insert_keeps_circles_and_flips_arcs() {
        let mut doc = CadDocument::new();
        // 0°→90° 호 (오른쪽 위 사분원)
        let mut arc = Arc::new();
        arc.radius = 1.0;
        arc.start_angle = 0.0;
        arc.end_angle = std::f64::consts::FRAC_PI_2;
        define(
            &mut doc,
            "B",
            Vector3::ZERO,
            vec![
                EntityType::Circle(Circle::from_coords(3.0, 0.0, 0.0, 1.0)),
                EntityType::Arc(arc),
            ],
        );
        doc.add_entity(EntityType::Insert(
            Insert::new("B", Vector3::ZERO).with_scale(-1.0, 1.0, 1.0),
        ))
        .unwrap();
        let d = from_document(&doc);
        // 원은 x 가 뒤집혀 (−3,0) 에 그대로 원, 호도 호로 남는다 (타원이 되면 안 됨)
        match &d.entities[0].shape {
            Shape::Circle { center, radius } => {
                assert!(close(*center, (-3.0, 0.0)) && *radius == 1.0)
            }
            s => panic!("{s:?}"),
        }
        assert!(
            matches!(d.entities[1].shape, Shape::Arc { .. }),
            "{:?}",
            d.entities[1].shape
        );
        // 호는 왼쪽 위 사분원이 되어야 한다: 중간점이 (−0.707, 0.707)
        let s = crate::display::segments(&d);
        let o = s.origin;
        let mid_ok = s.coords.chunks(2).any(|q| {
            let p = Vec2::new(q[0] as f64 + o.x, q[1] as f64 + o.y);
            close(
                Vec2::new((p.x * 1e3).round() / 1e3, (p.y * 1e3).round() / 1e3),
                (-0.707, 0.707),
            )
        });
        assert!(mid_ok, "대칭한 호가 왼쪽 위로 가야 함");
    }

    #[test]
    fn non_uniform_scale_turns_circle_into_ellipse() {
        let mut doc = CadDocument::new();
        define(
            &mut doc,
            "B",
            Vector3::ZERO,
            vec![EntityType::Circle(Circle::from_coords(0.0, 0.0, 0.0, 1.0))],
        );
        doc.add_entity(EntityType::Insert(
            Insert::new("B", Vector3::ZERO).with_scale(2.0, 1.0, 1.0),
        ))
        .unwrap();
        let d = from_document(&doc);
        let b = d.bbox();
        assert!(
            (b.width() - 4.0).abs() < 1e-9 && (b.height() - 2.0).abs() < 1e-9,
            "{b:?}"
        );
    }

    #[test]
    fn nested_and_array_and_cycle() {
        let mut doc = CadDocument::new();
        define(
            &mut doc,
            "LEAF",
            Vector3::ZERO,
            vec![EntityType::Line(Line::from_coords(
                0.0, 0.0, 0.0, 1.0, 0.0, 0.0,
            ))],
        );
        define(
            &mut doc,
            "MID",
            Vector3::ZERO,
            vec![
                EntityType::Insert(Insert::new("LEAF", Vector3::new(0.0, 10.0, 0.0))),
                // 자기 자신을 부르는 깨진 정의
                EntityType::Insert(Insert::new("MID", Vector3::ZERO)),
            ],
        );
        // 2 행 × 3 열 배열, 간격 100
        let ins = Insert::new("MID", Vector3::new(1000.0, 0.0, 0.0)).with_array(3, 2, 100.0, 100.0);
        doc.add_entity(EntityType::Insert(ins)).unwrap();
        let d = from_document(&doc);
        assert_eq!(d.entities.len(), 6, "배열 6 칸 × 선 1");
        assert!(d.skipped.contains_key("블록 순환·깊이 초과"));
        let firsts: Vec<_> = (0..6).map(|i| line_of(&d, i).0).collect();
        assert!(
            firsts.iter().any(|p| close(*p, (1200.0, 110.0))),
            "{firsts:?}"
        );
    }
}
