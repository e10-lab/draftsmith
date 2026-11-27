//! 변경 이력 (Loro CRDT, 구현 노트 §85).
//!
//! `.dsmith` 안 `document.loro` 에 도면을 Loro 문서로도 담는다. 서버 없이 같은 파일에서 갈라진 두 사본을 합칠 수 있다
//! (`dsmith merge`). 나중에 동기화 서버(M5)가 이 문서의 증분을 주고받는다.
//!
//! 구성 (§80 측정에서 가벼웠던 꼴):
//! - `entities` 맵: 고유 번호(16 진 글) → 객체 JSON 글. 객체 하나가 바뀌면 그 칸만 다시 쓴다 (마지막에 쓴 것이 이김)
//! - `order` 목록: 그리는 순서 (고유 번호 글). 넣기·지우기는 목록 연산이라 동시에 해도 합쳐진다
//! - `meta` 맵: 객체 밖 필드(레이어·블록·건축 객체 …) 이름 → 그 JSON 의 지문(FNV-1a 64, 16 진). 바뀐 필드만 쓴다.
//!   내용은 `drawing.json` 에만 있고, 도면을 만들 때 지문이 맞는 사본의 필드를 쓴다 — 실무 도면은 블록 정의 JSON 이
//!   53 MB 라 내용을 넣으면 (상태 + 이력 두 벌) 이력이 27~47 MB 가 된다

use std::collections::HashMap;

use loro::{ExportMode, LoroDoc, LoroList, LoroMap, LoroValue};

use crate::drawing::{Drawing, Entity};

/// 도면의 Loro 문서와, 마지막으로 맞춘 내용 (다음에 바뀐 것만 쓰려고)
pub struct Crdt {
    doc: LoroDoc,
    /// 고유 번호 → 마지막으로 쓴 JSON
    ents: HashMap<u64, String>,
    order: Vec<u64>,
    /// 필드 이름 → 지문
    meta: HashMap<String, String>,
}

