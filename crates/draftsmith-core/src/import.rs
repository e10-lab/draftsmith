//! DXF·DWG 가져오기 (acadrust → 도면 모델).
//!
//! 모형 공간 객체를 읽고, 블록 참조(INSERT)는 블록 정의를 변환해 펼친다 (구현 노트 §5).
//! 가져오는 모양: 선·폴리선(LW·2D)·원·호·타원·스플라인·솔리드·해치(채움·무늬)·치수(치수 블록)·
//! 한 줄 문자·여러 줄 문자·블록 속성값. 그 밖의 객체는 [`Drawing::skipped`] 에 개수만 남긴다.

use std::collections::HashMap;
use std::path::Path;

use acadrust::types::{Color, Vector3};
use acadrust::{CadDocument, DwgReader, DxfReader, EntityType};
use thiserror::Error;

use crate::curve;
use crate::drawing::{Drawing, Entity, LinePattern, LineStyle, Shape, TextShape, VAlign};
use crate::fill::PatternLine;
use crate::geom::{Affine, Bbox, Vec2};
use crate::layout::{Layout, ViewportView};
use crate::text::{plain_mtext, plain_text};

/// 해치 하나에서 만들 무늬 선분의 최대 수. 넘으면 무늬 대신 경계선만 그린다.
pub const MAX_PATTERN_SEGMENTS: usize = 200_000;

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

/// DXF 또는 DWG 파일을 읽는다. 엄격하게 읽다가 실패하면 복구 모드(깨진 객체는 건너뜀)로 다시 읽고,
/// 그때는 [`Drawing::recovered`] 를 켠다 (구현 노트 §14).
pub fn read_file(path: impl AsRef<Path>) -> Result<Drawing, ImportError> {
    let path = path.as_ref();
    if crate::docfile::is_dsmith(path) {
        return crate::docfile::load(path).map_err(ImportError::Cad);
    }
    // 연속지적도 SHP (§58)
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("shp"))
    {
        return crate::shp::read(path).map_err(ImportError::Cad);
    }
    // IFC 는 건축 객체로 (§47)
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("ifc"))
    {
        let bytes = std::fs::read(path).map_err(|e| ImportError::Cad(e.to_string()))?;
        return crate::ifc_read::read(&String::from_utf8_lossy(&bytes)).map_err(ImportError::Cad);
    }
    let dwg = is_dwg(path)?;
    let strict = if dwg {
        DwgReader::from_file(path).and_then(|mut r| r.read())
    } else {
        DxfReader::from_file(path).and_then(|r| r.with_configuration(dxf_config(false)).read())
    };
    match strict {
        Ok(doc) => Ok(from_document(&doc)),
        Err(first) => {
            let recovered = if dwg {
                DwgReader::from_file(path).and_then(|mut r| r.recover())
            } else {
                DxfReader::from_file(path)
                    .and_then(|r| r.with_configuration(dxf_config(true)).recover())
            };
            match recovered {
                Ok(outcome) => {
                    let mut d = from_document(&outcome.document);
                    d.recovered = Some(first.to_string());
                    Ok(d)
                }
                // 복구도 안 되면 처음 오류를 알린다 (더 알아보기 쉬움)
                Err(_) => Err(ImportError::Cad(first.to_string())),
            }
        }
    }
}

/// DXF 읽기 설정: 문자 코드가 적히지 않은 옛 DXF(2007 이전)는 한글(CP949)로 본다
fn dxf_config(failsafe: bool) -> acadrust::DxfReaderConfiguration {
    acadrust::DxfReaderConfiguration {
        failsafe,
        default_encoding: Some("ANSI_949".into()),
    }
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
    /// 블록 참조의 선 종류 — 블록 안 ByBlock 선 종류 객체가 이것으로 (배율에 블록 축척은 아직 안 곱함)
    linetype: Option<LineStyle>,
    /// 블록 참조의 선 굵기 — 블록 안 ByBlock 선 굵기 객체가 이것으로
    weight: Option<u16>,
    /// 블록 참조의 색 번호 — 블록 안 ByBlock 색 객체가 이것으로
    aci: Option<u8>,
}

