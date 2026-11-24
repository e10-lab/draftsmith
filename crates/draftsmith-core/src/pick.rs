//! 객체 고르기: 커서에서 가장 가까운 객체를 찾고, 그 객체의 속성(종류·길이·면적 등)을 계산한다.
//!
//! 객체마다 경계 상자를 R*-tree 에 넣어 후보를 줄이고, 후보는 실제 모양까지의 거리로 고른다.
//! 채움·해치는 안쪽을 누르면 거리 0. 문자는 글자 상자 안을 누르면 거리 0.

use rstar::primitives::{GeomWithData, Rectangle};
use rstar::{AABB, RTree};

use crate::curve;
use crate::drawing::{Drawing, Shape};
use crate::fill;
use crate::geom::Vec2;

type Item = GeomWithData<Rectangle<[f64; 2]>, u32>;

/// 객체 색인
pub struct PickIndex {
    tree: RTree<Item>,
}

/// 점 p 에서 선분 ab 까지 거리
pub fn seg_dist(p: Vec2, a: Vec2, b: Vec2) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let l2 = dx * dx + dy * dy;
    if l2 < 1e-300 {
        return p.dist(a);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / l2).clamp(0.0, 1.0);
    p.dist(Vec2::new(a.x + dx * t, a.y + dy * t))
}

fn chain_dist(p: Vec2, pts: &[Vec2], closed: bool) -> f64 {
    let mut d = f64::INFINITY;
    for w in pts.windows(2) {
        d = d.min(seg_dist(p, w[0], w[1]));
    }
    if closed && pts.len() > 2 {
        d = d.min(seg_dist(p, pts[pts.len() - 1], pts[0]));
    }
    if pts.len() == 1 {
        d = p.dist(pts[0]);
    }
    d
}

/// 각 θ 가 start 에서 반시계로 sweep 안에 드나
fn on_arc(theta: f64, start: f64, sweep: f64) -> bool {
    let mut t = (theta - start) % std::f64::consts::TAU;
    if t < 0.0 {
        t += std::f64::consts::TAU;
    }
    t <= sweep + 1e-12
}

/// 문자 상자 네 꼭짓점 (도면 좌표, 대략: 글자 수 × 높이 × 폭 배율)
pub fn text_box(t: &crate::drawing::TextShape) -> [Vec2; 4] {
    let lines: Vec<&str> = t.value.split('\n').collect();
    let n = lines.len().max(1) as f64;
    let w = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as f64
        * t.height
        * t.width_factor;
    let gap = t.height * 5.0 / 3.0;
    // 첫 줄 바탕선 기준 덩어리 [−h(위), (n−1)·gap(아래)] — 위쪽이 +y 인 도면 좌표로
    let top = t.height;
    let bottom = -(n - 1.0) * gap;
    let dy = match t.valign {
        crate::drawing::VAlign::Baseline => 0.0,
        crate::drawing::VAlign::Bottom => -bottom,
        crate::drawing::VAlign::Middle => -(top + bottom) / 2.0,
        crate::drawing::VAlign::Top => -top,
    };
    let x0 = -w * t.halign;
    let (s, c) = t.rotation.sin_cos();
    let at = |x: f64, y: f64| Vec2::new(t.at.x + x * c - y * s, t.at.y + x * s + y * c);
    [
        at(x0, bottom + dy),
        at(x0 + w, bottom + dy),
        at(x0 + w, top + dy),
        at(x0, top + dy),
    ]
}

