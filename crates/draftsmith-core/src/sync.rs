//! 실시간 공동작업 (구현 노트 §90) — 네트워크와 뗀 규칙. 서버(`dsmith-sync`)와 앱이 이것을 쓴다.
//!
//! - 서버가 기준이다: 방(도면)마다 편집기를 하나 들고, 들어온 작업을 도착 순서대로 **고유 번호로** 맞춰(`fit_by_id`) 적용한 뒤
//!   다른 사람들에게 보낸다. 보낸 사람에게는 받았다고(`Ack`) 알린다.
//! - 같은 객체를 동시에 고쳐 맞지 않으면(보낸 사람이 알던 내용과 서버의 지금 내용이 다르면) 그 작업은 버리고, 보낸 사람에게
//!   그 객체들의 서버 내용(`Fix`)을 보낸다.
//! - 앱은 자기 작업을 먼저 적용하고(기다리지 않음) 서버로 보낸다. 서버에서 온 작업·`Fix` 는 무조건 받아들인다(`force_by_id`).
//!   그래서 모두 서버 도면으로 모인다. 다른 객체를 동시에 고치면 둘 다 들어간다.
//! - 동시에 더한 객체의 그리는 순서는 사람마다 조금 다를 수 있다 (내용은 같다).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::drawing::{Drawing, Entity};
use crate::edit::{Editor, Op};

/// 주고받는 말 (JSON 한 줄)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum Msg {
    /// 앱 → 서버: 지금 도면으로 방을 만든다 (이미 있으면 그 방에 들어간다)
    Create { room: String, drawing: Box<Drawing> },
    /// 앱 → 서버: 방에 들어간다
    Join { room: String },
    /// 서버 → 앱: 방의 지금 도면 (들어올 때)
    Welcome {
        drawing: Box<Drawing>,
        people: usize,
    },
    /// 앱 → 서버: 내 작업 (`n` 은 내 번호 — `Ack`·`Fix` 가 가리킨다)
    Op { n: u64, op: Op },
    /// 서버 → 다른 사람들: 적용한 작업
    Applied { op: Op },
    /// 서버 → 보낸 사람: 받았다
    Ack { n: u64 },
    /// 서버 → 보낸 사람: 충돌해 버린 작업 `n` — 그 객체들의 서버 내용 (None 이면 서버에는 없다)
    Fix {
        n: u64,
        ents: Vec<(u64, Option<Entity>)>,
    },
    /// 서버 → 모두: 방 사람 수가 바뀌었다
    People { n: usize },
    /// 서버 → 앱: 잘못
    Error { msg: String },
}

/// 고유 번호로 맞춘 결과
#[derive(Debug, Clone, PartialEq)]
pub enum Fit {
    /// 지금 도면에 그대로 적용할 수 있게 고친 작업
    Ok(Op),
    /// 맞지 않는 객체 번호들
    Conflict(Vec<u64>),
}

/// 작업을 지금 도면에 고유 번호로 맞춘다 (서버 — 엄격히). 더하기는 이미 있는 번호를 빼고, 지우기는 이미 없는 것을 빼고,
/// 바꾸기·지우기는 지금 내용이 보낸 사람이 알던 내용과 같아야 한다. 객체 밖 필드는 마지막에 쓴 것이 이긴다
pub fn fit_by_id(d: &Drawing, op: &Op) -> Fit {
    match op {
        Op::Batch(ops) => {
            let mut tmp = d.clone();
            let mut out = Vec::new();
            let mut bad = Vec::new();
            for o in ops {
                match fit_by_id(&tmp, o) {
                    Fit::Ok(f) => {
                        apply_plain(&mut tmp, &f);
                        out.push(f);
                    }
                    Fit::Conflict(ids) => bad.extend(ids),
                }
            }
            if bad.is_empty() {
                Fit::Ok(Op::Batch(out))
            } else {
                Fit::Conflict(bad)
            }
        }
        Op::Add { items, .. } => {
            let have: HashSet<u64> = d.entities.iter().map(|e| e.id).collect();
            Fit::Ok(Op::Add {
                start: d.entities.len(),
                items: items
                    .iter()
                    .filter(|e| e.id == 0 || !have.contains(&e.id))
                    .cloned()
                    .collect(),
            })
        }
        Op::Remove { items } => {
            let mut out = Vec::new();
            let mut bad = Vec::new();
            for (_, e) in items {
                match d.index_of(e.id) {
                    None => {}
                    Some(i) if d.entities[i] == *e => out.push((i, e.clone())),
                    Some(_) => bad.push(e.id),
                }
            }
            if !bad.is_empty() {
                return Fit::Conflict(bad);
            }
            out.sort_by_key(|x| x.0);
            Fit::Ok(Op::Remove { items: out })
        }
        Op::Replace { items } => {
            let mut out = Vec::new();
            let mut bad = Vec::new();
            for (_, old, new) in items {
                match d.index_of(old.id) {
                    Some(i) if d.entities[i] == *old => out.push((i, old.clone(), new.clone())),
                    _ => bad.push(old.id),
                }
            }
            if bad.is_empty() {
                Fit::Ok(Op::Replace { items: out })
            } else {
                Fit::Conflict(bad)
            }
        }
        _ => Fit::Ok(force_by_id(d, op)),
    }
}

