//! 편집 엔진: 도면을 바꾸는 모든 일은 여기를 거쳐 「작업」 으로 남고, 되돌리기·다시 하기가 된다 (구현 노트 §18).
//!
//! 작업은 객체 번호(목록 위치) 기준이다. 되돌리기는 늘 마지막 작업부터라 번호가 어긋나지 않는다.
//! 공동작업(M5)에서는 이 작업들을 그대로 주고받는다 ([`Op`] 가 직렬화된다).

use serde::{Deserialize, Serialize};

use crate::curve;
use crate::drawing::{Drawing, Entity, Layer, Shape};
use crate::geom::{Affine, Vec2};

/// 작업 하나
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Op {
    /// 끝에 객체 더하기 (`start` 부터)
    Add { start: usize, items: Vec<Entity> },
    /// 객체 지우기 (번호 오름차순, 지운 객체)
    Remove { items: Vec<(usize, Entity)> },
    /// 객체 바꾸기 (번호, 이전, 이후)
    Replace { items: Vec<(usize, Entity, Entity)> },
    /// 레이어 표 바꾸기 (이전, 이후)
    Layers { old: Vec<Layer>, new: Vec<Layer> },
    /// 현재 레이어 바꾸기
    CurrentLayer { old: u32, new: u32 },
    /// 건축 객체 모델 바꾸기 (이전, 이후) — M3
    Arch {
        old: Box<crate::arch::ArchModel>,
        new: Box<crate::arch::ArchModel>,
    },
    /// 배치(도면 시트) 목록 바꾸기 (이전, 이후) — M4
    Layouts {
        old: Vec<crate::layout::Layout>,
        new: Vec<crate::layout::Layout>,
    },
    /// 여러 작업을 하나로
    Batch(Vec<Op>),
}

/// 편집 중인 도면
#[derive(Debug, Default)]
pub struct Editor {
    pub d: Drawing,
    undo: Vec<Op>,
    redo: Vec<Op>,
    /// 저장한 뒤 바뀌었나
    pub dirty: bool,
    /// 작업 기록 (공동작업·자동 저장용, 되돌리기와 무관하게 쌓인다)
    pub journal: Vec<Op>,
}

fn apply(d: &mut Drawing, op: &Op) {
    match op {
        Op::Add { start, items } => {
            d.entities.truncate(*start);
            d.entities.extend(items.iter().cloned());
        }
        Op::Remove { items } => {
            for (i, _) in items.iter().rev() {
                if *i < d.entities.len() {
                    d.entities.remove(*i);
                }
            }
        }
        Op::Replace { items } => {
            for (i, _, new) in items {
                if let Some(e) = d.entities.get_mut(*i) {
                    *e = new.clone();
                }
            }
        }
        Op::Layers { new, .. } => d.layers = new.clone(),
        Op::CurrentLayer { new, .. } => d.current_layer = *new,
        Op::Arch { new, .. } => d.arch = (**new).clone(),
        Op::Layouts { new, .. } => d.layouts = new.clone(),
        Op::Batch(ops) => ops.iter().for_each(|o| apply(d, o)),
    }
}

fn revert(d: &mut Drawing, op: &Op) {
    match op {
        Op::Add { start, .. } => d.entities.truncate(*start),
        Op::Remove { items } => {
            for (i, e) in items {
                let i = (*i).min(d.entities.len());
                d.entities.insert(i, e.clone());
            }
        }
        Op::Replace { items } => {
            for (i, old, _) in items {
                if let Some(e) = d.entities.get_mut(*i) {
                    *e = old.clone();
                }
            }
        }
        Op::Layers { old, .. } => d.layers = old.clone(),
        Op::CurrentLayer { old, .. } => d.current_layer = *old,
        Op::Arch { old, .. } => d.arch = (**old).clone(),
        Op::Layouts { old, .. } => d.layouts = old.clone(),
        Op::Batch(ops) => ops.iter().rev().for_each(|o| revert(d, o)),
    }
}

impl Editor {
    pub fn new(d: Drawing) -> Editor {
        Editor {
            d,
            ..Default::default()
        }
    }

    /// 작업을 실행하고 기록한다
    pub fn run(&mut self, op: Op) {
        apply(&mut self.d, &op);
        self.journal.push(op.clone());
        self.undo.push(op);
        self.redo.clear();
        self.dirty = true;
    }

