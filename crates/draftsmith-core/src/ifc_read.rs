//! IFC 가져오기 (구현 노트 §47): IFC2x3·IFC4 STEP 파일에서 층·벽·문·창·실·기둥·그리드를 건축 객체로.
//!
//! 평면 도구라서 3D 를 다 읽지 않는다. 객체마다 놓는 자리(IfcLocalPlacement 사슬)와
//! 몸체(밀어 올린 단면 — 사각형·다각형·원)의 바닥 모양만 읽고, Z 축은 위를 향한다고 본다.
//! - 벽: 축선(Axis) 이 있으면 그 두 끝, 없으면 바닥 모양의 긴 쪽이 축 · 짧은 쪽이 두께
//! - 문·창: IfcRelFillsElement → 구멍 → IfcRelVoidsElement → 벽. 가운데를 벽 축에 내려 위치를 잡는다
//! - 실: 바닥 다각형, 이름은 LongName (없으면 Name)
//! - 층: IfcBuildingStorey 의 Elevation, 층고는 다음 층까지 (마지막은 앞 층고)
//!
//! 길이 단위는 IfcSIUnit(.LENGTHUNIT.) 의 접두어로 mm 로 바꾼다 (없으면 m).

use std::collections::HashMap;

use crate::arch::{Column, Grid, Level, Opening, OpeningKind, Room, Wall, WallKind};
use crate::drawing::Drawing;
use crate::geom::Vec2;

/// STEP 값
#[derive(Debug, Clone, PartialEq)]
enum V {
    Ref(usize),
    Str(String),
    Num(f64),
    Enum(String),
    List(Vec<V>),
    Null,
    /// IFCLABEL('x') 같이 형 이름이 붙은 값
    Typed(String, Box<V>),
}

impl V {
    fn r(&self) -> Option<usize> {
        match self {
            V::Ref(i) => Some(*i),
            _ => None,
        }
    }
    fn n(&self) -> Option<f64> {
        match self {
            V::Num(x) => Some(*x),
            V::Typed(_, v) => v.n(),
            _ => None,
        }
    }
    fn s(&self) -> Option<&str> {
        match self {
            V::Str(x) => Some(x),
            V::Typed(_, v) => v.s(),
            _ => None,
        }
    }
    fn e(&self) -> Option<&str> {
        match self {
            V::Enum(x) => Some(x),
            _ => None,
        }
    }
    fn list(&self) -> &[V] {
        match self {
            V::List(x) => x,
            _ => &[],
        }
    }
}

/// 읽은 STEP 파일: 번호 → (형 이름 대문자, 인자)
struct Step {
    rec: HashMap<usize, (String, Vec<V>)>,
}

