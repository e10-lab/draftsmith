//! 가리개 (WIPEOUT, 구현 노트 §60).
//!
//! AutoCAD 는 그리기 순서대로 그리고, 가리개는 바탕색 채움이라 **그보다 앞에 그린** 객체를 가린다 (뒤에 그린 것은 위에 보임).
//! 화면은 채움을 먼저·선을 나중에 한꺼번에 그려 순서를 지키지 못하므로, 보이는 도면 단계에서 가리개 앞의 선·곡선을
//! 가리개 고리 밖 부분만 남기도록 잘라 낸다. 가리개 자신은 빈 선분 묶음으로 남겨 객체 번호를 지킨다.
//! 글자·채움은 자르지 않는다 (글자 뒤 가리개는 보통 글자보다 뒤에 그려 글자 밑 선을 지우는 용도).

use crate::curve;
use crate::drawing::{Drawing, Entity, Shape};
use crate::fill;
use crate::geom::{Bbox, Vec2};

/// 모양 → 꺾은선들 (자를 수 있는 것만)
fn chains(s: &Shape, tol: f64) -> Option<Vec<Vec<Vec2>>> {
    Some(match s {
        Shape::Line { a, b } => vec![vec![*a, *b]],
        Shape::Polyline { points, closed } => vec![curve::polyline_points(points, *closed, tol)],
        Shape::Circle { center, radius } => {
            let mut p = Vec::new();
            curve::ellipse_points(
                *center,
                Vec2::new(*radius, 0.0),
                Vec2::new(0.0, *radius),
                0.0,
                std::f64::consts::TAU,
                tol,
                &mut p,
            );
            vec![p]
        }
        Shape::Arc {
            center,
            radius,
            start,
            end,
        } => {
            let mut p = Vec::new();
            curve::ellipse_points(
                *center,
                Vec2::new(*radius, 0.0),
                Vec2::new(0.0, *radius),
                *start,
                curve::ccw_sweep(*start, *end),
                tol,
                &mut p,
            );
            vec![p]
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
            vec![p]
        }
        Shape::Segments { lines } => lines.iter().map(|(a, b)| vec![*a, *b]).collect(),
        _ => return None,
    })
}

/// 선분 a→b 와 선분 c→d 의 교점 매개변수 (a→b 위, 0..1)
fn cross_t(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> Option<f64> {
    let (r, s) = (
        Vec2::new(b.x - a.x, b.y - a.y),
        Vec2::new(d.x - c.x, d.y - c.y),
    );
    let den = r.x * s.y - r.y * s.x;
    if den.abs() < 1e-18 {
        return None;
    }
    let (qx, qy) = (c.x - a.x, c.y - a.y);
    let t = (qx * s.y - qy * s.x) / den;
    let u = (qx * r.y - qy * r.x) / den;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then_some(t)
}

/// 꺾은선들에서 고리 안 부분을 지운 선분들. 하나도 안 잘렸으면 None
fn cut(chs: &[Vec<Vec2>], ring: &[Vec2]) -> Option<Vec<(Vec2, Vec2)>> {
    let n = ring.len();
    let mut out = Vec::new();
    let mut changed = false;
    for ch in chs {
        for w in ch.windows(2) {
            let (a, b) = (w[0], w[1]);
            let mut ts = vec![0.0, 1.0];
            for i in 0..n {
                if let Some(t) = cross_t(a, b, ring[i], ring[(i + 1) % n]) {
                    ts.push(t);
                }
            }
            ts.sort_by(f64::total_cmp);
            for p in ts.windows(2) {
                if p[1] - p[0] < 1e-12 {
                    continue;
                }
                let at = |t: f64| Vec2::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
                if fill::inside(at((p[0] + p[1]) / 2.0), ring) {
                    changed = true;
                } else {
                    out.push((at(p[0]), at(p[1])));
                }
            }
        }
    }
    changed.then_some(out)
}

/// 가리개 하나 (고리와 그 범위)
#[derive(Debug, Clone, PartialEq)]
pub struct Wipe {
    pub ring: Vec<Vec2>,
    pub bbox: Bbox,
}

/// 가리개 모양이면 그 고리 (점 셋 이상)
pub fn wipe_of(s: &Shape) -> Option<Wipe> {
    match s {
        Shape::Wipeout { ring } if ring.len() >= 3 => {
            let mut b = Bbox::EMPTY;
            ring.iter().for_each(|p| b.add(*p));
            Some(Wipe {
                ring: ring.clone(),
                bbox: b,
            })
        }
        _ => None,
    }
}

/// 객체 하나를 그보다 뒤(위)에 그린 가리개들로 자른다. 잘렸으면 참. 가리개 자신은 자르지 않는다
pub fn clip<'a>(e: &mut Entity, later: impl IntoIterator<Item = &'a Wipe>) -> bool {
    if matches!(e.shape, Shape::Wipeout { .. }) {
        return false;
    }
    let eb = e.shape.bbox();
    let mut over = later
        .into_iter()
        .filter(|w| {
            let b = &w.bbox;
            b.min.x <= eb.max.x && b.max.x >= eb.min.x && b.min.y <= eb.max.y && b.max.y >= eb.min.y
        })
        .peekable();
    if over.peek().is_none() {
        return false;
    }
    let tol = (eb.width().max(eb.height()) * 1e-3).max(1e-6);
    let Some(mut chs) = chains(&e.shape, tol) else {
        return false;
    };
    let mut any = false;
    for w in over {
        if let Some(segs) = cut(&chs, &w.ring) {
            any = true;
            chs = segs.into_iter().map(|(a, b)| vec![a, b]).collect();
        }
    }
    if any {
        e.shape = Shape::Segments {
            lines: chs.into_iter().map(|c| (c[0], c[1])).collect(),
        };
    }
    any
}

