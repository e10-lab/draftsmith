//! 배치(종이 공간)와 뷰포트.
//!
//! 배치를 고르면 종이 공간 객체에 뷰포트마다 모형 공간 객체를 옮겨(이동·회전·축척) 뷰포트 사각형으로 잘라
//! 넣은 「종이 좌표 도면」 하나를 만든다 ([`compose`]). 결과는 보통 [`Drawing`] 이라 화면·고르기·스냅·PDF 가
//! 그대로 쓴다 (구현 노트 §13). 잘린 객체는 선분 묶음이 되어 원래 종류·선 종류는 잃는다.

use crate::drawing::{Drawing, Entity, Shape};
use crate::geom::{Affine, Bbox, Vec2};
use crate::pick;

/// 뷰포트 하나 (종이 공간 사각형 안에 모형 공간을 보여 줌)
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ViewportView {
    /// 종이 위 가운데·크기 (종이 단위, 보통 mm)
    pub center: Vec2,
    pub width: f64,
    pub height: f64,
    /// 모형 쪽 보는 가운데 (화면 좌표계 — 대상점·비틀기 적용 뒤)
    pub view_center: Vec2,
    /// 모형 쪽 보는 높이 (모형 단위) → 축척 = 높이 / 보는 높이
    pub view_height: f64,
    /// 비틀기 (라디안)
    pub twist: f64,
    /// 대상점 (모형)
    pub target: Vec2,
    /// 이 뷰포트에서 얼린 레이어 (도면 레이어 번호)
    pub frozen: Vec<u32>,
    /// 이 뷰포트가 보여 줄 층 (없으면 시트의 층, §53)
    #[serde(default)]
    pub level: Option<u64>,
}

impl ViewportView {
    /// 모형 → 종이 변환
    pub fn transform(&self) -> Affine {
        let k = if self.view_height > 0.0 {
            self.height / self.view_height
        } else {
            1.0
        };
        Affine::translate(-self.target.x, -self.target.y)
            .then(&Affine::rotate(-self.twist))
            .then(&Affine::translate(-self.view_center.x, -self.view_center.y))
            .then(&Affine::scale(k, k))
            .then(&Affine::translate(self.center.x, self.center.y))
    }

    /// 종이 위 사각형
    pub fn rect(&self) -> Bbox {
        let mut b = Bbox::EMPTY;
        b.add(Vec2::new(
            self.center.x - self.width / 2.0,
            self.center.y - self.height / 2.0,
        ));
        b.add(Vec2::new(
            self.center.x + self.width / 2.0,
            self.center.y + self.height / 2.0,
        ));
        b
    }
}

/// 배치 하나
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Layout {
    pub name: String,
    /// 탭 순서
    pub order: i16,
    /// 종이 공간 객체 (블록은 펼침, 레이어 번호는 도면과 같음)
    pub paper: Vec<Entity>,
    pub viewports: Vec<ViewportView>,
    /// 건축 객체를 보일 층 (DraftSmith 시트 — 구현 노트 §21). None 이면 지금 층
    #[serde(default)]
    pub level: Option<u64>,
    /// 도면 목록 시트: 종이에 모든 시트의 목록 표를 그때그때 그린다 (§34)
    #[serde(default)]
    pub list: bool,
}

fn inside(b: &Bbox, p: Vec2) -> bool {
    p.x >= b.min.x && p.x <= b.max.x && p.y >= b.min.y && p.y <= b.max.y
}

fn contains(outer: &Bbox, inner: &Bbox) -> bool {
    !inner.is_empty() && inside(outer, inner.min) && inside(outer, inner.max)
}

fn disjoint(a: &Bbox, b: &Bbox) -> bool {
    a.is_empty()
        || b.is_empty()
        || a.max.x < b.min.x
        || a.min.x > b.max.x
        || a.max.y < b.min.y
        || a.min.y > b.max.y
}

/// 선분을 사각형으로 자른다 (Liang–Barsky). 밖이면 None
pub fn clip_segment(a: Vec2, b: Vec2, r: &Bbox) -> Option<(Vec2, Vec2)> {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [
        (-dx, a.x - r.min.x),
        (dx, r.max.x - a.x),
        (-dy, a.y - r.min.y),
        (dy, r.max.y - a.y),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
            if t0 > t1 {
                return None;
            }
        }
    }
    Some((
        Vec2::new(a.x + dx * t0, a.y + dy * t0),
        Vec2::new(a.x + dx * t1, a.y + dy * t1),
    ))
}

/// 사각형 한 변: 안쪽인가, 변과 만나는 점
type Keep = fn(Vec2, &Bbox) -> bool;
type Cut = fn(Vec2, Vec2, &Bbox) -> Vec2;

/// 고리를 사각형으로 자른다 (Sutherland–Hodgman). 볼록한 사각형이라 짝홀 규칙이 그대로 남는다.
pub fn clip_ring(ring: &[Vec2], r: &Bbox) -> Vec<Vec2> {
    let mut out: Vec<Vec2> = ring.to_vec();
    let edges: [(Keep, Cut); 4] = [
        (
            |p, r| p.x >= r.min.x,
            |a, b, r| a.lerp(b, (r.min.x - a.x) / (b.x - a.x)),
        ),
        (
            |p, r| p.x <= r.max.x,
            |a, b, r| a.lerp(b, (r.max.x - a.x) / (b.x - a.x)),
        ),
        (
            |p, r| p.y >= r.min.y,
            |a, b, r| a.lerp(b, (r.min.y - a.y) / (b.y - a.y)),
        ),
        (
            |p, r| p.y <= r.max.y,
            |a, b, r| a.lerp(b, (r.max.y - a.y) / (b.y - a.y)),
        ),
    ];
    for (keep, cut) in edges {
        let input = std::mem::take(&mut out);
        let n = input.len();
        for i in 0..n {
            let (a, b) = (input[i], input[(i + 1) % n]);
            match (keep(a, r), keep(b, r)) {
                (true, true) => out.push(b),
                (true, false) => out.push(cut(a, b, r)),
                (false, true) => {
                    out.push(cut(a, b, r));
                    out.push(b);
                }
                (false, false) => {}
            }
        }
    }
    out
}