/// 서버에서 온 작업을 지금 도면에 무조건 맞춘다 (앱 — 서버가 기준). 바꾸기는 그 번호의 객체를 새 내용으로(없으면 더하기),
/// 지우기는 그 번호가 있으면 지우기, 더하기는 없는 번호만, 객체 밖 필드는 새 값으로
pub fn force_by_id(d: &Drawing, op: &Op) -> Op {
    match op {
        Op::Batch(ops) => {
            let mut tmp = d.clone();
            let mut out = Vec::new();
            for o in ops {
                let f = force_by_id(&tmp, o);
                apply_plain(&mut tmp, &f);
                out.push(f);
            }
            Op::Batch(out)
        }
        Op::Add { .. } => match fit_by_id(d, op) {
            Fit::Ok(f) => f,
            Fit::Conflict(_) => unreachable!("더하기는 충돌하지 않는다"),
        },
        Op::Remove { items } => {
            let mut out: Vec<(usize, Entity)> = items
                .iter()
                .filter_map(|(_, e)| d.index_of(e.id).map(|i| (i, d.entities[i].clone())))
                .collect();
            out.sort_by_key(|x| x.0);
            out.dedup_by_key(|x| x.0);
            Op::Remove { items: out }
        }
        Op::Replace { items } => set_entities(
            d,
            &items
                .iter()
                .map(|(_, old, new)| (old.id, Some(new.clone())))
                .collect::<Vec<_>>(),
        ),
        Op::Layers { new, .. } => Op::Layers {
            old: d.layers.clone(),
            new: new.clone(),
        },
        Op::CurrentLayer { new, .. } => Op::CurrentLayer {
            old: d.current_layer,
            new: *new,
        },
        // 건축 모델: 바뀐 객체만 지금 모델에 얹는다 (§91)
        Op::Arch { old, new } => Op::Arch {
            old: Box::new(d.arch.clone()),
            new: Box::new(arch_merge(old, new, &d.arch, &HashSet::new())),
        },
        Op::Layouts { new, .. } => Op::Layouts {
            old: d.layouts.clone(),
            new: new.clone(),
        },
        Op::Blocks { new, .. } => Op::Blocks {
            old: d.blocks.clone(),
            new: new.clone(),
        },
    }
}

/// 번호별로 객체를 정해진 내용으로 (`None` 이면 지우기, 없던 번호면 끝에 더하기) — `Fix` 받기
pub fn set_entities(d: &Drawing, ents: &[(u64, Option<Entity>)]) -> Op {
    let mut replace = Vec::new();
    let mut remove = Vec::new();
    let mut add = Vec::new();
    for (id, e) in ents {
        match (d.index_of(*id), e) {
            (Some(i), Some(e)) => {
                if d.entities[i] != *e {
                    replace.push((i, d.entities[i].clone(), e.clone()));
                }
            }
            (Some(i), None) => remove.push((i, d.entities[i].clone())),
            (None, Some(e)) => add.push(e.clone()),
            (None, None) => {}
        }
    }
    remove.sort_by_key(|x| x.0);
    remove.dedup_by_key(|x| x.0);
    // 바꾸기 → 지우기(번호가 바뀌니 뒤에서) → 더하기 순서. 지우기 전 번호로 바꾸기를 먼저 한다
    let mut ops = Vec::new();
    if !replace.is_empty() {
        ops.push(Op::Replace { items: replace });
    }
    let n_after = d.entities.len() - remove.len();
    if !remove.is_empty() {
        ops.push(Op::Remove { items: remove });
    }
    if !add.is_empty() {
        ops.push(Op::Add {
            start: n_after,
            items: add,
        });
    }
    Op::Batch(ops)
}