    /// 다른 곳(공동작업)에서 온 작업: 실행하되 되돌리기 기록에는 넣지 않는다
    pub fn run_remote(&mut self, op: &Op) {
        apply(&mut self.d, op);
        self.dirty = true;
    }

    /// 지금 상태까지 온 작업들 (되돌린 것은 빠짐) — 변경 기록 내보내기용 (M5)
    pub fn history(&self) -> &[Op] {
        &self.undo
    }

    /// 다른 사람의 작업들을 합친다 (M5, 구현 노트 §23). 객체 더하기는 지금 끝에 붙이고, 번호가 맞지 않는
    /// 지우기·바꾸기는 건너뛴다. 합친 것은 되돌리기 한 단계. (합친 수, 건너뛴 수)
    pub fn merge(&mut self, ops: &[Op]) -> (usize, usize) {
        let mut done = Vec::new();
        let mut skipped = 0;
        for op in ops {
            match self.fit_op(op) {
                Some(o) => {
                    apply(&mut self.d, &o);
                    done.push(o);
                }
                None => skipped += 1,
            }
        }
        let n = done.len();
        if n > 0 {
            // 이미 적용했으니 기록만 (되돌리기 한 단계)
            let op = Op::Batch(done);
            self.journal.push(op.clone());
            self.undo.push(op);
            self.redo.clear();
            self.dirty = true;
        }
        (n, skipped)
    }

    /// 지금 도면에 맞게 고친 작업 (맞출 수 없으면 None)
    fn fit_op(&self, op: &Op) -> Option<Op> {
        let n = self.d.entities.len();
        Some(match op {
            Op::Add { items, .. } => Op::Add {
                start: n,
                items: items.clone(),
            },
            Op::Remove { items } => {
                // 같은 자리에 같은 객체가 있어야 지운다
                if items
                    .iter()
                    .any(|(i, e)| self.d.entities.get(*i) != Some(e))
                {
                    return None;
                }
                op.clone()
            }
            Op::Replace { items } => {
                if items
                    .iter()
                    .any(|(i, old, _)| self.d.entities.get(*i) != Some(old))
                {
                    return None;
                }
                op.clone()
            }
            Op::Layers { new, .. } => Op::Layers {
                old: self.d.layers.clone(),
                new: new.clone(),
            },
            Op::CurrentLayer { new, .. } => Op::CurrentLayer {
                old: self.d.current_layer,
                new: *new,
            },
            Op::Arch { new, .. } => Op::Arch {
                old: Box::new(self.d.arch.clone()),
                new: new.clone(),
            },
            Op::Layouts { new, .. } => Op::Layouts {
                old: self.d.layouts.clone(),
                new: new.clone(),
            },
            Op::Batch(ops) => {
                // 묶음은 앞 작업을 적용한 도면 기준이라 사본에서 차례로 맞춘다
                let mut tmp = Editor::new(self.d.clone());
                let mut out = Vec::new();
                for o in ops {
                    let f = tmp.fit_op(o)?;
                    apply(&mut tmp.d, &f);
                    out.push(f);
                }
                Op::Batch(out)
            }
        })
    }

    /// 배치(도면 시트) 더하기
    pub fn add_layout(&mut self, l: crate::layout::Layout) {
        let old = self.d.layouts.clone();
        let mut new = old.clone();
        new.push(l);
        self.run(Op::Layouts { old, new });
    }

    /// 배치 지우기
    pub fn remove_layout(&mut self, i: usize) {
        if i >= self.d.layouts.len() {
            return;
        }
        let old = self.d.layouts.clone();
        let mut new = old.clone();
        new.remove(i);
        self.run(Op::Layouts { old, new });
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        let Some(op) = self.undo.pop() else {
            return false;
        };
        revert(&mut self.d, &op);
        self.redo.push(op);
        self.dirty = true;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(op) = self.redo.pop() else {
            return false;
        };
        apply(&mut self.d, &op);
        self.undo.push(op);
        self.dirty = true;
        true
    }

    /// 객체 더하기 (현재 레이어, 레이어 색). 더한 번호 범위
    pub fn add_shapes(&mut self, shapes: Vec<Shape>) -> std::ops::Range<usize> {
        let layer = self
            .d
            .current_layer
            .min(self.d.layers.len().saturating_sub(1) as u32);
        let items: Vec<Entity> = shapes
            .into_iter()
            .map(|shape| Entity {
                layer,
                color: None,
                aci: None,
                linetype: None,
                weight: None,
                shape,
            })
            .collect();
        self.add_entities(items)
    }

