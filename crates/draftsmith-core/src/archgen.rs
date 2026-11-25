//! 건축 객체 → 평면도 선·면적표·일람표·법규 검토 (M3·M4, 구현 노트 §19).
//!
//! 건축 객체(`arch::ArchModel`)는 도면 모델에 저장되고, 화면·PDF·DXF 에 보이는 선은 여기서 만든다
//! ([`with_arch`]: 모형 객체 뒤에 붙인다). 지금 층(active level)의 벽·문·창·실만 평면에 그리고,
//! 그리드·대지는 모든 층에 그린다.

use serde::{Deserialize, Serialize};

use crate::arch::{ArchModel, Grid, Level, Opening, OpeningKind, Room, Site, Wall, WallKind};
use crate::drawing::{Drawing, Entity, Layer, Shape, TextShape, VAlign};
use crate::edit::{Editor, Op};
use crate::geom::Vec2;

/// 1 평 = 3.305785 ㎡
pub const PYEONG_M2: f64 = 3.305_785;

/// 건축 객체 레이어 (이름, 색) — KS F 1542 체계를 참고한 이름 (구현 노트 §21)
pub const LAYERS: &[(&str, [u8; 3])] = &[
    ("A-WALL", [255, 255, 255]),
    ("A-WALL-PATT", [128, 128, 128]),
    ("A-DOOR", [255, 255, 0]),
    ("A-GLAZ", [0, 255, 255]),
    ("A-AREA", [255, 0, 255]),
    ("A-AREA-IDEN", [255, 255, 255]),
    ("A-GRID", [255, 0, 0]),
    ("A-GRID-IDEN", [255, 255, 255]),
    ("C-PROP", [0, 255, 0]),
    ("A-COLS", [255, 127, 0]),
    ("A-COLS-PATT", [128, 128, 128]),
    ("A-STRS", [255, 255, 255]),
    ("A-ANNO-DIMS", [0, 255, 0]),
];

/// 치수 글자: 세 자리마다 쉼표 (12,000)
pub fn comma(v: f64) -> String {
    let n = v.round() as i64;
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// 새 도면: 기본 레이어(국내 건축 도면 관례)와 「1층」
pub fn new_drawing() -> Drawing {
    let mut d = Drawing {
        units: 4,
        ..Default::default()
    };
    d.layer_index("0");
    for (name, color) in [
        ("A-ANNO-TEXT", [255, 255, 255]),
        ("A-ANNO-DIMS", [0, 255, 0]),
        ("A-DETL", [255, 255, 255]),
        ("A-FURN", [128, 128, 128]),
        ("A-ANNO-TTLB", [255, 255, 255]),
    ] {
        let i = d.layer_index(name) as usize;
        d.layers[i].color = color;
    }
    for (name, color) in LAYERS {
        let i = d.layer_index(name) as usize;
        d.layers[i].color = *color;
    }
    // 그리드는 1점 쇄선
    d.linetypes.push(crate::drawing::LinePattern {
        name: "CENTER".into(),
        dashes: vec![1200.0, -300.0, 300.0, -300.0],
    });
    d.current_layer = 0;
    d.arch.ensure_level();
    d
}

/// 생성한 객체 하나의 주인 (종류, 번호)
type Owner = (&'static str, u64);

struct Gen<'a> {
    d: &'a mut Drawing,
    out: Vec<(Entity, Owner)>,
}

impl Gen<'_> {
    fn layer(&mut self, name: &str) -> u32 {
        if let Some(i) = self.d.layers.iter().position(|l| l.name == name) {
            return i as u32;
        }
        let color = LAYERS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, c)| *c)
            .unwrap_or([255, 255, 255]);
        self.d.layers.push(Layer {
            name: name.into(),
            color,
            visible: true,
        });
        (self.d.layers.len() - 1) as u32
    }

    fn put(&mut self, layer: &str, shape: Shape, owner: Owner) {
        let layer = self.layer(layer);
        let linetype = if self.d.layers[layer as usize].name == "A-GRID" {
            self.d
                .linetypes
                .iter()
                .position(|p| p.name.eq_ignore_ascii_case("CENTER"))
                .map(|i| crate::drawing::LineStyle {
                    pattern: i as u16,
                    scale: 1.0,
                })
        } else {
            None
        };
        let weight = match self.d.layers[layer as usize].name.as_str() {
            "A-WALL" => Some(50),
            "A-DOOR" | "A-GLAZ" => Some(25),
            // 치수는 가늘게 (0.13 mm)
            n if n.contains("DIM") => Some(13),
            _ => None,
        };
        self.out.push((
            Entity {
                layer,
                color: None,
                aci: None,
                linetype,
                weight,
                shape,
            },
            owner,
        ));
    }

    fn line(&mut self, layer: &str, a: Vec2, b: Vec2, owner: Owner) {
        if a.dist(b) > 1e-9 {
            self.put(layer, Shape::Line { a, b }, owner);
        }
    }

    fn text(&mut self, layer: &str, at: Vec2, height: f64, value: String, owner: Owner) {
        self.put(
            layer,
            Shape::Text(TextShape {
                at,
                height,
                rotation: 0.0,
                width_factor: 1.0,
                halign: 0.5,
                valign: VAlign::Middle,
                value,
            }),
            owner,
        );
    }
}

fn add(a: Vec2, b: Vec2) -> Vec2 {
    Vec2::new(a.x + b.x, a.y + b.y)
}
fn sub(a: Vec2, b: Vec2) -> Vec2 {
    Vec2::new(a.x - b.x, a.y - b.y)
}
fn mul(a: Vec2, k: f64) -> Vec2 {
    Vec2::new(a.x * k, a.y * k)
}
fn dot(a: Vec2, b: Vec2) -> f64 {
    a.x * b.x + a.y * b.y
}

/// 직선 p + t·d 와 q + s·e 의 교점에서 t
fn line_t(p: Vec2, d: Vec2, q: Vec2, e: Vec2) -> Option<f64> {
    let den = d.x * e.y - d.y * e.x;
    if den.abs() < 1e-12 {
        return None;
    }
    Some(((q.x - p.x) * e.y - (q.y - p.y) * e.x) / den)
}

/// 벽 끝 처리 결과: 면(왼쪽·오른쪽)이 끝나는 매개변수 t (벽 길이 단위, a 에서 0), 마구리선을 그릴지
struct End {
    left: f64,
    right: f64,
    cap: bool,
}

/// 벽 `w` 의 끝 `at_a`(a 끝이면 참)에서 이웃 벽과의 접합을 푼다 (L: 꺾어 잇기, T: 상대 면에서 자르기)
fn wall_end(w: &Wall, walls: &[&Wall], at_a: bool) -> End {
    let len = w.length();
    let dir = w.dir();
    let n = Vec2::new(-dir.y, dir.x);
    let h = w.thickness / 2.0;
    let p = if at_a { w.a } else { w.b };
    let tol = (w.thickness * 0.1).max(1.0);
    let base = if at_a { 0.0 } else { len };
    let mut end = End {
        left: base,
        right: base,
        cap: true,
    };
    // 이 끝에서 벽 몸통 쪽 방향
    let inward = if at_a { dir } else { mul(dir, -1.0) };
    for o in walls.iter().filter(|o| o.id != w.id) {
        let od = o.dir();
        let on = Vec2::new(-od.y, od.x);
        let oh = o.thickness / 2.0;
        // 상대 벽 끝이 여기 붙었나 (L), 상대 벽 몸통 중간에 붙었나 (T)
        let corner = o.a.dist(p) < tol || o.b.dist(p) < tol;
        let ol = o.length();
        let s = dot(sub(p, o.a), od);
        let off = dot(sub(p, o.a), on).abs();
        let tee = !corner && s > tol && s < ol - tol && off < oh + tol;
        if !corner && !tee {
            continue;
        }
        // 평행한 벽(일자로 이어짐)은 마구리만 없앤다
        if (dir.x * od.y - dir.y * od.x).abs() < 1e-6 {
            if corner {
                end.cap = false;
            }
            continue;
        }
        end.cap = false;
        let nb_body = if corner {
            // 상대 벽 몸통 방향 (붙은 끝에서 반대쪽으로)
            if o.a.dist(p) < tol { od } else { mul(od, -1.0) }
        } else {
            Vec2::default()
        };
        for (side, k) in [(&mut end.left, 1.0), (&mut end.right, -1.0)] {
            let face0 = add(w.a, mul(n, k * h));
            let cands: Vec<f64> = [1.0, -1.0]
                .iter()
                .filter_map(|&ok| line_t(face0, dir, add(o.a, mul(on, ok * oh)), od))
                .collect();
            if cands.is_empty() {
                continue;
            }
            // 몸통 쪽 매개변수가 큰 쪽(자르기) / 작은 쪽(늘리기)
            let toward_body = |t: f64| dot(sub(add(face0, mul(dir, t)), p), inward);
            let pick_trim = if tee {
                true
            } else {
                dot(mul(n, k), nb_body) > 0.0
            };
            let t = if pick_trim {
                cands.iter().copied().fold(f64::NEG_INFINITY, |a, b| {
                    if toward_body(b) > toward_body(a) || a.is_infinite() {
                        b
                    } else {
                        a
                    }
                })
            } else {
                cands.iter().copied().fold(f64::INFINITY, |a, b| {
                    if toward_body(b) < toward_body(a) || a.is_infinite() {
                        b
                    } else {
                        a
                    }
                })
            };
            // 너무 먼 교점(아주 비스듬한 벽)은 무시
            if (t - base).abs() < w.thickness * 4.0 + o.thickness * 4.0 {
                *side = t;
            }
        }
    }
    end
}

/// 문·창 기호 자동 붙이기: 종류별로 (폭, 높이) 같은 것끼리 같은 번호
pub fn opening_marks(m: &ArchModel) -> Vec<(u64, String)> {
    let mut kinds: Vec<(OpeningKind, i64, i64)> = Vec::new();
    let mut out = Vec::new();
    for o in &m.openings {
        if !o.mark.is_empty() {
            out.push((o.id, o.mark.clone()));
            continue;
        }
        let key = (o.kind, o.width.round() as i64, o.height.round() as i64);
        let i = kinds
            .iter()
            .filter(|k| k.0 == o.kind)
            .position(|k| *k == key)
            .unwrap_or_else(|| {
                kinds.push(key);
                kinds.iter().filter(|k| k.0 == o.kind).count() - 1
            });
        let prefix = if o.kind == OpeningKind::Door {
            "D"
        } else {
            "W"
        };
        out.push((o.id, format!("{prefix}{}", i + 1)));
    }
    out
}

/// 건축 객체로 평면 선을 만들어 모형 객체 뒤에 붙인 도면. 건축 객체가 없으면 None
/// 보이는 도면: 건축 객체·블록·치수를 펼친 것 (펼칠 것이 없으면 그대로)
pub fn shown(d: &Drawing) -> Drawing {
    with_arch(d).unwrap_or_else(|| d.clone())
}

pub fn with_arch(d: &Drawing) -> Option<Drawing> {
    with_arch_owned(d).map(|(v, _)| v)
}

