//! 도면 모델.
//!
//! 지금은 가져온 DXF·DWG 를 보여 주는 데 필요한 작도 객체만 있다 (M0·M1).
//! 건축 객체(벽·문·창·실·그리드·층)는 M3 에서 같은 문서에 더한다 — 로드맵 참고.

use std::collections::BTreeMap;

use crate::geom::{Affine, Bbox, Vec2};

/// 레이어
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub name: String,
    /// 표시 색 (RGB)
    pub color: [u8; 3],
    /// 켜짐 (꺼진 레이어·동결 레이어는 false)
    pub visible: bool,
}

/// 작도 객체 하나
#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    /// [`Drawing::layers`] 의 번호
    pub layer: u32,
    /// 객체 자체 색. `None` 이면 레이어 색
    pub color: Option<[u8; 3]>,
    pub shape: Shape,
}

/// 모양. 각도는 라디안, 반시계 방향.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Line {
        a: Vec2,
        b: Vec2,
    },
    /// 폴리선. 꼭짓점마다 다음 꼭짓점까지의 불지(bulge, 0 이면 직선)를 함께 둔다.
    Polyline {
        points: Vec<(Vec2, f64)>,
        closed: bool,
    },
    Circle {
        center: Vec2,
        radius: f64,
    },
    Arc {
        center: Vec2,
        radius: f64,
        start: f64,
        end: f64,
    },
    /// 타원(호). `center + u·cos t + v·sin t`, t 는 `start` 에서 `end` 까지 (반시계, 라디안).
    /// u·v 는 서로 켤레인 반지름 벡터라, 원·호에 아무 아핀 변환(비균등 축척·대칭)을 걸어도 이 꼴로 남는다.
    Ellipse {
        center: Vec2,
        u: Vec2,
        v: Vec2,
        start: f64,
        end: f64,
    },
    /// 한 줄 문자. 글꼴 처리는 M0 ⑥ 에서 — 지금은 위치·크기만 쓴다.
    Text {
        at: Vec2,
        height: f64,
        rotation: f64,
        value: String,
    },
}

/// 불지 호 하나를 꼭짓점으로 펼친다 (끝점 `b` 는 넣지 않음). 비균등 변환 전에 쓴다.
fn bulge_points(a: Vec2, b: Vec2, bulge: f64, out: &mut Vec<(Vec2, f64)>) {
    out.push((a, 0.0));
    let chord = a.dist(b);
    if bulge.abs() < 1e-9 || chord < 1e-12 {
        return;
    }
    let sweep = 4.0 * bulge.atan();
    let r = chord / (2.0 * (sweep / 2.0).sin().abs());
    let m = a.lerp(b, 0.5);
    let h = (r * r - (chord / 2.0).powi(2)).max(0.0).sqrt();
    let (ux, uy) = ((b.x - a.x) / chord, (b.y - a.y) / chord);
    // 반원보다 작은 호는 중심이 진행 방향 왼쪽(불지 양수일 때), 큰 호는 반대쪽
    let side = if (bulge > 0.0) == (sweep.abs() < std::f64::consts::PI) {
        1.0
    } else {
        -1.0
    };
    let c = Vec2::new(m.x - uy * h * side, m.y + ux * h * side);
    let start = (a.y - c.y).atan2(a.x - c.x);
    let n = 32;
    for i in 1..n {
        let t = start + sweep * i as f64 / n as f64;
        out.push((Vec2::new(c.x + r * t.cos(), c.y + r * t.sin()), 0.0));
    }
}