/// STEP 문자열 풀기: '' → ', \X2\…\X0\ (UTF-16), \X\hh (Latin-1), \S\c
fn unescape(raw: &str) -> String {
    let mut out = String::new();
    let b: Vec<char> = raw.chars().collect();
    let mut i = 0;
    while i < b.len() {
        let rest: String = b[i..b.len().min(i + 4)].iter().collect();
        if rest.eq_ignore_ascii_case("\\X2\\") {
            i += 4;
            let mut units = Vec::new();
            while i + 4 <= b.len() && b[i] != '\\' {
                let h: String = b[i..i + 4].iter().collect();
                units.push(u16::from_str_radix(&h, 16).unwrap_or(0xfffd));
                i += 4;
            }
            out.push_str(&String::from_utf16_lossy(&units));
            // \X0\
            i += 4;
        } else if rest.len() >= 3 && rest[..3].eq_ignore_ascii_case("\\X\\") && i + 5 <= b.len() {
            let h: String = b[i + 3..i + 5].iter().collect();
            out.push(u8::from_str_radix(&h, 16).map(|c| c as char).unwrap_or('?'));
            i += 5;
        } else if rest.len() >= 3 && rest[..3].eq_ignore_ascii_case("\\S\\") && i + 3 < b.len() {
            out.push(char::from_u32(b[i + 3] as u32 + 128).unwrap_or('?'));
            i += 4;
        } else if rest.starts_with("\\\\") {
            out.push('\\');
            i += 2;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn value(&mut self) -> Option<V> {
        self.ws();
        let c = *self.s.get(self.i)?;
        Some(match c {
            b'$' | b'*' => {
                self.i += 1;
                V::Null
            }
            b'#' => {
                self.i += 1;
                let st = self.i;
                while self.i < self.s.len() && self.s[self.i].is_ascii_digit() {
                    self.i += 1;
                }
                V::Ref(
                    std::str::from_utf8(&self.s[st..self.i])
                        .ok()?
                        .parse()
                        .ok()?,
                )
            }
            b'\'' => {
                self.i += 1;
                let mut raw = Vec::new();
                loop {
                    let c = *self.s.get(self.i)?;
                    self.i += 1;
                    if c == b'\'' {
                        if self.s.get(self.i) == Some(&b'\'') {
                            raw.push(b'\'');
                            self.i += 1;
                        } else {
                            break;
                        }
                    } else {
                        raw.push(c);
                    }
                }
                V::Str(unescape(&String::from_utf8_lossy(&raw)))
            }
            b'.' => {
                self.i += 1;
                let st = self.i;
                while self.i < self.s.len() && self.s[self.i] != b'.' {
                    self.i += 1;
                }
                let e = String::from_utf8_lossy(&self.s[st..self.i]).to_uppercase();
                self.i += 1;
                V::Enum(e)
            }
            b'(' => {
                self.i += 1;
                V::List(self.args()?)
            }
            b'"' => {
                // 2진 값은 쓰지 않는다
                self.i += 1;
                while self.i < self.s.len() && self.s[self.i] != b'"' {
                    self.i += 1;
                }
                self.i += 1;
                V::Null
            }
            c if c == b'-' || c == b'+' || c.is_ascii_digit() => {
                let st = self.i;
                self.i += 1;
                while self.i < self.s.len()
                    && (self.s[self.i].is_ascii_digit() || b".eE+-".contains(&self.s[self.i]))
                {
                    self.i += 1;
                }
                V::Num(
                    std::str::from_utf8(&self.s[st..self.i])
                        .ok()?
                        .parse()
                        .unwrap_or(0.0),
                )
            }
            c if c.is_ascii_alphabetic() => {
                let st = self.i;
                while self.i < self.s.len()
                    && (self.s[self.i].is_ascii_alphanumeric() || self.s[self.i] == b'_')
                {
                    self.i += 1;
                }
                let name = String::from_utf8_lossy(&self.s[st..self.i]).to_uppercase();
                self.ws();
                if self.s.get(self.i) == Some(&b'(') {
                    self.i += 1;
                    let mut a = self.args()?;
                    V::Typed(
                        name,
                        Box::new(if a.len() == 1 {
                            a.remove(0)
                        } else {
                            V::List(a)
                        }),
                    )
                } else {
                    V::Enum(name)
                }
            }
            _ => return None,
        })
    }
    /// '(' 다음부터 ')' 까지
    fn args(&mut self) -> Option<Vec<V>> {
        let mut out = Vec::new();
        loop {
            self.ws();
            match *self.s.get(self.i)? {
                b')' => {
                    self.i += 1;
                    return Some(out);
                }
                b',' => self.i += 1,
                _ => out.push(self.value()?),
            }
        }
    }
}

impl Step {
    fn parse(text: &str) -> Result<Step, String> {
        let bytes = text.as_bytes();
        let start =
            find_ci(bytes, b"DATA;").ok_or("IFC(STEP) 파일이 아닙니다 — DATA 부분이 없습니다")?;
        let mut rec = HashMap::new();
        let mut i = start + 5;
        // 문장(; 로 끝남)마다. 글자 안의 ; 와 주석 /* */ 는 건너뛴다
        while i < bytes.len() {
            let st = i;
            let mut q = false;
            while i < bytes.len() {
                match bytes[i] {
                    b'\'' => q = !q,
                    b'/' if !q && bytes.get(i + 1) == Some(&b'*') => {
                        i = find(&bytes[i..], b"*/")
                            .map(|k| i + k + 1)
                            .unwrap_or(bytes.len());
                    }
                    b';' if !q => break,
                    _ => {}
                }
                i += 1;
            }
            let stmt = &bytes[st..i.min(bytes.len())];
            i += 1;
            let t = String::from_utf8_lossy(stmt);
            let t = t.trim();
            if t.eq_ignore_ascii_case("ENDSEC") {
                break;
            }
            let Some(rest) = t.strip_prefix('#') else {
                continue;
            };
            let Some(eq) = rest.find('=') else { continue };
            let Ok(id) = rest[..eq].trim().parse::<usize>() else {
                continue;
            };
            let body = rest[eq + 1..].trim();
            let Some(open) = body.find('(') else { continue };
            let name = body[..open].trim().to_uppercase();
            let mut p = P {
                s: body.as_bytes(),
                i: open + 1,
            };
            if let Some(args) = p.args() {
                rec.insert(id, (name, args));
            }
        }
        if rec.is_empty() {
            return Err("IFC 객체가 하나도 없습니다".into());
        }
        Ok(Step { rec })
    }

    fn get(&self, id: usize) -> Option<(&str, &[V])> {
        self.rec.get(&id).map(|(n, a)| (n.as_str(), a.as_slice()))
    }
    fn of<'a>(&'a self, kinds: &'a [&str]) -> impl Iterator<Item = (usize, &'a [V])> + 'a {
        let mut ids: Vec<_> = self
            .rec
            .iter()
            .filter(|(_, (n, _))| kinds.contains(&n.as_str()))
            .map(|(i, _)| *i)
            .collect();
        ids.sort_unstable();
        ids.into_iter().map(move |i| (i, self.rec[&i].1.as_slice()))
    }
    fn arg(&self, id: usize, k: usize) -> Option<&V> {
        self.get(id).and_then(|(_, a)| a.get(k))
    }
    /// IFCCARTESIANPOINT 좌표
    fn point(&self, id: usize) -> Option<[f64; 3]> {
        let (n, a) = self.get(id)?;
        if n != "IFCCARTESIANPOINT" {
            return None;
        }
        let c: Vec<f64> = a.first()?.list().iter().filter_map(V::n).collect();
        Some([
            *c.first()?,
            *c.get(1).unwrap_or(&0.0),
            *c.get(2).unwrap_or(&0.0),
        ])
    }
    fn dir(&self, v: Option<&V>) -> Option<Vec2> {
        let (n, a) = self.get(v?.r()?)?;
        if n != "IFCDIRECTION" {
            return None;
        }
        let c: Vec<f64> = a.first()?.list().iter().filter_map(V::n).collect();
        let d = Vec2::new(*c.first()?, *c.get(1).unwrap_or(&0.0));
        let l = d.dist(Vec2::default());
        (l > 1e-12).then(|| Vec2::new(d.x / l, d.y / l))
    }
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}
fn find_ci(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w.eq_ignore_ascii_case(n))
}

/// 평면 자리: 원점·x 축 방향·높이
#[derive(Debug, Clone, Copy)]
struct Place {
    o: Vec2,
    x: Vec2,
    z: f64,
}