/// 점 p 에서 모양까지의 거리 (`tol` 은 곡선을 펼칠 때의 허용 오차)
pub fn distance(shape: &Shape, p: Vec2, tol: f64) -> f64 {
    match shape {
        Shape::Insert { .. } => f64::INFINITY,
        Shape::Line { a, b } => seg_dist(p, *a, *b),
        Shape::Polyline { points, closed } => {
            chain_dist(p, &curve::polyline_points(points, *closed, tol), *closed)
        }
        Shape::Circle { center, radius } => (p.dist(*center) - radius).abs(),
        Shape::Arc {
            center,
            radius,
            start,
            end,
        } => {
            let theta = (p.y - center.y).atan2(p.x - center.x);
            let sweep = curve::ccw_sweep(*start, *end);
            if on_arc(theta, *start, sweep) {
                (p.dist(*center) - radius).abs()
            } else {
                let at =
                    |t: f64| Vec2::new(center.x + radius * t.cos(), center.y + radius * t.sin());
                p.dist(at(*start)).min(p.dist(at(*end)))
            }
        }
        Shape::Ellipse {
            center,
            u,
            v,
            start,
            end,
        } => {
            let mut pts = Vec::new();
            curve::ellipse_points(
                *center,
                *u,
                *v,
                *start,
                curve::ccw_sweep(*start, *end),
                tol,
                &mut pts,
            );
            chain_dist(p, &pts, false)
        }
        Shape::Fill { loops } => {
            let inside = loops.iter().filter(|r| fill::inside(p, r)).count() % 2 == 1;
            if inside {
                0.0
            } else {
                loops
                    .iter()
                    .map(|r| chain_dist(p, r, true))
                    .fold(f64::INFINITY, f64::min)
            }
        }
        Shape::Segments { lines } => lines
            .iter()
            .map(|(a, b)| seg_dist(p, *a, *b))
            .fold(f64::INFINITY, f64::min),
        Shape::Text(t) => {
            let b = text_box(t);
            if fill::inside(p, &b) {
                0.0
            } else {
                chain_dist(p, &b, true)
            }
        }
    }
}

impl PickIndex {
    /// 상자 고르기: `crossing` 이 거짓이면 상자 안에 다 든 객체(창), 참이면 상자에 걸친 객체도(걸침)
    pub fn in_box(&self, d: &Drawing, rect: &crate::geom::Bbox, crossing: bool) -> Vec<usize> {
        let env = AABB::from_corners([rect.min.x, rect.min.y], [rect.max.x, rect.max.y]);
        let tol = (rect.width().max(rect.height()) / 2000.0).max(1e-9);
        let mut out: Vec<usize> = self
            .tree
            .locate_in_envelope_intersecting(env)
            .filter_map(|it| {
                let i = it.data as usize;
                let e = &d.entities[i];
                let b = match &e.shape {
                    Shape::Text(t) => {
                        let mut bb = crate::geom::Bbox::EMPTY;
                        text_box(t).iter().for_each(|p| bb.add(*p));
                        bb
                    }
                    s => s.bbox(),
                };
                let inside = b.min.x >= rect.min.x
                    && b.max.x <= rect.max.x
                    && b.min.y >= rect.min.y
                    && b.max.y <= rect.max.y;
                if inside {
                    return Some(i);
                }
                if !crossing {
                    return None;
                }
                // 걸침: 외곽 선분이 상자에 닿거나, 채움·문자 안에 상자가 들었으면
                let hit = outline(&e.shape, tol, 200_000)
                    .into_iter()
                    .any(|(a, b)| crate::layout::clip_segment(a, b, rect).is_some())
                    || distance(&e.shape, rect.center(), tol) == 0.0;
                hit.then_some(i)
            })
            .collect();
        out.sort_unstable();
        out
    }

    /// 켜진 레이어 객체만 넣는다
    pub fn build(d: &Drawing) -> PickIndex {
        let items: Vec<Item> = d
            .entities
            .iter()
            .enumerate()
            .filter(|(_, e)| d.layers.get(e.layer as usize).is_none_or(|l| l.visible))
            .filter_map(|(i, e)| {
                let b = match &e.shape {
                    Shape::Text(t) => {
                        let mut bb = crate::geom::Bbox::EMPTY;
                        for p in text_box(t) {
                            bb.add(p);
                        }
                        bb
                    }
                    s => s.bbox(),
                };
                (!b.is_empty()).then(|| {
                    GeomWithData::new(
                        Rectangle::from_corners([b.min.x, b.min.y], [b.max.x, b.max.y]),
                        i as u32,
                    )
                })
            })
            .collect();
        PickIndex {
            tree: RTree::bulk_load(items),
        }
    }