/// 내용 지문 (FNV-1a 64) — 판이 바뀌어도 같은 값이라 파일에 남겨도 된다
fn digest(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

fn key(id: u64) -> String {
    format!("{id:x}")
}

/// 객체 밖 필드들을 (이름, JSON) 으로 — 필드마다 바로 글로 (객체까지 값 나무로 만들면 실무 도면에서 1 초 넘게)
fn meta_of(d: &Drawing) -> Result<Vec<(&'static str, String)>, String> {
    let Drawing {
        layers,
        entities: _,
        code_page,
        skipped,
        inserts,
        units,
        linetypes,
        layouts,
        recovered,
        current_layer,
        arch,
        blocks,
        fonts,
        next_id,
    } = d;
    let j = |v: &dyn erased::Ser| v.json();
    Ok(vec![
        ("layers", j(layers)?),
        ("code_page", j(code_page)?),
        ("skipped", j(skipped)?),
        ("inserts", j(inserts)?),
        ("units", j(units)?),
        ("linetypes", j(linetypes)?),
        ("layouts", j(layouts)?),
        ("recovered", j(recovered)?),
        ("current_layer", j(current_layer)?),
        ("arch", j(arch)?),
        ("blocks", j(blocks)?),
        ("fonts", j(fonts)?),
        ("next_id", j(next_id)?),
    ])
}

/// 여러 꼴의 값을 한 목록에서 JSON 글로
mod erased {
    pub trait Ser {
        fn json(&self) -> Result<String, String>;
    }
    impl<T: serde::Serialize> Ser for T {
        fn json(&self) -> Result<String, String> {
            serde_json::to_string(self).map_err(|e| e.to_string())
        }
    }
}

impl Crdt {
    fn empty() -> Crdt {
        Crdt {
            doc: LoroDoc::new(),
            ents: HashMap::new(),
            order: Vec::new(),
            meta: HashMap::new(),
        }
    }

    fn entities(&self) -> LoroMap {
        self.doc.get_map("entities")
    }
    fn order_list(&self) -> LoroList {
        self.doc.get_list("order")
    }
    fn meta_map(&self) -> LoroMap {
        self.doc.get_map("meta")
    }

    /// 도면에서 새로
    pub fn new(d: &Drawing) -> Result<Crdt, String> {
        let mut c = Crdt::empty();
        c.sync(d)?;
        Ok(c)
    }

    /// `document.loro` 에서. 들고 있는 내용 기억도 문서에서 다시 만든다
    pub fn from_bytes(b: &[u8]) -> Result<Crdt, String> {
        let mut c = Crdt::empty();
        c.doc
            .import(b)
            .map_err(|e| format!("변경 이력을 읽지 못했습니다: {e}"))?;
        c.reload();
        Ok(c)
    }

    /// 문서 지금 상태로 기억을 다시
    fn reload(&mut self) {
        self.ents.clear();
        if let LoroValue::Map(m) = self.entities().get_value() {
            for (k, v) in m.iter() {
                if let (Ok(id), LoroValue::String(s)) = (u64::from_str_radix(k, 16), v) {
                    self.ents.insert(id, s.to_string());
                }
            }
        }
        self.order = self.read_order();
        self.meta.clear();
        if let LoroValue::Map(m) = self.meta_map().get_value() {
            for (k, v) in m.iter() {
                if let LoroValue::String(s) = v {
                    self.meta.insert(k.to_string(), s.to_string());
                }
            }
        }
    }

    /// 순서 목록 (겹친 번호는 처음 것만 — 따로 만든 두 이력을 합치면 같은 번호가 두 번 들어갈 수 있다)
    fn read_order(&self) -> Vec<u64> {
        let mut seen = std::collections::HashSet::new();
        match self.order_list().get_value() {
            LoroValue::List(l) => l
                .iter()
                .filter_map(|v| match v {
                    LoroValue::String(s) => u64::from_str_radix(s, 16).ok(),
                    _ => None,
                })
                .filter(|id| seen.insert(*id))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// 스냅샷 바이트
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        self.doc
            .export(ExportMode::Snapshot)
            .map_err(|e| e.to_string())
    }

    /// 도면을 문서에 맞춘다 — 바뀐 객체·필드·순서만 쓴다. 번호 없는 객체(0)는 건너뛴다. 쓴 객체 수
    pub fn sync(&mut self, d: &Drawing) -> Result<usize, String> {
        let ents = self.entities();
        let mut n = 0;
        let mut now: Vec<u64> = Vec::with_capacity(d.entities.len());
        let mut alive = std::collections::HashSet::with_capacity(d.entities.len());
        for e in &d.entities {
            if e.id == 0 || !alive.insert(e.id) {
                continue;
            }
            now.push(e.id);
            let s = serde_json::to_string(e).map_err(|e| e.to_string())?;
            if self.ents.get(&e.id) != Some(&s) {
                ents.insert(&key(e.id), s.as_str())
                    .map_err(|e| e.to_string())?;
                self.ents.insert(e.id, s);
                n += 1;
            }
        }
        let gone: Vec<u64> = self
            .ents
            .keys()
            .filter(|id| !alive.contains(id))
            .copied()
            .collect();
        for id in gone {
            ents.delete(&key(id)).map_err(|e| e.to_string())?;
            self.ents.remove(&id);
            n += 1;
        }
        self.sync_order(&now)?;
        let meta = self.meta_map();
        for (k, s) in meta_of(d)? {
            let h = digest(&s);
            if self.meta.get(k) != Some(&h) {
                meta.insert(k, h.as_str()).map_err(|e| e.to_string())?;
                self.meta.insert(k.to_string(), h);
            }
        }
        self.doc.commit();
        Ok(n)
    }

    /// 순서 목록을 `now` 로: 없어진 번호는 지우고, 새 번호는 제자리에 넣는다 (남은 번호의 순서가 그대로일 때).
    /// 순서가 바뀌었으면 통째로 다시 쓴다
    fn sync_order(&mut self, now: &[u64]) -> Result<(), String> {
        let list = self.order_list();
        // 문서의 실제 목록 (겹친 번호까지)으로 지운다
        let raw: Vec<Option<u64>> = match list.get_value() {
            LoroValue::List(l) => l
                .iter()
                .map(|v| match v {
                    LoroValue::String(s) => u64::from_str_radix(s, 16).ok(),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let keep: std::collections::HashSet<u64> = now.iter().copied().collect();
        let mut seen = std::collections::HashSet::new();
        for (i, id) in raw.iter().enumerate().rev() {
            let drop = match id {
                Some(id) => !keep.contains(id),
                None => true,
            };
            if drop {
                list.delete(i, 1).map_err(|e| e.to_string())?;
            }
        }
        // 겹친 번호는 뒤의 것을 지운다 (뒤에서부터 지우려고 앞에서 처음 나온 자리를 먼저 센다)
        let raw: Vec<u64> = raw
            .into_iter()
            .flatten()
            .filter(|id| keep.contains(id))
            .collect();
        let mut first = std::collections::HashSet::new();
        let dup: Vec<bool> = raw.iter().map(|id| !first.insert(*id)).collect();
        for i in (0..raw.len()).rev() {
            if dup[i] {
                list.delete(i, 1).map_err(|e| e.to_string())?;
            }
        }
        let cur: Vec<u64> = raw.into_iter().filter(|id| seen.insert(*id)).collect();
        // 남은 번호가 새 순서의 부분 순서인가
        let curset: std::collections::HashSet<u64> = cur.iter().copied().collect();
        let mut it = now.iter().filter(|id| curset.contains(id));
        let same_order = cur.iter().all(|c| it.next() == Some(c));
        if !same_order {
            list.clear().map_err(|e| e.to_string())?;
            for (i, id) in now.iter().enumerate() {
                list.insert(i, key(*id).as_str())
                    .map_err(|e| e.to_string())?;
            }
        } else {
            for (i, id) in now.iter().enumerate() {
                if !curset.contains(id) {
                    list.insert(i, key(*id).as_str())
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        self.order = now.to_vec();
        Ok(())
    }

    /// 다른 사본의 변경 이력을 합친다 (같은 파일에서 갈라진 사본이면 각자의 편집이 모두 들어온다)
    pub fn merge(&mut self, other: &[u8]) -> Result<(), String> {
        self.doc
            .import(other)
            .map_err(|e| format!("변경 이력을 합치지 못했습니다: {e}"))?;
        self.reload();
        Ok(())
    }

    /// 문서 지금 상태의 도면. 객체 밖 필드는 `sources`(합친 사본들의 도면) 중 지문이 맞는 것에서 — 맞는 것이 없으면
    /// 첫 사본의 것
    pub fn drawing(&self, sources: &[&Drawing]) -> Result<Drawing, String> {
        if sources.is_empty() {
            return Err("사본이 없습니다".into());
        }
        let metas: Vec<Vec<(&'static str, String)>> = sources
            .iter()
            .map(|d| meta_of(d))
            .collect::<Result<_, _>>()?;
        // 필드 글들을 이어 한 번에 읽는다
        let mut text = String::from("{");
        for (i, (k, _)) in metas[0].iter().enumerate() {
            let want = self.meta.get(*k);
            let v = metas
                .iter()
                .map(|m| &m[i].1)
                .find(|v| want == Some(&digest(v)))
                .unwrap_or(&metas[0][i].1);
            text.push_str(&serde_json::to_string(k).map_err(|e| e.to_string())?);
            text.push(':');
            text.push_str(v);
            text.push(',');
        }
        text.push_str("\"entities\":[]}");
        let mut d: Drawing = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        for id in &self.order {
            if let Some(s) = self.ents.get(id) {
                let e: Entity = serde_json::from_str(s).map_err(|e| e.to_string())?;
                d.entities.push(e);
            }
        }
        d.next_id = d
            .next_id
            .max(d.entities.iter().map(|e| e.id).max().unwrap_or(0) + 1);
        Ok(d)
    }
}

/// 편집 중인 도면에 다른 사본을 합친다 (§86): 이력을 지금 도면에 맞추고 상대 이력(없으면 상대 도면으로 새로)을 합쳐,
/// 합친 도면으로 바꾼다(되돌릴 수 있다). 합친 뒤 객체 수
pub fn merge_into(
    ed: &mut crate::edit::Editor,
    c: &mut Crdt,
    other: &Drawing,
    theirs: Option<&[u8]>,
) -> Result<usize, String> {
    c.sync(&ed.d)?;
    let bytes = match theirs {
        Some(b) => b.to_vec(),
        None => Crdt::new(other)?.to_bytes()?,
    };
    c.merge(&bytes)?;
    let merged = c.drawing(&[&ed.d, other])?;
    let n = merged.entities.len();
    ed.replace_drawing(merged);
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawing::Shape;
    use crate::edit::Editor;
    use crate::geom::{Affine, Vec2};

    fn line(y: f64) -> Shape {
        Shape::Line {
            a: Vec2::new(0.0, y),
            b: Vec2::new(1000.0, y),
        }
    }

    fn ys(d: &Drawing) -> Vec<f64> {
        d.entities
            .iter()
            .map(|e| match e.shape {
                Shape::Line { a, .. } => a.y,
                Shape::Circle { radius, .. } => -radius,
                _ => f64::NAN,
            })
            .collect()
    }

    /// 같은 파일에서 갈라진 두 사본: 각자 옮기기·지우기·더하기를 하고 합치면 모두 들어온다 (§85)
    #[test]
    fn merge_two_copies() {
        let mut base = Editor::new(crate::archgen::new_drawing());
        base.add_shapes(vec![line(0.0), line(100.0), line(200.0)]);
        let c0 = Crdt::new(&base.d).unwrap();
        let file = c0.to_bytes().unwrap();
        // 되읽은 문서는 원래 도면과 같다
        let back = Crdt::from_bytes(&file)
            .unwrap()
            .drawing(&[&base.d])
            .unwrap();
        assert_eq!(back.entities, base.d.entities);
        assert_eq!(back.layers, base.d.layers);

        // 갑: 첫 선을 옮기고 새 선 하나 (번호 시작점을 띄워 을과 겹치지 않게)
        let mut a = Editor::new(back.clone());
        a.d.salt_ids(1);
        a.transform(&[0], &Affine::translate(0.0, 10.0));
        a.add_shapes(vec![line(300.0)]);
        let mut ca = Crdt::from_bytes(&file).unwrap();
        ca.sync(&a.d).unwrap();
        // 을: 가운데 선을 지우고 원 하나
        let mut b = Editor::new(back);
        b.d.salt_ids(2);
        b.remove(&[1]);
        b.add_shapes(vec![Shape::Circle {
            center: Vec2::new(0.0, 0.0),
            radius: 50.0,
        }]);
        let mut cb = Crdt::from_bytes(&file).unwrap();
        cb.sync(&b.d).unwrap();

        ca.merge(&cb.to_bytes().unwrap()).unwrap();
        let m = ca.drawing(&[&a.d, &b.d]).unwrap();
        let got = ys(&m);
        assert_eq!(got.len(), 4, "{got:?}");
        assert!(
            got.contains(&10.0)
                && got.contains(&200.0)
                && got.contains(&300.0)
                && got.contains(&-50.0)
        );
        assert!(!got.contains(&100.0) && !got.contains(&0.0));
        // 번호가 겹치지 않는다
        let ids: std::collections::HashSet<u64> = m.entities.iter().map(|e| e.id).collect();
        assert_eq!(ids.len(), 4);
        // 객체 밖 필드(레이어)는 지문이 맞는 사본에서: 갑이 레이어 색을 바꾸면 합친 도면에 갑의 색
        let mut a2 = Editor::new(a.d.clone());
        let name = a2.d.layers[0].name.clone();
        a2.edit_layer(0, &name, [1, 2, 3]);
        let mut ca2 = Crdt::from_bytes(&ca.to_bytes().unwrap()).unwrap();
        ca2.sync(&a2.d).unwrap();
        let mut cb2 = Crdt::from_bytes(&cb.to_bytes().unwrap()).unwrap();
        cb2.merge(&ca2.to_bytes().unwrap()).unwrap();
        let m2 = cb2.drawing(&[&b.d, &a2.d]).unwrap();
        assert_eq!(m2.layers[0].color, [1, 2, 3]);
        assert_eq!(m2.entities.len(), 4);
        // 반대로 합쳐도 같은 도면
        cb.merge(&ca.to_bytes().unwrap()).unwrap();
        assert_eq!(cb.drawing(&[&b.d, &a.d]).unwrap().entities, m.entities);
    }

    /// 순서가 바뀌면 순서 목록을 다시 쓴다, 바뀐 것만 쓴다
    #[test]
    fn sync_writes_only_changes() {
        let mut ed = Editor::new(crate::archgen::new_drawing());
        ed.add_shapes(vec![line(0.0), line(100.0), line(200.0)]);
        let mut c = Crdt::new(&ed.d).unwrap();
        assert_eq!(c.sync(&ed.d).unwrap(), 0);
        ed.transform(&[2], &Affine::translate(0.0, 1.0));
        assert_eq!(c.sync(&ed.d).unwrap(), 1);
        ed.d.entities.swap(0, 1);
        c.sync(&ed.d).unwrap();
        assert_eq!(c.drawing(&[&ed.d]).unwrap().entities, ed.d.entities);
    }

    /// .dsmith 안 document.loro 로 저장했다 열면 그대로 (§85)
    #[test]
    fn docfile_round_trip() {
        let mut ed = Editor::new(crate::archgen::new_drawing());
        ed.add_shapes(vec![line(0.0), line(100.0)]);
        let c = Crdt::new(&ed.d).unwrap();
        let loro = c.to_bytes().unwrap();
        let b = crate::docfile::to_bytes_with(&ed.d, Some(&loro)).unwrap();
        let (d, l) = crate::docfile::from_bytes_with(&b).unwrap();
        assert_eq!(d.entities, ed.d.entities);
        let back = Crdt::from_bytes(&l.unwrap())
            .unwrap()
            .drawing(&[&d])
            .unwrap();
        assert_eq!(back.entities, ed.d.entities);
        // 이력 없는 파일은 None
        let (_, none) =
            crate::docfile::from_bytes_with(&crate::docfile::to_bytes(&ed.d).unwrap()).unwrap();
        assert!(none.is_none());
    }

    /// 편집기에서 합친 도면으로 바꾸면 되돌릴 수 있다 (§86)
    #[test]
    fn editor_replace_drawing_undo() {
        let mut a = Editor::new(crate::archgen::new_drawing());
        a.add_shapes(vec![line(0.0), line(100.0)]);
        let before = a.d.entities.clone();
        let mut other = a.d.clone();
        other.entities.remove(0);
        other.entities.push(Entity {
            id: 0x7777,
            ..other.entities[0].clone()
        });
        a.replace_drawing(other.clone());
        assert_eq!(a.d.entities, other.entities);
        a.undo();
        assert_eq!(a.d.entities, before);
        a.redo();
        assert_eq!(a.d.entities, other.entities);
    }

    /// 편집 중인 도면에 사본 합치기 — 앱의 「다른 사본과 합치기」 (§86)
    #[test]
    fn merge_into_editor() {
        let mut base = Editor::new(crate::archgen::new_drawing());
        base.add_shapes(vec![line(0.0), line(100.0)]);
        let file = Crdt::new(&base.d).unwrap().to_bytes().unwrap();
        // 내 쪽: 선 하나 더 (저장 안 한 편집도 합칠 때 맞춘다)
        let mut me = Editor::new(base.d.clone());
        me.d.salt_ids(11);
        me.add_shapes(vec![line(300.0)]);
        let mut mine = Crdt::from_bytes(&file).unwrap();
        // 상대: 첫 선 지우고 저장
        let mut they = Editor::new(base.d.clone());
        they.d.salt_ids(22);
        they.remove(&[0]);
        let mut tc = Crdt::from_bytes(&file).unwrap();
        tc.sync(&they.d).unwrap();
        let n = merge_into(&mut me, &mut mine, &they.d, Some(&tc.to_bytes().unwrap())).unwrap();
        assert_eq!(n, 2);
        assert_eq!(ys(&me.d), vec![100.0, 300.0]);
        // 되돌리면 합치기 전
        me.undo();
        assert_eq!(ys(&me.d), vec![0.0, 100.0, 300.0]);
    }
}