impl Place {
    const ID: Place = Place {
        o: Vec2 { x: 0.0, y: 0.0 },
        x: Vec2 { x: 1.0, y: 0.0 },
        z: 0.0,
    };
    fn rot(&self, v: Vec2) -> Vec2 {
        Vec2::new(
            self.x.x * v.x - self.x.y * v.y,
            self.x.y * v.x + self.x.x * v.y,
        )
    }
    fn at(&self, v: Vec2) -> Vec2 {
        let r = self.rot(v);
        Vec2::new(self.o.x + r.x, self.o.y + r.y)
    }
    /// self 안에 놓인 c
    fn then(&self, c: &Place) -> Place {
        Place {
            o: self.at(c.o),
            x: self.rot(c.x),
            z: self.z + c.z,
        }
    }
}

struct Reader {
    st: Step,
    /// 길이 → mm
    k: f64,
    places: HashMap<usize, Place>,
}

impl Reader {
    fn len(&self, v: Option<&V>) -> Option<f64> {
        v.and_then(V::n).map(|x| x * self.k)
    }

    /// IfcAxis2Placement2D/3D
    fn axis(&self, id: Option<usize>) -> Place {
        let Some(id) = id else { return Place::ID };
        let Some((n, a)) = self.st.get(id) else {
            return Place::ID;
        };
        let p = a
            .first()
            .and_then(V::r)
            .and_then(|i| self.st.point(i))
            .unwrap_or([0.0; 3]);
        let x = match n {
            "IFCAXIS2PLACEMENT3D" => self.st.dir(a.get(2)),
            "IFCAXIS2PLACEMENT2D" => self.st.dir(a.get(1)),
            _ => None,
        };
        Place {
            o: Vec2::new(p[0] * self.k, p[1] * self.k),
            x: x.unwrap_or(Vec2::new(1.0, 0.0)),
            z: p[2] * self.k,
        }
    }

    /// IfcLocalPlacement 사슬 → 세계 자리
    fn place(&mut self, id: Option<usize>, depth: usize) -> Place {
        let Some(id) = id else { return Place::ID };
        if let Some(p) = self.places.get(&id) {
            return *p;
        }
        let p = match self.st.get(id) {
            Some(("IFCLOCALPLACEMENT", a)) if depth < 64 => {
                let rel = a.first().and_then(V::r);
                let axis = a.get(1).and_then(V::r);
                let parent = self.place(rel, depth + 1);
                parent.then(&self.axis(axis))
            }
            _ => Place::ID,
        };
        self.places.insert(id, p);
        p
    }

    /// 객체(IfcProduct) 의 세계 자리
    fn product_place(&mut self, args: &[V]) -> Place {
        self.place(args.get(5).and_then(V::r), 0)
    }

    /// 표현(IfcProductDefinitionShape) 에서 (표현 이름, 항목) 들
    fn reps(&self, args: &[V]) -> Vec<(String, usize)> {
        let mut out = Vec::new();
        let Some(pds) = args.get(6).and_then(V::r) else {
            return out;
        };
        for r in self.st.arg(pds, 2).map(V::list).unwrap_or(&[]) {
            let Some(rid) = r.r() else { continue };
            let name = self.st.arg(rid, 1).and_then(V::s).unwrap_or("").to_string();
            for it in self.st.arg(rid, 3).map(V::list).unwrap_or(&[]) {
                if let Some(i) = it.r() {
                    out.push((name.clone(), i));
                }
            }
        }
        out
    }