/// [`with_arch`] 와 같고, 덧붙인 객체마다 주인 (종류, 번호) 도 함께. 블록 참조를 펼친 것은 ("block", 참조 객체 번호)
pub fn with_arch_owned(d: &Drawing) -> Option<(Drawing, Vec<(String, u64)>)> {
    let has_insert = d
        .entities
        .iter()
        .any(|e| matches!(e.shape, Shape::Insert { .. }));
    if d.arch.is_empty() && !has_insert {
        return None;
    }
    let mut out = d.clone();
    let made = generate(&mut out);
    let owners = made
        .iter()
        .map(|(_, (k, id))| (k.to_string(), *id))
        .collect();
    out.entities.extend(made.into_iter().map(|(e, _)| e));
    Some((out, owners))
}

/// 생성 객체 `k` (모형 객체 뒤 k 번째)의 주인
pub fn owner(d: &Drawing, k: usize) -> Option<(String, u64)> {
    let mut c = d.clone();
    generate(&mut c)
        .get(k)
        .map(|(_, (kind, id))| (kind.to_string(), *id))
}

/// 생성 (레이어가 없으면 `d` 에 더한다)
fn generate(d: &mut Drawing) -> Vec<(Entity, Owner)> {
    let m = d.arch.clone();
    // 블록 참조 펼치기 (§30)
    let mut blocks = Vec::new();
    for (i, e) in d.entities.iter().enumerate() {
        if matches!(e.shape, Shape::Insert { .. }) {
            blocks.extend(
                crate::block::expand(d, e)
                    .into_iter()
                    .map(|x| (x, ("block", i as u64))),
            );
        }
    }
    let mut g = Gen { d, out: blocks };
    let level = m.active_level;
    let walls: Vec<&Wall> = m.walls.iter().filter(|w| w.level == level).collect();
    let marks = opening_marks(&m);
    let text_h = 250.0;

    // 대지
    if m.site.boundary.len() >= 3 {
        g.put(
            "C-PROP",
            Shape::Polyline {
                points: m.site.boundary.iter().map(|p| (*p, 0.0)).collect(),
                closed: true,
            },
            ("site", 0),
        );
    }
    // 그리드
    for gr in &m.grids {
        let own = ("grid", gr.id);
        let dir = sub(gr.b, gr.a);
        let l = gr.a.dist(gr.b).max(1e-9);
        let u = mul(dir, 1.0 / l);
        g.line("A-GRID", gr.a, gr.b, own);
        let r = 450.0;
        let c = sub(gr.a, mul(u, r));
        g.put(
            "A-GRID-IDEN",
            Shape::Circle {
                center: c,
                radius: r,
            },
            own,
        );
        g.text("A-GRID-IDEN", c, 350.0, gr.label.clone(), own);
    }
    // 실
    for room in m.rooms.iter().filter(|r| r.level == level) {
        let own = ("room", room.id);
        if room.boundary.len() >= 3 {
            g.put(
                "A-AREA",
                Shape::Polyline {
                    points: room.boundary.iter().map(|p| (*p, 0.0)).collect(),
                    closed: true,
                },
                own,
            );
        }
        let c = room.centroid();
        let m2 = room.area() * 1e-6;
        g.text(
            "A-AREA-IDEN",
            add(c, Vec2::new(0.0, text_h * 0.8)),
            text_h,
            room.name.clone(),
            own,
        );
        g.text(
            "A-AREA-IDEN",
            sub(c, Vec2::new(0.0, text_h * 0.8)),
            text_h * 0.7,
            format!("{m2:.2}㎡ ({:.1}평)", m2 / PYEONG_M2),
            own,
        );
    }
    // 벽·개구부
    for w in &walls {
        let own = ("wall", w.id);
        let len = w.length();
        if len < 1e-6 {
            continue;
        }
        let dir = w.dir();
        let n = Vec2::new(-dir.y, dir.x);
        let h = w.thickness / 2.0;
        let ea = wall_end(w, &walls, true);
        let eb = wall_end(w, &walls, false);
        // 개구부 구간 (벽 매개변수)
        let mut gaps: Vec<(f64, f64, &Opening)> = m
            .openings
            .iter()
            .filter(|o| o.wall == w.id)
            .map(|o| {
                (
                    (o.offset - o.width / 2.0).max(0.0),
                    (o.offset + o.width / 2.0).min(len),
                    o,
                )
            })
            .filter(|(s, e, _)| e > s)
            .collect();
        gaps.sort_by(|a, b| a.0.total_cmp(&b.0));
        let at = |t: f64, k: f64| add(add(w.a, mul(dir, t)), mul(n, k * h));
        // 면선: 개구부 사이 조각들
        for (k, s0, s1) in [(1.0, ea.left, eb.left), (-1.0, ea.right, eb.right)] {
            let mut cur = s0;
            for (gs, ge, _) in &gaps {
                g.line("A-WALL", at(cur, k), at(*gs, k), own);
                cur = *ge;
            }
            g.line("A-WALL", at(cur, k), at(s1, k), own);
        }
        // 마구리
        if ea.cap {
            g.line("A-WALL", at(ea.left, 1.0), at(ea.right, -1.0), own);
        }
        if eb.cap {
            g.line("A-WALL", at(eb.left, 1.0), at(eb.right, -1.0), own);
        }
        // 벽 해치 (외벽·내력벽은 진한 회색, 내벽은 엷게) — 개구부 사이 조각마다 사각형
        let mut pieces = Vec::new();
        let (mut l0, mut r0) = (ea.left, ea.right);
        for (gs, ge, _) in &gaps {
            pieces.push((l0, r0, *gs, *gs));
            l0 = *ge;
            r0 = *ge;
        }
        pieces.push((l0, r0, eb.left, eb.right));
        if w.kind != WallKind::Interior {
            for (la, ra, lb, rb) in pieces {
                if lb - la > 1e-6 || rb - ra > 1e-6 {
                    g.put(
                        "A-WALL-PATT",
                        Shape::Fill {
                            loops: vec![vec![at(la, 1.0), at(lb, 1.0), at(rb, -1.0), at(ra, -1.0)]],
                        },
                        own,
                    );
                }
            }
        }
        // 개구부
        for (gs, ge, o) in &gaps {
            let oown = if o.kind == OpeningKind::Door {
                ("door", o.id)
            } else {
                ("window", o.id)
            };
            let layer = if o.kind == OpeningKind::Door {
                "A-DOOR"
            } else {
                "A-GLAZ"
            };
            // 문틀(벽 두께 가로선)
            g.line(layer, at(*gs, 1.0), at(*gs, -1.0), oown);
            g.line(layer, at(*ge, 1.0), at(*ge, -1.0), oown);
            match o.kind {
                OpeningKind::Window => {
                    // 창: 바깥·안 면 선과 가운데 유리선
                    g.line(layer, at(*gs, 1.0), at(*ge, 1.0), oown);
                    g.line(layer, at(*gs, -1.0), at(*ge, -1.0), oown);
                    g.line(layer, at(*gs, 0.0), at(*ge, 0.0), oown);
                }
                OpeningKind::Door => {
                    // 문: 경첩 쪽 면에서 벽에 수직인 문짝 + 90° 열림 호
                    let side = if o.flip { -1.0 } else { 1.0 };
                    let (hinge_t, free_t) = if o.hinge_end { (*ge, *gs) } else { (*gs, *ge) };
                    let hinge = at(hinge_t, side);
                    let wdt = (ge - gs).abs();
                    let leaf = add(hinge, mul(n, side * wdt));
                    g.line(layer, hinge, leaf, oown);
                    let free = at(free_t, side);
                    let a0 = (leaf.y - hinge.y).atan2(leaf.x - hinge.x);
                    let a1 = (free.y - hinge.y).atan2(free.x - hinge.x);
                    // 반시계 방향 호가 90° 가 되게 시작·끝을 고른다
                    let ccw = |s: f64, e: f64| crate::curve::ccw_sweep(s, e);
                    let (s, e) = if ccw(a0, a1) <= std::f64::consts::PI {
                        (a0, a1)
                    } else {
                        (a1, a0)
                    };
                    g.put(
                        layer,
                        Shape::Arc {
                            center: hinge,
                            radius: wdt,
                            start: s,
                            end: e,
                        },
                        oown,
                    );
                }
            }
            // 창호 기호
            if let Some((_, mark)) = marks.iter().find(|(id, _)| *id == o.id) {
                let mid = at((gs + ge) / 2.0, 0.0);
                let off = mul(n, -(h + text_h * 1.6) * if o.flip { -1.0 } else { 1.0 });
                g.text(layer, add(mid, off), text_h * 0.8, mark.clone(), oown);
            }
        }
    }
    // 치수 (§28·§32·§36)
    for dm in m.dims.iter().filter(|x| x.level == level) {
        let own = ("dim", dm.id);
        let lay: &str = if dm.layer.is_empty() {
            "A-ANNO-DIMS"
        } else {
            &dm.layer
        };
        let h = dm.text_height.max(1e-6);
        let value = |v: f64| {
            if dm.plain {
                format!("{}", v.round())
            } else {
                comma(v)
            }
        };
        let text = |v: String| {
            if dm.text.trim().is_empty() {
                v
            } else {
                dm.text.replace("<>", &v)
            }
        };
        // 끝 표시: 사선(건축) 또는 화살표 — `dir` 은 치수선이 그 끝으로 가는 방향
        let end = |g: &mut Gen, p: Vec2, dir: Vec2| {
            let n = Vec2::new(-dir.y, dir.x);
            if dm.arrow {
                let back = sub(p, mul(dir, h * 0.7));
                g.put(
                    lay,
                    Shape::Fill {
                        loops: vec![vec![
                            p,
                            add(back, mul(n, h * 0.18)),
                            sub(back, mul(n, h * 0.18)),
                        ]],
                    },
                    own,
                );
            } else {
                let t = mul(add(dir, n), h * 0.25);
                g.line(lay, sub(p, t), add(p, t), own);
            }
        };
        // 읽을 수 있는 글자 방향
        let readable = |u: Vec2| {
            let mut rot = u.y.atan2(u.x);
            if rot > std::f64::consts::FRAC_PI_2 + 1e-9
                || rot <= -std::f64::consts::FRAC_PI_2 + 1e-9
            {
                rot += std::f64::consts::PI;
            }
            rot
        };
        let label = |g: &mut Gen, at: Vec2, rot: f64, v: String| {
            g.put(
                lay,
                Shape::Text(TextShape {
                    at,
                    height: h,
                    rotation: rot,
                    width_factor: 1.0,
                    halign: 0.5,
                    valign: VAlign::Bottom,
                    value: v,
                }),
                own,
            );
        };
        match dm.kind {
            crate::arch::DimKind::Linear => {
                let Some(k) = dm.geom(&m) else {
                    continue;
                };
                let (gap, ext) = (h * 0.4, h * 0.6);
                for (p, q) in [(k.a, k.da), (k.b, k.db)] {
                    let d = sub(q, p);
                    let l = d.x.hypot(d.y);
                    if l > gap {
                        let v = mul(d, 1.0 / l);
                        g.line(lay, add(p, mul(v, gap)), add(q, mul(v, ext)), own);
                    }
                }
                g.line(lay, k.da, k.db, own);
                let along = sub(k.db, k.da);
                let l = along.x.hypot(along.y).max(1e-12);
                let dir = mul(along, 1.0 / l);
                end(&mut g, k.db, dir);
                end(&mut g, k.da, mul(dir, -1.0));
                let rot = readable(k.u);
                let up = Vec2::new(-rot.sin(), rot.cos());
                label(
                    &mut g,
                    add(mul(add(k.da, k.db), 0.5), mul(up, h * 0.3)),
                    rot,
                    text(value(k.len)),
                );
            }
            crate::arch::DimKind::Radius | crate::arch::DimKind::Diameter => {
                let (c, p) = dm.ends(&m);
                let r = c.dist(p);
                if r < 1e-9 {
                    continue;
                }
                let u = mul(sub(p, c), 1.0 / r);
                let dia = dm.kind == crate::arch::DimKind::Diameter;
                let from = if dia { sub(c, mul(u, r)) } else { c };
                g.line(lay, from, p, own);
                end(&mut g, p, u);
                if dia {
                    end(&mut g, from, mul(u, -1.0));
                }
                let rot = readable(u);
                let up = Vec2::new(-rot.sin(), rot.cos());
                let mid = if dia { c } else { c.lerp(p, 0.5) };
                let v = if dia {
                    format!("Ø{}", value(2.0 * r))
                } else {
                    format!("R{}", value(r))
                };
                label(&mut g, add(mid, mul(up, h * 0.3)), rot, text(v));
            }
            crate::arch::DimKind::Angle => {
                let Some(v) = dm.vertex else {
                    continue;
                };
                let (a, b) = dm.ends(&m);
                let (t0, t1) = ((a.y - v.y).atan2(a.x - v.x), (b.y - v.y).atan2(b.x - v.x));
                let mut sw = crate::curve::ccw_sweep(t0, t1);
                let (mut s0, mut s1) = (t0, t1);
                if sw > std::f64::consts::PI {
                    std::mem::swap(&mut s0, &mut s1);
                    sw = std::f64::consts::TAU - sw;
                }
                let rad = dm.offset.abs().max(h);
                let at = |t: f64| Vec2::new(v.x + rad * t.cos(), v.y + rad * t.sin());
                g.put(
                    lay,
                    Shape::Arc {
                        center: v,
                        radius: rad,
                        start: s0.rem_euclid(std::f64::consts::TAU),
                        end: (s0 + sw).rem_euclid(std::f64::consts::TAU),
                    },
                    own,
                );
                // 반직선이 호까지 닿지 않으면 보조선
                for (t, p) in [(t0, a), (t1, b)] {
                    if v.dist(p) < rad {
                        g.line(lay, p, at(t), own);
                    }
                }
                end(&mut g, at(s0), Vec2::new(s0.sin(), -s0.cos()));
                end(&mut g, at(s1), Vec2::new(-s1.sin(), s1.cos()));
                let mid = s0 + sw / 2.0;
                let rot = readable(Vec2::new(-mid.sin(), mid.cos()));
                let deg = sw.to_degrees();
                let v = if (deg - deg.round()).abs() < 0.05 {
                    format!("{}°", deg.round())
                } else {
                    format!("{deg:.1}°")
                };
                label(&mut g, at(mid), rot, text(v));
            }
        }
    }
    // 기둥
    for c in m.columns.iter().filter(|x| x.level == level) {
        let own = ("column", c.id);
        let k = c.corners();
        g.put(
            "A-COLS-PATT",
            Shape::Fill {
                loops: vec![k.to_vec()],
            },
            own,
        );
        g.put(
            "A-COLS",
            Shape::Polyline {
                points: k.iter().map(|p| (*p, 0.0)).collect(),
                closed: true,
            },
            own,
        );
    }
    // 계단: 줄기마다 외곽·디딤판 선, 참 외곽, 중심선을 따라 오름 화살표 (§34)
    for st in m.stairs.iter().filter(|x| x.level == level) {
        let own = ("stair", st.id);
        let Some((flights, landing)) = st.flights() else {
            continue;
        };
        for f in &flights {
            let n = mul(Vec2::new(-f.u.y, f.u.x), st.width / 2.0);
            let q = |t: f64, k: f64| add(add(f.start, mul(f.u, t)), mul(n, k));
            g.put(
                "A-STRS",
                Shape::Polyline {
                    points: vec![
                        (q(0.0, -1.0), 0.0),
                        (q(f.len, -1.0), 0.0),
                        (q(f.len, 1.0), 0.0),
                        (q(0.0, 1.0), 0.0),
                    ],
                    closed: true,
                },
                own,
            );
            let r = f.risers.max(1) as f64;
            for k in 1..f.risers {
                let t = f.len * k as f64 / r;
                g.line("A-STRS", q(t, -1.0), q(t, 1.0), own);
            }
        }
        if let Some(sq) = landing {
            g.put(
                "A-STRS",
                Shape::Polyline {
                    points: sq.iter().map(|p| (*p, 0.0)).collect(),
                    closed: true,
                },
                own,
            );
        }
        // 화살표: 첫 단 가운데 → (꺾는 점) → 끝 조금 앞
        let f0 = flights[0];
        let last = flights[flights.len() - 1];
        let head = (st.width * 0.12).min(last.len * 0.2);
        let tip = add(last.start, mul(last.u, last.len - head * 0.2));
        let mut path = vec![add(
            f0.start,
            mul(f0.u, f0.len * 0.5 / f0.risers.max(1) as f64),
        )];
        if let Some(c) = st.mid {
            path.push(c);
        }
        path.push(tip);
        for w in path.windows(2) {
            g.line("A-STRS", w[0], w[1], own);
        }
        let back = sub(tip, mul(last.u, head));
        let side = mul(Vec2::new(-last.u.y, last.u.x), head * 0.4);
        g.put(
            "A-STRS",
            Shape::Fill {
                loops: vec![vec![tip, add(back, side), sub(back, side)]],
            },
            own,
        );
        // 「UP」 은 오르기 시작점 바깥에
        g.text(
            "A-STRS",
            sub(f0.start, mul(f0.u, text_h * 0.9)),
            text_h * 0.8,
            "UP".into(),
            own,
        );
    }
    g.out
}

