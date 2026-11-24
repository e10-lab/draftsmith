//! 도면 모델.
//!
//! 지금은 가져온 DXF·DWG 를 보여 주는 데 필요한 작도 객체만 있다 (M0·M1).
//! 건축 객체(벽·문·창·실·그리드·층)는 M3 에서 같은 문서에 더한다 — 로드맵 참고.

use std::collections::BTreeMap;

use crate::curve;
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
    /// 선 종류 (점선·중심선 등). `None` 이면 실선
    pub linetype: Option<LineStyle>,
    /// 선 굵기 (1/100 mm). `None` 이면 기본 굵기 (출력 설정에서 정함)
    pub weight: Option<u16>,
    pub shape: Shape,
}

/// 객체에 적용한 선 종류
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineStyle {
    /// [`Drawing::linetypes`] 의 번호
    pub pattern: u16,
    /// 무늬 배율 (전역 LTSCALE × 객체 배율 × 블록 축척)
    pub scale: f64,
}

/// 선 종류 무늬
#[derive(Debug, Clone, PartialEq)]
pub struct LinePattern {
    pub name: String,
    /// 대시 길이: 양수 = 선, 음수 = 빈칸, 0 = 점
    pub dashes: Vec<f64>,
}

/// 문자 세로 맞춤
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VAlign {
    /// 글자 바탕선 (TEXT 기본)
    #[default]
    Baseline,
    Bottom,
    Middle,
    /// 첫 줄 위 (MTEXT 기본)
    Top,
}

/// 문자
#[derive(Debug, Clone, PartialEq)]
pub struct TextShape {
    /// 맞춤 기준점
    pub at: Vec2,
    /// 글자 높이 (도면 단위)
    pub height: f64,
    /// 진행 방향 (라디안)
    pub rotation: f64,
    /// 가로 폭 배율 (1 = 보통)
    pub width_factor: f64,
    /// 가로 맞춤: 0 왼쪽, 0.5 가운데, 1 오른쪽
    pub halign: f64,
    pub valign: VAlign,
    /// 보이는 글자 (서식 코드 걷어 냄). 여러 줄이면 `\n`
    pub value: String,
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
    /// 칠한 영역 (짝홀 규칙). 솔리드 해치·SOLID·치수 화살표
    Fill {
        loops: Vec<Vec<Vec2>>,
    },
    /// 선분 묶음 (해치 무늬처럼 한 객체가 선분 여럿)
    Segments {
        lines: Vec<(Vec2, Vec2)>,
    },
    Text(TextShape),
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
                    // 등각이면 불지가 그대로
                    points.iter().map(|(p, g)| (t.apply(*p), *g)).collect()
                } else if t.is_anticonformal() {
                    // 대칭 + 균등 축척(+회전): 호 모양은 그대로, 방향만 바뀌어 불지 부호를 뒤집는다
                    points.iter().map(|(p, g)| (t.apply(*p), -*g)).collect()
                } else {
                    // 그 밖(비균등 축척 등): 호를 꼭짓점으로 펼친 뒤 변환 (호 하나 32 조각 이상)
                    let r = points
                        .iter()
                        .map(|(p, _)| p.x.abs().max(p.y.abs()))
                        .fold(1.0, f64::max);
                    curve::polyline_points(points, *closed, r * 1e-4)
                        .into_iter()
                        .map(|p| (t.apply(p), 0.0))
                        .collect()
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
            Shape::Fill { loops } => Shape::Fill {
                loops: loops
                    .iter()
                    .map(|r| r.iter().map(|p| t.apply(*p)).collect())
                    .collect(),
            },
            Shape::Segments { lines } => Shape::Segments {
                lines: lines
                    .iter()
                    .map(|(a, b)| (t.apply(*a), t.apply(*b)))
                    .collect(),
            },
            Shape::Text(x) => {
                // 높이는 글자 위쪽 방향이 늘어난 만큼, 방향은 진행 방향이 돈 만큼.
                // 대칭이면 글자는 뒤집지 않고 읽을 수 있게 둔다 (AutoCAD MIRRTEXT=0 과 같음).
                let (s, c) = x.rotation.sin_cos();
                let up = t.linear(Vec2::new(-s, c));
                let dir = t.linear(Vec2::new(c, s));
                let (dir, halign) = if t.mirrors() {
                    (Vec2::new(-dir.x, -dir.y), 1.0 - x.halign)
                } else {
                    (dir, x.halign)
                };
                let h = up.x.hypot(up.y);
                let w = dir.x.hypot(dir.y);
                Shape::Text(TextShape {
                    at: t.apply(x.at),
                    height: x.height * h,
                    rotation: dir.y.atan2(dir.x),
                    width_factor: x.width_factor * if h > 0.0 { w / h } else { 1.0 },
                    halign,
                    valign: x.valign,
                    value: x.value.clone(),
                })
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
            Shape::Fill { loops } => {
                for p in loops.iter().flatten() {
                    b.add(*p);
                }
            }
            Shape::Segments { lines } => {
                for (p, q) in lines {
                    b.add(*p);
                    b.add(*q);
                }
            }
            Shape::Text(x) => {
                // 대략: 글자 수 × 높이 (돌림은 무시)
                b.add(x.at);
                let w = x
                    .value
                    .lines()
                    .map(|l| l.chars().count())
                    .max()
                    .unwrap_or(0) as f64
                    * x.height
                    * x.width_factor;
                b.add(Vec2::new(x.at.x + w, x.at.y + x.height));
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
    /// 도면 단위 (DXF `$INSUNITS`: 0 단위 없음, 1 인치, 2 피트, 4 mm, 5 cm, 6 m)
    pub units: i16,
    /// 선 종류 무늬 (실선이 아닌 것만)
    pub linetypes: Vec<LinePattern>,
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
                Shape::Fill { .. } => "채움",
                Shape::Segments { .. } => "해치 무늬",
                Shape::Text(_) => "문자",
            };
            *m.entry(k).or_insert(0) += 1;
        }
        m
    }

    /// 도면 단위 1 이 몇 미터인가. 단위가 없으면 국내 건축 도면 관례대로 mm 로 본다 (두 번째 값 true).
    pub fn meters_per_unit(&self) -> (f64, bool) {
        match self.units {
            1 => (0.0254, false),
            2 => (0.3048, false),
            4 => (0.001, false),
            5 => (0.01, false),
            6 => (1.0, false),
            _ => (0.001, true),
        }
    }

    /// 객체의 표시 색 (객체 색이 없으면 레이어 색)
    pub fn color_of(&self, e: &Entity) -> [u8; 3] {
        e.color
            .or_else(|| self.layers.get(e.layer as usize).map(|l| l.color))
            .unwrap_or([255, 255, 255])
    }
}
