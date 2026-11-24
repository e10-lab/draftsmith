//! 도면 모델.
//!
//! 지금은 가져온 DXF·DWG 를 보여 주는 데 필요한 작도 객체만 있다 (M0·M1).
//! 건축 객체(벽·문·창·실·그리드·층)는 M3 에서 같은 문서에 더한다 — 로드맵 참고.

use std::collections::BTreeMap;

use crate::geom::{Bbox, Vec2};

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
    /// 한 줄 문자. 글꼴 처리는 M0 ⑥ 에서 — 지금은 위치·크기만 쓴다.
    Text {
        at: Vec2,
        height: f64,
        rotation: f64,
        value: String,
    },
}

impl Shape {
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
    /// 가져오지 못한 객체 종류와 개수 (예: `Insert` → 120). 화면·CLI 에 알린다
    pub skipped: BTreeMap<String, usize>,
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
