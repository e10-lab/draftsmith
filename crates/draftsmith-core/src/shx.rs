//! SHX 글꼴 (구현 노트 §59): AutoCAD 모양 글꼴(shapes 1.0·1.1), 큰 글꼴(bigfont 1.0 — 한글·한자 2 바이트),
//! 유니코드 글꼴(unifont 1.0)을 읽어 글자마다 선(획)으로 푼다.
//!
//! 모양 명세 바이트 (AutoCAD 「Shape Descriptions」):
//! - 0x10 이상: 길이(위 4 비트) × 방향(아래 4 비트, 16 방위) 벡터
//! - 0 끝 · 1 펜 내림 · 2 펜 올림 · 3 n 나눔 · 4 n 곱함 · 5 위치 넣기 · 6 위치 꺼내기 · 7 부분 모양
//! - 8 (dx,dy) · 9 (dx,dy)… (0,0) 까지 · 10 팔분 호 · 11 분수 호 · 12 볼록 호 · 13 볼록 호들 · 14 세로 글자에서만
//!
//! 글꼴 파일은 저장소에 넣지 않는다 (저작권). 앱은 사용자가 고른 폴더에서 읽는다.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use crate::drawing::{Drawing, Entity, Shape, TextShape, VAlign};
use crate::geom::{Affine, Vec2};

/// 글꼴 종류
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Shapes,
    Big,
    Uni,
}