    /// `p` 에서 `radius` 안의 가장 가까운 객체 번호. 같은 거리면 나중에 그린(위에 있는) 객체.
    pub fn pick(&self, d: &Drawing, p: Vec2, radius: f64) -> Option<usize> {
        let env = AABB::from_corners([p.x - radius, p.y - radius], [p.x + radius, p.y + radius]);
        let tol = radius / 4.0;
        let mut best: Option<(f64, usize)> = None;
        for it in self.tree.locate_in_envelope_intersecting(env) {
            let i = it.data as usize;
            let dist = distance(&d.entities[i].shape, p, tol);
            if dist <= radius && best.is_none_or(|(bd, bi)| dist < bd || (dist == bd && i > bi)) {
                best = Some((dist, i));
            }
        }
        best.map(|(_, i)| i)
    }
}

/// 고리 넓이 (부호 없음)
fn ring_area(r: &[Vec2]) -> f64 {
    let n = r.len();
    ((0..n)
        .map(|i| r[i].x * r[(i + 1) % n].y - r[(i + 1) % n].x * r[i].y)
        .sum::<f64>()
        / 2.0)
        .abs()
}

fn chain_len(pts: &[Vec2], closed: bool) -> f64 {
    let mut l: f64 = pts.windows(2).map(|w| w[0].dist(w[1])).sum();
    if closed && pts.len() > 2 {
        l += pts[pts.len() - 1].dist(pts[0]);
    }
    l
}

/// 면적을 ㎡·평으로 (도면 단위에 따라). 단위가 없으면 mm 로 본다고 밝힌다.
fn area_m2(d: &Drawing, a: f64) -> (String, String) {
    let (m, assumed) = d.meters_per_unit();
    let m2 = a * m * m;
    let label = if assumed {
        "면적 (mm 로 봄)"
    } else {
        "면적 (㎡)"
    };
    (
        label.into(),
        format!("{:.2} ㎡ · {:.2} 평", m2, m2 / 3.305_785),
    )
}

