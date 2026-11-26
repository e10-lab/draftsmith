//! 도면 모델.
//!
//! 지금은 가져온 DXF·DWG 를 보여 주는 데 필요한 작도 객체만 있다 (M0·M1).
//! 건축 객체(벽·문·창·실·그리드·층)는 M3 에서 같은 문서에 더한다 — 로드맵 참고.

use std::collections::BTreeMap;

use crate::curve;
use crate::geom::{Affine, Bbox, Vec2};

/// 레이어
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Layer {
    pub name: String,
    /// 표시 색 (RGB)
    pub color: [u8; 3],
    /// 켜짐 (꺼진 레이어·동결 레이어는 false)
    pub visible: bool,
}

/// 작도 객체 하나
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Entity {
    /// 객체 고유 번호 (§82). 0 은 「아직 없음」 — 편집기가 작업을 실행할 때 매긴다. 블록 정의 안 객체·보이는 도면이
    /// 만든 객체는 0. 옛 파일은 0 으로 읽혀 열 때 매긴다
    #[serde(default)]
    pub id: u64,
    /// [`Drawing::layers`] 의 번호
    pub layer: u32,
    /// 객체 자체 색. `None` 이면 레이어 색
    pub color: Option<[u8; 3]>,
    /// 색 번호 (ACI 1–255, ByLayer·ByBlock 을 풀어 낸 것). 트루컬러면 None — 출력 스타일(CTB)에 쓴다
    pub aci: Option<u8>,
    /// 선 종류 (점선·중심선 등). `None` 이면 실선
    pub linetype: Option<LineStyle>,
    /// 선 굵기 (1/100 mm). `None` 이면 기본 굵기 (출력 설정에서 정함)
    pub weight: Option<u16>,
    pub shape: Shape,
}

/// 객체에 적용한 선 종류
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LineStyle {
    /// [`Drawing::linetypes`] 의 번호
    pub pattern: u16,
    /// 무늬 배율 (전역 LTSCALE × 객체 배율 × 블록 축척)
    pub scale: f64,
}

/// 선 종류 무늬
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LinePattern {
    pub name: String,
    /// 대시 길이: 양수 = 선, 음수 = 빈칸, 0 = 점
    pub dashes: Vec<f64>,
}

/// 문자 세로 맞춤
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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
    /// 가져온 글자 스타일 이름 (SHX 로 그릴 때 글꼴을 찾는다, §59). DraftSmith 가 만든 글자는 빈칸
    #[serde(default)]
    pub style: String,
}

/// 모양. 각도는 라디안, 반시계 방향.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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
    /// 폭 있는 폴리선 (§65): 꼭짓점·볼록 값은 폴리선과 같고, 꼭짓점마다 그 변의 (시작 폭, 끝 폭). 채운 띠로 그린다
    Wide {
        points: Vec<(Vec2, f64)>,
        closed: bool,
        widths: Vec<(f64, f64)>,
    },
    /// 가리개 (WIPEOUT, §60): 이 고리 안에서 그리기 순서상 앞에 있는 선을 지운다. 보이는 도면 단계에서 풀린다
    Wipeout {
        ring: Vec<Vec2>,
    },
    Text(TextShape),
    /// 무늬 해치 (§45): 경계 고리(짝홀)와 무늬 선 정의. 화면·출력에는 무늬 선분이 대신 나오고, DXF·DWG 로는 HATCH 로
    Hatch {
        loops: Vec<Vec<Vec2>>,
        lines: Vec<crate::fill::PatternLine>,
        /// 무늬 이름 (ANSI31 등, DraftSmith 무늬는 한글 이름)
        name: String,
    },
    /// 블록 참조 (DraftSmith 에서 만든 블록 — 구현 노트 §30). 화면·출력에는 펼친 객체가 대신 나온다
    Insert {
        block: String,
        at: Vec2,
        /// x·y 배율 (y 가 음수면 대칭)
        scale: Vec2,
        rotation: f64,
        /// 속성값 (§46) — 글자는 블록 안 좌표 (참조와 함께 옮겨지고 돈다)
        #[serde(default)]
        attrs: Vec<Attr>,
    },
}