struct Converter<'a> {
    doc: &'a CadDocument,
    d: Drawing,
    layer_ids: HashMap<String, u32>,
    /// 지금 펼치고 있는 블록 이름들 (자기 자신을 부르는 정의 막기)
    stack: Vec<String>,
    /// 선 종류 이름(소문자) → [`Drawing::linetypes`] 번호 (실선은 없음)
    lt_ids: HashMap<String, u16>,
    /// 레이어 번호 → 레이어 선 종류
    layer_lt: Vec<Option<u16>>,
    /// 전역 선 종류 배율 (LTSCALE)
    ltscale: f64,
    /// 지금 넣는 객체의 선 종류 (`style` 이 정하고 `push` 가 쓴다)
    cur_lt: Option<LineStyle>,
    /// 레이어 번호 → 레이어 선 굵기 (1/100 mm)
    layer_w: Vec<Option<u16>>,
    /// 지금 넣는 객체의 선 굵기
    cur_w: Option<u16>,
    /// 레이어 번호 → 레이어 색 번호
    layer_aci: Vec<Option<u8>>,
    /// 지금 넣는 객체의 색 번호
    cur_aci: Option<u8>,
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

    /// 객체의 레이어·색·선 종류. 블록 안이면 「0」 레이어와 ByBlock 색·선 종류를 블록 참조에게서 받는다.
    /// 선 종류는 `cur_lt` 에 넣는다 (블록 축척은 `push` 에서 곱함).
    fn style(
        &mut self,
        common: &acadrust::entities::EntityCommon,
        inherit: Option<Inherit>,
    ) -> (u32, Option<[u8; 3]>) {
        let layer = match inherit {
            Some(h) if common.layer == "0" => h.layer,
            _ => self.layer(&common.layer),
        };
        let color = match (&common.color, inherit) {
            (Color::ByBlock, Some(h)) => h.color,
            (c, _) => c.rgb().map(|(r, g, b)| [r, g, b]),
        };
        self.cur_aci = match (&common.color, inherit) {
            (Color::ByBlock, Some(h)) => h.aci,
            (Color::ByLayer, _) => self.layer_aci.get(layer as usize).copied().flatten(),
            (Color::Index(i), _) => Some(*i),
            _ => None,
        };
        let own = if common.linetype_scale > 0.0 {
            common.linetype_scale
        } else {
            1.0
        };
        let name = common.linetype.to_ascii_lowercase();
        self.cur_lt = match name.as_str() {
            "" | "bylayer" => self
                .layer_lt
                .get(layer as usize)
                .copied()
                .flatten()
                .map(|pattern| LineStyle {
                    pattern,
                    scale: self.ltscale * own,
                }),
            "byblock" => inherit.and_then(|h| h.linetype).map(|l| LineStyle {
                scale: l.scale * own,
                ..l
            }),
            n => self.lt_ids.get(n).map(|&pattern| LineStyle {
                pattern,
                scale: self.ltscale * own,
            }),
        };
        self.cur_w = match common.line_weight {
            acadrust::types::LineWeight::ByLayer => {
                self.layer_w.get(layer as usize).copied().flatten()
            }
            acadrust::types::LineWeight::ByBlock => inherit.and_then(|h| h.weight),
            acadrust::types::LineWeight::Default => None,
            acadrust::types::LineWeight::Value(v) => (v >= 0).then_some(v as u16),
        };
        (layer, color)
    }

    fn push(&mut self, layer: u32, color: Option<[u8; 3]>, shape: Shape, t: &Affine) {
        // 선 종류는 선·곡선에만. 블록 안이면 블록 축척만큼 무늬도 커진다
        let linetype = match shape {
            Shape::Fill { .. } | Shape::Segments { .. } | Shape::Text(_) => None,
            _ => self.cur_lt.map(|l| LineStyle {
                scale: l.scale * t.scale_x().max(t.det().abs().sqrt()),
                ..l
            }),
        };
        let shape = if *t == Affine::IDENTITY {
            shape
        } else {
            shape.transformed(t)
        };
        let weight = match shape {
            Shape::Fill { .. } | Shape::Text(_) => None,
            _ => self.cur_w,
        };
        self.d.entities.push(Entity {
            layer,
            aci: self.cur_aci,
            weight,
            linetype,
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
        let (layer, color) = self.style(common, inherit);
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
                self.font_use(&x.style);
                return self.push(layer, color, text_shape(x), &ocs(&x.normal).then(t));
            }
            EntityType::MText(x) => {
                self.font_use(&x.style);
                // 붙는 점 1–9: (위·가운데·아래) × (왼·가운데·오른)
                let k = (x.attachment_point as i32 - 1).clamp(0, 8);
                let valign = match k / 3 {
                    0 => VAlign::Top,
                    1 => VAlign::Middle,
                    _ => VAlign::Bottom,
                };
                Shape::Text(TextShape {
                    style: x.style.clone(),
                    at: v2(&x.insertion_point),
                    height: x.height,
                    rotation: x.rotation,
                    width_factor: 1.0,
                    halign: (k % 3) as f64 / 2.0,
                    valign,
                    value: plain_mtext(&x.value),
                })
            }
            EntityType::Solid(x) => {
                // SOLID 꼭짓점 순서는 1-2-4-3 (3·4 가 같으면 삼각형)
                let mut ring = vec![
                    v2(&x.first_corner),
                    v2(&x.second_corner),
                    v2(&x.fourth_corner),
                    v2(&x.third_corner),
                ];
                ring.dedup();
                if ring.len() > 1 && ring.first() == ring.last() {
                    ring.pop();
                }
                let s = Shape::Fill { loops: vec![ring] };
                return self.push(layer, color, s, &ocs(&x.normal).then(t));
            }
            EntityType::Spline(x) => {
                let pts = spline_entity_points(x);
                if pts.len() < 2 {
                    self.skip("SPLINE(빈 것)");
                    return;
                }
                Shape::Polyline {
                    points: pts.into_iter().map(|p| (p, 0.0)).collect(),
                    closed: x.flags.closed,
                }
            }
            EntityType::Hatch(x) => return self.hatch(x, layer, color, t),
            EntityType::Dimension(x) => {
                // 치수는 그려 둔 익명 블록(*D…)을 펼친다. 블록 안 좌표는 이미 세계 좌표.
                let name = x.base().block_name.clone();
                if name.is_empty() || self.doc.block_records.get(&name).is_none() {
                    // 블록이 없는 파일(다른 프로그램이 쓴 것 등): 선형·평행 치수는 간단히 그린다
                    match simple_dimension(x) {
                        Some(shapes) => {
                            for s in shapes {
                                self.push(layer, color, s, t);
                            }
                        }
                        None => self.skip("DIMENSION(치수 블록 없음)"),
                    }
                    return;
                }
                let linetype = self.cur_lt;
                let weight = self.cur_w;
                let aci = self.cur_aci;
                self.block(
                    &name,
                    &[Affine::IDENTITY.then(t)],
                    Inherit {
                        layer,
                        color,
                        linetype,
                        weight,
                        aci,
                    },
                );
                return;
            }
            EntityType::Insert(x) => return self.insert(x, layer, color, t),
            // 블록 안 속성 정의는 그리지 않는다 (AutoCAD 와 같게) — 블록 참조로 살리면 속성 정의로 남는다 (§49)
            EntityType::AttributeDefinition(_) if !self.stack.is_empty() => return,
            other => {
                self.skip(other.as_entity().entity_type());
                return;
            }
        };
        self.push(layer, color, shape, t);
    }

    /// 글자 스타일 쓰임 세기 (§57)
    fn font_use(&mut self, style: &str) {
        let key = if style.is_empty() { "Standard" } else { style };
        match self
            .d
            .fonts
            .iter_mut()
            .find(|f| f.style.eq_ignore_ascii_case(key))
        {
            Some(f) => f.count += 1,
            None => {
                let st = self.doc.text_styles.get(key).or_else(|| {
                    self.doc
                        .text_styles
                        .iter()
                        .find(|s| s.name.eq_ignore_ascii_case(key))
                });
                let font = st
                    .map(|s| {
                        if s.true_type_font.is_empty() {
                            s.font_file.clone()
                        } else {
                            s.true_type_font.clone()
                        }
                    })
                    .unwrap_or_default();
                self.d.fonts.push(crate::drawing::FontUse {
                    style: key.to_string(),
                    font,
                    big_font: st.map(|s| s.big_font_file.clone()).unwrap_or_default(),
                    count: 1,
                });
            }
        }
    }

    /// 블록 참조 펼치기:
    /// 블록 안 좌표 → (−기준점) → 축척 → 회전 → 삽입점(배열이면 칸마다) → 돌출 방향(OCS) → 바깥 변환 `t`
    /// 블록 정의를 `places` 의 변환마다 펼친다 (블록 참조·치수 공용).
    fn block(&mut self, name: &str, places: &[Affine], inherit: Inherit) {
        if self.stack.len() >= MAX_BLOCK_DEPTH || self.stack.iter().any(|n| n == name) {
            self.skip("블록 순환·깊이 초과");
            return;
        }
        let doc = self.doc;
        self.stack.push(name.to_string());
        for place in places {
            for e in doc.entities_in_block(name) {
                self.entity(e, place, Some(inherit));
            }
        }
        self.stack.pop();
    }

    /// 해치: 경계 고리를 만들고, 솔리드·그러데이션이면 채움, 무늬면 경계 안 무늬 선분.
    fn hatch(
        &mut self,
        h: &acadrust::entities::Hatch,
        layer: u32,
        color: Option<[u8; 3]>,
        t: &Affine,
    ) {
        let loops = hatch_loops(h);
        if loops.is_empty() {
            self.skip("HATCH(경계 없음)");
            return;
        }
        let place = ocs(&h.normal).then(t);
        let lines: Vec<PatternLine> = h
            .pattern
            .lines
            .iter()
            .map(|l| PatternLine {
                angle: l.angle,
                base: Vec2::new(l.base_point.x, l.base_point.y),
                offset: Vec2::new(l.offset.x, l.offset.y),
                dashes: l.dash_lengths.clone(),
            })
            .collect();
        // 무늬 해치는 무늬 정의째 (§45) — 화면에는 무늬 선분이 나온다
        let shape = if h.is_solid || lines.is_empty() {
            Shape::Fill { loops }
        } else {
            Shape::Hatch {
                loops,
                lines,
                name: h.pattern.name.clone(),
            }
        };
        self.push(layer, color, shape, &place);
    }

    /// 모형 공간의 선형(회전)·정렬 치수를 치수 객체(`arch::Dim`)로 살린다. 평면(+Z)일 때만. 살렸으면 참.
    /// 글자 높이는 치수 블록(*D) 안 글자에서, 없으면 머리글의 DIMTXT × DIMSCALE
    fn keep_dimension(&mut self, x: &acadrust::entities::Dimension) -> bool {
        use acadrust::entities::Dimension as D;
        let b = x.base();
        if b.normal.x.abs() > 1e-9 || b.normal.y.abs() > 1e-9 || b.normal.z <= 0.0 {
            return false;
        }
        // 반지름·지름·각도: (종류, a, b, 꼭짓점, 호 반지름) (§45)
        use crate::arch::DimKind as K;
        let special: Option<(K, Vec2, Vec2, Option<Vec2>, f64)> = match x {
            D::Radius(r) => Some((
                K::Radius,
                v2(&r.angle_vertex),
                v2(&r.definition_point),
                None,
                0.0,
            )),
            D::Diameter(r) => {
                let (p, q) = (v2(&r.angle_vertex), v2(&r.definition_point));
                Some((K::Diameter, p.lerp(q, 0.5), q, None, 0.0))
            }
            D::Angular3Pt(g) => {
                let v = v2(&g.angle_vertex);
                let arc = if g.definition_point.x != 0.0 || g.definition_point.y != 0.0 {
                    v2(&g.definition_point)
                } else {
                    v2(&b.text_middle_point)
                };
                Some((
                    K::Angle,
                    v2(&g.first_point),
                    v2(&g.second_point),
                    Some(v),
                    v.dist(arc),
                ))
            }
            D::Angular2Ln(g) => {
                // 두 선 (first→second, vertex→definition) 의 교점이 꼭짓점, 각 선에서 꼭짓점에서 먼 끝이 반직선 위 점
                let (a0, a1, b0, b1) = (
                    v2(&g.first_point),
                    v2(&g.second_point),
                    v2(&g.angle_vertex),
                    v2(&g.definition_point),
                );
                let (r, t) = (
                    Vec2::new(a1.x - a0.x, a1.y - a0.y),
                    Vec2::new(b1.x - b0.x, b1.y - b0.y),
                );
                let den = r.x * t.y - r.y * t.x;
                if den.abs() < 1e-12 {
                    return false;
                }
                let k = ((b0.x - a0.x) * t.y - (b0.y - a0.y) * t.x) / den;
                let v = Vec2::new(a0.x + r.x * k, a0.y + r.y * k);
                let far = |p: Vec2, q: Vec2| if v.dist(p) >= v.dist(q) { p } else { q };
                Some((
                    K::Angle,
                    far(a0, a1),
                    far(b0, b1),
                    Some(v),
                    v.dist(v2(&g.dimension_arc)),
                ))
            }
            _ => None,
        };
        if let Some((kind, a, bb, vertex, rad)) = special {
            let text_height = self.dim_text_height(&b.block_name);
            let text = match b.user_text.as_deref() {
                Some(t) if !t.trim().is_empty() && t.trim() != "<>" => crate::text::plain_mtext(t),
                _ => String::new(),
            };
            let _ = self.layer(&b.common.layer);
            let m = &mut self.d.arch;
            let level = m.ensure_level();
            let id = m.new_id();
            m.dims.push(crate::arch::Dim {
                id,
                level,
                a,
                b: bb,
                vertex,
                offset: rad,
                kind,
                text_height,
                text,
                layer: b.common.layer.clone(),
                arrow: true,
                plain: true,
                precision: self.doc.header.dim_decimal_places.clamp(0, 4) as u8,
                ..Default::default()
            });
            return true;
        }
        let (p1, p2, dp, rotation) = match x {
            D::Linear(l) => (
                v2(&l.first_point),
                v2(&l.second_point),
                v2(&l.definition_point),
                Some(l.rotation),
            ),
            D::Aligned(a) => (
                v2(&a.first_point),
                v2(&a.second_point),
                v2(&a.definition_point),
                None,
            ),
            _ => return false,
        };
        let u = match rotation {
            Some(r) => Vec2::new(r.cos(), r.sin()),
            None => {
                let l = p1.dist(p2);
                if l < 1e-12 {
                    return false;
                }
                Vec2::new((p2.x - p1.x) / l, (p2.y - p1.y) / l)
            }
        };
        if ((p2.x - p1.x) * u.x + (p2.y - p1.y) * u.y).abs() < 1e-12 {
            return false;
        }
        let n = Vec2::new(-u.y, u.x);
        let offset = (dp.x - p1.x) * n.x + (dp.y - p1.y) * n.y;
        let doc = self.doc;
        let from_block = doc.block_records.get(&b.block_name).and_then(|_| {
            doc.entities_in_block(&b.block_name).find_map(|e| match e {
                EntityType::Text(t) => Some(t.height),
                EntityType::MText(t) => Some(t.height),
                _ => None,
            })
        });
        // 원래 치수 끝이 화살표(SOLID·채움)였나
        let arrow = doc.block_records.get(&b.block_name).is_some()
            && doc
                .entities_in_block(&b.block_name)
                .any(|e| matches!(e, EntityType::Solid(_) | EntityType::Hatch(_)));
        let header = doc.header.dim_text_height
            * if doc.header.dim_scale > 0.0 {
                doc.header.dim_scale
            } else {
                1.0
            };
        let text_height =
            from_block
                .filter(|h| *h > 0.0)
                .unwrap_or(if header > 0.0 { header } else { 250.0 });
        let text = match b.user_text.as_deref() {
            Some(t) if !t.trim().is_empty() && t.trim() != "<>" => crate::text::plain_mtext(t),
            _ => String::new(),
        };
        // 레이어 표에 있게
        let _ = self.layer(&b.common.layer);
        let m = &mut self.d.arch;
        let level = m.ensure_level();
        let id = m.new_id();
        m.dims.push(crate::arch::Dim {
            id,
            level,
            a: p1,
            b: p2,
            offset,
            text_height,
            text,
            ka: None,
            kb: None,
            rotation,
            layer: b.common.layer.clone(),
            // 원래 모양을 따라: 화살표면 화살표, 값은 쉼표 없이 (AutoCAD 기본 단위 형식)
            arrow,
            plain: true,
            // 소수 자리는 머리글 DIMDEC (선형 치수, 0~4)
            precision: doc.header.dim_decimal_places.clamp(0, 4) as u8,
            ..Default::default()
        });
        true
    }

    /// 치수 글자 높이: 치수 블록(*D) 안 글자에서, 없으면 DIMTXT × DIMSCALE, 그것도 없으면 250
    fn dim_text_height(&self, block: &str) -> f64 {
        let doc = self.doc;
        let from_block = doc.block_records.get(block).and_then(|_| {
            doc.entities_in_block(block).find_map(|e| match e {
                EntityType::Text(t) => Some(t.height),
                EntityType::MText(t) => Some(t.height),
                _ => None,
            })
        });
        let header = doc.header.dim_text_height
            * if doc.header.dim_scale > 0.0 {
                doc.header.dim_scale
            } else {
                1.0
            };
        from_block
            .filter(|h| *h > 0.0)
            .unwrap_or(if header > 0.0 { header } else { 250.0 })
    }

    /// 모형 공간의 블록 참조를 `Shape::Insert` 로 살린다: 평면(+Z)이고 보이는 속성값이 없을 때 (배열은 칸마다 하나씩).
    /// 블록 정의는 처음 만날 때 기준점이 원점이 되게 펼쳐 `Drawing::blocks` 에 넣는다 (안의 블록은 펼친 채로).
    /// 살렸으면 참
    fn keep_insert(&mut self, x: &acadrust::entities::Insert) -> bool {
        let n = &x.normal;
        if n.x.abs() > 1e-9 || n.y.abs() > 1e-9 || n.z <= 0.0 {
            return false;
        }
        let shown: Vec<_> = x
            .attributes
            .iter()
            .filter(|a| !a.flags.invisible && !a.common.invisible && !a.value.is_empty())
            .collect();
        // 속성값이 있는 배열 블록은 펼친다 (속성은 첫 칸에만 붙으므로)
        if !shown.is_empty() && x.is_array() {
            return false;
        }
        let scale = Vec2::new(x.x_scale(), x.y_scale());
        // 속성값 → 블록 안 좌표 (§46)
        let local = crate::drawing::Block::transform(
            Vec2::new(x.insert_point.x, x.insert_point.y),
            scale,
            x.rotation,
        )
        .inverse();
        let mut attrs = Vec::new();
        for a in shown {
            let Some(inv) = local else { return false };
            if let Shape::Text(t) = Shape::Text(attrib_text(a)).transformed(&inv) {
                attrs.push(crate::drawing::Attr {
                    tag: a.tag.clone(),
                    text: t,
                });
            }
        }
        let Some(block) = self.doc.block_records.get(&x.block_name) else {
            return false;
        };
        if !self.d.blocks.iter().any(|b| b.name == x.block_name) {
            let base = block.base_point;
            let zero = self.layer("0");
            let saved = std::mem::take(&mut self.d.entities);
            let saved_inserts = self.d.inserts;
            self.block(
                &x.block_name,
                &[Affine::translate(-base.x, -base.y)],
                Inherit {
                    layer: zero,
                    color: None,
                    linetype: None,
                    weight: None,
                    aci: None,
                },
            );
            let entities = std::mem::replace(&mut self.d.entities, saved);
            self.d.inserts = saved_inserts;
            // 속성 정의 (보이는 것, 기준점이 원점이 되게)
            let to_local = Affine::translate(-base.x, -base.y);
            let attdefs = doc_attdefs(self.doc, &x.block_name)
                .filter_map(
                    |a| match Shape::Text(attdef_text(a)).transformed(&to_local) {
                        Shape::Text(text) => Some(crate::drawing::AttDef {
                            tag: a.tag.clone(),
                            prompt: a.prompt.clone(),
                            text,
                        }),
                        _ => None,
                    },
                )
                .collect();
            self.d.blocks.push(crate::drawing::Block {
                name: x.block_name.clone(),
                entities,
                attdefs,
            });
        }
        let (layer, color) = self.style(&x.common, None);
        // 배열 블록(MINSERT)은 칸마다 블록 참조 하나씩 (§43)
        let points = if x.is_array() {
            x.array_points()
        } else {
            vec![x.insert_point]
        };
        if points.is_empty() {
            return false;
        }
        for p in points {
            self.d.inserts += 1;
            self.d.entities.push(Entity {
                layer,
                aci: self.cur_aci,
                weight: self.cur_w,
                linetype: None,
                color,
                shape: Shape::Insert {
                    block: x.block_name.clone(),
                    at: Vec2::new(p.x, p.y),
                    scale,
                    rotation: x.rotation,
                    attrs: attrs.clone(),
                },
            });
        }
        true
    }

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
        let linetype = self.cur_lt;
        let weight = self.cur_w;
        let aci = self.cur_aci;
        let inherit = Some(Inherit {
            layer,
            color,
            linetype,
            weight,
            aci,
        });
        let places: Vec<Affine> = points
            .iter()
            .map(|p| local.then(&Affine::translate(p.x, p.y)).then(&outer))
            .collect();
        self.d.inserts += places.len();
        self.block(
            &x.block_name,
            &places,
            Inherit {
                layer,
                color,
                linetype,
                weight,
                aci,
            },
        );

        // 속성값(ATTRIB)은 이미 세계 좌표 — 블록 참조의 바깥 변환만 건다
        for a in &x.attributes {
            if a.flags.invisible || a.common.invisible || a.value.is_empty() {
                continue;
            }
            let (al, ac) = self.style(&a.common, inherit);
            let s = Shape::Text(TextShape {
                style: String::new(),
                at: v2(&a.insertion_point),
                height: a.height,
                rotation: a.rotation,
                width_factor: if a.width_factor > 0.0 {
                    a.width_factor
                } else {
                    1.0
                },
                halign: 0.0,
                valign: VAlign::Baseline,
                value: plain_text(&a.value),
            });
            self.push(al, ac, s, t);
        }
    }
}

