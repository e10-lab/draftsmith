//! 그립 편집 (M2, 구현 노트 §33): 고른 객체의 손잡이 점과, 그 점을 옮긴 새 모양.
//!
//! 순서(번호)는 모양마다 고정이다 — 화면은 번호를 들고 있다가 옮길 곳과 함께 돌려준다.
//! - 선: 0 시작, 1 끝, 2 가운데(통째로 옮김)
//! - 폴리선: 꼭짓점마다
//! - 원: 0 중심(옮김), 1 오른쪽 사분점(반지름)
//! - 호: 0 중심(옮김), 1 시작, 2 끝, 3 가운데 (끝·가운데는 세 점을 지나는 호로 다시)
//! - 타원·문자·블록 참조: 0 기준점(옮김)

use std::f64::consts::TAU;

use crate::curve::ccw_sweep;
use crate::drawing::Shape;
use crate::geom::{Affine, Vec2};

/// 손잡이 점들
pub fn grips(s: &Shape) -> Vec<Vec2> {
    match s {
        Shape::Line { a, b } => vec![*a, *b, a.lerp(*b, 0.5)],
        Shape::Polyline { points, .. } => points.iter().map(|(p, _)| *p).collect(),
        Shape::Circle { center, radius } => vec![*center, Vec2::new(center.x + radius, center.y)],
        Shape::Arc {
            center,
            radius,
            start,
            end,
        } => {
            let at = |t: f64| Vec2::new(center.x + radius * t.cos(), center.y + radius * t.sin());
            let mid = start + ccw_sweep(*start, *end) / 2.0;
            vec![*center, at(*start), at(*end), at(mid)]
        }
        Shape::Ellipse { center, .. } => vec![*center],
        Shape::Text(t) => vec![t.at],
        Shape::Insert { at, .. } => vec![*at],
        Shape::Fill { .. } | Shape::Segments { .. } => Vec::new(),
    }
}

/// 세 점을 지나는 원 (중심, 반지름)
fn circle3(a: Vec2, b: Vec2, c: Vec2) -> Option<(Vec2, f64)> {
    let d = 2.0 * (a.x * (b.y - c.y) + b.x * (c.y - a.y) + c.x * (a.y - b.y));
    if d.abs() < 1e-12 {
        return None;
    }
    let s = |p: Vec2| p.x * p.x + p.y * p.y;
    let ux = (s(a) * (b.y - c.y) + s(b) * (c.y - a.y) + s(c) * (a.y - b.y)) / d;
    let uy = (s(a) * (c.x - b.x) + s(b) * (a.x - c.x) + s(c) * (b.x - a.x)) / d;
    let o = Vec2::new(ux, uy);
    Some((o, o.dist(a)))
}

/// 시작·지나는 점·끝 → 반시계 호
pub fn arc3(a: Vec2, m: Vec2, e: Vec2) -> Option<Shape> {
    let (o, r) = circle3(a, m, e)?;
    let ang = |p: Vec2| (p.y - o.y).atan2(p.x - o.x);
    let (mut s, mut t) = (ang(a), ang(e));
    if ccw_sweep(s, ang(m)) > ccw_sweep(s, t) {
        std::mem::swap(&mut s, &mut t);
    }
    Some(Shape::Arc {
        center: o,
        radius: r,
        start: s.rem_euclid(TAU),
        end: t.rem_euclid(TAU),
    })
}

/// 손잡이 k 를 `to` 로 옮긴 모양 (못 옮기면 None)
pub fn move_grip(s: &Shape, k: usize, to: Vec2) -> Option<Shape> {
    let g = grips(s);
    let from = *g.get(k)?;
    let shift = |s: &Shape| s.transformed(&Affine::translate(to.x - from.x, to.y - from.y));
    Some(match s {
        Shape::Line { a, b } => match k {
            0 => Shape::Line { a: to, b: *b },
            1 => Shape::Line { a: *a, b: to },
            _ => shift(s),
        },
        Shape::Polyline { points, closed } => {
            let mut p = points.clone();
            p[k].0 = to;
            Shape::Polyline {
                points: p,
                closed: *closed,
            }
        }
        Shape::Circle { center, .. } => match k {
            0 => shift(s),
            _ => Shape::Circle {
                center: *center,
                radius: center.dist(to).max(1e-9),
            },
        },
        Shape::Arc { .. } => match k {
            0 => shift(s),
            _ => {
                let (mut a, mut e, mut m) = (g[1], g[2], g[3]);
                match k {
                    1 => a = to,
                    2 => e = to,
                    _ => m = to,
                }
                arc3(a, m, e)?
            }
        },
        _ => shift(s),
    })
}

/// 반지름이 0 에 가까운 원 같은 쓸모없는 모양인가
pub fn degenerate(s: &Shape) -> bool {
    match s {
        Shape::Line { a, b } => a.dist(*b) < 1e-9,
        Shape::Circle { radius, .. } | Shape::Arc { radius, .. } => {
            *radius < 1e-9 || !radius.is_finite()
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn line_and_circle_grips() {
        let l = Shape::Line {
            a: Vec2::new(0.0, 0.0),
            b: Vec2::new(10.0, 0.0),
        };
        assert_eq!(grips(&l).len(), 3);
        assert_eq!(
            move_grip(&l, 1, Vec2::new(5.0, 5.0)),
            Some(Shape::Line {
                a: Vec2::new(0.0, 0.0),
                b: Vec2::new(5.0, 5.0)
            })
        );
        assert_eq!(
            move_grip(&l, 2, Vec2::new(5.0, 3.0)),
            Some(Shape::Line {
                a: Vec2::new(0.0, 3.0),
                b: Vec2::new(10.0, 3.0)
            })
        );
        let c = Shape::Circle {
            center: Vec2::default(),
            radius: 1.0,
        };
        assert_eq!(
            move_grip(&c, 1, Vec2::new(0.0, 4.0)),
            Some(Shape::Circle {
                center: Vec2::default(),
                radius: 4.0
            })
        );
    }

    #[test]
    fn arc_end_grip_keeps_through_point() {
        // 반지름 10 위쪽 반원, 끝점을 옮겨도 가운데(0,10)를 지난다
        let a = Shape::Arc {
            center: Vec2::default(),
            radius: 10.0,
            start: 0.0,
            end: PI,
        };
        let n = move_grip(&a, 2, Vec2::new(-6.0, 8.0)).unwrap();
        match n {
            Shape::Arc { center, radius, .. } => {
                assert!(center.dist(Vec2::default()) < 1e-9 && (radius - 10.0).abs() < 1e-9);
                let g = grips(&Shape::Arc {
                    center,
                    radius,
                    start: 0.0,
                    end: 0.0,
                });
                assert_eq!(g.len(), 4);
            }
            s => panic!("{s:?}"),
        }
    }
}
