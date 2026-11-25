//! 모형 공간의 도면 틀 찾기 (구현 노트 §63).
//!
//! 국내 실무 도면은 배치(종이 공간) 대신 모형 공간에 도면 틀 블록을 여러 장 나란히 두고 그 안에 그리는 일이 흔하다.
//! 틀마다 한 쪽씩 PDF 로 내려면 틀을 찾아 용지·축척을 알아내야 한다.
//! - 틀: 두 번 이상 넣은 블록 중 종이 비율(√2, ±3 %)인 가장 큰 것. 없으면 같은 크기로 두 번 이상 나온 종이 비율 사각형.
//! - 용지·축척: 틀 크기를 A4~A0(가로·세로) × 표준 축척 중 가장 가까운 조합으로 (오차 2 % 안).

use crate::drawing::{Block, Drawing, Shape};
use crate::geom::{Bbox, Vec2};

/// 찾은 도면 틀 하나
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// 틀 범위 (도면 좌표)
    pub area: Bbox,
    /// 용지 이름 (A3 등)·크기 (mm, 가로 × 세로)
    pub paper: &'static str,
    pub paper_mm: (f64, f64),
    /// 축척 분모
    pub scale: f64,
}

const PAPERS: [(&str, f64, f64); 5] = [
    ("A4", 297.0, 210.0),
    ("A3", 420.0, 297.0),
    ("A2", 594.0, 420.0),
    ("A1", 841.0, 594.0),
    ("A0", 1189.0, 841.0),
];

const SCALES: [f64; 28] = [
    1.0, 2.0, 5.0, 10.0, 20.0, 25.0, 30.0, 40.0, 50.0, 60.0, 75.0, 80.0, 100.0, 120.0, 150.0,
    200.0, 250.0, 300.0, 400.0, 500.0, 600.0, 750.0, 800.0, 1000.0, 1200.0, 1500.0, 2000.0, 3000.0,
];

/// 종이 비율(√2)인가 (가로·세로 어느 쪽이든)
fn paper_ratio(w: f64, h: f64) -> bool {
    if w <= 0.0 || h <= 0.0 {
        return false;
    }
    let r = w.max(h) / w.min(h);
    (r / std::f64::consts::SQRT_2 - 1.0).abs() < 0.03
}