/// TEXT 의 맞춤: 왼쪽·바탕선(또는 양쪽 맞춤·채움)이면 삽입점, 아니면 맞춤점이 기준
fn text_shape(x: &acadrust::entities::Text) -> Shape {
    use acadrust::entities::{TextHorizontalAlignment as H, TextVerticalAlignment as V};
    let (halign, mut valign) = match x.horizontal_alignment {
        H::Center => (0.5, None),
        H::Right => (1.0, None),
        H::Middle => (0.5, Some(VAlign::Middle)),
        _ => (0.0, None),
    };
    let v = match x.vertical_alignment {
        V::Bottom => VAlign::Bottom,
        V::Middle => VAlign::Middle,
        V::Top => VAlign::Top,
        _ => VAlign::Baseline,
    };
    let valign = valign.take().unwrap_or(v);
    let plain = matches!(x.horizontal_alignment, H::Left | H::Aligned | H::Fit)
        && valign == VAlign::Baseline;
    let at = if plain {
        v2(&x.insertion_point)
    } else {
        x.alignment_point
            .as_ref()
            .map(v2)
            .unwrap_or_else(|| v2(&x.insertion_point))
    };
    Shape::Text(TextShape {
        style: x.style.clone(),
        at,
        height: x.height,
        rotation: x.rotation,
        width_factor: if x.width_factor > 0.0 {
            x.width_factor
        } else {
            1.0
        },
        halign,
        valign,
        value: plain_text(&x.value),
    })
}