/// 가리개 자신은 보이지 않게 (번호는 그대로)
pub fn hide(e: &mut Entity) {
    if matches!(e.shape, Shape::Wipeout { .. }) {
        e.shape = Shape::Segments { lines: Vec::new() };
    }
}

/// 보이는 도면의 가리개를 풀어 앞의 선을 자른다. 자른 객체 수
pub fn apply(d: &mut Drawing) -> usize {
    let wipes: Vec<(usize, Wipe)> = d
        .entities
        .iter()
        .enumerate()
        .filter_map(|(i, e)| wipe_of(&e.shape).map(|w| (i, w)))
        .collect();
    let mut n = 0;
    if let Some(last) = wipes.last().map(|w| w.0) {
        // 마지막 가리개보다 앞의 객체만 (자기보다 뒤·위에 있는 가리개만 가린다)
        for j in 0..last {
            let later = wipes.iter().filter(|(i, _)| *i > j).map(|(_, w)| w);
            if clip(&mut d.entities[j], later) {
                n += 1;
            }
        }
    }
    d.entities.iter_mut().for_each(hide);
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ent(shape: Shape) -> Entity {
        Entity {
            layer: 0,
            color: None,
            aci: None,
            linetype: None,
            weight: None,
            shape,
        }
    }

    /// DXF·DWG 로 내보냈다 다시 읽어도 가리개 (§60)
    #[test]
    fn wipeout_round_trip() {
        let mut d = crate::archgen::new_drawing();
        let ring = vec![
            Vec2::new(100.0, 50.0),
            Vec2::new(400.0, 50.0),
            Vec2::new(250.0, 300.0),
        ];
        d.entities = vec![ent(Shape::Wipeout { ring: ring.clone() })];
        let dir = std::env::temp_dir().join(format!("dsmith-wipe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for ext in ["dxf", "dwg"] {
            let p = dir.join(format!("w.{ext}"));
            crate::export::write(&d, &p).unwrap();
            let back = crate::import::read_file(&p).unwrap();
            let got = back
                .entities
                .iter()
                .find_map(|e| match &e.shape {
                    Shape::Wipeout { ring } => Some(ring.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{ext}: 가리개 없음"));
            assert_eq!(got.len(), 3, "{ext} {got:?}");
            for (a, b) in got.iter().zip(&ring) {
                assert!(a.dist(*b) < 1e-6, "{ext} {a:?} {b:?}");
            }
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn wipeout_hides_earlier_lines_only() {
        let mut d = crate::archgen::new_drawing();
        let sq = vec![
            Vec2::new(4.0, -1.0),
            Vec2::new(6.0, -1.0),
            Vec2::new(6.0, 1.0),
            Vec2::new(4.0, 1.0),
        ];
        d.entities = vec![
            ent(Shape::Line {
                a: Vec2::new(0.0, 0.0),
                b: Vec2::new(10.0, 0.0),
            }),
            ent(Shape::Wipeout { ring: sq }),
            // 가리개 뒤(위)에 그린 선은 그대로
            ent(Shape::Line {
                a: Vec2::new(5.0, -5.0),
                b: Vec2::new(5.0, 5.0),
            }),
        ];
        assert_eq!(apply(&mut d), 1);
        match &d.entities[0].shape {
            Shape::Segments { lines } => {
                assert_eq!(lines.len(), 2);
                assert!(
                    lines[0].1.dist(Vec2::new(4.0, 0.0)) < 1e-9
                        && lines[1].0.dist(Vec2::new(6.0, 0.0)) < 1e-9
                );
            }
            s => panic!("{s:?}"),
        }
        assert_eq!(d.entities[1].shape, Shape::Segments { lines: Vec::new() });
        assert!(matches!(d.entities[2].shape, Shape::Line { .. }));
    }
}