/// 번호를 매기지 않고 그대로 적용 (맞춘 작업용)
fn apply_plain(d: &mut Drawing, op: &Op) {
    let mut ed = Editor::new(std::mem::take(d));
    ed.run_remote(op);
    *d = ed.d;
}

/// 서버의 방 하나 — 네트워크와 뗀 규칙
pub struct Room {
    pub ed: Editor,
}

impl Room {
    pub fn new(d: Drawing) -> Room {
        Room { ed: Editor::new(d) }
    }

    /// 들어온 작업: (보낸 사람에게 줄 말, 다른 사람들에게 줄 말)
    pub fn handle(&mut self, n: u64, op: &Op) -> (Msg, Option<Msg>) {
        match fit_by_id(&self.ed.d, op) {
            Fit::Ok(f) => {
                self.ed.run_remote(&f);
                (Msg::Ack { n }, Some(Msg::Applied { op: f }))
            }
            Fit::Conflict(ids) => {
                let ents = ids
                    .into_iter()
                    .map(|id| {
                        (
                            id,
                            self.ed
                                .d
                                .index_of(id)
                                .map(|i| self.ed.d.entities[i].clone()),
                        )
                    })
                    .collect();
                (Msg::Fix { n, ents }, None)
            }
        }
    }
}

/// 객체 밖 필드 종류 (마지막에 쓴 것이 이기는 것들). 건축 객체는 하나씩(§91)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Field {
    Layers,
    /// 건축 모델의 (필드 이름, 객체 번호 — 번호 목록이 아닌 필드면 None)
    Arch(String, Option<u64>),
    Layouts,
    Blocks,
}

/// 작업에 든 객체 밖 필드 종류들
fn fields(op: &Op, out: &mut Vec<Field>) {
    match op {
        Op::Layers { .. } => out.push(Field::Layers),
        Op::Arch { old, new } => out.extend(
            arch_keys(old, new)
                .into_iter()
                .map(|(k, id)| Field::Arch(k, id)),
        ),
        Op::Layouts { .. } => out.push(Field::Layouts),
        Op::Blocks { .. } => out.push(Field::Blocks),
        Op::Batch(ops) => ops.iter().for_each(|o| fields(o, out)),
        _ => {}
    }
}

/// 보낼 작업에서 현재 레이어 바꾸기를 뺀다 — 현재 레이어는 사람마다 따로
pub fn shareable(op: &Op) -> Option<Op> {
    match op {
        Op::CurrentLayer { .. } => None,
        Op::Batch(ops) => {
            let v: Vec<Op> = ops.iter().filter_map(shareable).collect();
            (!v.is_empty()).then_some(Op::Batch(v))
        }
        o => Some(o.clone()),
    }
}

/// 받은 작업에서 내가 아직 서버 확인을 못 받은 필드 종류(건축 객체는 그 객체)의 바꾸기를 뺀다 — 내 것이 서버에 나중에
/// 닿아 이길 것이므로. 건축 모델 작업은 그 객체들만 빼고 나머지를 지금 모델에 얹는다
fn without_fields(d: &Drawing, op: &Op, skip: &[Field]) -> Option<Op> {
    match op {
        Op::Batch(ops) => Some(Op::Batch(
            ops.iter()
                .filter_map(|o| without_fields(d, o, skip))
                .collect(),
        )),
        Op::Arch { old, new } => {
            let keys: HashSet<(String, Option<u64>)> = skip
                .iter()
                .filter_map(|f| match f {
                    Field::Arch(k, id) => Some((k.clone(), *id)),
                    _ => None,
                })
                .collect();
            Some(Op::Arch {
                old: Box::new(d.arch.clone()),
                new: Box::new(arch_merge(old, new, &d.arch, &keys)),
            })
        }
        o => {
            let mut f = Vec::new();
            fields(o, &mut f);
            (!f.iter().any(|x| skip.contains(x))).then(|| o.clone())
        }
    }
}