    /// 몸체 항목 → (바닥 다각형(놓인 자리 안 좌표), 높이). 원은 다각형으로
    fn footprint(&self, item: usize, depth: usize) -> Option<(Vec<Vec2>, f64)> {
        if depth > 16 {
            return None;
        }
        let (n, a) = self.st.get(item)?;
        match n {
            "IFCEXTRUDEDAREASOLID" => {
                let pos = self.axis(a.get(1).and_then(V::r));
                let h = self.len(a.get(3)).unwrap_or(0.0).abs();
                let ring = self.profile(a.first()?.r()?)?;
                Some((ring.into_iter().map(|p| pos.at(p)).collect(), h))
            }
            // 잘라 낸 몸체(Revit 벽 윗면 등)는 처음 몸체로
            "IFCBOOLEANCLIPPINGRESULT" | "IFCBOOLEANRESULT" => {
                self.footprint(a.get(1)?.r()?, depth + 1)
            }
            "IFCMAPPEDITEM" => {
                let map = a.first()?.r()?;
                let origin = self.axis(self.st.arg(map, 0).and_then(V::r));
                let rep = self.st.arg(map, 1)?.r()?;
                for it in self.st.arg(rep, 3)?.list() {
                    if let Some(f) = it.r().and_then(|i| self.footprint(i, depth + 1)) {
                        return Some((f.0.into_iter().map(|p| origin.at(p)).collect(), f.1));
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// 단면 → 다각형
    fn profile(&self, id: usize) -> Option<Vec<Vec2>> {
        let (n, a) = self.st.get(id)?;
        match n {
            "IFCRECTANGLEPROFILEDEF" | "IFCROUNDEDRECTANGLEPROFILEDEF" => {
                let pos = self.axis(a.get(2).and_then(V::r));
                let (w, d) = (self.len(a.get(3))? / 2.0, self.len(a.get(4))? / 2.0);
                Some(
                    [(-w, -d), (w, -d), (w, d), (-w, d)]
                        .iter()
                        .map(|&(x, y)| pos.at(Vec2::new(x, y)))
                        .collect(),
                )
            }
            "IFCCIRCLEPROFILEDEF" => {
                let pos = self.axis(a.get(2).and_then(V::r));
                let r = self.len(a.get(3))?;
                Some(
                    (0..24)
                        .map(|i| {
                            let t = i as f64 * std::f64::consts::TAU / 24.0;
                            pos.at(Vec2::new(r * t.cos(), r * t.sin()))
                        })
                        .collect(),
                )
            }
            "IFCARBITRARYCLOSEDPROFILEDEF" | "IFCARBITRARYPROFILEDEFWITHVOIDS" => {
                self.curve(a.get(2)?.r()?)
            }
            _ => None,
        }
    }

    /// 다각 곡선 꼭짓점 (닫는 점은 뺀다)
    fn curve(&self, id: usize) -> Option<Vec<Vec2>> {
        let (n, a) = self.st.get(id)?;
        let mut pts: Vec<Vec2> = match n {
            "IFCPOLYLINE" => a
                .first()?
                .list()
                .iter()
                .filter_map(|p| p.r().and_then(|i| self.st.point(i)))
                .map(|p| Vec2::new(p[0] * self.k, p[1] * self.k))
                .collect(),
            "IFCINDEXEDPOLYCURVE" => {
                let (_, pl) = self.st.get(a.first()?.r()?)?;
                pl.first()?
                    .list()
                    .iter()
                    .filter_map(|p| {
                        let c: Vec<f64> = p.list().iter().filter_map(V::n).collect();
                        Some(Vec2::new(*c.first()? * self.k, *c.get(1)? * self.k))
                    })
                    .collect()
            }
            "IFCCOMPOSITECURVE" => {
                let mut out = Vec::new();
                for seg in a.first()?.list() {
                    let c = self.st.arg(seg.r()?, 2)?.r()?;
                    out.extend(self.curve(c).unwrap_or_default());
                }
                out
            }
            "IFCTRIMMEDCURVE" => self.curve(a.first()?.r()?)?,
            _ => return None,
        };
        if pts.len() > 1 && pts.first().unwrap().dist(*pts.last().unwrap()) < 1e-6 {
            pts.pop();
        }
        (pts.len() >= 2).then_some(pts)
    }

    /// 객체의 세계 바닥 모양과 높이 ('Body' 표현 먼저)
    fn body(&mut self, args: &[V]) -> Option<(Vec<Vec2>, f64, Place)> {
        let pl = self.product_place(args);
        let reps = self.reps(args);
        let mut order: Vec<&(String, usize)> = reps
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case("Body"))
            .collect();
        order.extend(reps.iter().filter(|(n, _)| !n.eq_ignore_ascii_case("Body")));
        for (_, it) in order {
            if let Some((ring, h)) = self.footprint(*it, 0) {
                return Some((ring.into_iter().map(|p| pl.at(p)).collect(), h, pl));
            }
        }
        None
    }

    /// 'Axis' 표현의 두 끝 (세계)
    fn axis_line(&mut self, args: &[V]) -> Option<(Vec2, Vec2)> {
        let pl = self.product_place(args);
        for (n, it) in self.reps(args) {
            if n.eq_ignore_ascii_case("Axis")
                && let Some(p) = self.curve(it)
            {
                return Some((pl.at(p[0]), pl.at(*p.last()?)));
            }
        }
        None
    }
}

/// 가장 긴 변 방향에 맞춘 둘레 상자: (가운데, 방향, 길이, 너비)
fn oriented(ring: &[Vec2]) -> Option<(Vec2, Vec2, f64, f64)> {
    let n = ring.len();
    let (mut best, mut u) = (0.0, Vec2::new(1.0, 0.0));
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        let l = a.dist(b);
        if l > best {
            best = l;
            u = Vec2::new((b.x - a.x) / l, (b.y - a.y) / l);
        }
    }
    if best < 1e-9 {
        return None;
    }
    let v = Vec2::new(-u.y, u.x);
    let dot = |p: Vec2, d: Vec2| p.x * d.x + p.y * d.y;
    let (mut s0, mut s1, mut t0, mut t1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for &p in ring {
        s0 = s0.min(dot(p, u));
        s1 = s1.max(dot(p, u));
        t0 = t0.min(dot(p, v));
        t1 = t1.max(dot(p, v));
    }
    let (sm, tm) = ((s0 + s1) / 2.0, (t0 + t1) / 2.0);
    Some((
        Vec2::new(u.x * sm + v.x * tm, u.y * sm + v.y * tm),
        u,
        s1 - s0,
        t1 - t0,
    ))
}

/// 벽 끝 맞추기: 한쪽 면 기준으로 그린 벽(다른 프로그램)은 중심선 끝이 서로 닿지 않아 모서리가 빈다.
/// 같은 층의 나란하지 않은 두 벽에서, 끝이 두 중심선의 교점에서 두 두께 안에 있고 교점이 상대 벽(두께/2 만큼 늘린)
/// 위에 있으면 끝을 교점으로 옮긴다 — 그러면 평면 생성이 L·T 맞댐으로 잇는다 (§47).
fn join_corners(walls: &mut [Wall]) {
    let n = walls.len();
    for i in 0..n {
        for j in 0..n {
            if i == j || walls[i].level != walls[j].level {
                continue;
            }
            let (p, q) = (walls[i].a, walls[i].b);
            let (c, d) = (walls[j].a, walls[j].b);
            let (r, s) = (
                Vec2::new(q.x - p.x, q.y - p.y),
                Vec2::new(d.x - c.x, d.y - c.y),
            );
            let den = r.x * s.y - r.y * s.x;
            let (lr, ls) = (p.dist(q), c.dist(d));
            if den.abs() < 1e-6 * lr * ls {
                continue;
            }
            let t = ((c.x - p.x) * s.y - (c.y - p.y) * s.x) / den;
            let x = Vec2::new(p.x + r.x * t, p.y + r.y * t);
            // 교점이 상대 벽 위(끝에서 두께/2 까지 늘림)
            let u = ((x.x - c.x) * s.x + (x.y - c.y) * s.y) / (ls * ls);
            let ext = walls[j].thickness / 2.0 / ls;
            if u < -ext || u > 1.0 + ext {
                continue;
            }
            let reach = walls[i].thickness + walls[j].thickness;
            for end in [0, 1] {
                let e = if end == 0 { walls[i].a } else { walls[i].b };
                let dd = e.dist(x);
                if dd > 1e-9 && dd <= reach {
                    // 벽이 뒤집힐 만큼 옮기지는 않는다
                    let other = if end == 0 { walls[i].b } else { walls[i].a };
                    if other.dist(x) < 1.0 {
                        continue;
                    }
                    if end == 0 {
                        walls[i].a = x;
                    } else {
                        walls[i].b = x;
                    }
                }
            }
        }
    }
}

/// 길이 단위 → mm 배율
fn unit_scale(st: &Step) -> f64 {
    for (_, a) in st.of(&["IFCSIUNIT"]) {
        if a.get(1).and_then(V::e) == Some("LENGTHUNIT") {
            return match a.get(2).and_then(V::e) {
                Some("MILLI") => 1.0,
                Some("CENTI") => 10.0,
                Some("DECI") => 100.0,
                Some("KILO") => 1e6,
                _ => 1000.0,
            };
        }
    }
    for (_, a) in st.of(&["IFCCONVERSIONBASEDUNIT"]) {
        if a.get(1).and_then(V::e) == Some("LENGTHUNIT") {
            match a.get(2).and_then(V::s).map(str::to_uppercase).as_deref() {
                Some("FOOT") => return 304.8,
                Some("INCH") => return 25.4,
                _ => {}
            }
        }
    }
    1000.0
}

/// IFC 파일 읽기 → 건축 객체가 든 새 도면
pub fn read(text: &str) -> Result<Drawing, String> {
    let st = Step::parse(text)?;
    let k = unit_scale(&st);
    let mut r = Reader {
        st,
        k,
        places: HashMap::new(),
    };
    let mut d = crate::archgen::new_drawing();
    let m = &mut d.arch;
    m.levels.clear();

    // 층 (높이 순)
    let mut storeys: Vec<(usize, String, f64)> = Vec::new();
    let ids: Vec<(usize, Vec<V>)> =
        r.st.of(&["IFCBUILDINGSTOREY"])
            .map(|(i, a)| (i, a.to_vec()))
            .collect();
    for (i, a) in &ids {
        let z = r.product_place(a).z;
        let elev = r.len(a.get(9)).unwrap_or(z);
        let name = a
            .get(2)
            .and_then(V::s)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("{}층", storeys.len() + 1));
        storeys.push((*i, name, elev));
    }
    storeys.sort_by(|a, b| a.2.total_cmp(&b.2));
    let mut level_of_storey: HashMap<usize, u64> = HashMap::new();
    for (n, (sid, name, elev)) in storeys.iter().enumerate() {
        let height = match storeys.get(n + 1) {
            Some(next) if next.2 - elev > 100.0 => next.2 - elev,
            _ if n > 0 => elev - storeys[n - 1].2,
            _ => 3000.0,
        };
        let id = m.new_id();
        m.levels.push(Level {
            id,
            name: name.clone(),
            elevation: *elev,
            height: height.max(100.0),
        });
        level_of_storey.insert(*sid, id);
    }
    if m.levels.is_empty() {
        let id = m.new_id();
        m.levels.push(Level {
            id,
            name: "1층".into(),
            elevation: 0.0,
            height: 3000.0,
        });
    }
    m.active_level = m.levels[0].id;

    // 객체 → 층: 담김 관계, 층에 모인 실, 실에 담긴 객체
    let mut parent: HashMap<usize, usize> = HashMap::new();
    for (_, a) in r.st.of(&["IFCRELCONTAINEDINSPATIALSTRUCTURE"]) {
        if let Some(s) = a.get(5).and_then(V::r) {
            for e in a.get(4).map(V::list).unwrap_or(&[]) {
                if let Some(e) = e.r() {
                    parent.insert(e, s);
                }
            }
        }
    }
    for (_, a) in r.st.of(&["IFCRELAGGREGATES"]) {
        if let Some(s) = a.get(4).and_then(V::r) {
            for e in a.get(5).map(V::list).unwrap_or(&[]) {
                if let Some(e) = e.r() {
                    parent.entry(e).or_insert(s);
                }
            }
        }
    }
    let levels: Vec<(u64, f64)> = m.levels.iter().map(|l| (l.id, l.elevation)).collect();
    let level_for = |r: &mut Reader, id: usize, args: &[V]| -> u64 {
        let mut cur = id;
        for _ in 0..8 {
            match parent.get(&cur) {
                Some(p) => {
                    if let Some(l) = level_of_storey.get(p) {
                        return *l;
                    }
                    cur = *p;
                }
                None => break,
            }
        }
        // 관계가 없으면 높이가 가장 가까운 아래층
        let z = r.product_place(args).z;
        levels
            .iter()
            .rev()
            .find(|(_, e)| *e <= z + 1.0)
            .or(levels.first())
            .map(|l| l.0)
            .unwrap_or(0)
    };
    let elev_of = |lv: u64| {
        levels
            .iter()
            .find(|l| l.0 == lv)
            .map(|l| l.1)
            .unwrap_or(0.0)
    };

    // 벽
    let mut wall_of: HashMap<usize, u64> = HashMap::new();
    let walls: Vec<(usize, Vec<V>)> =
        r.st.of(&[
            "IFCWALL",
            "IFCWALLSTANDARDCASE",
            "IFCWALLELEMENTEDCASE",
            "IFCCURTAINWALL",
        ])
        .map(|(i, a)| (i, a.to_vec()))
        .collect();
    for (i, a) in &walls {
        let body = r.body(a);
        let axis = r.axis_line(a);
        let ob = body.as_ref().and_then(|b| oriented(&b.0));
        let (pa, pb, thick) = match (axis, ob) {
            (Some((p, q)), ob) => {
                // 두께: 바닥 모양을 축에 수직으로 잰 폭
                let t = body
                    .as_ref()
                    .and_then(|b| {
                        let l = p.dist(q);
                        if l < 1e-9 {
                            return None;
                        }
                        let v = Vec2::new(-(q.y - p.y) / l, (q.x - p.x) / l);
                        let s: Vec<f64> =
                            b.0.iter()
                                .map(|c| (c.x - p.x) * v.x + (c.y - p.y) * v.y)
                                .collect();
                        let w = s.iter().cloned().fold(f64::MIN, f64::max)
                            - s.iter().cloned().fold(f64::MAX, f64::min);
                        (w > 1.0).then_some(w)
                    })
                    .or(ob.map(|o| o.3.min(o.2)));
                (p, q, t.unwrap_or(200.0))
            }
            (None, Some((c, u, l, w))) => {
                let (u, l, w) = if w > l {
                    (Vec2::new(-u.y, u.x), w, l)
                } else {
                    (u, l, w)
                };
                (
                    Vec2::new(c.x - u.x * l / 2.0, c.y - u.y * l / 2.0),
                    Vec2::new(c.x + u.x * l / 2.0, c.y + u.y * l / 2.0),
                    w,
                )
            }
            _ => continue,
        };
        if pa.dist(pb) < 1.0 {
            continue;
        }
        let level = level_for(&mut r, *i, a);
        let id = m.new_id();
        let name = a.get(2).and_then(V::s).unwrap_or("").to_string();
        let kind =
            if name.contains("내벽") || name.to_lowercase().contains("interior") || thick < 150.0
            {
                WallKind::Interior
            } else {
                WallKind::Exterior
            };
        m.walls.push(Wall {
            id,
            level,
            a: pa,
            b: pb,
            thickness: thick.max(1.0),
            height: body.as_ref().map(|b| b.1).filter(|h| *h > 1.0),
            kind,
        });
        wall_of.insert(*i, id);
    }

    join_corners(&mut m.walls);

    // 문·창: 구멍 → 벽, 채움 → 구멍
    let mut hole_wall: HashMap<usize, usize> = HashMap::new();
    for (_, a) in r.st.of(&["IFCRELVOIDSELEMENT"]) {
        if let (Some(w), Some(h)) = (a.get(4).and_then(V::r), a.get(5).and_then(V::r)) {
            hole_wall.insert(h, w);
        }
    }
    let mut fill_hole: HashMap<usize, usize> = HashMap::new();
    for (_, a) in r.st.of(&["IFCRELFILLSELEMENT"]) {
        if let (Some(h), Some(e)) = (a.get(4).and_then(V::r), a.get(5).and_then(V::r)) {
            fill_hole.insert(e, h);
        }
    }
    let fills: Vec<(usize, String, Vec<V>)> =
        r.st.of(&[
            "IFCDOOR",
            "IFCDOORSTANDARDCASE",
            "IFCWINDOW",
            "IFCWINDOWSTANDARDCASE",
        ])
        .map(|(i, a)| (i, r.st.get(i).unwrap().0.to_string(), a.to_vec()))
        .collect();
    for (i, kind, a) in &fills {
        let door = kind.starts_with("IFCDOOR");
        let hole = fill_hole.get(i).copied();
        let hole_args: Option<Vec<V>> = hole.and_then(|h| r.st.get(h).map(|(_, a)| a.to_vec()));
        let Some(wid) = hole
            .and_then(|h| hole_wall.get(&h))
            .and_then(|w| wall_of.get(w))
            .copied()
        else {
            continue;
        };
        let wall = m.walls.iter().find(|w| w.id == wid).cloned().unwrap();
        // 가운데·폭: 자기 몸체 → 구멍 몸체 → 놓인 점
        let shape = r
            .body(a)
            .or_else(|| hole_args.as_ref().and_then(|h| r.body(h)));
        let pl = r.product_place(a);
        let u = wall.dir();
        let (center, along) = match &shape {
            Some((ring, ..)) => {
                let s: Vec<f64> = ring
                    .iter()
                    .map(|p| (p.x - wall.a.x) * u.x + (p.y - wall.a.y) * u.y)
                    .collect();
                let (lo, hi) = (
                    s.iter().cloned().fold(f64::MAX, f64::min),
                    s.iter().cloned().fold(f64::MIN, f64::max),
                );
                ((lo + hi) / 2.0, hi - lo)
            }
            None => ((pl.o.x - wall.a.x) * u.x + (pl.o.y - wall.a.y) * u.y, 0.0),
        };
        let width = r
            .len(a.get(9))
            .filter(|w| *w > 1.0)
            .unwrap_or(if along > 1.0 {
                along
            } else if door {
                900.0
            } else {
                1200.0
            });
        let height = r
            .len(a.get(8))
            .filter(|h| *h > 1.0)
            .or(shape.as_ref().map(|s| s.1).filter(|h| *h > 1.0))
            .unwrap_or(if door { 2100.0 } else { 1200.0 });
        let lv_elev = elev_of(wall.level);
        let z = hole_args
            .as_ref()
            .map(|h| r.product_place(h).z)
            .unwrap_or(pl.z)
            .min(pl.z);
        let sill = if door { 0.0 } else { (z - lv_elev).max(0.0) };
        let len = wall.length();
        let offset = center.clamp(width / 2.0, (len - width / 2.0).max(width / 2.0));
        let id = m.new_id();
        m.openings.push(Opening {
            id,
            wall: wid,
            kind: if door {
                OpeningKind::Door
            } else {
                OpeningKind::Window
            },
            offset,
            width,
            height,
            sill: if door {
                0.0
            } else if sill > 1.0 {
                sill
            } else {
                900.0
            },
            flip: false,
            hinge_end: false,
            mark: String::new(),
        });
    }

    // 실
    let spaces: Vec<(usize, Vec<V>)> =
        r.st.of(&["IFCSPACE"])
            .map(|(i, a)| (i, a.to_vec()))
            .collect();
    for (i, a) in &spaces {
        let Some((ring, ..)) = r.body(a) else {
            continue;
        };
        if ring.len() < 3 {
            continue;
        }
        let long = a
            .get(7)
            .and_then(V::s)
            .filter(|s| !s.trim().is_empty() && !s.ends_with(" m2"));
        let name = long.or(a.get(2).and_then(V::s)).unwrap_or("실").to_string();
        let level = level_for(&mut r, *i, a);
        let id = m.new_id();
        m.rooms.push(Room {
            id,
            level,
            name,
            boundary: ring,
            finish_floor: String::new(),
            finish_wall: String::new(),
            finish_ceiling: String::new(),
        });
    }

    // 기둥
    let cols: Vec<(usize, Vec<V>)> =
        r.st.of(&["IFCCOLUMN", "IFCCOLUMNSTANDARDCASE"])
            .map(|(i, a)| (i, a.to_vec()))
            .collect();
    for (i, a) in &cols {
        let Some((ring, ..)) = r.body(a) else {
            continue;
        };
        // 원기둥(꼭짓점이 모두 가운데에서 같은 거리)은 지름 정사각으로 (사각 기둥만 있으므로)
        let n = ring.len() as f64;
        let mid = ring.iter().fold(Vec2::default(), |s, p| {
            Vec2::new(s.x + p.x / n, s.y + p.y / n)
        });
        let rs: Vec<f64> = ring.iter().map(|p| p.dist(mid)).collect();
        let rmax = rs.iter().cloned().fold(0.0, f64::max);
        let round = ring.len() >= 12 && rs.iter().all(|r| (r - rmax).abs() < rmax * 1e-6);
        let ob = if round {
            Some((mid, Vec2::new(1.0, 0.0), 2.0 * rmax, 2.0 * rmax))
        } else {
            oriented(&ring)
        };
        let Some((c, u, l, w)) = ob else {
            continue;
        };
        let level = level_for(&mut r, *i, a);
        let id = m.new_id();
        m.columns.push(Column {
            id,
            level,
            at: c,
            width: l,
            depth: w,
            angle: u.y.atan2(u.x),
        });
    }

    // 그리드: IfcGridAxis 의 선
    let grids: Vec<(usize, Vec<V>)> =
        r.st.of(&["IFCGRID"])
            .map(|(i, a)| (i, a.to_vec()))
            .collect();
    for (_, a) in &grids {
        let pl = r.product_place(a);
        for k in 7..=9 {
            for ax in a.get(k).map(V::list).unwrap_or(&[]).to_vec() {
                let Some(ax) = ax.r() else { continue };
                let label = r.st.arg(ax, 0).and_then(V::s).unwrap_or("").to_string();
                let Some(p) = r.st.arg(ax, 1).and_then(V::r).and_then(|c| r.curve(c)) else {
                    continue;
                };
                let id = m.new_id();
                m.grids.push(Grid {
                    id,
                    label,
                    a: pl.at(p[0]),
                    b: pl.at(*p.last().unwrap()),
                });
            }
        }
    }

    if m.walls.is_empty() && m.rooms.is_empty() && m.columns.is_empty() {
        return Err(
            "읽을 수 있는 벽·실·기둥이 없습니다 (평면 몸체가 밀어 올린 단면이 아니면 건너뜁니다)"
                .into(),
        );
    }
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_strings_and_values() {
        assert_eq!(unescape("\\X2\\BCBD\\X0\\"), "벽");
        assert_eq!(unescape("a''b"), "a''b");
        let st = Step::parse("ISO-10303-21;\nDATA;\n#1=IFCLABEL('it''s; ok');\n#2=IFCX(#1,$,.T.,(1.,-2.E3),IFCREAL(3.));\nENDSEC;").unwrap();
        assert_eq!(st.arg(1, 0), Some(&V::Str("it's; ok".into())));
        let (_, a) = st.get(2).unwrap();
        assert_eq!(a[1], V::Null);
        assert_eq!(a[3].list()[1], V::Num(-2000.0));
        assert_eq!(a[4].n(), Some(3.0));
    }

    /// 우리 IFC 를 다시 읽으면 벽·문·창·실·기둥·층이 같다
    #[test]
    fn round_trip_own_ifc() {
        let d = crate::demo::demo_building().unwrap();
        let text = crate::ifc::write(&d.arch, "왕복");
        let back = read(&text).unwrap();
        let (a, b) = (&d.arch, &back.arch);
        assert_eq!(b.levels.len(), a.levels.len());
        assert_eq!(
            b.levels.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
            a.levels.iter().map(|l| l.name.as_str()).collect::<Vec<_>>()
        );
        assert_eq!(
            b.walls.len(),
            a.walls.iter().filter(|w| w.length() > 1e-6).count()
        );
        assert_eq!(b.openings.len(), a.openings.len());
        assert_eq!(
            b.rooms.len(),
            a.rooms.iter().filter(|r| r.boundary.len() >= 3).count()
        );
        assert_eq!(b.columns.len(), a.columns.len());
        // 벽 끝·두께, 문 위치·폭이 맞다
        for (w, v) in a.walls.iter().zip(&b.walls) {
            assert!(w.a.dist(v.a) < 1e-3 && w.b.dist(v.b) < 1e-3, "{w:?} {v:?}");
            assert!((w.thickness - v.thickness).abs() < 1e-3);
        }
        // 쓰는 순서가 벽 순서라 (벽 번호, 위치) 로 맞춰 본다
        let key = |m: &crate::arch::ArchModel, o: &Opening| {
            (
                m.walls.iter().position(|w| w.id == o.wall),
                (o.offset * 1000.0).round() as i64,
            )
        };
        let mut oa: Vec<_> = a.openings.iter().collect();
        let mut ob: Vec<_> = b.openings.iter().collect();
        oa.sort_by_key(|o| key(a, o));
        ob.sort_by_key(|o| key(b, o));
        for (o, p) in oa.into_iter().zip(ob) {
            assert_eq!(o.kind, p.kind);
            assert!(
                (o.offset - p.offset).abs() < 1e-3 && (o.width - p.width).abs() < 1e-3,
                "{o:?} {p:?}"
            );
            assert!((o.sill - p.sill).abs() < 1e-3 || o.kind == OpeningKind::Door);
        }
        for (x, y) in a.rooms.iter().zip(&b.rooms) {
            assert_eq!(x.name, y.name);
            assert!((x.area() - y.area()).abs() < 1.0);
        }
        // 층 소속도 같다
        let lv = |m: &crate::arch::ArchModel, id: u64| m.levels.iter().position(|l| l.id == id);
        for (w, v) in a.walls.iter().zip(&b.walls) {
            assert_eq!(lv(a, w.level), lv(b, v.level));
        }
    }

    /// IfcOpenShell API 로 만든 파일 (testdata/make_ifcopenshell_house.py):
    /// 한쪽 면 기준 벽, 자리 원점이 문 끝인 문, LongName 실, 원기둥
    #[test]
    fn reads_ifcopenshell_file() {
        let d = read(include_str!("../testdata/ifcopenshell-house.ifc")).unwrap();
        let a = &d.arch;
        assert_eq!(
            a.levels
                .iter()
                .map(|l| (l.name.as_str(), l.elevation))
                .collect::<Vec<_>>(),
            [("1F", 0.0), ("2F", 3200.0)]
        );
        assert_eq!(a.walls.len(), 3);
        let w = &a.walls[1];
        assert!(
            w.a.dist(Vec2::new(5900.0, 100.0)) < 1e-6 && w.b.dist(Vec2::new(5900.0, 4000.0)) < 1e-6,
            "{w:?}"
        );
        // 한쪽 면 기준 벽끼리 모서리를 중심선 교점으로 맞춘다
        assert!(
            a.walls[0].b.dist(Vec2::new(5900.0, 100.0)) < 1e-6,
            "{:?}",
            a.walls[0]
        );
        assert_eq!(a.walls[2].level, a.levels[1].id);
        let o = &a.openings[0];
        assert_eq!((o.kind, o.wall), (OpeningKind::Door, a.walls[0].id));
        assert!(
            (o.offset - 2450.0).abs() < 1e-6 && (o.width - 900.0).abs() < 1e-6,
            "{o:?}"
        );
        assert_eq!(a.rooms[0].name, "거실");
        assert!((a.rooms[0].area() - 24e6).abs() < 1.0);
        let c = &a.columns[0];
        assert!(
            c.at.dist(Vec2::new(3000.0, 2000.0)) < 1e-6 && (c.width - 500.0).abs() < 1e-6,
            "{c:?}"
        );
    }

    /// 미터 단위 + 축선 없는 벽 (바닥 사각형만)
    #[test]
    fn metres_and_body_only_wall() {
        let text = "ISO-10303-21;HEADER;ENDSEC;DATA;
#1=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);
#2=IFCCARTESIANPOINT((0.,0.,0.));
#3=IFCAXIS2PLACEMENT3D(#2,$,$);
#4=IFCLOCALPLACEMENT($,#3);
#5=IFCBUILDINGSTOREY('a',$,'1F',$,$,#4,$,$,.ELEMENT.,0.);
#6=IFCCARTESIANPOINT((2.5,0.1));
#7=IFCAXIS2PLACEMENT2D(#6,$);
#8=IFCRECTANGLEPROFILEDEF(.AREA.,$,#7,5.,0.2);
#9=IFCDIRECTION((0.,0.,1.));
#10=IFCEXTRUDEDAREASOLID(#8,#3,#9,2.7);
#11=IFCSHAPEREPRESENTATION($,'Body','SweptSolid',(#10));
#12=IFCPRODUCTDEFINITIONSHAPE($,$,(#11));
#13=IFCCARTESIANPOINT((10.,0.,0.));
#14=IFCDIRECTION((0.,1.,0.));
#15=IFCAXIS2PLACEMENT3D(#13,$,#14);
#16=IFCLOCALPLACEMENT(#4,#15);
#17=IFCWALL('b',$,'W',$,$,#16,#12,$,$);
#18=IFCRELCONTAINEDINSPATIALSTRUCTURE('c',$,$,$,(#17),#5);
ENDSEC;END-ISO-10303-21;";
        let d = read(text).unwrap();
        let w = &d.arch.walls[0];
        assert!((w.length() - 5000.0).abs() < 1e-6);
        assert!((w.thickness - 200.0).abs() < 1e-6);
        // y 축 방향으로 돈 자리: (10000,0) 에서 위로
        let lo = if w.a.y < w.b.y { w.a } else { w.b };
        assert!(lo.dist(Vec2::new(10000.0 - 100.0, 0.0)) < 1e-6, "{w:?}");
        assert_eq!(w.height, Some(2700.0));
    }
}
