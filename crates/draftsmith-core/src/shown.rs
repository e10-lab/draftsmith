//! 보이는 도면 캐시 (구현 노트 §72).
//!
//! `archgen::with_arch_scaled` 는 편집마다 모형을 통째로 복제하고, 블록 참조를 모두 펼치고(실무 도면 4.9 만 객체),
//! 해치를 선분으로·글자를 SHX 획으로 바꾸고, 가리개로 앞의 선을 자른다. 여기서는 모형 객체마다 남긴 것(자기 자신과
//! 펼친 객체)을 기억해 두고, 이전 결과 도면을 넘겨받아 바뀐 모형 객체 몫만 갈아 끼운다. 건축 객체가 만드는 선은 작아서
//! 매번 새로 만든다. 결과는 늘 `with_arch_scaled` 와 같다.
//!
//! 보이는 도면 순서: [모형 객체 (처리한)] [펼친 객체 (모형 순서대로)] [건축 객체 선] [SHX 에 없는 글자].
//! 가리개는 자기보다 뒤의 객체는 못 가리므로, 바뀐 객체에 가리개가 없으면 나머지 객체의 잘린 모양은 그대로다.
//! 바뀐 객체(앞·뒤)에 가리개가 있거나, 블록 정의·레이어 「0」 자리·글꼴·SHX 폴더가 바뀌면 새로 만든다.

use std::path::PathBuf;

use crate::drawing::{Block, Drawing, Entity, FontUse, Shape};
use crate::shx::Styles;
use crate::wipeout::{self, Wipe};

/// 모형 객체 하나가 보이는 도면에 남긴 것
#[derive(Debug, Clone)]
struct Piece {
    /// 펼친 객체 수
    exp: usize,
    /// SHX 에 없어 맨 뒤에 내장 글꼴로 붙는 글자 (자기 몫, 펼친 객체 몫)
    extra_own: Vec<Entity>,
    extra_exp: Vec<Entity>,
    /// 자기나 펼친 객체 중에 가리개가 있나
    wipe: bool,
}

/// 보이는 도면 안 자리: (0 모형 · 1 펼친 것, 모형 객체 번호, 펼친 것 안 번호). 크기 순서가 그리기 순서다
type Pos = (u8, usize, usize);

/// 보이는 도면과 객체 주인 (`with_arch_scaled` 와 같은 꼴)
pub type Shown = (Drawing, Vec<(String, u64)>);

/// 보이는 도면 캐시
#[derive(Default)]
pub struct ShownCache {
    valid: bool,
    blocks: Vec<Block>,
    zero: Option<usize>,
    fonts: Vec<FontUse>,
    shx: Option<PathBuf>,
    /// 지난번 모형 객체
    src: Vec<Entity>,
    pieces: Vec<Piece>,
    /// 가리개들 (보이는 도면 순서)
    wipes: Vec<(Pos, Wipe)>,
    /// 지난 결과의 모형·펼친 부분 길이 (그 뒤는 매번 새로)
    base: usize,
    /// 지난 결과의 객체 수·주인 수 (넘겨받은 결과가 맞는지 보려고)
    len: (usize, usize),
    /// 건축 객체 선 (지난번 입력과 결과 크기)
    tail: Option<Tail>,
    /// 마지막에 다시 처리한 모형 객체 수 (측정·시험용)
    pub changed: usize,
}

/// 건축 객체 선(꼬리)을 만든 입력과, 결과의 객체 수·주인 수·덧붙은 글자
#[derive(Default)]
struct Tail {
    arch: crate::arch::ArchModel,
    scale: f64,
    layers: Vec<crate::drawing::Layer>,
    linetypes: Vec<crate::drawing::LinePattern>,
    len: usize,
    owners: usize,
    extra: Vec<Entity>,
}