    pub fn add_entities(&mut self, items: Vec<Entity>) -> std::ops::Range<usize> {
        let start = self.d.entities.len();
        let n = items.len();
        if n > 0 {
            self.run(Op::Add { start, items });
        }
        start..start + n
    }

    /// 객체 지우기
    pub fn remove(&mut self, idx: &[usize]) {
        let mut v: Vec<usize> = idx
            .iter()
            .copied()
            .filter(|&i| i < self.d.entities.len())
            .collect();
        v.sort_unstable();
        v.dedup();
        if v.is_empty() {
            return;
        }
        let items = v
            .into_iter()
            .map(|i| (i, self.d.entities[i].clone()))
            .collect();
        self.run(Op::Remove { items });
    }

    fn replace_with(&mut self, idx: &[usize], f: impl Fn(&Entity) -> Entity) {
        let items: Vec<(usize, Entity, Entity)> = idx
            .iter()
            .filter(|&&i| i < self.d.entities.len())
            .map(|&i| {
                let old = self.d.entities[i].clone();
                let new = f(&old);
                (i, old, new)
            })
            .collect();
        if !items.is_empty() {
            self.run(Op::Replace { items });
        }
    }

    /// 이동·회전·대칭·축척
    pub fn transform(&mut self, idx: &[usize], t: &Affine) {
        self.replace_with(idx, |e| Entity {
            shape: e.shape.transformed(t),
            ..e.clone()
        });
    }

    /// 변환한 복사본 더하기
    pub fn copy(&mut self, idx: &[usize], t: &Affine) -> std::ops::Range<usize> {
        let items: Vec<Entity> = idx
            .iter()
            .filter_map(|&i| self.d.entities.get(i))
            .map(|e| Entity {
                shape: e.shape.transformed(t),
                ..e.clone()
            })
            .collect();
        self.add_entities(items)
    }

    /// 레이어 옮기기
    pub fn set_layer(&mut self, idx: &[usize], layer: u32) {
        self.replace_with(idx, |e| Entity { layer, ..e.clone() });
    }

    /// 색 바꾸기 (None = 레이어 색)
    pub fn set_color(&mut self, idx: &[usize], color: Option<[u8; 3]>, aci: Option<u8>) {
        self.replace_with(idx, |e| Entity {
            color,
            aci,
            ..e.clone()
        });
    }

    /// 레이어 더하기 (같은 이름이 있으면 그 번호)
    pub fn add_layer(&mut self, name: &str, color: [u8; 3]) -> u32 {
        if let Some(i) = self.d.layers.iter().position(|l| l.name == name) {
            return i as u32;
        }
        let old = self.d.layers.clone();
        let mut new = old.clone();
        new.push(Layer {
            name: name.to_string(),
            color,
            visible: true,
        });
        self.run(Op::Layers { old, new });
        (self.d.layers.len() - 1) as u32
    }

    /// 레이어 속성 바꾸기 (이름·색)
    pub fn edit_layer(&mut self, index: u32, name: &str, color: [u8; 3]) {
        let old = self.d.layers.clone();
        let mut new = old.clone();
        if let Some(l) = new.get_mut(index as usize) {
            l.name = name.to_string();
            l.color = color;
            self.run(Op::Layers { old, new });
        }
    }

    pub fn set_current_layer(&mut self, layer: u32) {
        if (layer as usize) < self.d.layers.len() && layer != self.d.current_layer {
            self.run(Op::CurrentLayer {
                old: self.d.current_layer,
                new: layer,
            });
        }
    }

    /// 간격 띄우기: 객체 하나를 `dist` 만큼, `side` 점 쪽으로 평행 복사
    pub fn offset(&mut self, index: usize, dist: f64, side: Vec2) -> Option<usize> {
        let e = self.d.entities.get(index)?.clone();
        let shape = offset_shape(&e.shape, dist, side)?;
        Some(self.add_entities(vec![Entity { shape, ..e }]).start)
    }
}