// ── 편집 ────────────────────────────────────────────────────────────────

/// 건축 객체 편집 요청 (화면 → 앱)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ArchReq {
    /// 벽 (꺾은선을 따라 여러 개, closed 면 닫힘)
    Walls {
        points: Vec<[f64; 2]>,
        closed: bool,
        thickness: f64,
        wall: WallKind,
    },
    /// 문·창 (점에서 가장 가까운 벽에)
    Opening {
        at: [f64; 2],
        opening: OpeningKind,
        width: f64,
        height: f64,
        sill: f64,
    },
    /// 실 (경계)
    Room {
        boundary: Vec<[f64; 2]>,
        name: String,
    },
    /// 실 자동 찾기: 점을 둘러싼 벽 안쪽 면으로 경계를 만든다
    RoomAt { at: [f64; 2], name: String },
    /// 그리드 한 줄
    Grid {
        a: [f64; 2],
        b: [f64; 2],
        label: String,
    },
    /// 그리드 여럿 (간격 목록으로 X·Y 방향)
    GridSet {
        origin: [f64; 2],
        xs: Vec<f64>,
        ys: Vec<f64>,
        extend: f64,
    },
    /// 층 더하기
    Level {
        name: String,
        elevation: f64,
        height: f64,
    },
    /// 지금 층
    ActiveLevel { id: u64 },
    /// 대지
    Site {
        boundary: Vec<[f64; 2]>,
        zone: String,
        coverage_limit: f64,
        far_limit: f64,
    },
    /// 지우기
    Delete { what: String, id: u64 },
    /// 속성 고치기 (이름=값 목록)
    Set {
        what: String,
        id: u64,
        fields: Vec<(String, String)>,
    },
    /// 벽·실·그리드 옮기기
    Move {
        what: String,
        id: u64,
        dx: f64,
        dy: f64,
    },
    /// 치수 (두 점, 치수선이 지날 점)
    Dim {
        a: [f64; 2],
        b: [f64; 2],
        at: [f64; 2],
        text_height: f64,
        /// 끝 모양 화살표 (거짓이면 사선)
        #[serde(default)]
        arrow: bool,
        /// 쉼표 없이
        #[serde(default)]
        plain: bool,
    },
    /// 반지름·지름 치수 (§36): 중심, 원 위 점
    DimRadial {
        center: [f64; 2],
        point: [f64; 2],
        diameter: bool,
        text_height: f64,
        #[serde(default)]
        arrow: bool,
        #[serde(default)]
        plain: bool,
    },
    /// 각도 치수: 꼭짓점, 두 반직선 위 점, 호가 지날 점
    DimAngle {
        vertex: [f64; 2],
        a: [f64; 2],
        b: [f64; 2],
        at: [f64; 2],
        text_height: f64,
        #[serde(default)]
        arrow: bool,
    },
    /// 기둥
    Column {
        at: [f64; 2],
        width: f64,
        depth: f64,
    },
    /// 곧은 계단 (a 에서 b 로 오름)
    Stair {
        a: [f64; 2],
        b: [f64; 2],
        width: f64,
        risers: u32,
        /// 꺾는 점 (있으면 꺾인 계단)
        #[serde(default)]
        mid: Option<[f64; 2]>,
    },
    /// 그립 옮기기 (§33): 객체의 손잡이 k 를 to 로
    Grip {
        what: String,
        id: u64,
        k: usize,
        to: [f64; 2],
    },
    /// 지금 층을 복사해 새 층 (벽·개구부·실)
    CopyLevel { name: String },
}

fn p2(a: [f64; 2]) -> Vec2 {
    Vec2::new(a[0], a[1])
}