impl Shape {
    /// 화면·출력용 모양: 무늬 해치는 무늬 선분(무늬가 없으면 칠)으로, 나머지는 그대로 (§45)
    pub fn drawn(&self) -> Shape {
        match self {
            Shape::Hatch { loops, lines, .. } => {
                if lines.is_empty() {
                    return Shape::Fill {
                        loops: loops.clone(),
                    };
                }
                match crate::fill::pattern_segments(loops, lines, 200_000) {
                    Some(segs) => Shape::Segments { lines: segs },
                    // 너무 촘촘하면 경계선만
                    None => Shape::Segments {
                        lines: loops
                            .iter()
                            .flat_map(|r| (0..r.len()).map(move |i| (r[i], r[(i + 1) % r.len()])))
                            .collect(),
                    },
                }
            }
            s => s.clone(),
        }
    }

    /// 아핀 변환을 건 모양. 원은 변환이 등각(회전·균등 축척)이면 원으로, 아니면 타원이 된다.
    pub fn transformed(&self, t: &Affine) -> Shape {
        match self {
            Shape::Wide {
                points,
                closed,
                widths,
            } => {
                // 폴리선처럼 옮기고, 폭은 평균 배율로 (비균등 축척으로 호가 펼쳐지면 첫 폭)
                let k = t.det().abs().sqrt();
                match (Shape::Polyline {
                    points: points.clone(),
                    closed: *closed,
                })
                .transformed(t)
                {
                    Shape::Polyline {
                        points: p,
                        closed: c,
                    } => {
                        let w: Vec<(f64, f64)> = if p.len() == points.len() {
                            widths.iter().map(|(a, b)| (a * k, b * k)).collect()
                        } else {
                            let f = widths.first().copied().unwrap_or((0.0, 0.0));
                            vec![(f.0 * k, f.0 * k); p.len()]
                        };
                        Shape::Wide {
                            points: p,
                            closed: c,
                            widths: w,
                        }
                    }
                    other => other,
                }
            }
            Shape::Wipeout { ring } => Shape::Wipeout {
                ring: ring.iter().map(|p| t.apply(*p)).collect(),
            },
            Shape::Hatch { loops, lines, name } => {
                // 무늬 선: 기준점은 점처럼, 방향·이동은 선형 부분으로, 대시 길이는 방향 배율만큼
                let lines = lines
                    .iter()
                    .map(|l| {
                        let d = t.linear(Vec2::new(l.angle.cos(), l.angle.sin()));
                        let k = d.x.hypot(d.y);
                        crate::fill::PatternLine {
                            angle: d.y.atan2(d.x),
                            base: t.apply(l.base),
                            offset: t.linear(l.offset),
                            dashes: l.dashes.iter().map(|x| x * k).collect(),
                        }
                    })
                    .collect();
                Shape::Hatch {
                    loops: loops
                        .iter()
                        .map(|r| r.iter().map(|p| t.apply(*p)).collect())
                        .collect(),
                    lines,
                    name: name.clone(),
                }
            }
            Shape::Insert {
                block,
                at,
                scale,
                rotation,
                attrs,
            } => {
                // 대칭이면 y 배율을 뒤집고 각도는 거울 쪽으로 (구현 노트 §30)
                let k = t.scale_x();
                let (rot, sy) = if t.mirrors() {
                    (t.angle() - rotation, -scale.y)
                } else {
                    (t.angle() + rotation, scale.y)
                };
                Shape::Insert {
                    block: block.clone(),
                    at: t.apply(*at),
                    scale: Vec2::new(scale.x * k, sy * k),
                    rotation: rot,
                    attrs: attrs.clone(),
                }
            }
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
                    // 스타일은 그대로 — 잃으면 SHX 큰 글꼴이 빠져 한글이 내장 글꼴로 바뀐다 (§68)
                    style: x.style.clone(),
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
            Shape::Wide { points, widths, .. } => {
                let h = widths.iter().map(|(a, b)| a.max(*b)).fold(0.0, f64::max) / 2.0;
                for (p, _) in points {
                    b.add(Vec2::new(p.x - h, p.y - h));
                    b.add(Vec2::new(p.x + h, p.y + h));
                }
            }
            Shape::Wipeout { ring } => {
                for p in ring {
                    b.add(*p);
                }
            }
            Shape::Hatch { loops, .. } => {
                for p in loops.iter().flatten() {
                    b.add(*p);
                }
            }
            Shape::Insert { at, .. } => b.add(*at),
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
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
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
    /// 배치 (종이 공간). 탭 순서대로
    pub layouts: Vec<crate::layout::Layout>,
    /// 깨진 곳이 있어 복구 모드로 읽었으면 처음 오류 내용
    pub recovered: Option<String>,
    /// 새로 그리는 객체가 들어갈 레이어 (현재 레이어)
    pub current_layer: u32,
    /// 건축 객체 (벽·문·창·실·그리드·층) — M3
    pub arch: crate::arch::ArchModel,
    /// 블록 정의 (DraftSmith 에서 만든 것 — 가져온 DWG 블록은 펼쳐서 들어온다)
    pub blocks: Vec<Block>,
    /// 가져온 글자들이 쓰던 글꼴 (글자 스타일별, §57) — 화면·PDF 는 모두 내장 글꼴로 대체
    pub fonts: Vec<FontUse>,
    /// 다음에 줄 객체 고유 번호 (§82). 모든 객체 번호보다 크다
    pub next_id: u64,
}

/// 글자 스타일 하나가 쓰던 글꼴과 글자 수 (§57)
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct FontUse {
    pub style: String,
    /// 글꼴 파일 (SHX 또는 TTF 이름)
    pub font: String,
    /// 큰 글꼴 (한글 SHX 등)
    pub big_font: String,
    /// 이 스타일로 쓴 글자 객체 수 (블록 안 포함, 펼친 수)
    pub count: usize,
    /// 원래 스타일 표의 글꼴 파일·트루타입 이름 (내보낼 때 그대로, §60-1)
    pub file: String,
    pub ttf: String,
}

impl Shape {
    /// 폭 있는 폴리선을 폭 없는 폴리선과 대표 폭(가장 흔한 시작 폭)으로 — 편집은 폴리선으로 하고 폭을 되붙인다 (§67)
    pub fn thin(&self) -> Option<(Shape, f64)> {
        let Shape::Wide {
            points,
            closed,
            widths,
        } = self
        else {
            return None;
        };
        let w = widths.first().map(|(a, _)| *a).unwrap_or(0.0);
        Some((
            Shape::Polyline {
                points: points.clone(),
                closed: *closed,
            },
            w,
        ))
    }

    /// 폴리선(또는 선·호)에 고정 폭을 붙인다. 폭이 0 이면 그대로
    pub fn widen(self, w: f64) -> Shape {
        if w <= 0.0 {
            return self;
        }
        let (points, closed) = match self {
            Shape::Polyline { points, closed } => (points, closed),
            Shape::Line { a, b } => (vec![(a, 0.0), (b, 0.0)], false),
            other => return other,
        };
        let n = points.len();
        Shape::Wide {
            points,
            closed,
            widths: vec![(w, w); n],
        }
    }
}

/// 폭 있는 폴리선 `orig` 을 평행 복사(간격 띄우기)한 폴리선에 폭을 되붙인다 (§76): 꼭짓점 수가 같으면 꼭짓점마다 그대로
pub fn rewiden_parallel(orig: &Shape, new: Shape) -> Shape {
    if let (
        Shape::Wide {
            points: op,
            closed: oc,
            widths,
        },
        Shape::Polyline { points, closed },
    ) = (orig, &new)
        && points.len() == op.len()
        && closed == oc
    {
        return Shape::Wide {
            points: points.clone(),
            closed: *closed,
            widths: widths.clone(),
        };
    }
    rewiden(orig, new)
}

/// 폭 있는 폴리선 `orig` 을 편집해 나온 폴리선(또는 선)에 폭을 되붙인다 (§76). 폭이 모두 같으면 [`Shape::widen`] 과 같다.
/// 아니면 새 변 양 끝 폭을 원래 폴리선에서 그 변 가운데가 가장 가까운 변의 그 자리 폭(시작 → 끝 길이 비례)으로 —
/// 자르면 잘린 자리 폭, 모깎기 호는 앞뒤 변 폭을 잇는다
pub fn rewiden(orig: &Shape, new: Shape) -> Shape {
    let Shape::Wide {
        points: op,
        closed: oc,
        widths: ow,
    } = orig
    else {
        return new;
    };
    let w0 = ow.first().map(|w| w.0).unwrap_or(0.0);
    if ow.iter().all(|w| w.0 == w0 && w.1 == w0) {
        return new.widen(w0);
    }
    let (points, closed) = match new {
        Shape::Polyline { points, closed } => (points, closed),
        Shape::Line { a, b } => (vec![(a, 0.0), (b, 0.0)], false),
        other => return other,
    };
    let n = points.len();
    // 원래 변들을 잘게 펼쳐 (변 번호, 변 안 비율 시작·끝, 점 둘)
    let on = op.len();
    let osegs = if *oc { on } else { on.saturating_sub(1) };
    let mut subs: Vec<(usize, f64, f64, Vec2, Vec2)> = Vec::new();
    for j in 0..osegs {
        let (p, g) = op[j];
        let q = op[(j + 1) % on].0;
        let mut pts = Vec::new();
        let tol = (p.dist(q) * 1e-3).max(1e-9);
        crate::curve::bulge_points(p, q, g, tol, &mut pts);
        pts.push(q);
        let len: f64 = pts
            .windows(2)
            .map(|w| w[0].dist(w[1]))
            .sum::<f64>()
            .max(1e-300);
        let mut acc = 0.0;
        for w in pts.windows(2) {
            let l = w[0].dist(w[1]);
            subs.push((j, acc / len, (acc + l) / len, w[0], w[1]));
            acc += l;
        }
    }
    // 점 x 에서 가장 가까운 원래 자리 (변 번호, 비율). `only` 면 그 변에서만
    let near = |x: Vec2, only: Option<usize>| -> (usize, f64) {
        let mut best = (f64::INFINITY, 0usize, 0.0);
        for &(j, f0, f1, a, b) in &subs {
            if only.is_some_and(|o| o != j) {
                continue;
            }
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let l2 = dx * dx + dy * dy;
            let t = if l2 < 1e-300 {
                0.0
            } else {
                (((x.x - a.x) * dx + (x.y - a.y) * dy) / l2).clamp(0.0, 1.0)
            };
            let d = x.dist(Vec2::new(a.x + dx * t, a.y + dy * t));
            if d < best.0 {
                best = (d, j, f0 + (f1 - f0) * t);
            }
        }
        (best.1, best.2)
    };
    let width_at = |j: usize, f: f64| ow[j].0 + (ow[j].1 - ow[j].0) * f;
    let segs = if closed { n } else { n.saturating_sub(1) };
    let mut widths = vec![(0.0, 0.0); n];
    for (i, w) in widths.iter_mut().enumerate().take(segs) {
        let (p, g) = points[i];
        let q = points[(i + 1) % n].0;
        // 변 가운데 (호면 호 위 가운데)
        let mut mid = Vec::new();
        crate::curve::bulge_points(p, q, g, (p.dist(q) * 1e-2).max(1e-9), &mut mid);
        mid.push(q);
        let m = if mid.len() > 2 {
            mid[mid.len() / 2]
        } else {
            p.lerp(q, 0.5)
        };
        let (j, _) = near(m, None);
        let (_, fa) = near(p, Some(j));
        let (_, fb) = near(q, Some(j));
        *w = (width_at(j, fa), width_at(j, fb));
    }
    Shape::Wide {
        points,
        closed,
        widths,
    }
}

/// 두 객체 목록의 같은 앞부분·뒷부분 길이 (겹치지 않게) — 편집 뒤 바뀐 가운데만 다시 셈하려고 (§69)
pub fn align(old: &[Entity], new: &[Entity]) -> (usize, usize) {
    let n = old.len().min(new.len());
    let pre = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let suf = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(n - pre)
        .take_while(|(a, b)| a == b)
        .count();
    (pre, suf)
}

/// 폭 있는 폴리선을 채울 사각형들 (§65): 호는 꼭짓점으로 펼쳐 폭을 길이 비례로 잇고, 변마다 사각형 + 꺾인 곳은
/// 바깥 면을 맞대는 마이터(너무 길면 빗면)로 틈을 메운다. 사각형 꼭짓점은 반시계 아니어도 된다
pub fn wide_quads(
    points: &[(Vec2, f64)],
    closed: bool,
    widths: &[(f64, f64)],
    tol: f64,
) -> Vec<[Vec2; 4]> {
    let n = points.len();
    if n < 2 {
        return Vec::new();
    }
    let segs = if closed { n } else { n - 1 };
    // (점, 반폭) 사슬 — 호는 펼쳐서
    let mut chain: Vec<(Vec2, f64)> = Vec::new();
    for i in 0..segs {
        let (p, g) = points[i];
        let q = points[(i + 1) % n].0;
        let (w0, w1) = widths.get(i).copied().unwrap_or((0.0, 0.0));
        let mut pts = Vec::new();
        crate::curve::bulge_points(p, q, g, tol, &mut pts);
        pts.push(q);
        let total: f64 = pts
            .windows(2)
            .map(|w| w[0].dist(w[1]))
            .sum::<f64>()
            .max(1e-12);
        let mut run = 0.0;
        for (k, pt) in pts.iter().enumerate() {
            if k > 0 {
                run += pts[k - 1].dist(*pt);
            }
            let h = (w0 + (w1 - w0) * run / total) / 2.0;
            // 변 사이 꼭짓점은 두 번 들어가지 않게 (폭이 바뀌는 곳은 뒤 변 시작 폭으로 덮어씀)
            if k == 0 && !chain.is_empty() {
                if let Some(last) = chain.last_mut() {
                    last.1 = h.max(last.1);
                }
                continue;
            }
            chain.push((*pt, h));
        }
    }
    let m = chain.len();
    let mut out = Vec::new();
    let side = |a: Vec2, b: Vec2| {
        let l = a.dist(b).max(1e-12);
        Vec2::new(-(b.y - a.y) / l, (b.x - a.x) / l)
    };
    for i in 0..m - 1 {
        let ((a, ha), (b, hb)) = (chain[i], chain[i + 1]);
        if a.dist(b) < 1e-12 || (ha <= 0.0 && hb <= 0.0) {
            continue;
        }
        let nrm = side(a, b);
        out.push([
            Vec2::new(a.x + nrm.x * ha, a.y + nrm.y * ha),
            Vec2::new(b.x + nrm.x * hb, b.y + nrm.y * hb),
            Vec2::new(b.x - nrm.x * hb, b.y - nrm.y * hb),
            Vec2::new(a.x - nrm.x * ha, a.y - nrm.y * ha),
        ]);
    }
    // 꺾인 곳 메우기 (닫혔으면 처음·끝도)
    let joints: Vec<usize> = if closed {
        (0..m).collect()
    } else {
        (1..m.saturating_sub(1)).collect()
    };
    for j in joints {
        let (prev, cur, next) = (chain[(j + m - 1) % m].0, chain[j], chain[(j + 1) % m].0);
        let (c, h) = cur;
        if h <= 0.0 || prev.dist(c) < 1e-12 || next.dist(c) < 1e-12 {
            continue;
        }
        let (n0, n1) = (side(prev, c), side(c, next));
        let turn = (c.x - prev.x) * (next.y - c.y) - (c.y - prev.y) * (next.x - c.x);
        if turn.abs() < 1e-12 {
            continue;
        }
        // 바깥쪽 (왼쪽으로 꺾으면 오른쪽 면이 바깥)
        let s = if turn > 0.0 { -1.0 } else { 1.0 };
        let pa = Vec2::new(c.x + n0.x * h * s, c.y + n0.y * h * s);
        let pb = Vec2::new(c.x + n1.x * h * s, c.y + n1.y * h * s);
        let bis = Vec2::new(n0.x + n1.x, n0.y + n1.y);
        let bl = (bis.x * bis.x + bis.y * bis.y).sqrt();
        let cosh = if bl > 1e-12 {
            (bis.x * n0.x + bis.y * n0.y) / bl
        } else {
            0.0
        };
        if bl > 1e-12 && cosh > 0.25 {
            // 마이터 꼭짓점: 이등분선 방향으로 h / cos(반각)
            let k = h * s / cosh / bl;
            let mp = Vec2::new(c.x + bis.x * k, c.y + bis.y * k);
            out.push([c, pa, mp, pb]);
        } else {
            out.push([c, pa, pb, c]);
        }
    }
    out
}

/// 블록 참조의 속성값 하나: 태그와 글자 (값은 `text.value`, 위치는 블록 안 좌표)
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Attr {
    pub tag: String,
    pub text: TextShape,
}

/// 블록 정의의 속성 정의 (ATTDEF): 넣을 때 참조마다 이 글자를 기본값으로 붙인다 (§49)
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AttDef {
    pub tag: String,
    /// 넣을 때 묻는 말
    pub prompt: String,
    /// 기본값 글자 (블록 안 좌표, 값은 `text.value`)
    pub text: TextShape,
}

/// 블록 정의: 기준점(0,0) 기준 객체들
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Block {
    pub name: String,
    pub entities: Vec<Entity>,
    #[serde(default)]
    pub attdefs: Vec<AttDef>,
}

impl Block {
    /// 블록 참조의 변환: 배율 → 회전 → 이동
    pub fn transform(at: Vec2, scale: Vec2, rotation: f64) -> Affine {
        Affine::scale(scale.x, scale.y)
            .then(&Affine::rotate(rotation))
            .then(&Affine::translate(at.x, at.y))
    }
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

    /// 번호가 없거나(0) 앞 객체와 겹치는 객체에 새 고유 번호를 준다 (§82). 새로 준 수
    pub fn assign_ids(&mut self) -> usize {
        let max = self.entities.iter().map(|e| e.id).max().unwrap_or(0);
        self.next_id = self.next_id.max(max + 1).max(1);
        let mut seen = std::collections::HashSet::with_capacity(self.entities.len());
        let mut n = 0;
        for e in self.entities.iter_mut() {
            if e.id == 0 || !seen.insert(e.id) {
                e.id = self.next_id;
                self.next_id += 1;
                seen.insert(e.id);
                n += 1;
            }
        }
        n
    }

    /// 객체 고유 번호 → 지금 목록 번호
    pub fn index_of(&self, id: u64) -> Option<usize> {
        (id != 0).then(|| self.entities.iter().position(|e| e.id == id))?
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
                Shape::Wide { .. } => "폭 있는 폴리선",
                Shape::Wipeout { .. } => "가리개",
                Shape::Hatch { .. } => "해치 무늬",
                Shape::Insert { .. } => "블록 참조",
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

    /// 색 번호(ACI 1–255)의 RGB
    pub fn aci_rgb(i: u8) -> [u8; 3] {
        acadrust::types::Color::from_index(i as i16)
            .rgb()
            .map(|(r, g, b)| [r, g, b])
            .unwrap_or([255, 255, 255])
    }

    /// 객체의 표시 색 (객체 색이 없으면 레이어 색)
    pub fn color_of(&self, e: &Entity) -> [u8; 3] {
        e.color
            .or_else(|| self.layers.get(e.layer as usize).map(|l| l.color))
            .unwrap_or([255, 255, 255])
    }
}

#[cfg(test)]
mod wide_tests {
    use super::*;

    /// 폭 있는 폴리선 (§65): ㄱ자 폭 100 — 변마다 사각형 + 꺾인 곳 마이터, 넓이가 띠 넓이와 같다
    #[test]
    fn wide_quads_l_shape() {
        let pts = vec![
            (Vec2::new(0.0, 0.0), 0.0),
            (Vec2::new(1000.0, 0.0), 0.0),
            (Vec2::new(1000.0, 1000.0), 0.0),
        ];
        let q = wide_quads(&pts, false, &[(100.0, 100.0); 3], 1.0);
        assert_eq!(q.len(), 3);
        let area = |r: &[Vec2; 4]| {
            (0..4)
                .map(|i| r[i].x * r[(i + 1) % 4].y - r[(i + 1) % 4].x * r[i].y)
                .sum::<f64>()
                .abs()
                / 2.0
        };
        let total: f64 = q.iter().map(area).sum();
        assert!(
            (total - (2.0 * 1000.0 * 100.0 + 50.0 * 50.0)).abs() < 1e-6,
            "{total}"
        );
        assert!(
            q.iter()
                .flatten()
                .any(|p| p.dist(Vec2::new(1050.0, -50.0)) < 1e-9)
        );
        let w = Shape::Wide {
            points: pts,
            closed: false,
            widths: vec![(100.0, 100.0); 3],
        };
        match w.transformed(&crate::geom::Affine::scale(2.0, 2.0)) {
            Shape::Wide { widths, .. } => assert_eq!(widths[0], (200.0, 200.0)),
            s => panic!("{s:?}"),
        }
    }

    /// 폭이 변하는 폴리선을 고친 뒤 폭 되붙이기 (§76)
    #[test]
    fn rewiden_keeps_varying_widths() {
        let pl = |pts: &[(f64, f64, f64)]| Shape::Polyline {
            points: pts.iter().map(|&(x, y, g)| (Vec2::new(x, y), g)).collect(),
            closed: false,
        };
        let widths = |s: &Shape| match s {
            Shape::Wide { widths, .. } => widths.clone(),
            s => panic!("{s:?}"),
        };
        let close =
            |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6;
        // 화살표처럼 가늘어지는 ㄱ자: 변 0 은 1 → 3, 변 1 은 3 → 5
        let orig = Shape::Wide {
            points: vec![
                (Vec2::new(0.0, 0.0), 0.0),
                (Vec2::new(10.0, 0.0), 0.0),
                (Vec2::new(10.0, 10.0), 0.0),
            ],
            closed: false,
            widths: vec![(1.0, 3.0), (3.0, 5.0), (0.0, 0.0)],
        };
        // 자르기: 변 1 가운데에서 끝 — 잘린 자리 폭 4
        let w = widths(&rewiden(
            &orig,
            pl(&[(0.0, 0.0, 0.0), (10.0, 0.0, 0.0), (10.0, 5.0, 0.0)]),
        ));
        assert!(close(w[0], (1.0, 3.0)) && close(w[1], (3.0, 4.0)), "{w:?}");
        // 선 하나로 줄면 (변 0 의 앞 절반)
        let w = widths(&rewiden(
            &orig,
            Shape::Line {
                a: Vec2::new(0.0, 0.0),
                b: Vec2::new(5.0, 0.0),
            },
        ));
        assert!(close(w[0], (1.0, 2.0)), "{w:?}");
        // 모깎기: 꼭짓점이 늘어도 앞뒤 변은 제 폭 (호 양 끝도 그 자리 폭)
        let g = (std::f64::consts::FRAC_PI_4 / 2.0).tan();
        let w = widths(&rewiden(
            &orig,
            pl(&[
                (0.0, 0.0, 0.0),
                (8.0, 0.0, g),
                (10.0, 2.0, 0.0),
                (10.0, 10.0, 0.0),
            ]),
        ));
        assert!(close(w[0], (1.0, 2.6)) && close(w[2], (3.4, 5.0)), "{w:?}");
        // 평행 복사는 꼭짓점 수가 같으면 그대로
        let w = widths(&rewiden_parallel(
            &orig,
            pl(&[(0.0, 1.0, 0.0), (9.0, 1.0, 0.0), (9.0, 10.0, 0.0)]),
        ));
        assert_eq!(w, vec![(1.0, 3.0), (3.0, 5.0), (0.0, 0.0)]);
        // 폭이 고르면 전처럼 고정 폭
        let even = Shape::Wide {
            points: vec![(Vec2::new(0.0, 0.0), 0.0), (Vec2::new(10.0, 0.0), 0.0)],
            closed: false,
            widths: vec![(2.0, 2.0); 2],
        };
        let w = widths(&rewiden(
            &even,
            pl(&[(0.0, 0.0, 0.0), (4.0, 0.0, 0.0), (9.0, 0.0, 0.0)]),
        ));
        assert_eq!(w, vec![(2.0, 2.0); 3]);
        // 실제 간격 띄우기 경로
        let off = crate::edit::offset_shape(&orig, 0.5, Vec2::new(5.0, 5.0)).unwrap();
        assert_eq!(widths(&off), vec![(1.0, 3.0), (3.0, 5.0), (0.0, 0.0)]);
    }
}
#[cfg(test)]
mod text_style_tests {
    use super::*;

    /// 글자를 옮겨도 스타일이 남는다 (§68)
    #[test]
    fn transform_keeps_style() {
        let tx = Shape::Text(TextShape {
            at: Vec2::default(),
            height: 1.0,
            rotation: 0.0,
            width_factor: 1.0,
            halign: 0.0,
            valign: VAlign::Baseline,
            value: "회전".into(),
            style: "K1".into(),
        });
        match tx.transformed(
            &crate::geom::Affine::translate(10.0, 0.0).then(&crate::geom::Affine::scale(-1.0, 1.0)),
        ) {
            Shape::Text(t) => assert_eq!(t.style, "K1"),
            s => panic!("{s:?}"),
        }
    }
}
