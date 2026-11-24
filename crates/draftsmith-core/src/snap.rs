//! 객체 스냅: 끝점·중간점·중심·사분점·삽입점을 R*-tree 에 넣고 커서에서 가장 가까운 것을 찾는다.
//!
//! 앱은 도면을 열거나 레이어를 켜고 끌 때 색인을 다시 만들고, 마우스가 움직일 때 [`SnapIndex::nearest`] 를 부른다
//! (M0 ⑤ — 스냅 계산 위치. 지연은 구현 노트 §7).

use rstar::RTree;
use rstar::primitives::GeomWithData;

use crate::curve;
use crate::drawing::{Drawing, Shape};
use crate::geom::Vec2;

/// 스냅 종류 (화면 표시 모양이 다르다)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SnapKind {
    /// 끝점 □
    End = 0,
    /// 중간점 △
    Mid = 1,
    /// 중심 ○
    Center = 2,
    /// 사분점 ◇
    Quadrant = 3,
    /// 문자 삽입점 ×
    Insert = 4,
}

type Item = GeomWithData<[f64; 2], SnapKind>;

/// 스냅 점 색인
pub struct SnapIndex {
    tree: RTree<Item>,
}

/// 찾은 스냅 점
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snap {
    pub at: Vec2,
    pub kind: SnapKind,
}

impl SnapIndex {
    /// 켜진 레이어의 객체에서 스냅 점을 모은다. 해치 무늬 선은 너무 많아 넣지 않는다.
    pub fn build(d: &Drawing) -> SnapIndex {
        let mut items: Vec<Item> = Vec::new();
        let mut add = |p: Vec2, k: SnapKind| {
            if p.x.is_finite() && p.y.is_finite() {
                items.push(GeomWithData::new([p.x, p.y], k));
            }
        };
        for e in &d.entities {
            if d.layers.get(e.layer as usize).is_some_and(|l| !l.visible) {
                continue;
            }
            match &e.shape {
                Shape::Line { a, b } => {
                    add(*a, SnapKind::End);
                    add(*b, SnapKind::End);
                    add(a.lerp(*b, 0.5), SnapKind::Mid);
                }
                Shape::Polyline { points, closed } => {
                    let n = points.len();
                    for (i, &(p, g)) in points.iter().enumerate() {
                        add(p, SnapKind::End);
                        if i + 1 < n || (*closed && n > 1) {
                            let q = points[(i + 1) % n].0;
                            match curve::bulge_arc(p, q, g) {
                                None => add(p.lerp(q, 0.5), SnapKind::Mid),
                                Some((c, r, s, sw)) => {
                                    let t = s + sw / 2.0;
                                    add(
                                        Vec2::new(c.x + r * t.cos(), c.y + r * t.sin()),
                                        SnapKind::Mid,
                                    );
                                    add(c, SnapKind::Center);
                                }
                            }
                        }
                    }
                }
                Shape::Circle { center, radius } => {
                    add(*center, SnapKind::Center);
                    for (dx, dy) in [(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)] {
                        add(
                            Vec2::new(center.x + dx * radius, center.y + dy * radius),
                            SnapKind::Quadrant,
                        );
                    }
                }
                Shape::Arc {
                    center,
                    radius,
                    start,
                    end,
                } => {
                    let at = |t: f64| {
                        Vec2::new(center.x + radius * t.cos(), center.y + radius * t.sin())
                    };
                    add(*center, SnapKind::Center);
                    add(at(*start), SnapKind::End);
                    add(at(*end), SnapKind::End);
                    add(
                        at(start + curve::ccw_sweep(*start, *end) / 2.0),
                        SnapKind::Mid,
                    );
                }
                Shape::Ellipse {
                    center,
                    u,
                    v,
                    start,
                    end,
                } => {
                    let at = |t: f64| {
                        Vec2::new(
                            center.x + u.x * t.cos() + v.x * t.sin(),
                            center.y + u.y * t.cos() + v.y * t.sin(),
                        )
                    };
                    add(*center, SnapKind::Center);
                    if curve::ccw_sweep(*start, *end) < std::f64::consts::TAU - 1e-9 {
                        add(at(*start), SnapKind::End);
                        add(at(*end), SnapKind::End);
                    }
                }
                Shape::Fill { loops } => {
                    // 치수 화살표·솔리드 꼭짓점
                    for p in loops.iter().flatten() {
                        add(*p, SnapKind::End);
                    }
                }
                Shape::Segments { .. } => {}
                Shape::Text(t) => add(t.at, SnapKind::Insert),
            }
        }
        SnapIndex {
            tree: RTree::bulk_load(items),
        }
    }

    pub fn len(&self) -> usize {
        self.tree.size()
    }

    pub fn is_empty(&self) -> bool {
        self.tree.size() == 0
    }

    /// `p` 에서 `radius` 안의 가장 가까운 스냅 점
    pub fn nearest(&self, p: Vec2, radius: f64) -> Option<Snap> {
        let q = [p.x, p.y];
        let hit = self.tree.nearest_neighbor(q)?;
        let [x, y] = *hit.geom();
        let at = Vec2::new(x, y);
        (at.dist(p) <= radius).then_some(Snap { at, kind: hit.data })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawing::{Drawing, Entity, Shape};

    #[test]
    fn finds_endpoint_mid_and_center() {
        let mut d = Drawing::default();
        let layer = d.layer_index("0");
        let mut put = |shape| {
            d.entities.push(Entity {
                layer,
                weight: None,
                linetype: None,
                color: None,
                shape,
            })
        };
        put(Shape::Line {
            a: Vec2::new(0.0, 0.0),
            b: Vec2::new(10.0, 0.0),
        });
        put(Shape::Circle {
            center: Vec2::new(100.0, 100.0),
            radius: 5.0,
        });
        let idx = SnapIndex::build(&d);
        let s = idx.nearest(Vec2::new(9.6, 0.3), 1.0).unwrap();
        assert_eq!((s.at, s.kind), (Vec2::new(10.0, 0.0), SnapKind::End));
        assert_eq!(
            idx.nearest(Vec2::new(5.2, 0.0), 1.0).unwrap().kind,
            SnapKind::Mid
        );
        assert_eq!(
            idx.nearest(Vec2::new(100.5, 99.0), 2.0).unwrap().kind,
            SnapKind::Center
        );
        assert!(idx.nearest(Vec2::new(50.0, 50.0), 1.0).is_none());
    }
}