impl Shape {
    /// 아핀 변환을 건 모양. 원은 변환이 등각(회전·균등 축척)이면 원으로, 아니면 타원이 된다.
    pub fn transformed(&self, t: &Affine) -> Shape {
        match self {
            Shape::Line { a, b } => Shape::Line {
                a: t.apply(*a),
                b: t.apply(*b),
            },
            Shape::Polyline { points, closed } => {
                let pts = if t.is_conformal() || points.iter().all(|(_, g)| g.abs() < 1e-9) {
                    // 등각이면 불지가 그대로 (대칭은 is_conformal 에서 빠지므로 부호도 그대로)
                    points.iter().map(|(p, g)| (t.apply(*p), *g)).collect()
                } else if t.is_anticonformal() {
                    // 대칭 + 균등 축척(+회전): 호 모양은 그대로, 방향만 바뀌어 불지 부호를 뒤집는다
                    points.iter().map(|(p, g)| (t.apply(*p), -*g)).collect()
                } else {
                    // 그 밖(비균등 축척 등): 호를 꼭짓점으로 펼친 뒤 변환
                    let n = points.len();
                    let mut flat = Vec::new();
                    for i in 0..n {
                        let (p, g) = points[i];
                        if i + 1 < n || *closed {
                            bulge_points(p, points[(i + 1) % n].0, g, &mut flat);
                        } else {
                            flat.push((p, 0.0));
                        }
                    }
                    flat.into_iter().map(|(p, _)| (t.apply(p), 0.0)).collect()
                };
                Shape::Polyline {
                    points: pts,
                    closed: *closed,
                }
            }
            Shape::Circle { center, radius } => {
                if t.is_conformal() || t.is_anticonformal() {
                    Shape::Circle {
                        center: t.apply(*center),
                        radius: radius * t.scale_x(),
                    }
                } else {
                    Shape::Ellipse {
                        center: t.apply(*center),
                        u: t.linear(Vec2::new(*radius, 0.0)),
                        v: t.linear(Vec2::new(0.0, *radius)),
                        start: 0.0,
                        end: std::f64::consts::TAU,
                    }
                }
            }
            Shape::Arc {
                center,
                radius,
                start,
                end,
            } => {
                if t.is_conformal() {
                    let rot = t.angle();
                    Shape::Arc {
                        center: t.apply(*center),
                        radius: radius * t.scale_x(),
                        start: start + rot,
                        end: end + rot,
                    }
                } else if t.is_anticonformal() {
                    // 대칭: 각도 θ 인 점이 β − θ 로 간다 (β = 변환 후 x 축 방향).
                    // 반시계 호가 시계 방향이 되므로 시작·끝을 맞바꾼다.
                    let beta = t.angle();
                    Shape::Arc {
                        center: t.apply(*center),
                        radius: radius * t.scale_x(),
                        start: beta - end,
                        end: beta - start,
                    }
                } else {
                    // 대칭이 있으면 v 가 뒤집혀 그리는 방향이 저절로 맞는다
                    Shape::Ellipse {
                        center: t.apply(*center),
                        u: t.linear(Vec2::new(*radius, 0.0)),
                        v: t.linear(Vec2::new(0.0, *radius)),
                        start: *start,
                        end: *end,
                    }
                }
            }
            Shape::Ellipse {
                center,
                u,
                v,
                start,
                end,
            } => Shape::Ellipse {
                center: t.apply(*center),
                u: t.linear(*u),
                v: t.linear(*v),
                start: *start,
                end: *end,
            },
            Shape::Text {
                at,
                height,
                rotation,
                value,
            } => {
                // 높이는 글자 위쪽 방향이 늘어난 만큼, 방향은 글자 진행 방향이 돈 만큼
                let (s, c) = rotation.sin_cos();
                let up = t.linear(Vec2::new(-s, c));
                let dir = t.linear(Vec2::new(c, s));
                Shape::Text {
                    at: t.apply(*at),
                    height: height * up.x.hypot(up.y),
                    rotation: dir.y.atan2(dir.x),
                    value: value.clone(),
                }
            }
        }
    }

    pub fn bbox(&self) -> Bbox {
        let mut b = Bbox::EMPTY;
        match self {
            Shape::Line { a, b: e } => {
                b.add(*a);
                b.add(*e);
            }
            Shape::Polyline { points, .. } => {
                // 불지 호가 꼭짓점 밖으로 나가는 몫은 무시한다 (화면 맞춤에만 쓰므로 충분)
                for (p, _) in points {
                    b.add(*p);
                }
            }
            Shape::Circle { center, radius } | Shape::Arc { center, radius, .. } => {
                b.add(Vec2::new(center.x - radius, center.y - radius));
                b.add(Vec2::new(center.x + radius, center.y + radius));
            }
            Shape::Ellipse { center, u, v, .. } => {
                // 전체 타원을 감싸는 상자 (부분 호도 이것으로 충분)
                let (hx, hy) = (u.x.hypot(v.x), u.y.hypot(v.y));
                b.add(Vec2::new(center.x - hx, center.y - hy));
                b.add(Vec2::new(center.x + hx, center.y + hy));
            }
            Shape::Text {
                at, height, value, ..
            } => {
                b.add(*at);
                // 대략 폭: 글자 수 × 높이
                let w = value.chars().count() as f64 * height;
                b.add(Vec2::new(at.x + w, at.y + height));
            }
        }
        b
    }
}

/// 도면 한 장
#[derive(Debug, Clone, Default)]
pub struct Drawing {
    pub layers: Vec<Layer>,
    pub entities: Vec<Entity>,
    /// 원본 파일의 문자 코드 (`ANSI_949` 등). 새 도면이면 빈 문자열
    pub code_page: String,
    /// 가져오지 못한 객체 종류와 개수 (예: `HATCH` → 120). 화면·CLI 에 알린다
    pub skipped: BTreeMap<String, usize>,
    /// 펼친 블록 참조 수 (배열 블록은 칸마다, 블록 안 블록도 하나씩)
    pub inserts: usize,
}

impl Drawing {
    /// 레이어 이름으로 번호를 찾고, 없으면 만든다.
    pub fn layer_index(&mut self, name: &str) -> u32 {
        if let Some(i) = self.layers.iter().position(|l| l.name == name) {
            return i as u32;
        }
        self.layers.push(Layer {
            name: name.to_string(),
            color: [255, 255, 255],
            visible: true,
        });
        (self.layers.len() - 1) as u32
    }

    pub fn bbox(&self) -> Bbox {
        let mut b = Bbox::EMPTY;
        for e in &self.entities {
            b.merge(&e.shape.bbox());
        }
        b
    }

    /// 객체 종류별 개수 (CLI `info` 용)
    pub fn counts(&self) -> BTreeMap<&'static str, usize> {
        let mut m = BTreeMap::new();
        for e in &self.entities {
            let k = match e.shape {
                Shape::Line { .. } => "선",
                Shape::Polyline { .. } => "폴리선",
                Shape::Circle { .. } => "원",
                Shape::Arc { .. } => "호",
                Shape::Ellipse { .. } => "타원",
                Shape::Text { .. } => "문자",
            };
            *m.entry(k).or_insert(0) += 1;
        }
        m
    }

    /// 객체의 표시 색 (객체 색이 없으면 레이어 색)
    pub fn color_of(&self, e: &Entity) -> [u8; 3] {
        e.color
            .or_else(|| self.layers.get(e.layer as usize).map(|l| l.color))
            .unwrap_or([255, 255, 255])
    }
}