/// 변환한 모양을 사각형으로 자른다. 다 들어가면 그대로, 다 밖이면 None, 걸치면 잘린 선분·채움.
fn clip_shape(s: Shape, r: &Bbox, tol: f64) -> Option<Shape> {
    let b = s.bbox();
    if disjoint(&b, r) {
        return None;
    }
    if contains(r, &b) {
        return Some(s);
    }
    match s {
        Shape::Text(t) => inside(r, t.at).then_some(Shape::Text(t)),
        Shape::Fill { loops } => {
            let loops: Vec<Vec<Vec2>> = loops
                .iter()
                .map(|l| clip_ring(l, r))
                .filter(|l| l.len() >= 3)
                .collect();
            (!loops.is_empty()).then_some(Shape::Fill { loops })
        }
        other => {
            let lines: Vec<(Vec2, Vec2)> = pick::outline(&other, tol, usize::MAX)
                .into_iter()
                .filter_map(|(a, b)| clip_segment(a, b, r))
                .collect();
            (!lines.is_empty()).then_some(Shape::Segments { lines })
        }
    }
}

/// 배치 `i` 를 종이 좌표 도면 하나로. 레이어 표·선 종류는 원래 도면과 같다.
pub fn compose(d: &Drawing, i: usize) -> Option<Drawing> {
    let layout = d.layouts.get(i)?;
    let mut out = Drawing {
        layers: d.layers.clone(),
        linetypes: d.linetypes.clone(),
        code_page: d.code_page.clone(),
        // 종이 단위는 mm 로 본다 (배치는 거의 mm)
        units: 4,
        ..Default::default()
    };
    // 뷰포트(아래) → 종이 객체(위) 순서로 쌓는다
    for v in &layout.viewports {
        let t = v.transform();
        let r = v.rect();
        let k = t.scale_x().max(1e-12);
        // 잘린 곡선 펼치기 허용 오차: 종이 위 0.05 mm
        let tol = 0.05 / k;
        for e in &d.entities {
            if v.frozen.contains(&e.layer) {
                continue;
            }
            let Some(shape) = clip_shape(e.shape.transformed(&t), &r, tol * k) else {
                continue;
            };
            let linetype = e.linetype.map(|l| crate::drawing::LineStyle {
                scale: l.scale * k,
                ..l
            });
            out.entities.push(Entity {
                shape,
                linetype,
                ..e.clone()
            });
        }
    }
    out.entities.extend(layout.paper.iter().cloned());
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawing::Drawing;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Bbox {
        let mut b = Bbox::EMPTY;
        b.add(Vec2::new(x0, y0));
        b.add(Vec2::new(x1, y1));
        b
    }

    #[test]
    fn clipping() {
        let r = rect(0.0, 0.0, 10.0, 10.0);
        let (a, b) = clip_segment(Vec2::new(-5.0, 5.0), Vec2::new(15.0, 5.0), &r).unwrap();
        assert_eq!((a, b), (Vec2::new(0.0, 5.0), Vec2::new(10.0, 5.0)));
        assert!(clip_segment(Vec2::new(-5.0, -5.0), Vec2::new(-1.0, 20.0), &r).is_none());
        let sq = [
            Vec2::new(5.0, 5.0),
            Vec2::new(15.0, 5.0),
            Vec2::new(15.0, 15.0),
            Vec2::new(5.0, 15.0),
        ];
        let c = clip_ring(&sq, &r);
        let b = {
            let mut b = Bbox::EMPTY;
            c.iter().for_each(|p| b.add(*p));
            b
        };
        assert_eq!(b, rect(5.0, 5.0, 10.0, 10.0));
    }

    #[test]
    fn viewport_scales_and_clips_model() {
        // 모형: 0..10000 가로선 하나. 뷰포트: 종이 (100,100) 가운데 200×100, 1:100 (보는 높이 10000)
        let mut d = Drawing::default();
        let layer = d.layer_index("0");
        d.entities.push(Entity {
            layer,
            aci: None,
            weight: None,
            linetype: None,
            color: None,
            shape: Shape::Line {
                a: Vec2::new(0.0, 5000.0),
                b: Vec2::new(40000.0, 5000.0),
            },
        });
        d.layouts.push(Layout {
            name: "배치1".into(),
            order: 1,
            paper: Vec::new(),
            viewports: vec![ViewportView {
                center: Vec2::new(100.0, 100.0),
                width: 200.0,
                height: 100.0,
                view_center: Vec2::new(10000.0, 5000.0),
                view_height: 10000.0,
                twist: 0.0,
                target: Vec2::default(),
                frozen: Vec::new(),
                level: None,
            }],
            level: None,
            list: false,
        });
        let c = compose(&d, 0).unwrap();
        assert_eq!(c.entities.len(), 1);
        // 모형 0..40000 → 종이 0..400 이지만 뷰포트 0..200 에서 잘림
        match &c.entities[0].shape {
            Shape::Segments { lines } => {
                let (a, b) = lines[0];
                assert!(
                    (a.x - 0.0).abs() < 1e-9
                        && (b.x - 200.0).abs() < 1e-9
                        && (a.y - 100.0).abs() < 1e-9,
                    "{a:?} {b:?}"
                );
            }
            s => panic!("{s:?}"),
        }
    }
}