/// 건축 객체의 손잡이 점 (§33). 순서는 [`grip_arch`] 와 같다
/// - 벽: a, b, 가운데(옮김) · 그리드: a, b · 기둥: 가운데 · 계단: a, b
/// - 치수: a, b, 치수선 가운데(거리) · 실·대지: 경계 꼭짓점 · 문·창: 가운데(벽을 따라)
pub fn arch_grips(m: &ArchModel, what: &str, id: u64) -> Vec<Vec2> {
    match what {
        "wall" => m
            .wall(id)
            .map(|w| vec![w.a, w.b, w.a.lerp(w.b, 0.5)])
            .unwrap_or_default(),
        "grid" => m
            .grids
            .iter()
            .find(|g| g.id == id)
            .map(|g| vec![g.a, g.b])
            .unwrap_or_default(),
        "column" => m
            .columns
            .iter()
            .find(|c| c.id == id)
            .map(|c| vec![c.at])
            .unwrap_or_default(),
        "stair" => m
            .stairs
            .iter()
            .find(|x| x.id == id)
            .map(|x| [x.a, x.b].into_iter().chain(x.mid).collect())
            .unwrap_or_default(),
        "dim" => m
            .dims
            .iter()
            .find(|x| x.id == id)
            .map(|x| match x.kind {
                crate::arch::DimKind::Linear => x
                    .geom(m)
                    .map(|k| vec![k.a, k.b, k.da.lerp(k.db, 0.5)])
                    .unwrap_or_default(),
                _ => {
                    let (a, b) = x.ends(m);
                    [a, b].into_iter().chain(x.vertex).collect()
                }
            })
            .unwrap_or_default(),
        "room" => m
            .rooms
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.boundary.clone())
            .unwrap_or_default(),
        "site" => m.site.boundary.clone(),
        "door" | "window" => m
            .openings
            .iter()
            .find(|o| o.id == id)
            .and_then(|o| m.wall(o.wall).map(|w| add(w.a, mul(w.dir(), o.offset))))
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

/// 건축 객체 손잡이 옮기기
fn grip_arch(m: &mut ArchModel, what: &str, id: u64, k: usize, to: Vec2) -> Result<(), String> {
    let g = arch_grips(m, what, id);
    let from = *g.get(k).ok_or("손잡이가 없습니다")?;
    let d = sub(to, from);
    match what {
        "wall" => {
            let w = m
                .walls
                .iter_mut()
                .find(|w| w.id == id)
                .ok_or("벽이 없습니다")?;
            match k {
                0 => w.a = to,
                1 => w.b = to,
                _ => {
                    w.a = add(w.a, d);
                    w.b = add(w.b, d);
                }
            }
            if w.a.dist(w.b) < 1e-6 {
                return Err("벽 길이가 0 이 됩니다".into());
            }
        }
        "grid" => {
            let x = m
                .grids
                .iter_mut()
                .find(|x| x.id == id)
                .ok_or("그리드가 없습니다")?;
            if k == 0 { x.a = to } else { x.b = to }
        }
        "column" => {
            let x = m
                .columns
                .iter_mut()
                .find(|x| x.id == id)
                .ok_or("기둥이 없습니다")?;
            x.at = to;
        }
        "stair" => {
            let x = m
                .stairs
                .iter_mut()
                .find(|x| x.id == id)
                .ok_or("계단이 없습니다")?;
            match k {
                0 => x.a = to,
                1 => x.b = to,
                _ => x.mid = Some(to),
            }
            if x.flights().is_none() {
                return Err("계단 줄기가 너무 짧아집니다".into());
            }
        }
        "dim" => {
            let mm = m.clone();
            let x = m
                .dims
                .iter_mut()
                .find(|x| x.id == id)
                .ok_or("치수가 없습니다")?;
            if x.kind != crate::arch::DimKind::Linear {
                match k {
                    0 => x.a = to,
                    1 => x.b = to,
                    _ => x.vertex = Some(to),
                }
                x.ka = None;
                x.kb = None;
                return Ok(());
            }
            let geo = x.geom(&mm).ok_or("치수 모양이 없습니다")?;
            match k {
                // 끝점을 옮기면 그 자리 객체에 다시 묶는다. 회전 치수는 치수선 자리를 그대로 둔다
                0 | 1 => {
                    let tol = (x.text_height * 0.02).max(1.0);
                    let anchor = mm.anchor_at(x.level, to, geo.u, tol);
                    if k == 0 {
                        x.a = to;
                        x.ka = anchor;
                        if x.rotation.is_some() {
                            x.offset = dot(sub(geo.da, to), geo.n);
                        }
                    } else {
                        x.b = to;
                        x.kb = anchor;
                    }
                }
                _ => x.offset += dot(d, geo.n),
            }
        }
        "room" | "site" => {
            let pts = if what == "room" {
                &mut m
                    .rooms
                    .iter_mut()
                    .find(|r| r.id == id)
                    .ok_or("실이 없습니다")?
                    .boundary
            } else {
                &mut m.site.boundary
            };
            if let Some(p) = pts.get_mut(k) {
                *p = to;
            }
        }
        "door" | "window" => {
            let o = m
                .openings
                .iter()
                .find(|o| o.id == id)
                .cloned()
                .ok_or("개구부가 없습니다")?;
            let w = m.wall(o.wall).cloned().ok_or("벽이 없습니다")?;
            let len = w.length();
            let t = dot(sub(to, w.a), w.dir())
                .clamp(o.width / 2.0, (len - o.width / 2.0).max(o.width / 2.0));
            if let Some(x) = m.openings.iter_mut().find(|x| x.id == id) {
                x.offset = t;
            }
        }
        _ => return Err(format!("{what} 은 손잡이로 고칠 수 없습니다")),
    }
    Ok(())
}

/// 점에서 가장 가까운 벽과 그 벽 위 위치(a 에서 거리)
pub fn nearest_wall(m: &ArchModel, p: Vec2) -> Option<(u64, f64, f64)> {
    m.walls
        .iter()
        .filter(|w| w.level == m.active_level)
        .map(|w| {
            let d = w.dir();
            let t = dot(sub(p, w.a), d).clamp(0.0, w.length());
            let q = add(w.a, mul(d, t));
            (w.id, t, q.dist(p))
        })
        .min_by(|a, b| a.2.total_cmp(&b.2))
}

/// 점을 둘러싼 실 경계 찾기: 지금 층 벽의 안쪽 면을 선분으로 모아, 점에서 사방으로 쏜 반직선이 처음 닿는
/// 면들로 다각형을 만든다 (직교 평면에서 잘 맞는 단순한 방법 — 구현 노트 §19)
pub fn find_room(m: &ArchModel, p: Vec2) -> Option<Vec<Vec2>> {
    let mut faces: Vec<(Vec2, Vec2)> = Vec::new();
    for w in m.walls.iter().filter(|w| w.level == m.active_level) {
        let d = w.dir();
        let n = Vec2::new(-d.y, d.x);
        let h = w.thickness / 2.0;
        let ext = mul(d, w.thickness);
        for k in [1.0, -1.0] {
            let a = add(sub(w.a, ext), mul(n, k * h));
            let b = add(add(w.b, ext), mul(n, k * h));
            faces.push((a, b));
        }
    }
    if faces.is_empty() {
        return None;
    }
    // 360 방향으로 쏴서 가장 가까운 면까지
    let rays = 720;
    let mut pts = Vec::with_capacity(rays);
    for i in 0..rays {
        let ang = std::f64::consts::TAU * i as f64 / rays as f64;
        let dir = Vec2::new(ang.cos(), ang.sin());
        let mut best = f64::INFINITY;
        for (a, b) in &faces {
            let e = sub(*b, *a);
            if let (Some(t), Some(s)) = (line_t(p, dir, *a, e), line_t(*a, e, p, dir))
                && t > 0.0
                && (0.0..=1.0).contains(&s)
            {
                best = best.min(t);
            }
        }
        if !best.is_finite() {
            return None; // 열린 곳이 있으면 실이 아니다
        }
        pts.push(add(p, mul(dir, best)));
    }
    // 같은 면 위에 이어진 점들을 줄여 꼭짓점만 남긴다
    let mut poly: Vec<Vec2> = Vec::new();
    let n = pts.len();
    for i in 0..n {
        let (a, b, c) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
        let cross = (b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x);
        let scale = a.dist(b).max(1e-9) * b.dist(c).max(1e-9);
        if (cross / scale).abs() > 0.02 {
            poly.push(b);
        }
    }
    if poly.len() < 3 {
        return None;
    }
    // 모서리 근처 점을 면 교점으로 다듬기는 생략 — 반직선 720 개라 모서리 오차는 작다
    Some(poly)
}

fn set_field(m: &mut ArchModel, what: &str, id: u64, k: &str, v: &str) -> Result<(), String> {
    let num = || {
        v.trim()
            .parse::<f64>()
            .map_err(|_| format!("「{k}」 에 숫자를 넣어 주세요"))
    };
    match what {
        "wall" => {
            let w = m
                .walls
                .iter_mut()
                .find(|w| w.id == id)
                .ok_or("벽이 없습니다")?;
            match k {
                "두께" => w.thickness = num()?.max(1.0),
                "높이" => {
                    w.height = if v.trim().is_empty() {
                        None
                    } else {
                        Some(num()?)
                    }
                }
                "종류" => {
                    w.kind = match v.trim() {
                        "내벽" => WallKind::Interior,
                        "내력벽" => WallKind::Structural,
                        _ => WallKind::Exterior,
                    }
                }
                _ => {}
            }
        }
        "door" | "window" => {
            let o = m
                .openings
                .iter_mut()
                .find(|o| o.id == id)
                .ok_or("문·창이 없습니다")?;
            match k {
                "폭" => o.width = num()?.max(100.0),
                "높이" => o.height = num()?,
                "창대 높이" => o.sill = num()?,
                "위치" => o.offset = num()?,
                "기호" => o.mark = v.trim().to_string(),
                "여는 쪽 뒤집기" => o.flip = matches!(v.trim(), "예" | "1" | "true"),
                "경첩 반대" => o.hinge_end = matches!(v.trim(), "예" | "1" | "true"),
                _ => {}
            }
        }
        "room" => {
            let r = m
                .rooms
                .iter_mut()
                .find(|r| r.id == id)
                .ok_or("실이 없습니다")?;
            match k {
                "실 이름" => r.name = v.trim().to_string(),
                "바닥 마감" => r.finish_floor = v.trim().to_string(),
                "벽 마감" => r.finish_wall = v.trim().to_string(),
                "천장 마감" => r.finish_ceiling = v.trim().to_string(),
                _ => {}
            }
        }
        "grid" => {
            let g = m
                .grids
                .iter_mut()
                .find(|g| g.id == id)
                .ok_or("그리드가 없습니다")?;
            if k == "기호" {
                g.label = v.trim().to_string();
            }
        }
        "dim" => {
            let x = m
                .dims
                .iter_mut()
                .find(|x| x.id == id)
                .ok_or("치수가 없습니다")?;
            match k {
                "글자" => x.text = v.trim().to_string(),
                "글자 높이" => x.text_height = num()?.max(1e-3),
                "치수선 거리" | "호 반지름" => x.offset = num()?,
                "끝 모양" => x.arrow = v.trim() == "화살표",
                "쉼표" => x.plain = v.trim() == "아니오",
                _ => {}
            }
        }
        "column" => {
            let x = m
                .columns
                .iter_mut()
                .find(|x| x.id == id)
                .ok_or("기둥이 없습니다")?;
            match k {
                "폭" => x.width = num()?.max(1.0),
                "깊이" => x.depth = num()?.max(1.0),
                "회전" => x.angle = num()?.to_radians(),
                _ => {}
            }
        }
        "stair" => {
            let x = m
                .stairs
                .iter_mut()
                .find(|x| x.id == id)
                .ok_or("계단이 없습니다")?;
            match k {
                "폭" => x.width = num()?.max(1.0),
                "단 수" => x.risers = (num()?.round() as u32).max(1),
                _ => {}
            }
        }
        "level" => {
            let l = m
                .levels
                .iter_mut()
                .find(|l| l.id == id)
                .ok_or("층이 없습니다")?;
            match k {
                "층 이름" => l.name = v.trim().to_string(),
                "바닥 높이" => l.elevation = num()?,
                "층고" => l.height = num()?.max(1.0),
                _ => {}
            }
        }
        _ => return Err(format!("모르는 객체: {what}")),
    }
    Ok(())
}

/// 편집 요청을 실행한다 (되돌리기 가능)
pub fn apply(ed: &mut Editor, req: ArchReq) -> Result<(), String> {
    let old = ed.d.arch.clone();
    let mut m = old.clone();
    let level = m.ensure_level();
    match req {
        ArchReq::Walls {
            points,
            closed,
            thickness,
            wall,
        } => {
            let pts: Vec<Vec2> = points.iter().map(|p| p2(*p)).collect();
            let n = pts.len();
            let segs = if closed && n > 2 {
                n
            } else {
                n.saturating_sub(1)
            };
            for i in 0..segs {
                let (a, b) = (pts[i], pts[(i + 1) % n]);
                if a.dist(b) < 1.0 {
                    continue;
                }
                let id = m.new_id();
                m.walls.push(Wall {
                    id,
                    level,
                    a,
                    b,
                    thickness: thickness.max(10.0),
                    height: None,
                    kind: wall,
                });
            }
        }
        ArchReq::Opening {
            at,
            opening,
            width,
            height,
            sill,
        } => {
            let (wall, t, _) =
                nearest_wall(&m, p2(at)).ok_or("문·창을 낼 벽이 없습니다 — 먼저 벽을 그리세요")?;
            let len = m.wall(wall).map(|w| w.length()).unwrap_or(0.0);
            let offset = t.clamp(width / 2.0, (len - width / 2.0).max(width / 2.0));
            let id = m.new_id();
            m.openings.push(Opening {
                id,
                wall,
                kind: opening,
                offset,
                width,
                height,
                sill,
                flip: false,
                hinge_end: false,
                mark: String::new(),
            });
        }
        ArchReq::Room { boundary, name } => {
            let id = m.new_id();
            m.rooms.push(Room {
                id,
                level,
                name,
                boundary: boundary.iter().map(|p| p2(*p)).collect(),
                finish_floor: String::new(),
                finish_wall: String::new(),
                finish_ceiling: String::new(),
            });
        }
        ArchReq::RoomAt { at, name } => {
            let boundary =
                find_room(&m, p2(at)).ok_or("이 점을 둘러싼 닫힌 벽을 찾지 못했습니다")?;
            let id = m.new_id();
            m.rooms.push(Room {
                id,
                level,
                name,
                boundary,
                finish_floor: String::new(),
                finish_wall: String::new(),
                finish_ceiling: String::new(),
            });
        }
        ArchReq::Grid { a, b, label } => {
            let id = m.new_id();
            m.grids.push(Grid {
                id,
                label,
                a: p2(a),
                b: p2(b),
            });
        }
        ArchReq::Dim {
            a,
            b,
            at,
            text_height,
            arrow,
            plain,
        } => {
            let (a, b, at) = (p2(a), p2(b), p2(at));
            let len = a.dist(b);
            if len < 1e-9 {
                return Err("두 점이 같습니다".into());
            }
            let u = mul(sub(b, a), 1.0 / len);
            let offset = dot(sub(at, a), Vec2::new(-u.y, u.x));
            // 끝점이 벽·그리드·기둥 위면 묶는다 (§28)
            let tol = (text_height * 0.02).max(1.0);
            let ka = m.anchor_at(level, a, u, tol);
            let kb = m.anchor_at(level, b, u, tol);
            let id = m.new_id();
            m.dims.push(crate::arch::Dim {
                id,
                level,
                a,
                b,
                offset,
                text_height,
                text: String::new(),
                ka,
                kb,
                rotation: None,
                layer: String::new(),
                arrow,
                plain,
                ..Default::default()
            });
        }
        ArchReq::DimRadial {
            center,
            point,
            diameter,
            text_height,
            arrow,
            plain,
        } => {
            if p2(center).dist(p2(point)) < 1e-9 {
                return Err("반지름이 0 입니다".into());
            }
            let id = m.new_id();
            m.dims.push(crate::arch::Dim {
                id,
                level,
                a: p2(center),
                b: p2(point),
                text_height,
                kind: if diameter {
                    crate::arch::DimKind::Diameter
                } else {
                    crate::arch::DimKind::Radius
                },
                arrow,
                plain,
                ..Default::default()
            });
        }
        ArchReq::DimAngle {
            vertex,
            a,
            b,
            at,
            text_height,
            arrow,
        } => {
            let v = p2(vertex);
            let rad = v.dist(p2(at));
            if rad < 1e-9 || v.dist(p2(a)) < 1e-9 || v.dist(p2(b)) < 1e-9 {
                return Err("각도 치수 점이 겹칩니다".into());
            }
            let id = m.new_id();
            m.dims.push(crate::arch::Dim {
                id,
                level,
                a: p2(a),
                b: p2(b),
                vertex: Some(v),
                offset: rad,
                text_height,
                kind: crate::arch::DimKind::Angle,
                arrow,
                ..Default::default()
            });
        }
        ArchReq::Column { at, width, depth } => {
            let id = m.new_id();
            m.columns.push(crate::arch::Column {
                id,
                level,
                at: p2(at),
                width: width.max(1.0),
                depth: depth.max(1.0),
                angle: 0.0,
            });
        }
        ArchReq::Stair {
            a,
            b,
            width,
            risers,
            mid,
        } => {
            if p2(a).dist(p2(b)) < 1e-6 {
                return Err("계단 길이가 0 입니다".into());
            }
            let id = m.new_id();
            m.stairs.push(crate::arch::Stair {
                id,
                level,
                a: p2(a),
                b: p2(b),
                width: width.max(1.0),
                risers: risers.max(1),
                mid: mid.map(p2),
            });
            if m.stairs.last().and_then(|x| x.flights()).is_none() {
                m.stairs.pop();
                return Err("꺾인 계단 줄기가 계단 폭보다 짧습니다".into());
            }
        }
        ArchReq::GridSet {
            origin,
            xs,
            ys,
            extend,
        } => {
            let o = p2(origin);
            let total_x: f64 = xs.iter().sum();
            let total_y: f64 = ys.iter().sum();
            let mut x = o.x;
            for (i, dx) in std::iter::once(0.0).chain(xs.iter().copied()).enumerate() {
                x += dx;
                let id = m.new_id();
                m.grids.push(Grid {
                    id,
                    label: format!("X{}", i + 1),
                    a: Vec2::new(x, o.y - extend),
                    b: Vec2::new(x, o.y + total_y + extend),
                });
            }
            let mut y = o.y;
            for (i, dy) in std::iter::once(0.0).chain(ys.iter().copied()).enumerate() {
                y += dy;
                let id = m.new_id();
                m.grids.push(Grid {
                    id,
                    label: format!("Y{}", i + 1),
                    a: Vec2::new(o.x - extend, y),
                    b: Vec2::new(o.x + total_x + extend, y),
                });
            }
        }
        ArchReq::Level {
            name,
            elevation,
            height,
        } => {
            let id = m.new_id();
            m.levels.push(Level {
                id,
                name,
                elevation,
                height,
            });
            m.levels.sort_by(|a, b| a.elevation.total_cmp(&b.elevation));
            m.active_level = id;
        }
        ArchReq::ActiveLevel { id } => {
            if m.level(id).is_none() {
                return Err("층이 없습니다".into());
            }
            m.active_level = id;
        }
        ArchReq::Site {
            boundary,
            zone,
            coverage_limit,
            far_limit,
        } => {
            m.site = Site {
                boundary: boundary.iter().map(|p| p2(*p)).collect(),
                zone,
                coverage_limit,
                far_limit,
            };
        }
        ArchReq::Delete { what, id } => match what.as_str() {
            "wall" => {
                m.walls.retain(|w| w.id != id);
                m.openings.retain(|o| o.wall != id);
            }
            "door" | "window" => m.openings.retain(|o| o.id != id),
            "room" => m.rooms.retain(|r| r.id != id),
            "grid" => m.grids.retain(|g| g.id != id),
            "dim" => m.dims.retain(|x| x.id != id),
            "column" => m.columns.retain(|x| x.id != id),
            "stair" => m.stairs.retain(|x| x.id != id),
            "site" => m.site.boundary.clear(),
            "level" => {
                if m.levels.len() <= 1 {
                    return Err("마지막 층은 지울 수 없습니다".into());
                }
                m.levels.retain(|l| l.id != id);
                let gone: Vec<u64> = m
                    .walls
                    .iter()
                    .filter(|w| w.level == id)
                    .map(|w| w.id)
                    .collect();
                m.walls.retain(|w| w.level != id);
                m.openings.retain(|o| !gone.contains(&o.wall));
                m.rooms.retain(|r| r.level != id);
                m.dims.retain(|x| x.level != id);
                m.columns.retain(|x| x.level != id);
                m.stairs.retain(|x| x.level != id);
                m.ensure_level();
            }
            _ => return Err(format!("모르는 객체: {what}")),
        },
        ArchReq::Set { what, id, fields } => {
            for (k, v) in &fields {
                set_field(&mut m, &what, id, k, v)?;
            }
        }
        ArchReq::Move { what, id, dx, dy } => {
            let d = Vec2::new(dx, dy);
            match what.as_str() {
                "wall" => {
                    if let Some(w) = m.walls.iter_mut().find(|w| w.id == id) {
                        w.a = add(w.a, d);
                        w.b = add(w.b, d);
                    }
                }
                "room" => {
                    if let Some(r) = m.rooms.iter_mut().find(|r| r.id == id) {
                        r.boundary.iter_mut().for_each(|p| *p = add(*p, d));
                    }
                }
                "grid" => {
                    if let Some(g) = m.grids.iter_mut().find(|g| g.id == id) {
                        g.a = add(g.a, d);
                        g.b = add(g.b, d);
                    }
                }
                "door" | "window" => {
                    if let Some(o) = m.openings.iter().find(|o| o.id == id).cloned()
                        && let Some(w) = m.wall(o.wall)
                    {
                        let t = o.offset + dot(d, w.dir());
                        let len = w.length();
                        if let Some(x) = m.openings.iter_mut().find(|x| x.id == id) {
                            x.offset =
                                t.clamp(o.width / 2.0, (len - o.width / 2.0).max(o.width / 2.0));
                        }
                    }
                }
                "dim" => {
                    // 치수 옮기기 = 치수선 옮기기 (끝점은 그대로 — 묶인 객체를 따른다)
                    let mm = m.clone();
                    if let Some(x) = m.dims.iter_mut().find(|x| x.id == id) {
                        if x.kind == crate::arch::DimKind::Linear {
                            if let Some(k) = x.geom(&mm) {
                                x.offset += dot(d, k.n);
                            }
                        } else {
                            // 반지름·지름·각도 치수는 통째로
                            let (a, b) = x.ends(&mm);
                            x.a = add(a, d);
                            x.b = add(b, d);
                            x.ka = None;
                            x.kb = None;
                            x.vertex = x.vertex.map(|v| add(v, d));
                        }
                    }
                }
                "column" => {
                    if let Some(x) = m.columns.iter_mut().find(|x| x.id == id) {
                        x.at = add(x.at, d);
                    }
                }
                "stair" => {
                    if let Some(x) = m.stairs.iter_mut().find(|x| x.id == id) {
                        x.a = add(x.a, d);
                        x.b = add(x.b, d);
                        x.mid = x.mid.map(|c| add(c, d));
                    }
                }
                _ => {}
            }
        }
        ArchReq::Grip { what, id, k, to } => grip_arch(&mut m, &what, id, k, p2(to))?,
        ArchReq::CopyLevel { name } => {
            let src = level;
            let (elev, height) = m
                .level(src)
                .map(|l| (l.elevation + l.height, l.height))
                .unwrap_or((3000.0, 3000.0));
            let new_level = m.new_id();
            m.levels.push(Level {
                id: new_level,
                name,
                elevation: elev,
                height,
            });
            let walls: Vec<Wall> = m.walls.iter().filter(|w| w.level == src).cloned().collect();
            for w in walls {
                let nid = m.new_id();
                let ops: Vec<Opening> = m
                    .openings
                    .iter()
                    .filter(|o| o.wall == w.id)
                    .cloned()
                    .collect();
                for o in ops {
                    let oid = m.new_id();
                    m.openings.push(Opening {
                        id: oid,
                        wall: nid,
                        ..o
                    });
                }
                m.walls.push(Wall {
                    id: nid,
                    level: new_level,
                    ..w
                });
            }
            let rooms: Vec<Room> = m.rooms.iter().filter(|r| r.level == src).cloned().collect();
            for r in rooms {
                let rid = m.new_id();
                m.rooms.push(Room {
                    id: rid,
                    level: new_level,
                    ..r
                });
            }
            let cols: Vec<_> = m
                .columns
                .iter()
                .filter(|x| x.level == src)
                .cloned()
                .collect();
            for c in cols {
                let id = m.new_id();
                m.columns.push(crate::arch::Column {
                    id,
                    level: new_level,
                    ..c
                });
            }
            let stairs: Vec<_> = m
                .stairs
                .iter()
                .filter(|x| x.level == src)
                .cloned()
                .collect();
            for st in stairs {
                let id = m.new_id();
                m.stairs.push(crate::arch::Stair {
                    id,
                    level: new_level,
                    ..st
                });
            }
            m.levels.sort_by(|a, b| a.elevation.total_cmp(&b.elevation));
            m.active_level = new_level;
        }
    }
    if m != old {
        ed.run(Op::Arch {
            old: Box::new(old),
            new: Box::new(m),
        });
    }
    Ok(())
}

// ── 요약·속성 ───────────────────────────────────────────────────────────

/// 화면 상태용 요약
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArchSummary {
    pub levels: Vec<(u64, String)>,
    pub active_level: u64,
    pub walls: usize,
    pub openings: usize,
    pub rooms: usize,
    pub grids: usize,
    pub has_site: bool,
    pub columns: usize,
    pub stairs: usize,
    pub dims: usize,
}

pub fn summary(m: &ArchModel) -> ArchSummary {
    ArchSummary {
        levels: m.levels.iter().map(|l| (l.id, l.name.clone())).collect(),
        active_level: m.active_level,
        walls: m.walls.len(),
        openings: m.openings.len(),
        rooms: m.rooms.len(),
        grids: m.grids.len(),
        has_site: m.site.boundary.len() >= 3,
        columns: m.columns.len(),
        stairs: m.stairs.len(),
        dims: m.dims.len(),
    }
}

/// 건축 객체 속성 (속성 칸 — 이름이 「…:고침」 으로 끝나면 고칠 수 있는 칸)
pub fn properties(m: &ArchModel, what: &str, id: u64) -> Vec<(String, String)> {
    let f = |v: f64| format!("{v:.0}");
    let mut out: Vec<(String, String)> = Vec::new();
    match what {
        "wall" => {
            if let Some(w) = m.wall(id) {
                out.push(("종류".into(), "벽".into()));
                out.push(("종류:고침".into(), w.kind.label().into()));
                out.push(("두께:고침".into(), f(w.thickness)));
                out.push(("높이:고침".into(), w.height.map(f).unwrap_or_default()));
                out.push(("길이".into(), f(w.length())));
                out.push((
                    "문·창".into(),
                    m.openings
                        .iter()
                        .filter(|o| o.wall == id)
                        .count()
                        .to_string(),
                ));
            }
        }
        "door" | "window" => {
            if let Some(o) = m.openings.iter().find(|o| o.id == id) {
                let mark = opening_marks(m)
                    .into_iter()
                    .find(|(i, _)| *i == id)
                    .map(|(_, s)| s)
                    .unwrap_or_default();
                out.push((
                    "종류".into(),
                    if o.kind == OpeningKind::Door {
                        "문"
                    } else {
                        "창"
                    }
                    .into(),
                ));
                out.push(("기호:고침".into(), mark));
                out.push(("폭:고침".into(), f(o.width)));
                out.push(("높이:고침".into(), f(o.height)));
                if o.kind == OpeningKind::Window {
                    out.push(("창대 높이:고침".into(), f(o.sill)));
                } else {
                    out.push((
                        "여는 쪽 뒤집기:고침".into(),
                        if o.flip { "예" } else { "아니오" }.into(),
                    ));
                    out.push((
                        "경첩 반대:고침".into(),
                        if o.hinge_end { "예" } else { "아니오" }.into(),
                    ));
                }
                out.push(("위치:고침".into(), f(o.offset)));
            }
        }
        "room" => {
            if let Some(r) = m.rooms.iter().find(|r| r.id == id) {
                let m2 = r.area() * 1e-6;
                out.push(("종류".into(), "실".into()));
                out.push(("실 이름:고침".into(), r.name.clone()));
                out.push((
                    "면적".into(),
                    format!("{m2:.2} ㎡ · {:.2} 평", m2 / PYEONG_M2),
                ));
                out.push(("바닥 마감:고침".into(), r.finish_floor.clone()));
                out.push(("벽 마감:고침".into(), r.finish_wall.clone()));
                out.push(("천장 마감:고침".into(), r.finish_ceiling.clone()));
            }
        }
        "grid" => {
            if let Some(g) = m.grids.iter().find(|g| g.id == id) {
                out.push(("종류".into(), "그리드".into()));
                out.push(("기호:고침".into(), g.label.clone()));
                out.push(("길이".into(), f(g.a.dist(g.b))));
            }
        }
        "dim" => {
            if let Some(x) = m.dims.iter().find(|x| x.id == id) {
                use crate::arch::DimKind as K;
                let (a, b) = x.ends(m);
                let (kind, value) = match x.kind {
                    K::Linear => (
                        if x.rotation.is_some() {
                            "치수 (회전)"
                        } else {
                            "치수"
                        },
                        comma(x.geom(m).map(|k| k.len).unwrap_or(0.0)),
                    ),
                    K::Radius => ("반지름 치수", format!("R{}", comma(a.dist(b)))),
                    K::Diameter => ("지름 치수", format!("Ø{}", comma(2.0 * a.dist(b)))),
                    K::Angle => ("각도 치수", String::new()),
                };
                out.push(("종류".into(), kind.into()));
                if !value.is_empty() {
                    out.push(("값".into(), value));
                }
                if x.kind == K::Linear {
                    let tie =
                        |k: &Option<crate::arch::Anchor>| match k.as_ref().map(|k| k.what.as_str())
                        {
                            Some("wall") => "벽",
                            Some("grid") => "그리드",
                            Some("column") => "기둥",
                            _ => "없음",
                        };
                    out.push(("묶음".into(), format!("{} · {}", tie(&x.ka), tie(&x.kb))));
                }
                out.push(("글자:고침".into(), x.text.clone()));
                out.push(("글자 높이:고침".into(), f(x.text_height)));
                if matches!(x.kind, K::Linear | K::Angle) {
                    out.push((
                        if x.kind == K::Angle {
                            "호 반지름:고침"
                        } else {
                            "치수선 거리:고침"
                        }
                        .into(),
                        f(x.offset),
                    ));
                }
                out.push((
                    "끝 모양:고침".into(),
                    if x.arrow { "화살표" } else { "사선" }.into(),
                ));
                out.push((
                    "쉼표:고침".into(),
                    if x.plain { "아니오" } else { "예" }.into(),
                ));
            }
        }
        "column" => {
            if let Some(x) = m.columns.iter().find(|x| x.id == id) {
                out.push(("종류".into(), "기둥".into()));
                out.push(("폭:고침".into(), f(x.width)));
                out.push(("깊이:고침".into(), f(x.depth)));
                out.push(("회전:고침".into(), format!("{:.1}", x.angle.to_degrees())));
            }
        }
        "stair" => {
            if let Some(x) = m.stairs.iter().find(|x| x.id == id) {
                let h = m.level(x.level).map(|l| l.height).unwrap_or(3000.0);
                let r = x.risers.max(1) as f64;
                out.push((
                    "종류".into(),
                    if x.mid.is_some() {
                        "꺾인 계단"
                    } else {
                        "계단"
                    }
                    .into(),
                ));
                out.push(("폭:고침".into(), f(x.width)));
                out.push(("단 수:고침".into(), x.risers.to_string()));
                out.push(("단 높이".into(), format!("{:.1}", h / r)));
                if let Some((fl, _)) = x.flights() {
                    let depth: Vec<String> = fl
                        .iter()
                        .map(|f| format!("{:.1}", f.len / f.risers.max(1) as f64))
                        .collect();
                    out.push(("디딤판 깊이".into(), depth.join(" / ")));
                    if fl.len() > 1 {
                        out.push((
                            "줄기 단 수".into(),
                            fl.iter()
                                .map(|f| f.risers.to_string())
                                .collect::<Vec<_>>()
                                .join(" + "),
                        ));
                    }
                }
            }
        }
        "site" => {
            let a = poly_area(&m.site.boundary) * 1e-6;
            out.push(("종류".into(), "대지".into()));
            out.push((
                "대지 면적".into(),
                format!("{a:.2} ㎡ · {:.2} 평", a / PYEONG_M2),
            ));
            out.push(("용도지역".into(), m.site.zone.clone()));
        }
        _ => {}
    }
    out.push(("_what".into(), what.into()));
    out.push(("_id".into(), id.to_string()));
    out
}

fn poly_area(r: &[Vec2]) -> f64 {
    let n = r.len();
    ((0..n)
        .map(|i| r[i].x * r[(i + 1) % n].y - r[(i + 1) % n].x * r[i].y)
        .sum::<f64>()
        / 2.0)
        .abs()
}

// ── 표 ─────────────────────────────────────────────────────────────────

/// 표 하나: 제목, 머리줄, 줄들
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Table {
    pub name: String,
    pub title: String,
    pub head: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// 면적표·창호일람표·실별 마감표·법규 검토
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tables {
    pub tables: Vec<Table>,
}

/// 층 바닥면적 (㎡): 그 층 실 면적의 합 — 벽 두께 중심선 면적이 아닌 안목 면적 합이라 법정 바닥면적보다 작다
/// (구현 노트 §22 — 법정 면적 산정은 벽 중심선 기준 외곽선이 필요)
fn level_area(m: &ArchModel, level: u64) -> f64 {
    m.rooms
        .iter()
        .filter(|r| r.level == level)
        .map(|r| r.area())
        .sum::<f64>()
        * 1e-6
}

/// 층 외곽 면적 (㎡): 그 층 외벽 중심선이 이루는 닫힌 다각형 면적 — 외벽이 닫혀 있지 않으면 실 면적 합
fn level_footprint(m: &ArchModel, level: u64) -> f64 {
    let ext: Vec<&Wall> = m
        .walls
        .iter()
        .filter(|w| w.level == level && w.kind == WallKind::Exterior)
        .collect();
    if let Some(poly) = chain_polygon(&ext) {
        return poly_area(&poly) * 1e-6;
    }
    level_area(m, level)
}

/// 끝이 이어진 벽들로 닫힌 다각형 (중심선)
fn chain_polygon(walls: &[&Wall]) -> Option<Vec<Vec2>> {
    if walls.len() < 3 {
        return None;
    }
    let tol = 1.0;
    let mut used = vec![false; walls.len()];
    let mut poly = vec![walls[0].a, walls[0].b];
    used[0] = true;
    loop {
        let last = *poly.last().unwrap();
        if last.dist(poly[0]) < tol && poly.len() > 3 {
            poly.pop();
            return Some(poly);
        }
        let next = walls
            .iter()
            .enumerate()
            .find(|(i, w)| !used[*i] && (w.a.dist(last) < tol || w.b.dist(last) < tol));
        let (i, w) = next?;
        used[i] = true;
        poly.push(if w.a.dist(last) < tol { w.b } else { w.a });
    }
}

pub fn tables(d: &Drawing) -> Tables {
    let m = &d.arch;
    let f2 = |v: f64| format!("{v:.2}");
    let mut out = Vec::new();

    // 면적표
    let mut area = Table {
        name: "면적표".into(),
        title: "면적표".into(),
        head: vec![
            "층".into(),
            "실".into(),
            "면적(㎡)".into(),
            "면적(평)".into(),
        ],
        rows: Vec::new(),
    };
    let mut total = 0.0;
    for l in &m.levels {
        let mut sub = 0.0;
        for r in m.rooms.iter().filter(|r| r.level == l.id) {
            let a = r.area() * 1e-6;
            sub += a;
            area.rows.push(vec![
                l.name.clone(),
                r.name.clone(),
                f2(a),
                f2(a / PYEONG_M2),
            ]);
        }
        let fp = level_footprint(m, l.id);
        area.rows.push(vec![
            l.name.clone(),
            "소계 (실 면적 합)".into(),
            f2(sub),
            f2(sub / PYEONG_M2),
        ]);
        area.rows.push(vec![
            l.name.clone(),
            "바닥면적 (외벽 중심선)".into(),
            f2(fp),
            f2(fp / PYEONG_M2),
        ]);
        total += fp;
    }
    area.rows.push(vec![
        "합계".into(),
        "연면적".into(),
        f2(total),
        f2(total / PYEONG_M2),
    ]);
    out.push(area);

    // 창호일람표
    let marks = opening_marks(m);
    // (기호, 종류, 폭, 높이, 창대, 수량, 층들)
    type Group = (String, String, f64, f64, f64, usize, Vec<String>);
    let mut groups: Vec<Group> = Vec::new();
    for o in &m.openings {
        let mark = marks
            .iter()
            .find(|(i, _)| *i == o.id)
            .map(|(_, s)| s.clone())
            .unwrap_or_default();
        let level = m
            .wall(o.wall)
            .and_then(|w| m.level(w.level))
            .map(|l| l.name.clone())
            .unwrap_or_default();
        if let Some(g) = groups.iter_mut().find(|g| g.0 == mark) {
            g.5 += 1;
            if !g.6.contains(&level) {
                g.6.push(level);
            }
        } else {
            let kind = if o.kind == OpeningKind::Door {
                "문"
            } else {
                "창"
            };
            groups.push((mark, kind.into(), o.width, o.height, o.sill, 1, vec![level]));
        }
    }
    groups.sort_by(|a, b| a.0.cmp(&b.0));
    out.push(Table {
        name: "창호일람표".into(),
        title: "창호일람표".into(),
        head: vec![
            "기호".into(),
            "종류".into(),
            "폭".into(),
            "높이".into(),
            "창대".into(),
            "수량".into(),
            "층".into(),
        ],
        rows: groups
            .into_iter()
            .map(|(mark, kind, w, h, s, n, lv)| {
                vec![
                    mark,
                    kind,
                    format!("{w:.0}"),
                    format!("{h:.0}"),
                    format!("{s:.0}"),
                    n.to_string(),
                    lv.join(", "),
                ]
            })
            .collect(),
    });

    // 실별 마감표
    out.push(Table {
        name: "실별마감표".into(),
        title: "실별 마감표".into(),
        head: vec![
            "층".into(),
            "실".into(),
            "바닥".into(),
            "벽".into(),
            "천장".into(),
        ],
        rows: m
            .levels
            .iter()
            .flat_map(|l| {
                m.rooms
                    .iter()
                    .filter(move |r| r.level == l.id)
                    .map(move |r| {
                        vec![
                            l.name.clone(),
                            r.name.clone(),
                            r.finish_floor.clone(),
                            r.finish_wall.clone(),
                            r.finish_ceiling.clone(),
                        ]
                    })
            })
            .collect(),
    });

    // 법규 검토 (건폐율·용적률)
    let site = poly_area(&m.site.boundary) * 1e-6;
    let building = m
        .levels
        .iter()
        .filter(|l| l.elevation >= 0.0)
        .map(|l| level_footprint(m, l.id))
        .fold(0.0, f64::max);
    let far_area: f64 = m
        .levels
        .iter()
        .filter(|l| l.elevation >= 0.0)
        .map(|l| level_footprint(m, l.id))
        .sum();
    let pct = |a: f64| if site > 0.0 { a / site * 100.0 } else { 0.0 };
    let verdict = |v: f64, lim: f64| {
        if lim <= 0.0 || site <= 0.0 {
            "-".to_string()
        } else if v <= lim {
            "적합".into()
        } else {
            "초과".into()
        }
    };
    let cov = pct(building);
    let far = pct(far_area);
    out.push(Table {
        name: "법규검토".into(),
        title: "법규 검토 (건폐율·용적률)".into(),
        head: vec!["항목".into(), "값".into(), "한도".into(), "판정".into()],
        rows: vec![
            vec![
                "대지 면적 (㎡)".into(),
                f2(site),
                String::new(),
                String::new(),
            ],
            vec![
                "용도지역".into(),
                m.site.zone.clone(),
                String::new(),
                String::new(),
            ],
            vec![
                "건축 면적 (㎡)".into(),
                f2(building),
                String::new(),
                String::new(),
            ],
            vec![
                "용적률 산정 연면적 (㎡)".into(),
                f2(far_area),
                String::new(),
                String::new(),
            ],
            vec![
                "건폐율 (%)".into(),
                f2(cov),
                f2(m.site.coverage_limit),
                verdict(cov, m.site.coverage_limit),
            ],
            vec![
                "용적률 (%)".into(),
                f2(far),
                f2(m.site.far_limit),
                verdict(far, m.site.far_limit),
            ],
        ],
    });
    // 도면 목록 (배치 순서)
    let mut list = Table {
        name: "도면목록".into(),
        title: "도면 목록".into(),
        head: vec![
            "순번".into(),
            "도면 번호".into(),
            "도면명".into(),
            "층".into(),
        ],
        rows: Vec::new(),
    };
    for (i, l) in d.layouts.iter().enumerate() {
        let (num, title) = match l.name.split_once(' ') {
            Some((a, b)) if a.chars().any(|c| c.is_ascii_digit()) => (a.to_string(), b.to_string()),
            _ => (String::new(), l.name.clone()),
        };
        let level = l
            .level
            .and_then(|id| m.level(id))
            .map(|x| x.name.clone())
            .unwrap_or_default();
        list.rows.push(vec![(i + 1).to_string(), num, title, level]);
    }
    out.push(list);
    Tables { tables: out }
}

/// 표 하나를 CSV 로
pub fn table_csv(t: &Tables, name: &str) -> Option<String> {
    let t = t.tables.iter().find(|x| x.name == name)?;
    let esc = |s: &str| {
        if s.contains([',', '"', '\n']) {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_string()
        }
    };
    let mut out = String::new();
    for row in std::iter::once(&t.head).chain(t.rows.iter()) {
        out += &row.iter().map(|c| esc(c)).collect::<Vec<_>>().join(",");
        out.push('\n');
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn house() -> Editor {
        let mut ed = Editor::new(new_drawing());
        // 10 m × 8 m 외벽 (두께 200) + 가운데 내벽
        apply(
            &mut ed,
            ArchReq::Walls {
                points: vec![[0.0, 0.0], [10000.0, 0.0], [10000.0, 8000.0], [0.0, 8000.0]],
                closed: true,
                thickness: 200.0,
                wall: WallKind::Exterior,
            },
        )
        .unwrap();
        apply(
            &mut ed,
            ArchReq::Walls {
                points: vec![[5000.0, 0.0], [5000.0, 8000.0]],
                closed: false,
                thickness: 100.0,
                wall: WallKind::Interior,
            },
        )
        .unwrap();
        apply(
            &mut ed,
            ArchReq::Opening {
                at: [2500.0, 0.0],
                opening: OpeningKind::Door,
                width: 900.0,
                height: 2100.0,
                sill: 0.0,
            },
        )
        .unwrap();
        apply(
            &mut ed,
            ArchReq::Opening {
                at: [7500.0, 8000.0],
                opening: OpeningKind::Window,
                width: 1800.0,
                height: 1500.0,
                sill: 900.0,
            },
        )
        .unwrap();
        ed
    }

    #[test]
    fn walls_openings_and_room_finding() {
        let mut ed = house();
        assert_eq!(ed.d.arch.walls.len(), 5);
        assert_eq!(ed.d.arch.openings.len(), 2);
        // 왼쪽 방: 안쪽 면 100..4950 × 100..7900
        apply(
            &mut ed,
            ArchReq::RoomAt {
                at: [2500.0, 4000.0],
                name: "거실".into(),
            },
        )
        .unwrap();
        let r = &ed.d.arch.rooms[0];
        let a = r.area() * 1e-6;
        let expect = (4.950 - 0.100) * (7.900 - 0.100);
        assert!((a - expect).abs() < 0.05, "{a} vs {expect}");
        // 평면 선이 생긴다 (벽·문·창·실)
        let v = with_arch(&ed.d).unwrap();
        assert!(v.entities.len() > 20);
        assert!(
            v.entities
                .iter()
                .any(|e| matches!(e.shape, Shape::Arc { .. })),
            "문 열림 호"
        );
        // 되돌리기
        ed.undo();
        assert!(ed.d.arch.rooms.is_empty());
    }

    #[test]
    fn tables_and_regulation() {
        let mut ed = house();
        apply(
            &mut ed,
            ArchReq::RoomAt {
                at: [2500.0, 4000.0],
                name: "거실".into(),
            },
        )
        .unwrap();
        apply(
            &mut ed,
            ArchReq::Site {
                boundary: vec![
                    [-5000.0, -5000.0],
                    [15000.0, -5000.0],
                    [15000.0, 13000.0],
                    [-5000.0, 13000.0],
                ],
                zone: "제2종일반주거지역".into(),
                coverage_limit: 60.0,
                far_limit: 200.0,
            },
        )
        .unwrap();
        let t = tables(&ed.d);
        let law = t.tables.iter().find(|x| x.name == "법규검토").unwrap();
        // 대지 20 × 18 = 360 ㎡, 건축면적(외벽 중심선) 80 ㎡ → 22.22 %
        assert_eq!(law.rows[0][1], "360.00");
        assert_eq!(law.rows[4][1], "22.22");
        assert_eq!(law.rows[4][3], "적합");
        let sched = t.tables.iter().find(|x| x.name == "창호일람표").unwrap();
        assert_eq!(sched.rows.len(), 2);
        assert!(table_csv(&t, "면적표").unwrap().contains("거실"));
    }

    #[test]
    fn copy_level_and_corner_joins() {
        let mut ed = house();
        apply(
            &mut ed,
            ArchReq::CopyLevel {
                name: "2층".into()
            },
        )
        .unwrap();
        assert_eq!(ed.d.arch.levels.len(), 2);
        assert_eq!(ed.d.arch.walls.len(), 10);
        // 모서리(0,0): 외벽 두 개가 꺾여 이어져 마구리가 없다 → 바깥 모서리 (−100, −100) 이 면선 끝
        let v = with_arch(&ed.d).unwrap();
        let has_corner = v.entities.iter().any(|e| match &e.shape {
            Shape::Line { a, b } => [a, b]
                .iter()
                .any(|p| (p.x + 100.0).abs() < 1e-6 && (p.y + 100.0).abs() < 1e-6),
            _ => false,
        });
        assert!(has_corner, "바깥 모서리 꺾어 잇기");
    }

    #[test]
    fn dims_columns_stairs() {
        let mut ed = Editor::new(new_drawing());
        apply(
            &mut ed,
            ArchReq::Dim {
                a: [0.0, 0.0],
                b: [12000.0, 0.0],
                at: [6000.0, -1000.0],
                text_height: 250.0,
                arrow: false,
                plain: false,
            },
        )
        .unwrap();
        apply(
            &mut ed,
            ArchReq::Column {
                at: [0.0, 0.0],
                width: 600.0,
                depth: 600.0,
            },
        )
        .unwrap();
        apply(
            &mut ed,
            ArchReq::Stair {
                a: [1000.0, 1000.0],
                b: [4000.0, 1000.0],
                width: 1200.0,
                risers: 18,
                mid: None,
            },
        )
        .unwrap();
        let m = &ed.d.arch;
        assert_eq!((m.dims.len(), m.columns.len(), m.stairs.len()), (1, 1, 1));
        assert_eq!(m.dims[0].offset, -1000.0);
        let g = with_arch(&ed.d).unwrap();
        let texts: Vec<String> = g
            .entities
            .iter()
            .filter_map(|e| match &e.shape {
                Shape::Text(t) => Some(t.value.clone()),
                _ => None,
            })
            .collect();
        assert!(texts.contains(&"12,000".to_string()), "{texts:?}");
        assert!(texts.contains(&"UP".to_string()));
        // 치수 글자 바꾸기
        let id = m.dims[0].id;
        apply(
            &mut ed,
            ArchReq::Set {
                what: "dim".into(),
                id,
                fields: vec![("글자".into(), "<> (외곽)".into())],
            },
        )
        .unwrap();
        let g = with_arch(&ed.d).unwrap();
        assert!(
            g.entities
                .iter()
                .any(|e| matches!(&e.shape, Shape::Text(t) if t.value == "12,000 (외곽)"))
        );
        // 3D: 기둥 12 + 계단 18 × 12
        assert_eq!(crate::model3d::mesh(&ed.d.arch).tris.len(), 12 + 18 * 12);
        assert_eq!(comma(1234567.0), "1,234,567");
    }

    #[test]
    fn dim_follows_grid() {
        let mut ed = Editor::new(new_drawing());
        apply(
            &mut ed,
            ArchReq::GridSet {
                origin: [0.0, 0.0],
                xs: vec![6000.0],
                ys: vec![5000.0],
                extend: 1500.0,
            },
        )
        .unwrap();
        apply(
            &mut ed,
            ArchReq::Dim {
                a: [0.0, 0.0],
                b: [6000.0, 0.0],
                at: [3000.0, -2000.0],
                text_height: 250.0,
                arrow: false,
                plain: false,
            },
        )
        .unwrap();
        let dm = ed.d.arch.dims[0].clone();
        // 교점이라 Y1 에도 걸리지만, 가로 치수라 세로 그리드(X1·X2)에 묶인다
        let x2 = ed.d.arch.grids.iter().find(|g| g.label == "X2").unwrap().id;
        assert_eq!(
            dm.kb.as_ref().map(|k| (k.what.as_str(), k.id)),
            Some(("grid", x2))
        );
        apply(
            &mut ed,
            ArchReq::Move {
                what: "grid".into(),
                id: x2,
                dx: 500.0,
                dy: 0.0,
            },
        )
        .unwrap();
        let (a, b) = ed.d.arch.dims[0].ends(&ed.d.arch);
        assert!((a.dist(b) - 6500.0).abs() < 1e-9);
        let g = with_arch(&ed.d).unwrap();
        assert!(
            g.entities
                .iter()
                .any(|e| matches!(&e.shape, Shape::Text(t) if t.value == "6,500"))
        );
        // 치수 옮기기는 치수선만
        apply(
            &mut ed,
            ArchReq::Move {
                what: "dim".into(),
                id: dm.id,
                dx: 0.0,
                dy: -500.0,
            },
        )
        .unwrap();
        assert_eq!(ed.d.arch.dims[0].offset, dm.offset - 500.0);
    }

    #[test]
    fn rotated_dim_measures_projection() {
        // 세로(π/2) 회전 치수: 두 점이 x 로 어긋나도 y 차이만 잰다
        let mut m = crate::arch::ArchModel::default();
        let level = m.ensure_level();
        m.dims.push(crate::arch::Dim {
            id: 99,
            level,
            a: Vec2::new(0.0, 0.0),
            b: Vec2::new(800.0, 3000.0),
            offset: -1000.0,
            text_height: 250.0,
            text: String::new(),
            ka: None,
            kb: None,
            rotation: Some(std::f64::consts::FRAC_PI_2),
            layer: String::new(),
            ..Default::default()
        });
        let k = m.dims[0].geom(&m).unwrap();
        assert!((k.len - 3000.0).abs() < 1e-9);
        // 치수선은 x = 1000 (n = (−1, 0), offset −1000) 에서 세로
        assert!(
            (k.da.x - 1000.0).abs() < 1e-9 && (k.db.x - 1000.0).abs() < 1e-9,
            "{k:?}"
        );
        let d = Drawing {
            arch: m,
            ..new_drawing()
        };
        let g = with_arch(&d).unwrap();
        assert!(
            g.entities
                .iter()
                .any(|e| matches!(&e.shape, Shape::Text(t) if t.value == "3,000"))
        );
    }

    #[test]
    fn l_shaped_stair() {
        let mut ed = Editor::new(new_drawing());
        apply(
            &mut ed,
            ArchReq::Stair {
                a: [0.0, 0.0],
                b: [3000.0, 3000.0],
                width: 1000.0,
                risers: 18,
                mid: Some([3000.0, 0.0]),
            },
        )
        .unwrap();
        let st = &ed.d.arch.stairs[0];
        let (fl, landing) = st.flights().unwrap();
        assert_eq!(
            (fl.len(), fl[0].risers, fl[1].risers, fl[1].first),
            (2, 9, 9, 9)
        );
        assert!(
            (fl[0].len - 2500.0).abs() < 1e-9 && fl[1].start.dist(Vec2::new(3000.0, 500.0)) < 1e-9
        );
        assert!(landing.is_some());
        // 3D: 단 18 개 + 참 하나
        assert_eq!(crate::model3d::mesh(&ed.d.arch).tris.len(), 19 * 12);
        // 너무 짧은 꺾인 계단은 거절
        assert!(
            apply(
                &mut ed,
                ArchReq::Stair {
                    a: [0.0, 0.0],
                    b: [400.0, 400.0],
                    width: 1000.0,
                    risers: 18,
                    mid: Some([400.0, 0.0])
                }
            )
            .is_err()
        );
        let p = properties(&ed.d.arch, "stair", ed.d.arch.stairs[0].id);
        assert!(
            p.iter().any(|(k, v)| k == "줄기 단 수" && v == "9 + 9"),
            "{p:?}"
        );
    }

    #[test]
    fn radial_and_angle_dims() {
        let mut ed = Editor::new(new_drawing());
        apply(
            &mut ed,
            ArchReq::DimRadial {
                center: [0.0, 0.0],
                point: [600.0, 0.0],
                diameter: false,
                text_height: 250.0,
                arrow: true,
                plain: false,
            },
        )
        .unwrap();
        apply(
            &mut ed,
            ArchReq::DimRadial {
                center: [0.0, 0.0],
                point: [0.0, 600.0],
                diameter: true,
                text_height: 250.0,
                arrow: true,
                plain: false,
            },
        )
        .unwrap();
        apply(
            &mut ed,
            ArchReq::DimAngle {
                vertex: [0.0, 0.0],
                a: [1000.0, 0.0],
                b: [1000.0, 1000.0],
                at: [800.0, 0.0],
                text_height: 250.0,
                arrow: true,
            },
        )
        .unwrap();
        let g = with_arch(&ed.d).unwrap();
        let texts: Vec<String> = g
            .entities
            .iter()
            .filter_map(|e| match &e.shape {
                Shape::Text(t) => Some(t.value.clone()),
                _ => None,
            })
            .collect();
        for want in ["R600", "Ø1,200", "45°"] {
            assert!(texts.iter().any(|t| t == want), "{want}: {texts:?}");
        }
        // 끝 모양·쉼표 바꾸기
        let id = ed.d.arch.dims[1].id;
        apply(
            &mut ed,
            ArchReq::Set {
                what: "dim".into(),
                id,
                fields: vec![("쉼표".into(), "아니오".into())],
            },
        )
        .unwrap();
        let g = with_arch(&ed.d).unwrap();
        assert!(
            g.entities
                .iter()
                .any(|e| matches!(&e.shape, Shape::Text(t) if t.value == "Ø1200"))
        );
        // 각도 치수 옮기기는 통째로
        let aid = ed.d.arch.dims[2].id;
        apply(
            &mut ed,
            ArchReq::Move {
                what: "dim".into(),
                id: aid,
                dx: 100.0,
                dy: 0.0,
            },
        )
        .unwrap();
        assert_eq!(ed.d.arch.dims[2].vertex, Some(Vec2::new(100.0, 0.0)));
    }
}