/// 점 p 가 선분 ab 의 왼쪽이면 양수
fn side_of(a: Vec2, b: Vec2, p: Vec2) -> f64 {
    (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
}

/// 두 직선(점 + 방향)의 교점
fn intersect(p: Vec2, d: Vec2, q: Vec2, e: Vec2) -> Option<Vec2> {
    let den = d.x * e.y - d.y * e.x;
    if den.abs() < 1e-12 {
        return None;
    }
    let t = ((q.x - p.x) * e.y - (q.y - p.y) * e.x) / den;
    Some(Vec2::new(p.x + d.x * t, p.y + d.y * t))
}

/// 꺾은선 간격 띄우기 (꼭짓점은 이웃 변 연장선의 교점 — 날카로운 모서리)
pub fn offset_chain(pts: &[Vec2], closed: bool, dist: f64) -> Vec<Vec2> {
    let n = pts.len();
    if n < 2 {
        return pts.to_vec();
    }
    let seg = |i: usize| {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let l = a.dist(b).max(1e-12);
        let nx = -(b.y - a.y) / l * dist;
        let ny = (b.x - a.x) / l * dist;
        (
            Vec2::new(a.x + nx, a.y + ny),
            Vec2::new(b.x - a.x, b.y - a.y),
        )
    };
    let m = if closed { n } else { n - 1 };
    let mut out = Vec::with_capacity(n);
    for (i, &orig) in pts.iter().enumerate() {
        let prev = if i == 0 {
            if closed { Some(m - 1) } else { None }
        } else {
            Some(i - 1)
        };
        let cur = if i < m { Some(i) } else { None };
        let p = match (prev, cur) {
            (Some(a), Some(b)) => {
                let (p0, d0) = seg(a);
                let (p1, d1) = seg(b);
                intersect(p0, d0, p1, d1).unwrap_or(p1)
            }
            (None, Some(b)) => seg(b).0,
            (Some(a), None) => {
                let (p0, d0) = seg(a);
                Vec2::new(p0.x + d0.x, p0.y + d0.y)
            }
            (None, None) => orig,
        };
        out.push(p);
    }
    out
}

/// 모양을 `side` 쪽으로 `dist` 만큼 평행 복사
pub fn offset_shape(s: &Shape, dist: f64, side: Vec2) -> Option<Shape> {
    match s {
        Shape::Line { a, b } => {
            let k = if side_of(*a, *b, side) >= 0.0 {
                dist
            } else {
                -dist
            };
            let pts = offset_chain(&[*a, *b], false, k);
            Some(Shape::Line {
                a: pts[0],
                b: pts[1],
            })
        }
        Shape::Circle { center, radius } => {
            let r = if side.dist(*center) > *radius {
                radius + dist
            } else {
                radius - dist
            };
            (r > 0.0).then_some(Shape::Circle {
                center: *center,
                radius: r,
            })
        }
        Shape::Arc {
            center,
            radius,
            start,
            end,
        } => {
            let r = if side.dist(*center) > *radius {
                radius + dist
            } else {
                radius - dist
            };
            (r > 0.0).then_some(Shape::Arc {
                center: *center,
                radius: r,
                start: *start,
                end: *end,
            })
        }
        Shape::Polyline { points, closed } => {
            let pts: Vec<Vec2> = if points.iter().any(|(_, g)| g.abs() > 1e-9) {
                let r = points
                    .iter()
                    .map(|(p, _)| p.x.abs().max(p.y.abs()))
                    .fold(1.0, f64::max);
                curve::polyline_points(points, *closed, r * 1e-4)
            } else {
                points.iter().map(|(p, _)| *p).collect()
            };
            if pts.len() < 2 {
                return None;
            }
            // 가장 가까운 변을 기준으로 어느 쪽인지 정한다
            let n = pts.len();
            let m = if *closed { n } else { n - 1 };
            let (mut best, mut bd) = (0usize, f64::INFINITY);
            for i in 0..m {
                let d = crate::pick::seg_dist(side, pts[i], pts[(i + 1) % n]);
                if d < bd {
                    bd = d;
                    best = i;
                }
            }
            let k = if side_of(pts[best], pts[(best + 1) % n], side) >= 0.0 {
                dist
            } else {
                -dist
            };
            let out = offset_chain(&pts, *closed, k);
            Some(Shape::Polyline {
                points: out.into_iter().map(|p| (p, 0.0)).collect(),
                closed: *closed,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ed() -> Editor {
        let mut d = Drawing::default();
        d.layer_index("0");
        Editor::new(d)
    }

    #[test]
    fn undo_redo_add_remove_move() {
        let mut e = ed();
        e.add_shapes(vec![
            Shape::Line {
                a: Vec2::new(0.0, 0.0),
                b: Vec2::new(10.0, 0.0),
            },
            Shape::Circle {
                center: Vec2::new(5.0, 5.0),
                radius: 1.0,
            },
        ]);
        assert_eq!(e.d.entities.len(), 2);
        e.transform(&[0], &Affine::translate(0.0, 5.0));
        assert_eq!(
            e.d.entities[0].shape,
            Shape::Line {
                a: Vec2::new(0.0, 5.0),
                b: Vec2::new(10.0, 5.0)
            }
        );
        e.remove(&[1]);
        assert_eq!(e.d.entities.len(), 1);
        e.undo();
        assert_eq!(e.d.entities.len(), 2);
        e.undo();
        assert_eq!(
            e.d.entities[0].shape,
            Shape::Line {
                a: Vec2::new(0.0, 0.0),
                b: Vec2::new(10.0, 0.0)
            }
        );
        e.redo();
        assert_eq!(
            e.d.entities[0].shape,
            Shape::Line {
                a: Vec2::new(0.0, 5.0),
                b: Vec2::new(10.0, 5.0)
            }
        );
        e.undo();
        e.undo();
        assert!(e.d.entities.is_empty());
        assert!(!e.can_undo());
    }

    #[test]
    fn offset_rectangle_outward() {
        let mut e = ed();
        let sq = vec![
            (Vec2::new(0.0, 0.0), 0.0),
            (Vec2::new(10.0, 0.0), 0.0),
            (Vec2::new(10.0, 10.0), 0.0),
            (Vec2::new(0.0, 10.0), 0.0),
        ];
        e.add_shapes(vec![Shape::Polyline {
            points: sq,
            closed: true,
        }]);
        let i = e.offset(0, 1.0, Vec2::new(-5.0, 5.0)).unwrap();
        match &e.d.entities[i].shape {
            Shape::Polyline { points, .. } => {
                assert_eq!(points[0].0, Vec2::new(-1.0, -1.0));
                assert_eq!(points[2].0, Vec2::new(11.0, 11.0));
            }
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn layers_and_current_layer_undo() {
        let mut e = ed();
        let w = e.add_layer("A-WALL", [255, 255, 255]);
        e.set_current_layer(w);
        e.add_shapes(vec![Shape::Line {
            a: Vec2::default(),
            b: Vec2::new(1.0, 0.0),
        }]);
        assert_eq!(e.d.entities[0].layer, w);
        e.undo();
        e.undo();
        assert_eq!(e.d.current_layer, 0);
        e.undo();
        assert_eq!(e.d.layers.len(), 1);
    }
}

/// 변경 기록 파일 (`.dsjournal`, JSON) — M5
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Journal {
    pub format: String,
    pub version: u32,
    pub ops: Vec<Op>,
}

/// 변경 기록 파일 확장자
pub const JOURNAL_EXT: &str = "dsjournal";

impl Journal {
    pub fn new(ops: Vec<Op>) -> Journal {
        Journal {
            format: "draftsmith-journal".into(),
            version: 1,
            ops,
        }
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec(self).map_err(|e| e.to_string())
    }

    pub fn from_bytes(b: &[u8]) -> Result<Journal, String> {
        let j: Journal =
            serde_json::from_slice(b).map_err(|e| format!("변경 기록을 읽지 못했습니다: {e}"))?;
        if j.format != "draftsmith-journal" {
            return Err("DraftSmith 변경 기록 파일이 아닙니다".into());
        }
        Ok(j)
    }
}

#[cfg(test)]
mod merge_tests {
    use super::*;
    use crate::drawing::Shape;
    use crate::geom::Vec2;

    fn line(x: f64) -> Shape {
        Shape::Line {
            a: Vec2::new(x, 0.0),
            b: Vec2::new(x, 1.0),
        }
    }

    #[test]
    fn merge_appends_and_skips_conflicts() {
        let base = Drawing::default();
        // 갑: 선 둘 그리고 첫 선 지움
        let mut a = Editor::new(base.clone());
        let _ = a.add_shapes(vec![line(0.0), line(1.0)]);
        a.remove(&[0]);
        let j =
            Journal::from_bytes(&Journal::new(a.history().to_vec()).to_bytes().unwrap()).unwrap();
        // 을: 따로 선 하나 그린 뒤 갑의 기록을 합침
        let mut b = Editor::new(base);
        let _ = b.add_shapes(vec![line(5.0)]);
        let (done, skipped) = b.merge(&j.ops);
        // 더하기는 붙고, 지우기는 자리의 객체가 달라 건너뜀
        assert_eq!((done, skipped), (1, 1));
        assert_eq!(b.d.entities.len(), 3);
        assert!(b.undo());
        assert_eq!(b.d.entities.len(), 1);
    }
}