/// 모형 객체 하나 처리: (자기, 펼친 객체들, 남긴 것). 가리개로 자르기는 아직
fn process(d: &Drawing, e: &Entity, styles: Option<&Styles>) -> (Entity, Vec<Entity>, Piece) {
    let mut own = e.clone();
    let mut exp = if matches!(e.shape, Shape::Insert { .. }) {
        crate::block::expand(d, e)
    } else {
        Vec::new()
    };
    let wipe = matches!(own.shape, Shape::Wipeout { .. })
        || exp.iter().any(|x| matches!(x.shape, Shape::Wipeout { .. }));
    let (mut extra_own, mut extra_exp) = (Vec::new(), Vec::new());
    for (k, x) in std::iter::once(&mut own).chain(exp.iter_mut()).enumerate() {
        // 무늬 해치는 화면·출력용 선분으로
        if matches!(x.shape, Shape::Hatch { .. }) {
            x.shape = x.shape.drawn();
        }
        if let Some(st) = styles {
            st.stroke(
                x,
                if k == 0 {
                    &mut extra_own
                } else {
                    &mut extra_exp
                },
            );
        }
    }
    let piece = Piece {
        exp: exp.len(),
        extra_own,
        extra_exp,
        wipe,
    };
    (own, exp, piece)
}