/// 큰 글꼴의 글자 코드 방식
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Enc {
    /// 완성형·Shift-JIS·GBK 등 (encoding_rs)
    Std(&'static encoding_rs::Encoding),
    /// 한글 조합형 (KS X 1001 부속서 3) — encoding_rs 에 없어 직접
    Johab,
}

impl Enc {
    /// 글자 하나 → 2 바이트 코드 (없으면 None)
    pub fn code(&self, ch: char) -> Option<u16> {
        match self {
            Enc::Johab => johab(ch),
            Enc::Std(e) => {
                let mut buf = [0u8; 4];
                let (b, _, bad) = e.encode(ch.encode_utf8(&mut buf));
                (!bad && b.len() == 2).then(|| (b[0] as u16) << 8 | b[1] as u16)
            }
        }
    }
}

/// 한글 음절 → 조합형 코드: 1 · 초성 5 비트 · 중성 5 비트 · 종성 5 비트
fn johab(ch: char) -> Option<u16> {
    const V: [u16; 21] = [
        3, 4, 5, 6, 7, 10, 11, 12, 13, 14, 15, 18, 19, 20, 21, 22, 23, 26, 27, 28, 29,
    ];
    let s = (ch as u32).checked_sub(0xac00)?;
    if s >= 11172 {
        return None;
    }
    let (l, v, t) = (s / 588, (s % 588) / 28, s % 28);
    let tc = match t {
        0 => 1,
        1..=16 => t + 1,
        _ => t + 2,
    };
    Some(0x8000 | ((l as u16 + 2) << 10) | (V[v as usize] << 5) | tc as u16)
}

/// 읽은 SHX 글꼴
#[derive(Debug, Clone)]
pub struct Font {
    pub kind: Kind,
    pub name: String,
    /// 기준선 위 높이 (글꼴 단위) — 글자 높이가 이 길이가 되게 줄인다
    pub above: f64,
    pub below: f64,
    /// 큰 글꼴의 글자 칸 폭 (부분 모양 상자 맞춤의 가로 기준)
    pub width: f64,
    /// 큰 글꼴 앞 바이트 범위
    pub ranges: Vec<(u16, u16)>,
    /// 큰 글꼴의 글자 코드 인코딩 (모양 번호로 짐작 — 가나 「あ」 82A0 면 Shift-JIS, 「가」 B0A1 이면 CP949)
    pub encoding: Option<Enc>,
    shapes: HashMap<u16, Vec<u8>>,
}

/// 글자 하나의 획(펜 내린 꺾은선들)과 다음 글자까지 거리 (글꼴 단위)
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Glyph {
    pub strokes: Vec<Vec<Vec2>>,
    pub advance: f64,
}

fn u16le(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}
fn u32le(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

/// 모양 정의에서 이름(0 으로 끝남)을 건너뛴 명세 바이트
fn spec(def: &[u8]) -> &[u8] {
    match def.iter().position(|&c| c == 0) {
        Some(k) => &def[k + 1..],
        None => &[],
    }
}

/// 이름 바이트 → 글자 (ASCII 밖은 Shift-JIS·CP949 일 수 있어 손실 허용)
fn name_of(def: &[u8]) -> String {
    let raw = &def[..def.iter().position(|&c| c == 0).unwrap_or(def.len())];
    String::from_utf8_lossy(raw).trim().to_string()
}

impl Font {
    pub fn parse(b: &[u8]) -> Result<Font, String> {
        let sig_end = b
            .iter()
            .take(64)
            .position(|&c| c == 0x1a)
            .ok_or("SHX 글꼴이 아닙니다")?;
        let sig = String::from_utf8_lossy(&b[..sig_end]).to_ascii_lowercase();
        let p = sig_end + 1;
        let mut shapes = HashMap::new();
        let bad = || "SHX 글꼴이 잘렸습니다".to_string();
        let (kind, ranges) = if sig.contains("bigfont") {
            let n = u16le(b, p + 2).ok_or_else(bad)? as usize;
            let nr = u16le(b, p + 4).ok_or_else(bad)? as usize;
            let mut q = p + 6;
            let mut ranges = Vec::new();
            for _ in 0..nr {
                ranges.push((
                    u16le(b, q).ok_or_else(bad)?,
                    u16le(b, q + 2).ok_or_else(bad)?,
                ));
                q += 4;
            }
            for k in 0..n {
                let e = q + 8 * k;
                let (num, len, off) = (
                    u16le(b, e).ok_or_else(bad)?,
                    u16le(b, e + 2).ok_or_else(bad)? as usize,
                    u32le(b, e + 4).ok_or_else(bad)? as usize,
                );
                if len > 0 && off + len <= b.len() {
                    shapes.insert(num, b[off..off + len].to_vec());
                }
            }
            (Kind::Big, ranges)
        } else if sig.contains("unifont") {
            let n = u32le(b, p).ok_or_else(bad)? as usize;
            let info = u16le(b, p + 4).ok_or_else(bad)? as usize;
            let mut q = p + 6;
            shapes.insert(0, b.get(q..q + info).ok_or_else(bad)?.to_vec());
            q += info;
            for _ in 1..n {
                let (num, len) = match (u16le(b, q), u16le(b, q + 2)) {
                    (Some(a), Some(l)) => (a, l as usize),
                    _ => break,
                };
                q += 4;
                let Some(def) = b.get(q..q + len) else { break };
                shapes.insert(num, def.to_vec());
                q += len;
            }
            (Kind::Uni, Vec::new())
        } else if sig.contains("shapes") {
            let n = u16le(b, p + 4).ok_or_else(bad)? as usize;
            let mut q = p + 6;
            let mut idx = Vec::with_capacity(n);
            for _ in 0..n {
                idx.push((
                    u16le(b, q).ok_or_else(bad)?,
                    u16le(b, q + 2).ok_or_else(bad)? as usize,
                ));
                q += 4;
            }
            for (num, len) in idx {
                let Some(def) = b.get(q..q + len) else { break };
                shapes.insert(num, def.to_vec());
                q += len;
            }
            (Kind::Shapes, Vec::new())
        } else {
            return Err("모르는 SHX 형식입니다".into());
        };
        // 0 번 모양: 이름, 위 높이, 아래 깊이(큰 글꼴은 0 다음 모드·폭)
        let info = shapes.get(&0).cloned().unwrap_or_default();
        let s = spec(&info);
        let above = s.first().copied().unwrap_or(0) as f64;
        let below = if kind == Kind::Big {
            0.0
        } else {
            s.get(1).copied().unwrap_or(0) as f64
        };
        let width = if kind == Kind::Big {
            s.get(3).copied().unwrap_or(0) as f64
        } else {
            0.0
        };
        let has = |c: u16| shapes.contains_key(&c);
        let encoding = (kind == Kind::Big).then(|| {
            if has(0x8861) && has(0x8862) {
                // 조합형: 「가」 8861 · 「각」 8862 (HGTXT 등)
                Enc::Johab
            } else if has(0x82a0) && has(0x8341) {
                Enc::Std(encoding_rs::SHIFT_JIS)
            } else if has(0xb0a1) && has(0xc7cf) {
                Enc::Std(encoding_rs::EUC_KR)
            } else if has(0xa4a2) {
                Enc::Std(encoding_rs::GBK)
            } else {
                Enc::Std(encoding_rs::EUC_KR)
            }
        });
        Ok(Font {
            kind,
            encoding,
            name: name_of(&info),
            above: if above > 0.0 { above } else { 10.0 },
            below,
            width: if width > 0.0 {
                width
            } else if above > 0.0 {
                above
            } else {
                10.0
            },
            ranges,
            shapes,
        })
    }

    pub fn len(&self) -> usize {
        self.shapes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }

    /// 큰 글꼴의 앞 바이트인가
    pub fn is_lead(&self, b: u8) -> bool {
        self.ranges
            .iter()
            .any(|&(a, c)| (a..=c).contains(&(b as u16)))
    }

    pub fn has(&self, code: u16) -> bool {
        code != 0 && self.shapes.contains_key(&code)
    }

    /// 모양 번호(글자 코드)의 획
    pub fn glyph(&self, code: u16) -> Option<Glyph> {
        if !self.has(code) {
            return None;
        }
        let mut st = State {
            pos: Vec2::default(),
            pen: true,
            scale: 1.0,
            fit: (1.0, 1.0),
            stack: Vec::new(),
            out: Vec::new(),
            cur: Vec::new(),
        };
        self.run(code, &mut st, 0);
        st.flush();
        Some(Glyph {
            strokes: st.out,
            advance: st.pos.x,
        })
    }

    fn run(&self, code: u16, st: &mut State, depth: usize) {
        let Some(def) = self.shapes.get(&code) else {
            return;
        };
        if depth > 8 {
            return;
        }
        let s = spec(def);
        let mut i = 0;
        while i < s.len() {
            if !self.step(s, &mut i, st, depth, true) {
                break;
            }
        }
    }

    /// 명령 하나 (draw 가 거짓이면 바이트만 건너뜀 — 14 번 다음 명령). 끝(0)이면 거짓
    fn step(&self, s: &[u8], i: &mut usize, st: &mut State, depth: usize, draw: bool) -> bool {
        let at = |k: usize| s.get(k).copied().unwrap_or(0);
        let sb = |k: usize| at(k) as i8 as f64;
        let c = at(*i);
        *i += 1;
        match c {
            0 => return false,
            1 => {
                if draw {
                    st.pen = true;
                    st.cur.clear();
                    st.cur.push(st.pos);
                }
            }
            2 => {
                if draw {
                    st.flush();
                    st.pen = false;
                }
            }
            3 => {
                let n = at(*i).max(1) as f64;
                *i += 1;
                if draw {
                    st.scale /= n;
                }
            }
            4 => {
                let n = at(*i).max(1) as f64;
                *i += 1;
                if draw {
                    st.scale *= n;
                }
            }
            5 => {
                if draw {
                    st.stack.push(st.pos);
                }
            }
            6 => {
                if draw {
                    st.flush();
                    if let Some(p) = st.stack.pop() {
                        st.pos = p;
                    }
                    if st.pen {
                        st.cur.push(st.pos);
                    }
                }
            }
            7 => {
                // 부분 모양: 큰 글꼴 7,0,번호(2),x,y,폭,높이 · 유니코드 7,번호(2) · 그 밖 7,번호
                let (num, fit) = match self.kind {
                    Kind::Big if at(*i) == 0 => {
                        let num = (at(*i + 1) as u16) << 8 | at(*i + 2) as u16;
                        let fit = (
                            at(*i + 3) as f64,
                            at(*i + 4) as f64,
                            at(*i + 5) as f64,
                            at(*i + 6) as f64,
                        );
                        *i += 7;
                        (num, Some(fit))
                    }
                    Kind::Uni => {
                        let num = (at(*i) as u16) << 8 | at(*i + 1) as u16;
                        *i += 2;
                        (num, None)
                    }
                    _ => {
                        let num = at(*i) as u16;
                        *i += 1;
                        (num, None)
                    }
                };
                if draw {
                    match fit {
                        Some((x, y, w, h)) => {
                            // 상자 (x,y)·폭 w·높이 h 에 맞춰: 가로·세로 배율이 따로 (글자 칸 폭·높이 기준)
                            let save = (st.pos, st.fit, st.pen);
                            st.flush();
                            let (fx, fy) = st.fit;
                            st.pos = Vec2::new(
                                st.pos.x + x * st.scale * fx,
                                st.pos.y + y * st.scale * fy,
                            );
                            st.fit = (fx * w / self.width, fy * h / self.above);
                            st.pen = true;
                            self.run(num, st, depth + 1);
                            st.flush();
                            st.pos = save.0;
                            st.fit = save.1;
                            st.pen = save.2;
                        }
                        None => self.run(num, st, depth + 1),
                    }
                }
            }
            8 => {
                let (dx, dy) = (sb(*i), sb(*i + 1));
                *i += 2;
                if draw {
                    st.line(dx, dy);
                }
            }
            9 => loop {
                let (dx, dy) = (sb(*i), sb(*i + 1));
                *i += 2;
                if (dx == 0.0 && dy == 0.0) || *i >= s.len() {
                    break;
                }
                if draw {
                    st.line(dx, dy);
                }
            },
            10 => {
                let r = at(*i) as f64;
                let o = at(*i + 1) as i8;
                *i += 2;
                if draw {
                    let s0 = ((o as u8 >> 4) & 7) as f64 * 45.0;
                    let n = match o as u8 & 7 {
                        0 => 8.0,
                        k => k as f64,
                    };
                    let sign = if o < 0 { -1.0 } else { 1.0 };
                    st.arc(r, s0, s0 + sign * n * 45.0);
                }
            }
            11 => {
                let so = at(*i) as f64;
                let eo = at(*i + 1) as f64;
                let r = (at(*i + 2) as f64) * 256.0 + at(*i + 3) as f64;
                let o = at(*i + 4) as i8;
                *i += 5;
                if draw {
                    let sign = if o < 0 { -1.0 } else { 1.0 };
                    let s_oct = ((o as u8 >> 4) & 7) as f64;
                    let n = match o as u8 & 7 {
                        0 => 8.0,
                        k => k as f64,
                    };
                    let a0 = s_oct * 45.0 + sign * so * 45.0 / 256.0;
                    let a1 = if eo == 0.0 {
                        (s_oct + sign * n) * 45.0
                    } else {
                        (s_oct + sign * (n - 1.0)) * 45.0 + sign * eo * 45.0 / 256.0
                    };
                    st.arc(r, a0, a1);
                }
            }
            12 => {
                let (dx, dy, bu) = (sb(*i), sb(*i + 1), sb(*i + 2));
                *i += 3;
                if draw {
                    st.bulge(dx, dy, bu);
                }
            }
            13 => loop {
                let (dx, dy) = (sb(*i), sb(*i + 1));
                *i += 2;
                if (dx == 0.0 && dy == 0.0) || *i >= s.len() {
                    break;
                }
                let bu = sb(*i);
                *i += 1;
                if draw {
                    st.bulge(dx, dy, bu);
                }
            },
            14 => {
                // 세로 글자에서만 — 가로 글자이므로 다음 명령을 건너뛴다
                self.step(s, i, st, depth, false);
            }
            v => {
                let len = (v >> 4) as f64;
                let (ux, uy) = DIRS[(v & 15) as usize];
                if draw {
                    st.line(len * ux, len * uy);
                }
            }
        }
        true
    }
}

// ── 글꼴 폴더와 글자 → 획 (§59) ───────────────────────────────────────────────

struct Library {
    dir: Option<PathBuf>,
    /// 소문자 파일 이름 → 글꼴 (없으면 None 으로 기억)
    cache: HashMap<String, Option<Arc<Font>>>,
}

fn lib() -> &'static RwLock<Library> {
    static L: OnceLock<RwLock<Library>> = OnceLock::new();
    L.get_or_init(|| {
        RwLock::new(Library {
            dir: None,
            cache: HashMap::new(),
        })
    })
}

/// SHX 글꼴을 찾을 폴더 (None 이면 쓰지 않음 — 모든 글자를 내장 글꼴로)
pub fn set_folder(dir: Option<PathBuf>) {
    if let Ok(mut l) = lib().write() {
        l.dir = dir;
        l.cache.clear();
    }
}

pub fn folder() -> Option<PathBuf> {
    lib().read().ok()?.dir.clone()
}

/// 폴더 안 SHX 파일 이름들
pub fn available() -> Vec<String> {
    let Some(dir) = folder() else {
        return Vec::new();
    };
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|r| {
            r.flatten()
                .filter_map(|e| {
                    let n = e.file_name().to_string_lossy().to_string();
                    n.to_ascii_lowercase().ends_with(".shx").then_some(n)
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// 이름으로 글꼴 (확장자 없으면 .shx, TTF 는 None)
pub fn font(name: &str) -> Option<Arc<Font>> {
    let mut key = Path::new(name.trim())
        .file_name()?
        .to_string_lossy()
        .to_ascii_lowercase();
    if key.is_empty() || key.ends_with(".ttf") || key.ends_with(".ttc") || key.ends_with(".otf") {
        return None;
    }
    if !key.contains('.') {
        key.push_str(".shx");
    }
    if let Some(f) = lib().read().ok()?.cache.get(&key) {
        return f.clone();
    }
    let dir = folder()?;
    let found = std::fs::read_dir(&dir).ok()?.flatten().find_map(|e| {
        (e.file_name().to_string_lossy().to_ascii_lowercase() == key).then(|| e.path())
    });
    let f = found
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| Font::parse(&b).ok())
        .map(Arc::new);
    if let Ok(mut l) = lib().write() {
        l.cache.insert(key, f.clone());
    }
    f
}

/// 내장 글꼴 (빠진 글자 폭)
fn face() -> Option<&'static ttf_parser::Face<'static>> {
    static F: OnceLock<Option<ttf_parser::Face<'static>>> = OnceLock::new();
    F.get_or_init(|| ttf_parser::Face::parse(crate::pdf::FONT, 0).ok())
        .as_ref()
}

/// 글자 하나를 그릴 수 있는 글꼴과 모양 번호: 큰 글꼴(2 바이트) → 기본 글꼴
fn code_for(ch: char, main: Option<&Font>, big: Option<&Font>) -> Option<(bool, u16)> {
    if let Some(b) = big
        && let Some(enc) = b.encoding
        && !ch.is_ascii()
        && let Some(code) = enc.code(ch)
        && b.is_lead((code >> 8) as u8)
        && b.has(code)
    {
        return Some((true, code));
    }
    let m = main?;
    let code = match m.kind {
        Kind::Uni => ch as u32,
        _ if (ch as u32) < 256 => ch as u32,
        _ => return None,
    };
    let code = u16::try_from(code).ok()?;
    m.has(code).then_some((false, code))
}

/// 줄 안 글자 하나: x, (획, 배율) — SHX 에 없으면 None, 글자
type Item = (f64, Option<(Glyph, f64)>, char);
/// 스타일의 (기본 글꼴, 큰 글꼴)
type FontPair = (Option<Arc<Font>>, Option<Arc<Font>>);

/// 글자 하나를 SHX 획으로 푼 결과: 획 선분(세계 좌표)과 SHX 에 없는 글자들(내장 글꼴 글자로)
pub struct Laid {
    pub lines: Vec<(Vec2, Vec2)>,
    pub rest: Vec<TextShape>,
}

/// 글자를 SHX 로 (§59). 그릴 수 있는 글자가 하나도 없으면 None
pub fn lay(t: &TextShape, main: Option<&Font>, big: Option<&Font>) -> Option<Laid> {
    let h = t.height;
    if h <= 0.0 || (main.is_none() && big.is_none()) {
        return None;
    }
    let wf = if t.width_factor > 0.0 {
        t.width_factor
    } else {
        1.0
    };
    let lines: Vec<&str> = t.value.split('\n').collect();
    let gap = h * 5.0 / 3.0;
    let n = lines.len() as f64;
    // 첫 줄 바탕선 높이 (블록 맞춤)
    let first = match t.valign {
        VAlign::Baseline => 0.0,
        VAlign::Bottom => (n - 1.0) * gap,
        VAlign::Middle => ((n - 1.0) * gap - h) / 2.0,
        VAlign::Top => -h,
    };
    let place = Affine::rotate(t.rotation).then(&Affine::translate(t.at.x, t.at.y));
    let mut out = Laid {
        lines: Vec::new(),
        rest: Vec::new(),
    };
    let mut any = false;
    for (li, line) in lines.iter().enumerate() {
        let y = first - li as f64 * gap;
        // 줄 폭을 먼저 (가로 맞춤)
        let mut items: Vec<Item> = Vec::new();
        let mut x = 0.0;
        for ch in line.chars() {
            match code_for(ch, main, big) {
                Some((is_big, code)) => {
                    let f = if is_big { big } else { main }.unwrap();
                    let g = f.glyph(code).unwrap_or_default();
                    let k = h / f.above;
                    let adv = g.advance * k * wf;
                    items.push((x, Some((g, k)), ch));
                    x += adv;
                    any = true;
                }
                None => {
                    let em = face()
                        .map(|f| crate::pdf::text_width(f, &ch.to_string()))
                        .unwrap_or(0.6);
                    items.push((x, None, ch));
                    x += em * h * crate::pdf::EM_PER_HEIGHT * wf;
                }
            }
        }
        let x0 = -t.halign * x;
        for (gx, g, ch) in items {
            match g {
                Some((g, k)) => {
                    for s in &g.strokes {
                        for w in s.windows(2) {
                            let p = |q: Vec2| {
                                place.apply(Vec2::new(x0 + gx + q.x * k * wf, y + q.y * k))
                            };
                            out.lines.push((p(w[0]), p(w[1])));
                        }
                    }
                }
                None if !ch.is_whitespace() => out.rest.push(TextShape {
                    at: place.apply(Vec2::new(x0 + gx, y)),
                    height: h,
                    rotation: t.rotation,
                    width_factor: wf,
                    halign: 0.0,
                    valign: VAlign::Baseline,
                    value: ch.to_string(),
                    style: String::new(),
                }),
                None => {}
            }
        }
    }
    any.then_some(out)
}

/// 보이는 도면의 글자 중 SHX 글꼴이 폴더에 있는 것을 획으로 바꾼다 (자리는 그대로 — 번호가 안 바뀜).
/// SHX 에 없는 글자는 내장 글꼴 글자로 맨 뒤에 붙는다. 바꾼 글자 수
pub fn apply(d: &mut Drawing) -> usize {
    if folder().is_none() || d.fonts.is_empty() {
        return 0;
    }
    // 스타일 → (기본 글꼴, 큰 글꼴)
    let styles: HashMap<String, FontPair> = d
        .fonts
        .iter()
        .map(|f| {
            (
                f.style.to_ascii_lowercase(),
                (font(&f.font), font(&f.big_font)),
            )
        })
        .filter(|(_, (a, b))| a.is_some() || b.is_some())
        .collect();
    if styles.is_empty() {
        return 0;
    }
    let mut extra: Vec<Entity> = Vec::new();
    let mut n = 0;
    for e in d.entities.iter_mut() {
        let Shape::Text(t) = &e.shape else { continue };
        let key = if t.style.is_empty() {
            "standard".to_string()
        } else {
            t.style.to_ascii_lowercase()
        };
        let Some((m, b)) = styles.get(&key) else {
            continue;
        };
        let Some(laid) = lay(t, m.as_deref(), b.as_deref()) else {
            continue;
        };
        for r in laid.rest {
            extra.push(Entity {
                shape: Shape::Text(r),
                ..e.clone()
            });
        }
        e.shape = Shape::Segments { lines: laid.lines };
        n += 1;
    }
    d.entities.extend(extra);
    n
}

/// 16 방위 단위 (0 동, 4 북, 8 서, C 남)
const DIRS: [(f64, f64); 16] = [
    (1.0, 0.0),
    (1.0, 0.5),
    (1.0, 1.0),
    (0.5, 1.0),
    (0.0, 1.0),
    (-0.5, 1.0),
    (-1.0, 1.0),
    (-1.0, 0.5),
    (-1.0, 0.0),
    (-1.0, -0.5),
    (-1.0, -1.0),
    (-0.5, -1.0),
    (0.0, -1.0),
    (0.5, -1.0),
    (1.0, -1.0),
    (1.0, -0.5),
];

struct State {
    pos: Vec2,
    pen: bool,
    scale: f64,
    /// 부분 모양 상자 맞춤의 가로·세로 배율
    fit: (f64, f64),
    stack: Vec<Vec2>,
    out: Vec<Vec<Vec2>>,
    cur: Vec<Vec2>,
}

impl State {
    fn flush(&mut self) {
        if self.cur.len() >= 2 {
            self.out.push(std::mem::take(&mut self.cur));
        } else {
            self.cur.clear();
        }
    }
    fn to(&mut self, p: Vec2) {
        if self.pen {
            if self.cur.is_empty() {
                self.cur.push(self.pos);
            }
            self.cur.push(p);
        }
        self.pos = p;
    }
    /// 글꼴 단위 변위 → 실제 변위 (배율·상자 맞춤)
    fn d(&self, dx: f64, dy: f64) -> (f64, f64) {
        (dx * self.scale * self.fit.0, dy * self.scale * self.fit.1)
    }
    fn line(&mut self, dx: f64, dy: f64) {
        let (dx, dy) = self.d(dx, dy);
        let p = Vec2::new(self.pos.x + dx, self.pos.y + dy);
        self.to(p);
    }
    /// 지금 점이 각 a0(도)에 있는 반지름 r 원을 a1 까지
    fn arc(&mut self, r: f64, a0: f64, a1: f64) {
        let (t0, t1) = (a0.to_radians(), a1.to_radians());
        let start = self.pos;
        let n = (((t1 - t0).abs() / (std::f64::consts::PI / 16.0)).ceil() as usize).max(2);
        for k in 1..=n {
            let t = t0 + (t1 - t0) * k as f64 / n as f64;
            // 시작점에서의 변위를 글꼴 단위로 셈한 뒤 배율 (상자 맞춤이면 타원)
            let (dx, dy) = self.d(r * (t.cos() - t0.cos()), r * (t.sin() - t0.sin()));
            self.to(Vec2::new(start.x + dx, start.y + dy));
        }
    }
    /// 볼록 호: 끝 (dx,dy), 볼록 값 −127..127 (높이 = 볼록 × 현 / 254)
    fn bulge(&mut self, dx: f64, dy: f64, b: f64) {
        let d = (dx * dx + dy * dy).sqrt();
        if b == 0.0 || d < 1e-12 {
            self.line(dx, dy);
            return;
        }
        // 폴리선 볼록 값 tan(θ/4) = 2h/d
        let bu = (b / 127.0).clamp(-1.0, 1.0);
        let th = 4.0 * bu.atan();
        let (ux, uy) = (dx / d, dy / d);
        let k = d / 2.0 / (th / 2.0).tan();
        // 글꼴 단위로 시작점 원점의 호를 셈하고, 변위에 배율
        let c = Vec2::new(dx / 2.0 - uy * k, dy / 2.0 + ux * k);
        let r = (c.x * c.x + c.y * c.y).sqrt();
        let t0 = (-c.y).atan2(-c.x);
        let start = self.pos;
        let n = ((th.abs() / (std::f64::consts::PI / 16.0)).ceil() as usize).max(2);
        for i in 1..=n {
            let t = t0 + th * i as f64 / n as f64;
            let (ex, ey) = self.d(c.x + r * t.cos(), c.y + r * t.sin());
            self.to(Vec2::new(start.x + ex, start.y + ey));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 시험용 모양 글꼴: 'A' = ㄷ 뒤집힌 꼴 (위 10, 오른 6, 아래 10) + 다음 글자까지 8
    fn shapes_font() -> Vec<u8> {
        let defs: Vec<(u16, Vec<u8>)> = vec![
            (0, [b"TEST\0".as_slice(), &[10, 2, 0, 0]].concat()),
            (
                65,
                [b"A\0".as_slice(), &[0xA4, 0x60, 0xAC, 2, 8, 2, 0, 0]].concat(),
            ),
        ];
        let mut b = b"AutoCAD-86 shapes 1.0\r\n\x1a".to_vec();
        b.extend(0u16.to_le_bytes());
        b.extend(65u16.to_le_bytes());
        b.extend((defs.len() as u16).to_le_bytes());
        for (n, d) in &defs {
            b.extend(n.to_le_bytes());
            b.extend((d.len() as u16).to_le_bytes());
        }
        for (_, d) in &defs {
            b.extend(d);
        }
        b
    }

    /// 시험용 큰 글꼴 (Shift-JIS): 'ア'(8341) = 상자, 'あ'(82A0) = 'ア' 를 폭 5·높이 10 상자에 맞춘 부분 모양 + 진행 10
    fn big_font() -> Vec<u8> {
        let defs: Vec<(u16, Vec<u8>)> = vec![
            (0, [b"BIG\0".as_slice(), &[10, 0, 0, 10, 0]].concat()),
            (
                0x8341,
                [b"\0".as_slice(), &[0xA4, 0xA0, 0xAC, 0xA8, 2, 8, 10, 0, 0]].concat(),
            ),
            (
                0x82a0,
                [
                    b"\0".as_slice(),
                    &[7, 0, 0x83, 0x41, 0, 0, 5, 10, 2, 8, 10, 0, 0],
                ]
                .concat(),
            ),
        ];
        let mut b = b"AutoCAD-86 bigfont 1.0\r\n\x1a".to_vec();
        b.extend(8u16.to_le_bytes());
        b.extend((defs.len() as u16).to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(0x81u16.to_le_bytes());
        b.extend(0x9fu16.to_le_bytes());
        let mut off = b.len() + 8 * defs.len();
        let mut body: Vec<u8> = Vec::new();
        for (n, d) in &defs {
            b.extend(n.to_le_bytes());
            b.extend((d.len() as u16).to_le_bytes());
            b.extend((off as u32).to_le_bytes());
            off += d.len();
            body.extend(d);
        }
        b.extend(body);
        b
    }

    #[test]
    fn parse_and_draw() {
        let f = Font::parse(&shapes_font()).unwrap();
        assert_eq!(
            (f.kind, f.name.as_str(), f.above, f.below),
            (Kind::Shapes, "TEST", 10.0, 2.0)
        );
        let g = f.glyph(65).unwrap();
        assert_eq!(
            g.strokes,
            vec![vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(0.0, 10.0),
                Vec2::new(6.0, 10.0),
                Vec2::new(6.0, 0.0)
            ]]
        );
        assert_eq!(g.advance, 8.0);
        let b = Font::parse(&big_font()).unwrap();
        assert_eq!(b.encoding, Some(Enc::Std(encoding_rs::SHIFT_JIS)));
        assert!(b.is_lead(0x82) && !b.is_lead(0x41));
        // 부분 모양 상자 맞춤: 가로 5/10 배
        let g = b.glyph(0x82a0).unwrap();
        let xs: Vec<f64> = g.strokes.iter().flatten().map(|p| p.x).collect();
        assert_eq!(xs.iter().cloned().fold(f64::MIN, f64::max), 5.0);
        assert_eq!(g.advance, 10.0);
    }

    #[test]
    fn lay_text_with_fallback() {
        let m = Font::parse(&shapes_font()).unwrap();
        let b = Font::parse(&big_font()).unwrap();
        let t = TextShape {
            at: Vec2::new(100.0, 0.0),
            height: 2.5,
            rotation: 0.0,
            width_factor: 1.0,
            halign: 0.0,
            valign: VAlign::Baseline,
            value: "Aあ가".into(),
            style: "S".into(),
        };
        let l = lay(&t, Some(&m), Some(&b)).unwrap();
        // 'A' 획 3 선분 + 'あ' 상자 4 선분, '가' 는 내장 글꼴 글자로
        assert_eq!(l.lines.len(), 7);
        assert_eq!(l.rest.len(), 1);
        assert_eq!(l.rest[0].value, "가");
        // 'A' 높이 2.5 (위 10 → 0.25 배), 'あ' 는 A 진행(8×0.25=2) 뒤
        assert!(
            l.lines
                .iter()
                .any(|(a, b)| a.dist(Vec2::new(100.0, 0.0)) < 1e-9
                    && b.dist(Vec2::new(100.0, 2.5)) < 1e-9)
        );
        assert!((l.rest[0].at.x - (100.0 + 2.0 + 2.5)).abs() < 1e-9);
        // 가운데 맞춤이면 줄 폭 반만큼 왼쪽
        let c = lay(
            &TextShape {
                halign: 0.5,
                ..t.clone()
            },
            Some(&m),
            Some(&b),
        )
        .unwrap();
        assert!(c.lines.iter().all(|(a, _)| a.x < 100.0 + 4.5));
    }

    /// 조합형 코드 계산 (KS X 1001 부속서 3)
    #[test]
    fn johab_codes() {
        assert_eq!(johab('가'), Some(0x8861));
        assert_eq!(johab('각'), Some(0x8862));
        assert_eq!(johab('힣'), Some(0xd3bd));
        assert_eq!(johab('A'), None);
    }

    /// 실제 한글·영문 글꼴로 (폴더가 있을 때만): SHX_DIR=… cargo test — 파일 이름에 whgtxt·hgtxt·txt·romans
    #[test]
    fn real_korean_fonts_if_given() {
        let Ok(dir) = std::env::var("SHX_DIR") else {
            return;
        };
        let find = |key: &str| {
            std::fs::read_dir(&dir)
                .unwrap()
                .flatten()
                .map(|e| e.path())
                .find(|p| {
                    let n = p
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .to_ascii_lowercase();
                    n.rsplit('-').next() == Some(format!("{key}.shx").as_str())
                })
        };
        let ld = |key: &str| find(key).map(|p| Font::parse(&std::fs::read(p).unwrap()).unwrap());
        let (wh, hg, txt, ro) = (ld("whgtxt"), ld("hgtxt"), ld("txt"), ld("romans"));
        if let Some(f) = &wh {
            assert_eq!(f.encoding, Some(Enc::Std(encoding_rs::EUC_KR)));
        }
        if let Some(f) = &hg {
            assert_eq!(f.encoding, Some(Enc::Johab));
        }
        for big in [&wh, &hg].into_iter().flatten() {
            for ch in "건축평면도층계단실면적".chars() {
                let (is_big, code) =
                    code_for(ch, None, Some(big)).unwrap_or_else(|| panic!("{ch}"));
                assert!(
                    is_big && !big.glyph(code).unwrap().strokes.is_empty(),
                    "{ch}"
                );
            }
            // KS 2350 자 밖은 없다 → 내장 글꼴
            assert!(code_for('똠', None, Some(big)).is_none());
        }
        for m in [&txt, &ro].into_iter().flatten() {
            assert_eq!(m.kind, Kind::Uni);
            for ch in "A-101 (2,400)".chars().filter(|c| *c != ' ') {
                assert!(code_for(ch, Some(m), None).is_some(), "{ch}");
            }
        }
    }

    /// 올려 받은 실제 글꼴로 (경로가 있을 때만): SHX=…/extfont2.shx cargo test
    #[test]
    fn real_bigfont_if_given() {
        let Ok(p) = std::env::var("SHX") else { return };
        let f = Font::parse(&std::fs::read(p).unwrap()).unwrap();
        assert_eq!(f.kind, Kind::Big);
        assert_eq!(f.encoding, Some(Enc::Std(encoding_rs::SHIFT_JIS)));
        for ch in "日本語東京漢字建築図面設計階段あいうアイウ".chars() {
            let (big, code) = code_for(ch, None, Some(&f)).unwrap_or_else(|| panic!("{ch}"));
            assert!(big);
            let g = f.glyph(code).unwrap();
            assert!(!g.strokes.is_empty() && g.advance > 0.0, "{ch}");
            // 글자 칸 안에 (부분 모양 맞춤이 틀리면 밖으로 나간다)
            for p in g.strokes.iter().flatten() {
                assert!(
                    p.x > -1.0 && p.x < f.width + 2.0 && p.y > -4.0 && p.y < f.above + 2.0,
                    "{ch} {p:?}"
                );
            }
        }
        // 한글은 이 글꼴(일본어)에 없다
        assert!(code_for('가', None, Some(&f)).is_none());
    }
}