/// 객체 속성: (이름, 값) 목록. 화면의 속성 칸에 그대로 보인다.
pub fn properties(d: &Drawing, i: usize) -> Vec<(String, String)> {
    let e = &d.entities[i];
    let f = |v: f64| format!("{:.1}", v);
    let deg = |r: f64| format!("{:.2}°", r.to_degrees().rem_euclid(360.0));
    let mut out: Vec<(String, String)> = Vec::new();
    let kind = match &e.shape {
        Shape::Insert { .. } => "블록 참조",
        Shape::Line { .. } => "선",
        Shape::Polyline { closed: true, .. } => "닫힌 폴리선",
        Shape::Polyline { .. } => "폴리선",
        Shape::Circle { .. } => "원",
        Shape::Arc { .. } => "호",
        Shape::Ellipse { .. } => "타원",
        Shape::Fill { .. } => "채움 (솔리드·해치)",
        Shape::Segments { .. } => "해치 무늬",
        Shape::Text(_) => "문자",
    };
    out.push(("종류".into(), kind.into()));
    if let Some(l) = d.layers.get(e.layer as usize) {
        out.push(("레이어".into(), l.name.clone()));
    }
    let [r, g, b] = d.color_of(e);
    out.push((
        "색".into(),
        format!(
            "RGB {r}, {g}, {b}{}",
            if e.color.is_none() {
                " (레이어)"
            } else {
                ""
            }
        ),
    ));
    match &e.shape {
        Shape::Insert {
            block,
            at,
            scale,
            rotation,
        } => {
            out.push(("블록".into(), block.clone()));
            out.push(("삽입점".into(), format!("{}, {}", f(at.x), f(at.y))));
            out.push(("배율".into(), format!("{} × {}", f(scale.x), f(scale.y))));
            out.push(("회전".into(), deg(*rotation)));
        }
        Shape::Line { a, b } => {
            out.push(("길이".into(), f(a.dist(*b))));
            out.push(("각도".into(), deg((b.y - a.y).atan2(b.x - a.x))));
            out.push(("시작점".into(), format!("{}, {}", f(a.x), f(a.y))));
            out.push(("끝점".into(), format!("{}, {}", f(b.x), f(b.y))));
        }
        Shape::Polyline { points, closed } => {
            let pts = curve::polyline_points(points, *closed, 1e-3);
            out.push(("꼭짓점".into(), points.len().to_string()));
            out.push(("길이".into(), f(chain_len(&pts, *closed))));
            if *closed {
                let a = ring_area(&pts);
                out.push(("면적".into(), f(a)));
                out.push(area_m2(d, a));
            }
        }
        Shape::Circle { center, radius } => {
            out.push(("중심".into(), format!("{}, {}", f(center.x), f(center.y))));
            out.push(("반지름".into(), f(*radius)));
            out.push(("둘레".into(), f(std::f64::consts::TAU * radius)));
            let a = std::f64::consts::PI * radius * radius;
            out.push(("면적".into(), f(a)));
            out.push(area_m2(d, a));
        }
        Shape::Arc {
            center,
            radius,
            start,
            end,
        } => {
            let sweep = curve::ccw_sweep(*start, *end);
            out.push(("중심".into(), format!("{}, {}", f(center.x), f(center.y))));
            out.push(("반지름".into(), f(*radius)));
            out.push(("사이각".into(), deg(sweep)));
            out.push(("호 길이".into(), f(radius * sweep)));
        }
        Shape::Ellipse { center, u, v, .. } => {
            out.push(("중심".into(), format!("{}, {}", f(center.x), f(center.y))));
            out.push((
                "반지름 (켤레)".into(),
                format!("{} / {}", f(u.x.hypot(u.y)), f(v.x.hypot(v.y))),
            ));
        }
        Shape::Fill { loops } => {
            // 짝홀 규칙 면적 = 삼각형 넓이 합
            let t = fill::triangulate(loops);
            let a: f64 = t.as_chunks::<3>().0.iter().map(|c| ring_area(&c[..])).sum();
            out.push(("고리".into(), loops.len().to_string()));
            out.push(("면적".into(), f(a)));
            out.push(area_m2(d, a));
        }
        Shape::Segments { lines } => {
            out.push(("선분".into(), lines.len().to_string()));
        }
        Shape::Text(t) => {
            out.push(("내용".into(), t.value.clone()));
            out.push(("높이".into(), f(t.height)));
            out.push(("방향".into(), deg(t.rotation)));
            if (t.width_factor - 1.0).abs() > 1e-9 {
                out.push(("폭 배율".into(), format!("{:.2}", t.width_factor)));
            }
        }
    }
    out
}