/// 건축 모델 두 판 사이에 바뀐 것: (필드 이름, 번호 목록이면 바뀐 객체 번호) (§91)
fn arch_keys(
    old: &crate::arch::ArchModel,
    new: &crate::arch::ArchModel,
) -> Vec<(String, Option<u64>)> {
    let (Ok(serde_json::Value::Object(o)), Ok(serde_json::Value::Object(n))) =
        (serde_json::to_value(old), serde_json::to_value(new))
    else {
        return vec![("*".into(), None)];
    };
    let mut out = Vec::new();
    for (k, nv) in &n {
        let ov = o.get(k).unwrap_or(&serde_json::Value::Null);
        if ov == nv || PERSONAL.contains(&k.as_str()) {
            continue;
        }
        match (by_id(ov), by_id(nv)) {
            (Some(om), Some(nm)) => {
                for (id, x) in &nm {
                    if om.get(id) != Some(x) {
                        out.push((k.clone(), Some(*id)));
                    }
                }
                for id in om.keys().filter(|id| !nm.contains_key(id)) {
                    out.push((k.clone(), Some(*id)));
                }
            }
            _ => out.push((k.clone(), None)),
        }
    }
    out
}

/// 사람마다 따로인 건축 모델 필드 — 지금 보는 층 (현재 레이어와 같다)
const PERSONAL: [&str; 1] = ["active_level"];

/// 번호(`id`)가 있는 객체들의 목록이면 번호 → 객체
fn by_id(v: &serde_json::Value) -> Option<std::collections::BTreeMap<u64, &serde_json::Value>> {
    let a = v.as_array()?;
    a.iter()
        .map(|x| Some((x.get("id")?.as_u64()?, x)))
        .collect()
}

/// 건축 모델 3 자 합치기 (§91): `old` → `new` 로 바뀐 것만 지금 모델 `cur` 에 얹는다. 번호 목록(벽·문·창·실 …)은 객체
/// 하나씩(더하기·지우기·바꾸기), 그 밖의 필드는 통째로, `next_id` 는 큰 쪽. `skip` 의 것은 건너뛴다
pub(crate) fn arch_merge(
    old: &crate::arch::ArchModel,
    new: &crate::arch::ArchModel,
    cur: &crate::arch::ArchModel,
    skip: &HashSet<(String, Option<u64>)>,
) -> crate::arch::ArchModel {
    use serde_json::Value;
    let (Ok(Value::Object(o)), Ok(Value::Object(n)), Ok(Value::Object(mut c))) = (
        serde_json::to_value(old),
        serde_json::to_value(new),
        serde_json::to_value(cur),
    ) else {
        return new.clone();
    };
    for (k, nv) in &n {
        let ov = o.get(k).unwrap_or(&Value::Null);
        if ov == nv || PERSONAL.contains(&k.as_str()) {
            continue;
        }
        if k == "next_id" {
            let m = nv
                .as_u64()
                .unwrap_or(0)
                .max(c.get(k).and_then(Value::as_u64).unwrap_or(0));
            c.insert(k.clone(), Value::from(m));
            continue;
        }
        let cv = c.get(k).cloned().unwrap_or(Value::Null);
        match (by_id(ov), by_id(nv), cv.as_array()) {
            (Some(om), Some(nm), Some(ca)) => {
                let mut list: Vec<Value> = ca.clone();
                let pos = |list: &Vec<Value>, id: u64| {
                    list.iter()
                        .position(|x| x.get("id").and_then(Value::as_u64) == Some(id))
                };
                for (id, x) in &nm {
                    if om.get(id) == Some(x) || skip.contains(&(k.clone(), Some(*id))) {
                        continue;
                    }
                    match pos(&list, *id) {
                        Some(i) => list[i] = (*x).clone(),
                        None => list.push((*x).clone()),
                    }
                }
                for id in om.keys().filter(|id| !nm.contains_key(id)) {
                    if skip.contains(&(k.clone(), Some(*id))) {
                        continue;
                    }
                    if let Some(i) = pos(&list, *id) {
                        list.remove(i);
                    }
                }
                c.insert(k.clone(), Value::Array(list));
            }
            _ => {
                if !skip.contains(&(k.clone(), None)) {
                    c.insert(k.clone(), nv.clone());
                }
            }
        }
    }
    serde_json::from_value(Value::Object(c)).unwrap_or_else(|_| new.clone())
}

/// 앱 쪽 상태: 서버 확인을 아직 못 받은 내 작업들
#[derive(Debug, Default)]
pub struct Client {
    next: u64,
    pending: Vec<(u64, Vec<Field>)>,
}