impl ShownCache {
    /// 보이는 도면. `prev` 는 이 캐시가 지난번에 돌려준 결과(돌려받아 고쳐 쓴다). 따로 만들 것이 없으면 None
    pub fn update(&mut self, d: &Drawing, scale: f64, prev: Option<Shown>) -> Option<Shown> {
        if !crate::archgen::needs_shown(d) {
            self.valid = false;
            return None;
        }
        let shx = crate::shx::folder();
        let styles = Styles::of(d);
        let zero = d.layers.iter().position(|l| l.name == "0");
        let prev = prev.filter(|p| (p.0.entities.len(), p.1.len()) == self.len);
        let mut fresh = !self.valid
            || prev.is_none()
            || self.zero != zero
            || self.shx != shx
            || self.fonts != d.fonts
            || self.blocks != d.blocks;
        let (mut pre, mut old_end, mut new_end) = (0, self.src.len(), d.entities.len());
        if !fresh {
            let (p, s) = crate::drawing::align(&self.src, &d.entities);
            (pre, old_end, new_end) = (p, self.src.len() - s, d.entities.len() - s);
            fresh = self.pieces[pre..old_end].iter().any(|p| p.wipe);
        }
        let mut mid: Vec<(Entity, Vec<Entity>, Piece)> = Vec::new();
        if !fresh {
            mid = d.entities[pre..new_end]
                .iter()
                .map(|e| process(d, e, styles.as_ref()))
                .collect();
            fresh = mid.iter().any(|m| m.2.wipe);
        }
        // 건축 객체 선(꼬리)은 건축 모델·축척·레이어·선 종류가 같으면 그대로 둔다
        let keep_tail = !fresh
            && self.tail.as_ref().is_some_and(|t| {
                t.scale == scale
                    && t.layers == d.layers
                    && t.linetypes == d.linetypes
                    && t.arch == d.arch
            });
        let (mut out, mut owners) = match prev {
            Some((p, o)) if !fresh => (p, o),
            _ => (Drawing::default(), Vec::new()),
        };
        let mut ents = std::mem::take(&mut out.entities);
        if fresh {
            self.pieces.clear();
            self.src.clear();
            self.wipes.clear();
            self.tail = None;
            (pre, old_end, new_end) = (0, 0, d.entities.len());
            mid = d
                .entities
                .iter()
                .map(|e| process(d, e, styles.as_ref()))
                .collect();
        } else {
            // 덧붙은 글자는 버리고, 꼬리는 그대로 둘 때만 남긴다
            let (tl, to) = match (&self.tail, keep_tail) {
                (Some(t), true) => (t.len, t.owners),
                _ => (0, 0),
            };
            ents.truncate(self.base + tl);
            owners.truncate(self.base - self.src.len() + to);
        }
        self.changed = new_end - pre;

        // 펼친 부분 먼저 (뒤에 있으므로), 그다음 모형 부분
        let n_old = self.pieces.len();
        let exp_before: usize = self.pieces[..pre].iter().map(|p| p.exp).sum();
        let exp_old: usize = self.pieces[pre..old_end].iter().map(|p| p.exp).sum();
        let exp_total_old: usize = self.pieces.iter().map(|p| p.exp).sum();
        let mut own_new = Vec::with_capacity(mid.len());
        let mut exp_new = Vec::new();
        let mut own_new_owners = Vec::new();
        let mut pieces_new = Vec::with_capacity(mid.len());
        for (k, (own, exp, piece)) in mid.into_iter().enumerate() {
            own_new_owners.extend(std::iter::repeat_n(
                ("block".to_string(), (pre + k) as u64),
                exp.len(),
            ));
            own_new.push(own);
            exp_new.extend(exp);
            pieces_new.push(piece);
        }
        let exp_new_len = exp_new.len();
        let ex0 = n_old + exp_before;
        ents.splice(ex0..ex0 + exp_old, exp_new);
        owners.splice(exp_before..exp_before + exp_old, own_new_owners);
        ents.splice(pre..old_end, own_new);
        self.pieces.splice(pre..old_end, pieces_new);
        // 뒤 모형 객체 번호가 밀렸으면 펼친 객체의 주인 번호·가리개 자리도 (꼬리 주인은 건축 객체라 그대로)
        let shift = |j: usize| (j as isize + new_end as isize - old_end as isize) as usize;
        if new_end != old_end {
            let exp_end = exp_total_old - exp_old + exp_new_len;
            for o in owners[exp_before + exp_new_len..exp_end].iter_mut() {
                o.1 = shift(o.1 as usize) as u64;
            }
            for (p, _) in self.wipes.iter_mut() {
                if p.1 >= old_end {
                    p.1 = shift(p.1);
                }
            }
        }

        // 가리개로 자르기: 새로면 모두, 아니면 바뀐 객체만 (가리개는 바뀐 곳 밖에만 있다)
        let n = self.pieces.len();
        let exp_at = n + exp_before;
        if fresh {
            let mut m = n;
            for (i, e) in ents[..n].iter().enumerate() {
                if let Some(w) = wipeout::wipe_of(&e.shape) {
                    self.wipes.push(((0, i, 0), w));
                }
            }
            for (j, p) in self.pieces.iter().enumerate() {
                for k in 0..p.exp {
                    if let Some(w) = wipeout::wipe_of(&ents[m + k].shape) {
                        self.wipes.push(((1, j, k), w));
                    }
                }
                m += p.exp;
            }
        }
        if !self.wipes.is_empty() {
            let wipes = &self.wipes;
            let later = |at: Pos| wipes.iter().filter(move |(p, _)| *p > at).map(|(_, w)| w);
            for (i, e) in ents.iter_mut().enumerate().take(new_end).skip(pre) {
                wipeout::clip(e, later((0, i, 0)));
            }
            let mut m = exp_at;
            for j in pre..new_end {
                for k in 0..self.pieces[j].exp {
                    wipeout::clip(&mut ents[m + k], later((1, j, k)));
                }
                m += self.pieces[j].exp;
            }
            if fresh {
                ents.iter_mut().for_each(wipeout::hide);
            }
        }
        self.base = n + self.pieces.iter().map(|p| p.exp).sum::<usize>();
        out.entities = ents;

        // 나머지 필드는 모형에서. 큰 것(블록 정의·배치·건축 모델)은 넘겨받은 것이 같으면 그대로
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
        } = d;
        out.code_page.clone_from(code_page);
        out.skipped.clone_from(skipped);
        out.inserts = *inserts;
        out.units = *units;
        out.recovered.clone_from(recovered);
        out.current_layer = *current_layer;
        out.fonts.clone_from(fonts);
        if fresh {
            out.blocks = blocks.clone();
        }
        if out.layouts != *layouts {
            out.layouts = layouts.clone();
        }
        if !keep_tail {
            // 건축 객체 선 (가리개보다 뒤라 잘리지 않는다). 레이어·선 종류는 생성이 더할 수 있어 모형 것으로 다시
            out.layers.clone_from(layers);
            out.linetypes.clone_from(linetypes);
            if out.arch != *arch {
                out.arch = arch.clone();
            }
            let made = crate::archgen::generate_arch(&mut out, scale, Vec::new());
            let len = made.len();
            let mut extra = Vec::new();
            for (mut e, (kind, id)) in made {
                if matches!(e.shape, Shape::Hatch { .. }) {
                    e.shape = e.shape.drawn();
                }
                if let Some(st) = &styles {
                    st.stroke(&mut e, &mut extra);
                }
                out.entities.push(e);
                owners.push((kind.to_string(), id));
            }
            self.tail = Some(Tail {
                arch: arch.clone(),
                scale,
                layers: layers.clone(),
                linetypes: linetypes.clone(),
                len,
                owners: len,
                extra,
            });
        }
        // SHX 에 없는 글자 (모형 몫, 펼친 몫, 건축 몫 순서)
        for p in &self.pieces {
            out.entities.extend(p.extra_own.iter().cloned());
        }
        for p in &self.pieces {
            out.entities.extend(p.extra_exp.iter().cloned());
        }
        if let Some(t) = &self.tail {
            out.entities.extend(t.extra.iter().cloned());
        }