/// 틀 크기(도면 단위, mm) → 가장 가까운 (용지, 축척)
pub fn fit_paper(w: f64, h: f64, mm_per_unit: f64) -> Option<(&'static str, (f64, f64), f64)> {
    let (w, h) = (w * mm_per_unit, h * mm_per_unit);
    let mut best: Option<(f64, &'static str, (f64, f64), f64)> = None;
    for (name, pw, ph) in PAPERS {
        for (a, b) in [(pw, ph), (ph, pw)] {
            for s in SCALES {
                let e = ((w / s - a) / a).abs().max(((h / s - b) / b).abs());
                if best.is_none_or(|x| e < x.0) {
                    best = Some((e, name, (a, b), s));
                }
            }
        }
    }
    best.filter(|b| b.0 < 0.02).map(|b| (b.1, b.2, b.3))
}

/// 블록 참조의 범위 (블록 정의를 참조 변환으로)
fn insert_box(d: &Drawing, block: &str, at: Vec2, scale: Vec2, rotation: f64) -> Option<Bbox> {
    let b = d.blocks.iter().find(|b| b.name == block)?;
    let t = Block::transform(at, scale, rotation);
    let mut bb = Bbox::EMPTY;
    for e in &b.entities {
        bb.merge(&e.shape.transformed(&t).bbox());
    }
    (!bb.is_empty()).then_some(bb)
}

/// 모형 공간의 도면 틀들 (위 줄부터, 줄 안에서는 왼쪽부터)
pub fn find(d: &Drawing) -> Vec<Frame> {
    let (m_per_unit, _) = d.meters_per_unit();
    let mm = m_per_unit * 1000.0;
    // 1) 두 번 이상 넣은 종이 비율 블록 — 블록별로 모아 가장 큰 것
    let mut by_block: Vec<(String, Vec<Bbox>)> = Vec::new();
    for e in &d.entities {
        if let Shape::Insert {
            block,
            at,
            scale,
            rotation,
            ..
        } = &e.shape
            && let Some(bb) = insert_box(d, block, *at, *scale, *rotation)
            && paper_ratio(bb.width(), bb.height())
        {
            match by_block.iter_mut().find(|(n, _)| n == block) {
                Some((_, v)) => v.push(bb),
                None => by_block.push((block.clone(), vec![bb])),
            }
        }
    }
    let area = |b: &Bbox| b.width() * b.height();
    let mut boxes: Vec<Bbox> = by_block
        .into_iter()
        .filter(|(_, v)| v.len() >= 2)
        .max_by(|a, b| area(&a.1[0]).total_cmp(&area(&b.1[0])))
        .map(|x| x.1)
        .unwrap_or_default();
    // 2) 없으면 같은 크기로 두 번 이상 나온 종이 비율 사각형 (닫힌 4 점 폴리선)
    if boxes.is_empty() {
        let mut rects: Vec<Bbox> = d
            .entities
            .iter()
            .filter_map(|e| match &e.shape {
                Shape::Polyline {
                    points,
                    closed: true,
                } if points.len() == 4 => {
                    let b = e.shape.bbox();
                    paper_ratio(b.width(), b.height()).then_some(b)
                }
                _ => None,
            })
            .collect();
        rects.sort_by(|a, b| area(b).total_cmp(&area(a)));
        if let Some(top) = rects.first() {
            let same: Vec<Bbox> = rects
                .iter()
                .filter(|b| {
                    (b.width() - top.width()).abs() < top.width() * 0.01
                        && (b.height() - top.height()).abs() < top.height() * 0.01
                })
                .copied()
                .collect();
            if same.len() >= 2 {
                boxes = same;
            }
        }
    }
    // 겹치는 것(같은 자리 두 번)은 하나로
    boxes.dedup_by(|a, b| a.min.dist(b.min) < 1e-6 && a.max.dist(b.max) < 1e-6);
    // 위 줄부터 (틀 높이 반 이상 차이 나면 다른 줄), 왼쪽부터
    let h = boxes.first().map(|b| b.height()).unwrap_or(1.0);
    boxes.sort_by(|a, b| {
        let (ra, rb) = (
            (-a.max.y / (h / 2.0)).round(),
            (-b.max.y / (h / 2.0)).round(),
        );
        ra.total_cmp(&rb).then(a.min.x.total_cmp(&b.min.x))
    });
    boxes
        .into_iter()
        .filter_map(|b| {
            let (paper, paper_mm, scale) = fit_paper(b.width(), b.height(), mm)?;
            Some(Frame {
                area: b,
                paper,
                paper_mm,
                scale,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawing::Entity;

    #[test]
    fn paper_fit() {
        assert_eq!(
            fit_paper(42000.0, 29700.0, 1.0),
            Some(("A3", (420.0, 297.0), 100.0))
        );
        assert_eq!(
            fit_paper(29700.0, 42000.0, 1.0),
            Some(("A3", (297.0, 420.0), 100.0))
        );
        assert_eq!(fit_paper(84100.0, 59400.0, 1.0).map(|x| x.2), Some(100.0));
        assert_eq!(fit_paper(1234.0, 567.0, 1.0), None);
    }

    /// 틀 블록을 두 번 넣으면 두 틀, 왼쪽부터
    #[test]
    fn frames_from_repeated_block() {
        let mut d = crate::archgen::new_drawing();
        let rect = |w: f64, h: f64| Shape::Polyline {
            points: vec![
                (Vec2::new(0.0, 0.0), 0.0),
                (Vec2::new(w, 0.0), 0.0),
                (Vec2::new(w, h), 0.0),
                (Vec2::new(0.0, h), 0.0),
            ],
            closed: true,
        };
        let ent = |shape: Shape| Entity {
            layer: 0,
            color: None,
            aci: None,
            linetype: None,
            weight: None,
            shape,
        };
        d.blocks.push(Block {
            name: "TITLE".into(),
            entities: vec![ent(rect(420.0, 297.0))],
            attdefs: Vec::new(),
        });
        let ins = |x: f64| {
            ent(Shape::Insert {
                block: "TITLE".into(),
                at: Vec2::new(x, 0.0),
                scale: Vec2::new(100.0, 100.0),
                rotation: 0.0,
                attrs: Vec::new(),
            })
        };
        d.entities = vec![ins(50000.0), ins(0.0), ent(rect(9000.0, 1000.0))];
        let f = find(&d);
        assert_eq!(f.len(), 2);
        assert_eq!((f[0].paper, f[0].scale), ("A3", 100.0));
        assert!(f[0].area.min.x.abs() < 1e-6 && (f[1].area.min.x - 50000.0).abs() < 1e-6);
        // 블록이 없으면 사각형으로
        let mut d2 = crate::archgen::new_drawing();
        d2.entities = vec![
            ent(rect(42000.0, 29700.0)),
            ent(rect(42000.0, 29700.0).transformed(&crate::geom::Affine::translate(0.0, -40000.0))),
        ];
        let f2 = find(&d2);
        assert_eq!(f2.len(), 2);
        assert!(f2[0].area.max.y > f2[1].area.max.y);
    }
}
