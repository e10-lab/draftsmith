//! 객체 스냅: 끝점·중간점·중심·사분점·삽입점을 R*-tree 에 넣고 커서에서 가장 가까운 것을 찾는다.
//!
//! 앱은 도면을 열거나 레이어를 켜고 끌 때 색인을 다시 만들고, 마우스가 움직일 때 [`SnapIndex::nearest`] 를 부른다
//! (M0 ⑤ — 스냅 계산 위치. 지연은 구현 노트 §7).

use rstar::RTree;
use rstar::primitives::GeomWithData;

use crate::curve;
use crate::drawing::{Drawing, Entity, Shape};
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
        for e in &d.entities {
            if d.layers.get(e.layer as usize).is_some_and(|l| !l.visible) {
                continue;
            }
            items.extend(points_of(e));
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

/// 객체 하나의 스냅 점
fn points_of(e: &Entity) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    let mut add = |p: Vec2, k: SnapKind| {
        if p.x.is_finite() && p.y.is_finite() {
            items.push(GeomWithData::new([p.x, p.y], k));
        }
    };
    match &e.shape {
        Shape::Wide { points, .. } => {
            for (p, _) in points {
                add(*p, SnapKind::End);
            }
        }
        Shape::Wipeout { .. } => {}
        Shape::Hatch { .. } => {}
        Shape::Insert { at, .. } => add(*at, SnapKind::Insert),
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
            let at = |t: f64| Vec2::new(center.x + radius * t.cos(), center.y + radius * t.sin());
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
    items
}

/// 스냅 색인 캐시 (§69): 편집 뒤 바뀐 객체의 점만 R*-tree 에서 빼고 넣는다.
/// 앞뒤로 같은 객체를 맞춰(끝에 더하기·가운데 지우기·제자리 고치기) 가운데만 바꾼다. 레이어가 바뀌거나 너무 많이 바뀌면 새로.
#[derive(Default)]
pub struct SnapCache {
    ents: Vec<Entity>,
    /// 객체마다 넣은 점 (꺼진 레이어면 빈 목록)
    items: Vec<Vec<Item>>,
    layers: Vec<crate::drawing::Layer>,
    index: Option<SnapIndex>,
    /// 마지막에 다시 셈한 객체 수 (측정용)
    pub changed: usize,
    /// 마지막 갱신에서 바뀐 범위 (앞 같은 수, 옛 끝, 새 끝). 새로 만들었으면 None — 고르기 색인이 같이 쓴다 (§71)
    pub diff: Option<(usize, usize, usize)>,
}

impl SnapCache {
    pub fn index(&self) -> Option<&SnapIndex> {
        self.index.as_ref()
    }

    pub fn update(&mut self, d: &Drawing) -> &SnapIndex {
        let pts = |e: &Entity| -> Vec<Item> {
            if d.layers.get(e.layer as usize).is_some_and(|l| !l.visible) {
                Vec::new()
            } else {
                points_of(e)
            }
        };
        let new = &d.entities;
        let fresh = self.index.is_none() || self.layers != d.layers;
        let (pre, suf) = if fresh {
            (0, 0)
        } else {
            crate::drawing::align(&self.ents, new)
        };
        let old_mid = pre..self.ents.len() - suf;
        let new_mid = pre..new.len() - suf;
        let removed: usize = self.items[old_mid.clone()].iter().map(|v| v.len()).sum();
        let added: Vec<Vec<Item>> = new[new_mid.clone()].iter().map(pts).collect();
        let n_add: usize = added.iter().map(|v| v.len()).sum();
        let total = self.index.as_ref().map(|i| i.len()).unwrap_or(0);
        self.changed = new_mid.len();
        self.diff = (!fresh).then_some((pre, old_mid.end, new_mid.end));
        if fresh || (removed + n_add) * 3 > total.max(1) {
            // 새로
            let items: Vec<Vec<Item>> = if fresh {
                new.iter().map(pts).collect()
            } else {
                let mut v: Vec<Vec<Item>> = self.items[..pre].to_vec();
                v.extend(added);
                v.extend(self.items[self.ents.len() - suf..].iter().cloned());
                v
            };
            self.index = Some(SnapIndex {
                tree: RTree::bulk_load(items.iter().flatten().copied().collect()),
            });
            self.items = items;
            self.changed = if fresh { new.len() } else { new_mid.len() };
        } else {
            let tree = &mut self.index.as_mut().unwrap().tree;
            for it in self.items[old_mid.clone()].iter().flatten() {
                tree.remove(it);
            }
            for it in added.iter().flatten() {
                tree.insert(*it);
            }
            self.items.splice(old_mid, added);
        }
        // 바뀐 가운데만 갈아 끼운다 (전체 복사는 큰 도면에서 수십 ms)
        if fresh {
            self.ents = new.clone();
            self.layers = d.layers.clone();
        } else {
            let end = self.ents.len() - suf;
            self.ents.splice(pre..end, new[new_mid].iter().cloned());
        }
        self.index.as_ref().unwrap()
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    fn all(i: &SnapIndex) -> Vec<([u64; 2], u8)> {
        let mut v: Vec<_> = i
            .tree
            .iter()
            .map(|it| {
                (
                    [it.geom()[0].to_bits(), it.geom()[1].to_bits()],
                    it.data as u8,
                )
            })
            .collect();
        v.sort();
        v
    }

    /// 캐시로 고친 색인이 새로 만든 것과 같고, 바뀐 객체만 다시 셈한다 (§69)
    #[test]
    fn cache_matches_fresh() {
        let d0 = crate::archgen::shown(&crate::demo::demo_building().unwrap());
        let mut c = SnapCache::default();
        assert_eq!(all(c.update(&d0)), all(&SnapIndex::build(&d0)));
        assert_eq!(c.changed, d0.entities.len());
        // 제자리 고치기: 한 선 옮기기
        let mut d1 = d0.clone();
        let i = d1
            .entities
            .iter()
            .position(|e| matches!(e.shape, Shape::Line { .. }))
            .unwrap();
        d1.entities[i].shape = d1.entities[i]
            .shape
            .transformed(&crate::geom::Affine::translate(10.0, 0.0));
        assert_eq!(all(c.update(&d1)), all(&SnapIndex::build(&d1)));
        assert_eq!(c.changed, 1);
        // 가운데 지우기
        let mut d2 = d1.clone();
        d2.entities.remove(i);
        assert_eq!(all(c.update(&d2)), all(&SnapIndex::build(&d2)));
        assert_eq!(c.changed, 0);
        // 끝에 더하기
        let mut d3 = d2.clone();
        d3.entities.push(d1.entities[i].clone());
        assert_eq!(all(c.update(&d3)), all(&SnapIndex::build(&d3)));
        assert_eq!(c.changed, 1);
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
                id: 0,
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