/// 블록 안의 보이는(상수 아닌) 속성 정의
fn doc_attdefs<'a>(
    doc: &'a CadDocument,
    block: &str,
) -> impl Iterator<Item = &'a acadrust::entities::AttributeDefinition> + 'a {
    doc.entities_in_block(block)
        .filter_map(|e| match e {
            EntityType::AttributeDefinition(a) if !a.flags.invisible && !a.flags.constant => {
                Some(a)
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .into_iter()
}

/// ATTDEF → 기본값 글자 (블록 안 좌표)
fn attdef_text(x: &acadrust::entities::AttributeDefinition) -> TextShape {
    let t = att_text(
        x.horizontal_alignment,
        x.vertical_alignment,
        &x.insertion_point,
        &x.alignment_point,
        x.height,
        x.rotation,
        x.width_factor,
        &x.default_value,
    );
    TextShape {
        style: x.text_style.clone(),
        ..t
    }
}

/// ATTRIB → 글자 (세계 좌표)
fn attrib_text(x: &acadrust::entities::AttributeEntity) -> TextShape {
    let t = att_text(
        x.horizontal_alignment,
        x.vertical_alignment,
        &x.insertion_point,
        &x.alignment_point,
        x.height,
        x.rotation,
        x.width_factor,
        &x.value,
    );
    TextShape {
        style: x.text_style.clone(),
        ..t
    }
}

#[allow(clippy::too_many_arguments)]
fn att_text(
    h: acadrust::entities::HorizontalAlignment,
    v: acadrust::entities::VerticalAlignment,
    insertion_point: &Vector3,
    alignment_point: &Vector3,
    height: f64,
    rotation: f64,
    width_factor: f64,
    value: &str,
) -> TextShape {
    use acadrust::entities::{HorizontalAlignment as H, VerticalAlignment as V};
    struct X<'a> {
        horizontal_alignment: H,
        vertical_alignment: V,
        insertion_point: &'a Vector3,
        alignment_point: &'a Vector3,
        height: f64,
        rotation: f64,
        width_factor: f64,
        value: &'a str,
    }
    let x = X {
        horizontal_alignment: h,
        vertical_alignment: v,
        insertion_point,
        alignment_point,
        height,
        rotation,
        width_factor,
        value,
    };
    let (halign, mid) = match x.horizontal_alignment {
        H::Center => (0.5, false),
        H::Right => (1.0, false),
        H::Middle => (0.5, true),
        _ => (0.0, false),
    };
    let valign = if mid {
        VAlign::Middle
    } else {
        match x.vertical_alignment {
            V::Bottom => VAlign::Bottom,
            V::Middle => VAlign::Middle,
            V::Top => VAlign::Top,
            _ => VAlign::Baseline,
        }
    };
    let plain = matches!(x.horizontal_alignment, H::Left | H::Aligned | H::Fit)
        && valign == VAlign::Baseline;
    TextShape {
        style: String::new(),
        at: v2(if plain {
            x.insertion_point
        } else {
            x.alignment_point
        }),
        height: x.height,
        rotation: x.rotation,
        width_factor: if x.width_factor > 0.0 {
            x.width_factor
        } else {
            1.0
        },
        halign,
        valign,
        value: plain_text(x.value),
    }
}

/// SPLINE 위의 점들 (조정점이 없으면 맞춤점을 잇는다)
fn spline_entity_points(x: &acadrust::entities::Spline) -> Vec<Vec2> {
    let ctrl: Vec<Vec2> = x.control_points.iter().map(v2).collect();
    if ctrl.len() >= 2 {
        let w = if x.flags.rational {
            x.weights.as_slice()
        } else {
            &[]
        };
        curve::spline_points(x.degree.max(1) as usize, &x.knots, &ctrl, w, 16)
    } else {
        x.fit_points.iter().map(v2).collect()
    }
}

/// 해치 경계 고리들 (해치 OCS 좌표). 곡선 변은 경계 크기의 1/5000 오차로 펼친다.
fn hatch_loops(h: &acadrust::entities::Hatch) -> Vec<Vec<Vec2>> {
    use acadrust::entities::BoundaryEdge as E;
    // 허용 오차를 정하려고 먼저 대략의 크기를 잰다
    let mut bb = Bbox::EMPTY;
    for path in &h.paths {
        for e in &path.edges {
            match e {
                E::Line(l) => {
                    bb.add(Vec2::new(l.start.x, l.start.y));
                    bb.add(Vec2::new(l.end.x, l.end.y));
                }
                E::CircularArc(a) => {
                    bb.add(Vec2::new(a.center.x - a.radius, a.center.y - a.radius));
                    bb.add(Vec2::new(a.center.x + a.radius, a.center.y + a.radius));
                }
                E::EllipticArc(a) => bb.add(Vec2::new(a.center.x, a.center.y)),
                E::Spline(s) => s.control_points.iter().for_each(|p| bb.add(v2(p))),
                E::Polyline(p) => p.vertices.iter().for_each(|v| bb.add(Vec2::new(v.x, v.y))),
            }
        }
    }
    let tol = (bb.width().hypot(bb.height()) / 5000.0).max(1e-9);
    let mut loops = Vec::new();
    for path in &h.paths {
        let mut ring: Vec<Vec2> = Vec::new();
        for e in &path.edges {
            let mut pts = Vec::new();
            match e {
                E::Line(l) => pts.push(Vec2::new(l.start.x, l.start.y)),
                E::CircularArc(a) => {
                    // 시계 방향 호는 각도가 거울상으로 저장된다: −시작에서 −끝까지 시계 방향
                    let c = Vec2::new(a.center.x, a.center.y);
                    let (u, v) = (Vec2::new(a.radius, 0.0), Vec2::new(0.0, a.radius));
                    let sweep = curve::ccw_sweep(a.start_angle, a.end_angle);
                    if a.counter_clockwise {
                        curve::ellipse_points(c, u, v, a.start_angle, sweep, tol, &mut pts);
                    } else {
                        curve::ellipse_points(c, u, v, -a.start_angle, -sweep, tol, &mut pts);
                    }
                    pts.pop();
                }
                E::EllipticArc(a) => {
                    let c = Vec2::new(a.center.x, a.center.y);
                    let u = Vec2::new(a.major_axis_endpoint.x, a.major_axis_endpoint.y);
                    let v = Vec2::new(-u.y * a.minor_axis_ratio, u.x * a.minor_axis_ratio);
                    let sweep = curve::ccw_sweep(a.start_angle, a.end_angle);
                    if a.counter_clockwise {
                        curve::ellipse_points(c, u, v, a.start_angle, sweep, tol, &mut pts);
                    } else {
                        curve::ellipse_points(c, u, v, -a.start_angle, -sweep, tol, &mut pts);
                    }
                    pts.pop();
                }
                E::Spline(sp) => {
                    let ctrl: Vec<Vec2> = sp.control_points.iter().map(v2).collect();
                    pts = if ctrl.len() >= 2 {
                        let w: Vec<f64> = if sp.rational {
                            sp.control_points
                                .iter()
                                .map(|p| if p.z > 0.0 { p.z } else { 1.0 })
                                .collect()
                        } else {
                            Vec::new()
                        };
                        curve::spline_points(sp.degree.max(1) as usize, &sp.knots, &ctrl, &w, 16)
                    } else {
                        sp.fit_points.iter().map(|p| Vec2::new(p.x, p.y)).collect()
                    };
                    pts.pop();
                }
                E::Polyline(p) => {
                    let v: Vec<(Vec2, f64)> = p
                        .vertices
                        .iter()
                        .map(|v| (Vec2::new(v.x, v.y), v.z))
                        .collect();
                    pts = curve::polyline_points(&v, true, tol);
                }
            }
            ring.extend(pts);
        }
        ring.dedup_by(|a, b| a.dist(*b) < 1e-12);
        if ring.len() > 1 && ring[0].dist(ring[ring.len() - 1]) < 1e-12 {
            ring.pop();
        }
        if ring.len() >= 3 {
            loops.push(ring);
        }
    }
    loops
}

/// 치수 블록이 없을 때의 대체 그림: 선형·평행 치수만. 치수선·보조선·사선 끝·치수 문자.
/// 치수 유형(글자 크기 등)은 읽지 않으므로 글자 높이는 치수 길이의 1/20 로 어림한다.
fn simple_dimension(x: &acadrust::entities::Dimension) -> Option<Vec<Shape>> {
    use acadrust::entities::Dimension as D;
    let (p1, p2, dp, dir) = match x {
        D::Linear(l) => {
            let (s, c) = l.rotation.sin_cos();
            (
                v2(&l.first_point),
                v2(&l.second_point),
                v2(&l.definition_point),
                Vec2::new(c, s),
            )
        }
        D::Aligned(a) => {
            let (p1, p2) = (v2(&a.first_point), v2(&a.second_point));
            let len = p1.dist(p2);
            if len < 1e-12 {
                return None;
            }
            (
                p1,
                p2,
                v2(&a.definition_point),
                Vec2::new((p2.x - p1.x) / len, (p2.y - p1.y) / len),
            )
        }
        _ => return None,
    };
    // 치수선은 정의점을 지나고 dir 방향. 두 점을 치수선에 내린다.
    let foot = |p: Vec2| {
        let t = (p.x - dp.x) * dir.x + (p.y - dp.y) * dir.y;
        Vec2::new(dp.x + dir.x * t, dp.y + dir.y * t)
    };
    let (q1, q2) = (foot(p1), foot(p2));
    let len = q1.dist(q2);
    if len < 1e-12 {
        return None;
    }
    let h = len / 20.0;
    // 건축 도면식 사선 끝 (45°)
    let tick = |q: Vec2| {
        let k = h * 0.35;
        let (dx, dy) = ((dir.x - dir.y) * k, (dir.y + dir.x) * k);
        Shape::Line {
            a: Vec2::new(q.x - dx, q.y - dy),
            b: Vec2::new(q.x + dx, q.y + dy),
        }
    };
    let b = x.base();
    let value = match &b.user_text {
        Some(t) if !t.is_empty() && t != "<>" => plain_mtext(t),
        _ => {
            let m = if b.actual_measurement > 0.0 {
                b.actual_measurement
            } else {
                len
            };
            format!("{}", (m * 100.0).round() / 100.0)
        }
    };
    let mid = q1.lerp(q2, 0.5);
    let tm = v2(&b.text_middle_point);
    let at = if tm.x == 0.0 && tm.y == 0.0 {
        Vec2::new(mid.x - dir.y * h * 0.6, mid.y + dir.x * h * 0.6)
    } else {
        tm
    };
    let mut rot = dir.y.atan2(dir.x);
    // 글자는 거꾸로 서지 않게
    if rot > std::f64::consts::FRAC_PI_2 + 1e-9 || rot <= -std::f64::consts::FRAC_PI_2 {
        rot += std::f64::consts::PI;
    }
    Some(vec![
        Shape::Line { a: p1, b: q1 },
        Shape::Line { a: p2, b: q2 },
        Shape::Line { a: q1, b: q2 },
        tick(q1),
        tick(q2),
        Shape::Text(TextShape {
            style: String::new(),
            at,
            height: h,
            rotation: rot,
            width_factor: 1.0,
            halign: 0.5,
            valign: VAlign::Middle,
            value,
        }),
    ])
}

/// acadrust 문서를 도면 모델로 바꾼다 (모형 공간, 블록 참조는 펼침).
pub fn from_document(doc: &CadDocument) -> Drawing {
    let mut c = Converter {
        doc,
        d: Drawing {
            code_page: doc.header.code_page.clone(),
            units: doc.header.insertion_units,
            ..Default::default()
        },
        layer_ids: HashMap::new(),
        stack: Vec::new(),
        lt_ids: HashMap::new(),
        layer_lt: Vec::new(),
        ltscale: if doc.header.linetype_scale > 0.0 {
            doc.header.linetype_scale
        } else {
            1.0
        },
        cur_lt: None,
        layer_w: Vec::new(),
        cur_w: None,
        layer_aci: Vec::new(),
        cur_aci: None,
    };
    // 선 종류 무늬 (대시가 있는 것만 — CONTINUOUS 등 실선은 넣지 않는다)
    for lt in doc.line_types.iter() {
        let dashes: Vec<f64> = lt.elements.iter().map(|e| e.length).collect();
        let period: f64 = dashes.iter().map(|x| x.abs()).sum();
        if dashes.len() < 2 || period <= 1e-12 || dashes.iter().all(|&x| x >= 0.0) {
            continue;
        }
        c.lt_ids
            .insert(lt.name.to_ascii_lowercase(), c.d.linetypes.len() as u16);
        c.d.linetypes.push(LinePattern {
            name: lt.name.clone(),
            dashes,
        });
    }
    for l in doc.layers.iter() {
        let i = c.layer(&l.name) as usize;
        let lt = c.lt_ids.get(&l.line_type.to_ascii_lowercase()).copied();
        if c.layer_lt.len() <= i {
            c.layer_lt.resize(i + 1, None);
        }
        c.layer_lt[i] = lt;
        if c.layer_w.len() <= i {
            c.layer_w.resize(i + 1, None);
        }
        c.layer_w[i] = match l.line_weight {
            acadrust::types::LineWeight::Value(v) if v >= 0 => Some(v as u16),
            _ => None,
        };
        let layer = &mut c.d.layers[i];
        if let Some((r, g, b)) = l.color.rgb() {
            layer.color = [r, g, b];
        }
        if c.layer_aci.len() <= i {
            c.layer_aci.resize(i + 1, None);
        }
        c.layer_aci[i] = match l.color {
            Color::Index(k) => Some(k),
            _ => None,
        };
        layer.visible = !l.flags.frozen && !l.is_off();
    }
    for e in doc.model_space_entities() {
        // 단순한 블록 참조는 블록으로 살린다 (§31)
        if let EntityType::Insert(x) = e
            && c.keep_insert(x)
        {
            continue;
        }
        // 선형·정렬 치수는 치수 객체로 (§32)
        if let EntityType::Dimension(x) = e
            && c.keep_dimension(x)
        {
            continue;
        }
        c.entity(e, &Affine::IDENTITY, None);
    }
    read_layouts(&mut c);
    // 글자 스타일 표 전부 (속성값만 쓰는 스타일도 SHX 로 그릴 수 있게, §59)
    for st in doc.text_styles.iter() {
        if !c
            .d
            .fonts
            .iter()
            .any(|f| f.style.eq_ignore_ascii_case(&st.name))
        {
            c.d.fonts.push(crate::drawing::FontUse {
                style: st.name.clone(),
                font: if st.true_type_font.is_empty() {
                    st.font_file.clone()
                } else {
                    st.true_type_font.clone()
                },
                big_font: st.big_font_file.clone(),
                count: 0,
            });
        }
    }
    c.d
}

/// 배치(종이 공간)마다 종이 객체를 펼치고 뷰포트를 모은다 (구현 노트 §13).
fn read_layouts(c: &mut Converter) {
    use acadrust::objects::ObjectType;
    let doc = c.doc;
    let mut layouts = Vec::new();
    for obj in doc.objects.values() {
        let ObjectType::Layout(l) = obj else { continue };
        if l.name.eq_ignore_ascii_case("Model") {
            continue;
        }
        let Some(br) = doc
            .block_records
            .iter()
            .find(|b| b.handle == l.block_record)
        else {
            continue;
        };
        // 종이 객체를 모형 객체와 섞지 않게 잠시 비운 목록에 받는다
        let model = std::mem::take(&mut c.d.entities);
        let mut viewports = Vec::new();
        for e in doc.entities_in_block(&br.name) {
            match e {
                EntityType::Viewport(v) => {
                    // 번호 1 은 종이 전체를 보여 주는 뷰포트라 뺀다. 꺼진 것·3D 방향도 뺀다
                    let dir = &v.view_direction;
                    if v.id == 1 || v.status.is_off || v.width <= 0.0 || v.height <= 0.0 {
                        continue;
                    }
                    if dir.x.abs() > 1e-6 || dir.y.abs() > 1e-6 || dir.z < 0.0 {
                        c.skip("VIEWPORT(3D 방향)");
                        continue;
                    }
                    let frozen = v
                        .frozen_layers
                        .iter()
                        .filter_map(|h| doc.layers.iter().find(|l| l.handle == *h))
                        .map(|l| l.name.clone())
                        .collect::<Vec<_>>()
                        .iter()
                        .map(|n| c.layer(n))
                        .collect();
                    viewports.push(ViewportView {
                        center: v2(&v.center),
                        width: v.width,
                        height: v.height,
                        view_center: v2(&v.view_center),
                        view_height: v.view_height,
                        twist: v.twist_angle,
                        target: v2(&v.view_target),
                        frozen,
                        level: None,
                    });
                }
                e => c.entity(e, &Affine::IDENTITY, None),
            }
        }
        let paper = std::mem::replace(&mut c.d.entities, model);
        if paper.is_empty() && viewports.is_empty() {
            continue; // 빈 배치 (새 도면의 Layout1 등)
        }
        layouts.push(Layout {
            name: l.name.clone(),
            order: l.tab_order,
            paper,
            viewports,
            level: None,
            list: false,
        });
    }
    layouts.sort_by_key(|l| l.order);
    c.d.layouts = layouts;
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

    /// 글자 스타일별 글꼴 쓰임 (§57)
    #[test]
    fn font_use_by_style() {
        let mut doc = CadDocument::new();
        let mut st = acadrust::tables::TextStyle::new("KOR");
        st.font_file = "romans.shx".into();
        st.big_font_file = "whgtxt.shx".into();
        doc.text_styles.add(st).unwrap();
        for k in 0..3 {
            let mut t =
                acadrust::entities::Text::with_value("가나다", Vector3::new(k as f64, 0.0, 0.0));
            t.style = "KOR".into();
            doc.add_entity(EntityType::Text(t)).unwrap();
        }
        let d = from_document(&doc);
        let f = d.fonts.iter().find(|f| f.style == "KOR").unwrap();
        assert_eq!(
            (f.font.as_str(), f.big_font.as_str(), f.count),
            ("romans.shx", "whgtxt.shx", 3)
        );
        let t = crate::archgen::tables(&d);
        let g = t.tables.iter().find(|x| x.name == "글꼴").unwrap();
        assert!(g.rows[0][4].contains("SHX"));
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
        let d = crate::block::flatten(&from_document(&doc));
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
        let d = crate::block::flatten(&from_document(&doc));
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
        let d = crate::block::flatten(&from_document(&doc));
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
        let d = crate::block::flatten(&from_document(&doc));
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
        let d = crate::block::flatten(&from_document(&doc));
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
        let d = crate::block::flatten(&from_document(&doc));
        assert_eq!(d.entities.len(), 6, "배열 6 칸 × 선 1");
        assert!(d.skipped.contains_key("블록 순환·깊이 초과"));
        let firsts: Vec<_> = (0..6).map(|i| line_of(&d, i).0).collect();
        assert!(
            firsts.iter().any(|p| close(*p, (1200.0, 110.0))),
            "{firsts:?}"
        );
    }

    #[test]
    fn dimension_without_block_is_drawn_simply() {
        use acadrust::entities::{Dimension, DimensionLinear};
        let mut doc = CadDocument::new();
        let mut dim = DimensionLinear::horizontal(
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(3000.0, 0.0, 0.0),
        );
        dim.definition_point = Vector3::new(3000.0, 500.0, 0.0);
        doc.add_entity(EntityType::Dimension(Box::new(Dimension::Linear(dim))))
            .unwrap();
        let d = crate::archgen::shown(&from_document(&doc));
        assert!(d.skipped.is_empty(), "{:?}", d.skipped);
        // 보조선 2 + 치수선 + 사선 2 + 문자
        assert_eq!(d.entities.len(), 6);
        let (a, b) = line_of(&d, 2);
        assert!(
            close(a, (0.0, 500.0)) && close(b, (3000.0, 500.0)),
            "{a:?} {b:?}"
        );
        match &d.entities[5].shape {
            // 가져온 치수는 원래 단위 형식(쉼표 없이)으로 (§36)
            Shape::Text(t) => assert_eq!(t.value, "3000"),
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn spline_becomes_polyline_through_ends() {
        use acadrust::entities::Spline;
        let mut doc = CadDocument::new();
        let mut sp = Spline::new();
        sp.degree = 2;
        sp.knots = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        sp.control_points = vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(1.0, 2.0, 0.0),
            Vector3::new(2.0, 0.0, 0.0),
        ];
        doc.add_entity(EntityType::Spline(sp)).unwrap();
        let d = crate::block::flatten(&from_document(&doc));
        match &d.entities[0].shape {
            Shape::Polyline { points, .. } => {
                assert!(close(points[0].0, (0.0, 0.0)));
                assert!(close(points[points.len() - 1].0, (2.0, 0.0)));
                assert!(points.len() > 8);
            }
            s => panic!("{s:?}"),
        }
    }
}