        if fresh {
            self.src = d.entities.clone();
            self.blocks = d.blocks.clone();
            self.fonts = d.fonts.clone();
            self.zero = zero;
            self.shx = shx;
            self.valid = true;
        } else {
            self.src
                .splice(pre..old_end, d.entities[pre..new_end].iter().cloned());
        }
        self.len = (out.entities.len(), owners.len());
        Some((out, owners))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Vec2;

    fn ent(layer: u32, shape: Shape) -> Entity {
        Entity {
            layer,
            color: None,
            aci: None,
            linetype: None,
            weight: None,
            shape,
        }
    }

    fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> Shape {
        Shape::Line {
            a: Vec2::new(x0, y0),
            b: Vec2::new(x1, y1),
        }
    }

    fn square(x: f64, y: f64, s: f64) -> Vec<Vec2> {
        vec![
            Vec2::new(x, y),
            Vec2::new(x + s, y),
            Vec2::new(x + s, y + s),
            Vec2::new(x, y + s),
        ]
    }

    /// 블록·가리개·해치가 섞인 건축 도면
    fn sample() -> Drawing {
        let mut d = crate::demo::demo_building().unwrap();
        let zero = d.layer_index("0");
        d.blocks.push(Block {
            name: "B".into(),
            entities: vec![
                ent(zero, line(0.0, 0.0, 1000.0, 0.0)),
                ent(
                    zero,
                    Shape::Wipeout {
                        ring: square(200.0, -100.0, 200.0),
                    },
                ),
                ent(zero, line(0.0, 50.0, 1000.0, 50.0)),
            ],
            attdefs: Vec::new(),
        });
        let ins = |x: f64| {
            ent(
                zero,
                Shape::Insert {
                    block: "B".into(),
                    at: Vec2::new(x, 0.0),
                    scale: Vec2::new(1.0, 1.0),
                    rotation: 0.0,
                    attrs: Vec::new(),
                },
            )
        };
        d.entities.push(ins(0.0));
        d.entities
            .push(ent(zero, line(-500.0, 100.0, 5000.0, 100.0)));
        d.entities.push(ins(3000.0));
        d.entities.push(ent(
            zero,
            Shape::Wipeout {
                ring: square(4000.0, 0.0, 300.0),
            },
        ));
        d.entities
            .push(ent(zero, line(-500.0, 200.0, 5000.0, 200.0)));
        d
    }

    fn check(c: &mut ShownCache, d: &Drawing, prev: Option<Shown>) -> Shown {
        let got = c.update(d, 100.0, prev).unwrap();
        let want = crate::archgen::with_arch_scaled(d, 100.0).unwrap();
        assert_eq!(got.0.entities.len(), want.0.entities.len());
        for (i, (a, b)) in got.0.entities.iter().zip(&want.0.entities).enumerate() {
            assert_eq!(a, b, "객체 {i}");
        }
        assert_eq!(got.1, want.1);
        assert_eq!(got.0.layers, want.0.layers);
        assert_eq!(got.0.linetypes, want.0.linetypes);
        got
    }