/// 강조 표시용 선분 목록 (도면 좌표). 너무 많으면 `limit` 개에서 자른다.
pub fn outline(shape: &Shape, tol: f64, limit: usize) -> Vec<(Vec2, Vec2)> {
    let chain = |pts: &[Vec2], closed: bool| {
        let mut v: Vec<(Vec2, Vec2)> = pts.windows(2).map(|w| (w[0], w[1])).collect();
        if closed && pts.len() > 2 {
            v.push((pts[pts.len() - 1], pts[0]));
        }
        v
    };
    let mut out = match shape {
        Shape::Insert { .. } => Vec::new(),
        Shape::Line { a, b } => vec![(*a, *b)],
        Shape::Polyline { points, closed } => {
            chain(&curve::polyline_points(points, *closed, tol), *closed)
        }
        Shape::Circle { center, radius } => {
            let mut p = Vec::new();
            let (u, v) = (Vec2::new(*radius, 0.0), Vec2::new(0.0, *radius));
            curve::ellipse_points(*center, u, v, 0.0, std::f64::consts::TAU, tol, &mut p);
            chain(&p, false)
        }
        Shape::Arc {
            center,
            radius,
            start,
            end,
        } => {
            let mut p = Vec::new();
            let (u, v) = (Vec2::new(*radius, 0.0), Vec2::new(0.0, *radius));
            curve::ellipse_points(
                *center,
                u,
                v,
                *start,
                curve::ccw_sweep(*start, *end),
                tol,
                &mut p,
            );
            chain(&p, false)
        }
        Shape::Ellipse {
            center,
            u,
            v,
            start,
            end,
        } => {
            let mut p = Vec::new();
            curve::ellipse_points(
                *center,
                *u,
                *v,
                *start,
                curve::ccw_sweep(*start, *end),
                tol,
                &mut p,
            );
            chain(&p, false)
        }
        Shape::Fill { loops } => loops.iter().flat_map(|r| chain(r, true)).collect(),
        Shape::Segments { lines } => lines.clone(),
        Shape::Text(t) => chain(&text_box(t), true),
    };
    out.truncate(limit);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawing::{Drawing, Entity, TextShape, VAlign};

    fn drawing() -> Drawing {
        let mut d = Drawing::default();
        let layer = d.layer_index("A-WALL");
        let mut put = |shape| {
            d.entities.push(Entity {
                layer,
                aci: None,
                weight: None,
                linetype: None,
                color: None,
                shape,
            })
        };
        put(Shape::Line {
            a: Vec2::new(0.0, 0.0),
            b: Vec2::new(100.0, 0.0),
        });
        put(Shape::Circle {
            center: Vec2::new(50.0, 50.0),
            radius: 10.0,
        });
        put(Shape::Fill {
            loops: vec![vec![
                Vec2::new(200.0, 0.0),
                Vec2::new(300.0, 0.0),
                Vec2::new(300.0, 100.0),
                Vec2::new(200.0, 100.0),
            ]],
        });
        put(Shape::Text(TextShape {
            at: Vec2::new(0.0, 200.0),
            height: 10.0,
            rotation: 0.0,
            width_factor: 1.0,
            halign: 0.0,
            valign: VAlign::Baseline,
            value: "거실".into(),
        }));
        d
    }

    #[test]
    fn picks_nearest_shape() {
        let d = drawing();
        let idx = PickIndex::build(&d);
        assert_eq!(idx.pick(&d, Vec2::new(30.0, 1.0), 2.0), Some(0));
        assert_eq!(idx.pick(&d, Vec2::new(50.0, 61.0), 2.0), Some(1));
        // 원 안쪽 한가운데는 원 선에서 멀다
        assert_eq!(idx.pick(&d, Vec2::new(50.0, 50.0), 2.0), None);
        // 채움은 안쪽을 눌러도 고른다
        assert_eq!(idx.pick(&d, Vec2::new(250.0, 50.0), 2.0), Some(2));
        // 문자 상자 안
        assert_eq!(idx.pick(&d, Vec2::new(5.0, 205.0), 1.0), Some(3));
    }

    #[test]
    fn properties_of_fill_and_line() {
        let d = drawing();
        let p = properties(&d, 2);
        assert!(p.contains(&("면적".into(), "10000.0".into())), "{p:?}");
        // 단위 없음 → mm 로 봄: 10000 mm² = 0.01 ㎡
        assert!(
            p.contains(&("면적 (mm 로 봄)".into(), "0.01 ㎡ · 0.00 평".into())),
            "{p:?}"
        );
        let p = properties(&d, 0);
        assert!(p.contains(&("길이".into(), "100.0".into())), "{p:?}");
        assert!(p.contains(&("레이어".into(), "A-WALL".into())), "{p:?}");
    }
}