impl Client {
    /// 내 작업(이미 적용함)을 보낼 말로. 나눌 것이 없으면 None
    pub fn local(&mut self, op: &Op) -> Option<Msg> {
        let op = shareable(op)?;
        self.next += 1;
        let mut f = Vec::new();
        fields(&op, &mut f);
        self.pending.push((self.next, f));
        Some(Msg::Op { n: self.next, op })
    }

    /// 서버에서 온 말을 편집기에 적용한다. 화면을 다시 그려야 하면 참
    pub fn receive(&mut self, ed: &mut Editor, msg: &Msg) -> bool {
        match msg {
            Msg::Ack { n } => {
                self.pending.retain(|p| p.0 != *n);
                false
            }
            Msg::Fix { n, ents } => {
                self.pending.retain(|p| p.0 != *n);
                let f = set_entities(&ed.d, ents);
                ed.run_remote(&f);
                true
            }
            Msg::Applied { op } => {
                let skip: Vec<Field> = self.pending.iter().flat_map(|p| p.1.clone()).collect();
                let Some(op) = without_fields(&ed.d, op, &skip) else {
                    return false;
                };
                let f = force_by_id(&ed.d, &op);
                ed.run_remote(&f);
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawing::Shape;
    use crate::geom::{Affine, Vec2};

    fn line(y: f64) -> Shape {
        Shape::Line {
            a: Vec2::new(0.0, y),
            b: Vec2::new(1000.0, y),
        }
    }

    /// 서버 하나와 앱 둘: 앱은 자기 작업을 먼저 적용하고, 서버로 보낸 말·받는 말의 차례를 시험에서 정한다
    struct World {
        room: Room,
        apps: Vec<Editor>,
        clients: Vec<Client>,
        /// 앱이 보낼 말 (아직 서버에 안 닿음)
        outbox: Vec<Vec<(u64, Op)>>,
        /// 앱이 받을 말
        inbox: Vec<Vec<Msg>>,
    }

    impl World {
        fn new() -> World {
            let mut base = Editor::new(crate::archgen::new_drawing());
            base.add_shapes(vec![line(0.0), line(100.0), line(200.0)]);
            let d = base.d.clone();
            let apps: Vec<Editor> = (0..2)
                .map(|k| {
                    let mut e = Editor::new(d.clone());
                    e.d.salt_ids(k + 100);
                    e
                })
                .collect();
            World {
                room: Room::new(d),
                apps,
                clients: vec![Client::default(), Client::default()],
                outbox: vec![Vec::new(), Vec::new()],
                inbox: vec![Vec::new(), Vec::new()],
            }
        }
        /// 앱 k 가 편집하고(먼저 적용) 마지막 작업을 보낼 말에 넣는다
        fn edit(&mut self, k: usize, f: impl FnOnce(&mut Editor)) {
            let before = self.apps[k].history().len();
            f(&mut self.apps[k]);
            for op in &self.apps[k].history()[before..] {
                if let Some(Msg::Op { n, op }) = self.clients[k].local(op) {
                    self.outbox[k].push((n, op));
                }
            }
        }
        /// 앱 k 가 보낸 말이 서버에 닿는다
        fn send(&mut self, k: usize) {
            for (n, op) in std::mem::take(&mut self.outbox[k]) {
                let (me, others) = self.room.handle(n, &op);
                self.inbox[k].push(me);
                if let Some(m) = others {
                    for j in 0..self.apps.len() {
                        if j != k {
                            self.inbox[j].push(m.clone());
                        }
                    }
                }
            }
        }
        /// 앱 k 가 받은 말을 적용한다
        fn recv(&mut self, k: usize) {
            for m in std::mem::take(&mut self.inbox[k]) {
                self.clients[k].receive(&mut self.apps[k], &m);
            }
        }
        fn settle(&mut self) {
            for k in 0..2 {
                self.send(k);
            }
            for k in 0..2 {
                self.recv(k);
            }
        }
        /// 모두 서버와 같은 객체들 (순서는 빼고)
        fn same(&self) {
            let key = |d: &Drawing| {
                let mut v: Vec<String> = d
                    .entities
                    .iter()
                    .map(|e| serde_json::to_string(e).unwrap())
                    .collect();
                v.sort();
                v
            };
            let s = key(&self.room.ed.d);
            for (k, a) in self.apps.iter().enumerate() {
                assert_eq!(key(&a.d), s, "앱 {k}");
            }
        }
    }

    /// 다른 객체를 동시에: 둘 다 들어가고 모두 같다
    #[test]
    fn concurrent_different_objects() {
        let mut w = World::new();
        w.edit(0, |e| e.transform(&[0], &Affine::translate(0.0, 5.0)));
        w.edit(1, |e| e.remove(&[1]));
        w.edit(1, |e| {
            e.add_shapes(vec![line(300.0)]);
        });
        w.edit(0, |e| {
            e.add_shapes(vec![line(400.0)]);
        });
        w.settle();
        w.same();
        assert_eq!(w.room.ed.d.entities.len(), 4);
    }

    /// 같은 객체를 동시에: 먼저 닿은 쪽이 이기고, 진 쪽은 서버 내용으로 맞춰져 모두 같다
    #[test]
    fn concurrent_same_object() {
        let mut w = World::new();
        w.edit(0, |e| e.transform(&[2], &Affine::translate(0.0, 7.0)));
        w.edit(1, |e| e.transform(&[2], &Affine::translate(0.0, -9.0)));
        w.edit(1, |e| e.remove(&[0]));
        w.send(0); // 앱 0 이 먼저 닿는다
        w.send(1);
        w.recv(0);
        w.recv(1);
        w.same();
        let ys: Vec<f64> = w
            .room
            .ed
            .d
            .entities
            .iter()
            .map(|e| match e.shape {
                Shape::Line { a, .. } => a.y,
                _ => f64::NAN,
            })
            .collect();
        assert!(ys.contains(&207.0) && !ys.contains(&191.0), "{ys:?}");
        // 지운 객체를 다른 쪽이 고친 경우도
        let mut w = World::new();
        w.edit(0, |e| e.remove(&[1]));
        w.edit(1, |e| e.transform(&[1], &Affine::translate(3.0, 0.0)));
        w.settle();
        w.same();
        assert_eq!(w.room.ed.d.entities.len(), 2);
    }

    /// 레이어 같은 객체 밖 필드는 나중에 닿은 것이 이긴다, 말은 JSON 으로 오간다
    #[test]
    fn meta_last_wins_and_json() {
        let mut w = World::new();
        let name = w.apps[0].d.layers[0].name.clone();
        let n2 = name.clone();
        w.edit(0, |e| e.edit_layer(0, &name, [1, 1, 1]));
        w.edit(1, |e| e.edit_layer(0, &n2, [2, 2, 2]));
        w.settle();
        w.same();
        assert_eq!(w.room.ed.d.layers[0].color, [2, 2, 2]);
        for a in &w.apps {
            assert_eq!(a.d.layers[0].color, [2, 2, 2]);
        }
        let m = Msg::Fix {
            n: 3,
            ents: vec![(5, None), (6, Some(w.room.ed.d.entities[0].clone()))],
        };
        let text = serde_json::to_string(&m).unwrap();
        let back: Msg = serde_json::from_str(&text).unwrap();
        assert_eq!(serde_json::to_string(&back).unwrap(), text);
    }

    /// 공동작업 중 되돌리기는 내 작업만 되감는다 — 그 사이 다른 사람이 더한 객체는 남고, 모두 같다
    #[test]
    fn shared_undo_keeps_others() {
        let mut w = World::new();
        w.edit(0, |e| {
            e.add_shapes(vec![line(500.0)]);
        });
        w.edit(1, |e| {
            e.add_shapes(vec![line(600.0)]);
        });
        w.settle();
        w.same();
        // 앱 0 이 되돌린다 (자기 선만 지워져야)
        let inv = w.apps[0].undo_shared().unwrap();
        if let Some(Msg::Op { n, op }) = w.clients[0].local(&inv) {
            w.outbox[0].push((n, op));
        }
        w.settle();
        w.same();
        let ys: Vec<f64> = w.apps[1]
            .d
            .entities
            .iter()
            .map(|e| match e.shape {
                Shape::Line { a, .. } => a.y,
                _ => f64::NAN,
            })
            .collect();
        assert!(ys.contains(&600.0) && !ys.contains(&500.0), "{ys:?}");
        // 다시 하기
        let again = w.apps[0].redo_shared().unwrap();
        if let Some(Msg::Op { n, op }) = w.clients[0].local(&again) {
            w.outbox[0].push((n, op));
        }
        w.settle();
        w.same();
        assert_eq!(w.room.ed.d.entities.len(), 5);
    }

    /// 모두 같아야 하는 건축 모델 (번호 시작점·지금 보는 층은 사람마다)
    fn shared(m: &crate::arch::ArchModel) -> crate::arch::ArchModel {
        crate::arch::ArchModel {
            next_id: 0,
            active_level: 0,
            ..m.clone()
        }
    }

    /// 앱 k 의 건축 모델을 바꾼다 (모델 통째 작업 — 앱의 건축 편집과 같다)
    fn arch_edit(w: &mut World, k: usize, f: impl FnOnce(&mut crate::arch::ArchModel)) {
        w.edit(k, |e| {
            let old = e.d.arch.clone();
            let mut new = old.clone();
            f(&mut new);
            e.run(Op::Arch {
                old: Box::new(old),
                new: Box::new(new),
            });
        });
    }

    fn walled() -> World {
        let mut w = World::new();
        // 벽 둘이 있는 도면에서 시작
        let mut m = crate::arch::ArchModel::default();
        let lv = m.ensure_level();
        for y in [0.0, 5000.0] {
            let id = m.new_id();
            m.walls.push(crate::arch::Wall {
                id,
                level: lv,
                a: Vec2::new(0.0, y),
                b: Vec2::new(6000.0, y),
                thickness: 200.0,
                ..Default::default()
            });
        }
        w.room.ed.d.arch = m.clone();
        for (k, a) in w.apps.iter_mut().enumerate() {
            a.d.arch = m.clone();
            a.d.salt_ids(k as u64 + 7);
        }
        w
    }

    /// 건축 객체 (§91): 다른 벽을 동시에 고치면 둘 다 남는다, 같은 벽은 어느 차례로 닿아도 모두 같다,
    /// 동시에 새 벽을 그려도 번호가 겹치지 않는다
    #[test]
    fn arch_objects_merge() {
        let mut w = walled();
        arch_edit(&mut w, 0, |m| m.walls[0].thickness = 300.0);
        arch_edit(&mut w, 1, |m| m.walls[1].thickness = 400.0);
        w.settle();
        let t: Vec<f64> = w.room.ed.d.arch.walls.iter().map(|x| x.thickness).collect();
        assert_eq!(t, vec![300.0, 400.0]);
        for a in &w.apps {
            assert_eq!(shared(&a.d.arch), shared(&w.room.ed.d.arch));
        }
        // 지금 보는 층은 사람마다 — 앱 0 이 바꿔도 다른 사람은 그대로
        arch_edit(&mut w, 0, |m| m.active_level = 999);
        w.settle();
        assert_eq!(w.apps[0].d.arch.active_level, 999);
        assert_ne!(w.apps[1].d.arch.active_level, 999);
        assert_ne!(w.room.ed.d.arch.active_level, 999);
        // 같은 벽을 동시에 — 두 차례 모두
        for first in [0, 1] {
            let mut w = walled();
            arch_edit(&mut w, 0, |m| m.walls[0].thickness = 111.0);
            arch_edit(&mut w, 1, |m| m.walls[0].thickness = 222.0);
            w.send(first);
            w.send(1 - first);
            w.recv(0);
            w.recv(1);
            for a in &w.apps {
                assert_eq!(shared(&a.d.arch), shared(&w.room.ed.d.arch), "차례 {first}");
            }
        }
        // 동시에 새 벽
        let mut w = walled();
        for k in 0..2 {
            arch_edit(&mut w, k, |m| {
                let lv = m.levels[0].id;
                let id = m.new_id();
                m.walls.push(crate::arch::Wall {
                    id,
                    level: lv,
                    a: Vec2::new(0.0, 9000.0 + k as f64),
                    b: Vec2::new(6000.0, 9000.0),
                    thickness: 200.0,
                    ..Default::default()
                });
            });
        }
        w.settle();
        assert_eq!(w.room.ed.d.arch.walls.len(), 4);
        let ids: HashSet<u64> = w.room.ed.d.arch.walls.iter().map(|x| x.id).collect();
        assert_eq!(ids.len(), 4);
        for a in &w.apps {
            assert_eq!(a.d.arch.walls.len(), 4);
        }
    }
}