    /// 고치기·지우기·넣기·가리개 바꾸기 뒤에도 늘 새로 만든 것과 같다 (§72)
    #[test]
    fn cache_matches_full() {
        let mut c = ShownCache::default();
        let d0 = sample();
        let mut s = check(&mut c, &d0, None);
        assert_eq!(c.changed, d0.entities.len());
        let n = d0.entities.len();
        // 가리개 앞의 선 고치기 — 그 객체만
        let mut d1 = d0.clone();
        d1.entities[n - 4].shape = line(-500.0, 120.0, 5000.0, 120.0);
        s = check(&mut c, &d1, Some(s));
        assert_eq!(c.changed, 1);
        // 가운데 선 지우기 (뒤 블록 참조·가리개 번호가 밀린다)
        let mut d2 = d1.clone();
        d2.entities.remove(n - 4);
        s = check(&mut c, &d2, Some(s));
        assert_eq!(c.changed, 0);
        // 앞쪽에 선 두 개 넣기
        let mut d3 = d2.clone();
        d3.entities.insert(3, ent(0, line(0.0, 10.0, 9000.0, 10.0)));
        d3.entities.insert(3, ent(0, line(0.0, 30.0, 9000.0, 30.0)));
        s = check(&mut c, &d3, Some(s));
        assert_eq!(c.changed, 2);
        // 블록 참조 더하기 (가리개를 품었으니 새로)
        let mut d4 = d3.clone();
        let blk = d0.entities[n - 5].clone();
        d4.entities.insert(5, blk);
        s = check(&mut c, &d4, Some(s));
        assert_eq!(c.changed, d4.entities.len());
        // 건축 객체만 바꿔도 (모형 객체는 그대로)
        let mut d5 = d4.clone();
        d5.arch.walls.pop();
        s = check(&mut c, &d5, Some(s));
        assert_eq!(c.changed, 0);
        // 넘겨받은 결과가 없으면 새로
        let _ = check(&mut c, &d5, None);
        assert_eq!(c.changed, d5.entities.len());
        drop(s);
    }
    /// 실무 도면으로: 여러 자리의 객체를 고치기·지우기·넣기 해도 새로 만든 것과 같다.
    /// `DSMITH_REAL=도면 DSMITH_SHX=폴더 cargo test --release -p draftsmith-core real_drawing -- --ignored`
    #[test]
    #[ignore]
    fn real_drawing() {
        let Ok(path) = std::env::var("DSMITH_REAL") else {
            return;
        };
        if let Ok(dir) = std::env::var("DSMITH_SHX") {
            crate::shx::set_folder(Some(dir.into()));
        }
        let mut d = crate::import::read_file(std::path::Path::new(&path)).unwrap();
        let mut c = ShownCache::default();
        let mut s = check(&mut c, &d, None);
        let n = d.entities.len();
        // 종류별로 하나씩 (블록 참조·글자·선·폴리선·가리개)
        let pick =
            |d: &Drawing, f: &dyn Fn(&Shape) -> bool| d.entities.iter().position(|e| f(&e.shape));
        let mv = crate::geom::Affine::translate(37.0, -11.0);
        let mut steps = 0;
        for f in [
            &(|s: &Shape| matches!(s, Shape::Insert { .. })) as &dyn Fn(&Shape) -> bool,
            &|s| matches!(s, Shape::Text(_)),
            &|s| matches!(s, Shape::Line { .. }),
            &|s| matches!(s, Shape::Polyline { .. }),
            &|s| matches!(s, Shape::Wipeout { .. }),
        ] {
            let Some(i) = pick(&d, f) else { continue };
            // 옮기기
            d.entities[i].shape = d.entities[i].shape.transformed(&mv);
            s = check(&mut c, &d, Some(s));
            // 복사해 끝에 더하기, 가운데에 넣기
            let e = d.entities[i].clone();
            d.entities.push(e.clone());
            s = check(&mut c, &d, Some(s));
            d.entities.insert(n / 3, e);
            s = check(&mut c, &d, Some(s));
            // 지우기
            d.entities.remove(i);
            s = check(&mut c, &d, Some(s));
            steps += 4;
        }
        eprintln!(
            "실무 도면 {} 객체, 편집 {steps} 번 모두 같음",
            d.entities.len()
        );
    }
}
