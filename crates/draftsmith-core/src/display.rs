//! 화면용 표시 데이터.
//!
//! 도면을 선분·삼각형 목록으로 펼쳐 바이너리 하나로 만든다. 앱은 이것을 Tauri 명령의 바이너리 응답으로
//! 화면에 넘기고, 화면은 그대로 WebGL2 버퍼에 올린다 (JSON 변환 없음 — 구현 노트 §4).
//! 문자는 따로 [`texts`] 가 만든다 (화면이 글꼴로 그림).
//!
//! 좌표는 도면 중심(`origin`)을 뺀 f32 로 넣는다. 도면 좌표가 TM 좌표처럼 커도 GPU 의
//! f32 정밀도에서 떨리지 않게 하기 위해서다 (원점 재설정).
//!
//! ## 선·채움 형식 `DSD3` (리틀 엔디언)
//! ```text
//! 0   [u8; 4]  "DSD3"
//! 4   u32      선분 개수 n
//! 8   u32      삼각형 개수 m
//! 12  f64 × 2  origin x, y            (도면 좌표)
//! 28  f32 × 4  범위 min x, min y, max x, max y (origin 기준)
//! 44  f32 × 4n 선분 x0, y0, x1, y1
//! ..  u8 × 4n  선분 색 r, g, b, a
//! ..  f32 × 6m 삼각형 x0, y0, x1, y1, x2, y2
//! ..  u8 × 4m  삼각형 색
//! ..  u8 × n   선분 굵기 (1/100 mm, 0 = 기본, 211 이 최대) — 화면 선 굵기 보기용 (구현 노트 §15)
//! ```
//!
//! ## 문자 형식 `DST1`
//! ```text
//! 0   [u8; 4]  "DST1"
//! 4   u32      문자 개수 k
//! 8   f64 × 2  origin (선·채움과 같음)
//! 24  문자 k 개, 각각:
//!     f32 x, y, 높이, 방향, 폭 배율, 가로 맞춤(0·0.5·1)   (24 바이트)
//!     u8  세로 맞춤 (0 바탕선, 1 아래, 2 가운데, 3 위), r, g, b
//!     u32 글자 바이트 수 → UTF-8 (`\n` 은 줄바꿈)
//! ```

use std::f64::consts::TAU;

use crate::curve;
use crate::drawing::{Drawing, Entity, Shape, VAlign};
use crate::fill;
use crate::geom::{Bbox, Vec2};

pub const MAGIC: &[u8; 4] = b"DSD3";
pub const HEADER_LEN: usize = 44;
pub const TEXT_MAGIC: &[u8; 4] = b"DST1";

/// 펼친 선분·삼각형
#[derive(Debug, Default)]
pub struct Segments {
    pub origin: Vec2,
    pub bbox: Bbox,
    /// x0, y0, x1, y1 (origin 기준)
    pub coords: Vec<f32>,
    /// r, g, b, a
    pub colors: Vec<u8>,
    /// 삼각형 x0, y0, x1, y1, x2, y2 (origin 기준)
    pub tris: Vec<f32>,
    pub tri_colors: Vec<u8>,
    /// 선분 굵기 (1/100 mm, 0 = 기본) — 바이너리에는 넣지 않음 (PDF 용)
    pub weights: Vec<u16>,
    /// 지금 넣는 객체의 굵기
    cur_w: u16,
    /// 선분·삼각형 색 번호 (ACI, 0 = 없음) — 바이너리에는 넣지 않음 (PDF 출력 스타일용)
    pub acis: Vec<u8>,
    pub tri_acis: Vec<u8>,
    cur_aci: u8,
}

impl Segments {
    /// 선분 수
    pub fn len(&self) -> usize {
        self.coords.len() / 4
    }

    pub fn is_empty(&self) -> bool {
        self.coords.is_empty() && self.tris.is_empty()
    }

    pub fn tri_count(&self) -> usize {
        self.tris.len() / 6
    }

    fn push(&mut self, a: Vec2, b: Vec2, c: [u8; 4]) {
        let o = self.origin;
        self.coords.extend_from_slice(&[
            (a.x - o.x) as f32,
            (a.y - o.y) as f32,
            (b.x - o.x) as f32,
            (b.y - o.y) as f32,
        ]);
        self.colors.extend_from_slice(&c);
        self.weights.push(self.cur_w);
        self.acis.push(self.cur_aci);
    }

    fn strip(&mut self, pts: &[Vec2], closed: bool, c: [u8; 4]) {
        for w in pts.windows(2) {
            self.push(w[0], w[1], c);
        }
        if closed && pts.len() > 2 {
            self.push(pts[pts.len() - 1], pts[0], c);
        }
    }

    fn tri(&mut self, t: &[Vec2], c: [u8; 4]) {
        let o = self.origin;
        for p in t {
            self.tris.push((p.x - o.x) as f32);
            self.tris.push((p.y - o.y) as f32);
        }
        self.tri_colors.extend_from_slice(&c);
        self.tri_acis.push(self.cur_aci);
    }

    /// 형식에 맞춘 바이너리
    pub fn to_bytes(&self) -> Vec<u8> {
        let (n, m) = (self.len(), self.tri_count());
        let mut out = Vec::with_capacity(HEADER_LEN + n * 21 + m * 28);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(n as u32).to_le_bytes());
        out.extend_from_slice(&(m as u32).to_le_bytes());
        out.extend_from_slice(&self.origin.x.to_le_bytes());
        out.extend_from_slice(&self.origin.y.to_le_bytes());
        let o = self.origin;
        let (lo, hi) = if self.bbox.is_empty() {
            (o, o)
        } else {
            (self.bbox.min, self.bbox.max)
        };
        for v in [lo.x - o.x, lo.y - o.y, hi.x - o.x, hi.y - o.y] {
            out.extend_from_slice(&(v as f32).to_le_bytes());
        }
        for v in &self.coords {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&self.colors);
        for v in &self.tris {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&self.tri_colors);
        out.extend(self.weights.iter().map(|&w| w.min(255) as u8));
        out
    }
}

/// 도면을 선분·삼각형으로 펼친다. 꺼진 레이어는 뺀다. 문자는 넣지 않는다 ([`texts`]).
/// 원호 허용 오차는 도면 크기의 1/20000 (화면 맞춤 상태에서 눈에 띄지 않는 정도).
pub fn segments(d: &Drawing) -> Segments {
    let bbox = d.bbox();
    let diag = bbox.width().hypot(bbox.height()).max(1e-9);
    let tol = diag / 20_000.0;
    let mut s = Segments {
        origin: bbox.center(),
        bbox,
        ..Default::default()
    };
    // 선 종류 무늬를 적용할 조각들 (조각마다 무늬를 처음부터 — AutoCAD 기본과 같음)
    let mut pieces: Vec<Vec<Vec2>> = Vec::new();

    for e in &d.entities {
        emit(d, e, tol, &mut s, &mut pieces);
    }
    s
}

/// 객체 하나를 선분·삼각형으로 (§66 — 캐시와 함께 쓰려고 떼어 냄)
fn emit(d: &Drawing, e: &Entity, tol: f64, s: &mut Segments, pieces: &mut Vec<Vec<Vec2>>) {
    if d.layers.get(e.layer as usize).is_some_and(|l| !l.visible) {
        return;
    }
    let [r, g, b] = d.color_of(e);
    let col = [r, g, b, 255];
    s.cur_w = e.weight.unwrap_or(0);
    s.cur_aci = e.aci.unwrap_or(0);
    let dash = e.linetype.and_then(|l| {
        d.linetypes
            .get(l.pattern as usize)
            .map(|p| (p.dashes.as_slice(), l.scale))
    });
    pieces.clear();
    match &e.shape {
        Shape::Wide {
            points,
            closed,
            widths,
        } => {
            for q in crate::drawing::wide_quads(points, *closed, widths, tol) {
                s.tri(&[q[0], q[1], q[2]], col);
                s.tri(&[q[0], q[2], q[3]], col);
            }
            return;
        }
        Shape::Wipeout { .. } => return,
        Shape::Hatch { .. } => return,
        Shape::Insert { .. } => return,
        Shape::Line { .. }
        | Shape::Polyline { .. }
        | Shape::Circle { .. }
        | Shape::Arc { .. }
        | Shape::Ellipse { .. } => line_chains(&e.shape, dash.is_some(), tol, pieces),
        Shape::Fill { loops } => {
            let t = fill::triangulate(loops);
            for tri in t.as_chunks::<3>().0 {
                s.tri(tri, col);
            }
        }
        Shape::Segments { lines } => {
            for (a, b) in lines {
                s.push(*a, *b, col);
            }
        }
        Shape::Text(_) => {}
    }
    for piece in pieces.iter() {
        match dash {
            Some((dashes, scale)) => dash_chain(s, piece, dashes, scale, tol, col),
            None => s.strip(piece, false, col),
        }
    }
}

/// 객체별 표시 데이터 캐시 (§66): 편집 뒤 바뀌지 않은 객체는 전에 펼친 것을 그대로 붙인다.
/// 도면 범위(원점·허용 오차가 범위에서 나온다)·레이어·선 종류가 바뀌면 모두 다시.
/// 앞뒤 같은 객체를 맞춰(§73) 가운데 객체를 지우거나 넣어 뒤 번호가 밀려도 뒤 객체는 다시 펼치지 않는다.
#[derive(Default)]
pub struct SegCache {
    key: Option<(
        Bbox,
        Vec<crate::drawing::Layer>,
        Vec<crate::drawing::LinePattern>,
    )>,
    /// 객체마다 객체·펼친 것·범위 (따로 두어 범위만 훑을 때 메모리를 덜 읽는다)
    ents: Vec<Entity>,
    segs: Vec<Segments>,
    bbs: Vec<Bbox>,
    /// 마지막에 다시 펼친 객체 수 (측정용)
    pub rebuilt: usize,
    /// 마지막 결과에서 객체마다 (선분 시작, 선분 수, 삼각형 시작, 삼각형 수)
    layout: Vec<(usize, usize, usize, usize)>,
    /// 바로 앞 결과와 비교해 부분 갱신이 되면 바뀐 객체 번호들 (§68)
    patch: Option<Vec<usize>>,
}

impl SegCache {
    /// 마지막 갱신의 원점 (도면 범위 가운데). 문자 데이터가 같은 원점을 쓴다 (§74)
    pub fn origin(&self) -> Option<Vec2> {
        self.key.as_ref().map(|k| k.0.center())
    }

    /// 바로 앞 갱신을 화면에 부분 갱신으로 보낼 수 있으면 그 바이너리 (`DSP1`). 바뀐 객체의 선분은 캐시에서 읽는다.
    /// 형식: 매직 · 범위 수 u32 · 범위마다 (선분 시작·수·삼각형 시작·수 u32 ×4, 좌표 f32, 색 u8×4, 굵기 u8, 삼각형 f32, 색 u8×4)
    pub fn patch_bytes(&self) -> Option<Vec<u8>> {
        let ids = self.patch.as_ref()?;
        let mut out = Vec::new();
        out.extend_from_slice(b"DSP1");
        out.extend_from_slice(&(ids.len() as u32).to_le_bytes());
        for &i in ids {
            let (lo, ln, to, tn) = self.layout[i];
            for v in [lo, ln, to, tn] {
                out.extend_from_slice(&(v as u32).to_le_bytes());
            }
            let s = &self.segs[i];
            for v in &s.coords {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.extend_from_slice(&s.colors);
            out.extend(s.weights.iter().map(|&w| w.min(255) as u8));
            for v in &s.tris {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.extend_from_slice(&s.tri_colors);
        }
        Some(out)
    }
}

impl Segments {
    fn append(&mut self, p: &Segments) {
        self.coords.extend_from_slice(&p.coords);
        self.colors.extend_from_slice(&p.colors);
        self.tris.extend_from_slice(&p.tris);
        self.tri_colors.extend_from_slice(&p.tri_colors);
        self.weights.extend_from_slice(&p.weights);
        self.acis.extend_from_slice(&p.acis);
        self.tri_acis.extend_from_slice(&p.tri_acis);
    }
}

/// 캐시를 지금 도면에 맞춘다: 바뀐 객체만 다시 펼치고, 부분 갱신이 되는지 정한다. (원점, 범위)
fn refresh(d: &Drawing, cache: &mut SegCache) -> (Vec2, Bbox) {
    let new = &d.entities;
    let styled = cache
        .key
        .as_ref()
        .is_some_and(|(_, l, t)| *l == d.layers && *t == d.linetypes);
    // 앞뒤 같은 객체 맞추기 (§73)
    let old_n = cache.ents.len();
    let (mut pre, mut old_end, mut new_end) = (0, 0, new.len());
    if styled {
        let (p, suf) = crate::drawing::align(&cache.ents, new);
        pre = p;
        (old_end, new_end) = (old_n - suf, new.len() - suf);
    }
    // 범위: 맞춘 앞뒤는 기억한 범위로, 가운데만 새로 (전체를 다시 재면 실무 도면 8 ms)
    let mid_bb: Vec<Bbox> = new[pre..new_end].iter().map(|e| e.shape.bbox()).collect();
    let mut bbox = Bbox::EMPTY;
    if styled {
        for b in cache.bbs[..pre].iter().chain(&cache.bbs[old_end..]) {
            bbox.merge(b);
        }
    }
    mid_bb.iter().for_each(|b| bbox.merge(b));
    let same = styled && cache.key.as_ref().is_some_and(|(b, _, _)| *b == bbox);
    let mut mid_bb = mid_bb;
    if !same {
        if styled {
            // 범위가 바뀌어 모두 다시 — 앞뒤 객체 범위는 기억한 것으로
            let mut all: Vec<Bbox> = cache.bbs[..pre].to_vec();
            all.append(&mut mid_bb);
            all.extend_from_slice(&cache.bbs[old_end..]);
            mid_bb = all;
        }
        cache.ents.clear();
        cache.segs.clear();
        cache.bbs.clear();
        (pre, old_end, new_end) = (0, 0, new.len());
        cache.key = Some((bbox, d.layers.clone(), d.linetypes.clone()));
    }
    let diag = bbox.width().hypot(bbox.height()).max(1e-9);
    let tol = diag / 20_000.0;
    let origin = bbox.center();
    // 부분 갱신은 범위·레이어·객체 수가 같고, 다시 펼친 객체의 선분·삼각형 수가 그대로일 때만 (§68)
    let mut patch = (same && old_n == new.len() && cache.layout.len() == new.len()).then(Vec::new);
    let mut scratch: Vec<Vec<Vec2>> = Vec::new();
    // 가운데: 같은 자리의 옛 객체와 같으면 그대로, 아니면 그 자리에서 다시 펼친다.
    // 목록을 통째로 빼고 넣으면 큰 도면에서 항목 옮기기만 수십 ms 라, 객체 수가 달라진 만큼만 넣고 뺀다
    let (old_len, new_len) = (old_end - pre, new_end - pre);
    let common = old_len.min(new_len);
    let mut made = |i: usize, e: &Entity, bb: Bbox, rebuilt: &mut usize| {
        let mut p = Segments {
            origin,
            bbox,
            ..Default::default()
        };
        emit(d, e, tol, &mut p, &mut scratch);
        *rebuilt += 1;
        if let Some(pt) = patch.as_mut() {
            pt.push(i);
        }
        (e.clone(), p, bb)
    };
    let mut rebuilt = 0;
    let mut bbs = mid_bb.into_iter();
    for (k, bb) in bbs.by_ref().take(common).enumerate() {
        let i = pre + k;
        if cache.ents[i] != new[i] {
            let (e, p, b) = made(i, &new[i], bb, &mut rebuilt);
            (cache.ents[i], cache.segs[i], cache.bbs[i]) = (e, p, b);
        }
    }
    if new_len > old_len {
        let add: Vec<(Entity, Segments, Bbox)> = bbs
            .enumerate()
            .map(|(k, bb)| {
                let i = pre + common + k;
                made(i, &new[i], bb, &mut rebuilt)
            })
            .collect();
        let at = pre + common;
        let (mut es, mut ps, mut bs) = (Vec::new(), Vec::new(), Vec::new());
        for (e, p, b) in add {
            es.push(e);
            ps.push(p);
            bs.push(b);
        }
        cache.ents.splice(at..at, es);
        cache.segs.splice(at..at, ps);
        cache.bbs.splice(at..at, bs);
    } else if old_len > new_len {
        cache.ents.drain(pre + common..old_end);
        cache.segs.drain(pre + common..old_end);
        cache.bbs.drain(pre + common..old_end);
    }
    cache.rebuilt = rebuilt;
    // 부분 갱신은 다시 펼친 객체의 선분·삼각형 수가 그대로일 때만 (그러면 모든 객체 자리가 그대로)
    if let Some(p) = &patch
        && p.iter().any(|&i| {
            cache
                .layout
                .get(i)
                .is_none_or(|l| (l.1, l.3) != (cache.segs[i].len(), cache.segs[i].tri_count()))
        })
    {
        patch = None;
    }
    cache.patch = patch;
    (origin, bbox)
}

/// 캐시의 객체별 결과를 이어 붙인 표시 데이터 (객체마다 자리도 기억)
fn assemble(cache: &mut SegCache, origin: Vec2, bbox: Bbox) -> Segments {
    // 크기를 먼저 세어 한 번에 잡는다
    let (nl, nt) = cache
        .segs
        .iter()
        .fold((0, 0), |(l, t), c| (l + c.len(), t + c.tri_count()));
    let mut s = Segments {
        origin,
        bbox,
        coords: Vec::with_capacity(nl * 4),
        colors: Vec::with_capacity(nl * 4),
        weights: Vec::with_capacity(nl),
        acis: Vec::with_capacity(nl),
        tris: Vec::with_capacity(nt * 6),
        tri_colors: Vec::with_capacity(nt * 4),
        tri_acis: Vec::with_capacity(nt),
        ..Default::default()
    };
    let mut layout = Vec::with_capacity(cache.segs.len());
    for c in &cache.segs {
        let (ln, tn) = (s.len(), s.tri_count());
        s.append(c);
        layout.push((ln, s.len() - ln, tn, s.tri_count() - tn));
    }
    cache.layout = layout;
    s
}

/// [`segments`] 와 같은 결과를, 바뀌지 않은 객체는 캐시에서
pub fn segments_cached(d: &Drawing, cache: &mut SegCache) -> Segments {
    let (origin, bbox) = refresh(d, cache);
    assemble(cache, origin, bbox)
}

/// 편집 뒤 화면에 보낼 바이너리: 부분 갱신(`DSP1`)이 되면 바뀐 객체 것만 (전체를 이어 붙이지 않는다, §73),
/// 아니면 전체(`DSD3`). 화면이 바로 앞 결과를 들고 있을 때만 쓴다
pub fn update_bytes(d: &Drawing, cache: &mut SegCache) -> Vec<u8> {
    let (origin, bbox) = refresh(d, cache);
    match cache.patch_bytes() {
        Some(b) => b,
        None => assemble(cache, origin, bbox).to_bytes(),
    }
}

/// 한 조각에 들어갈 수 있는 무늬 반복 수. 넘으면 (화면에서 어차피 실선처럼 보이므로) 실선으로 그린다.
pub const MAX_DASH_REPEATS: f64 = 2_000.0;

/// 꺾은선을 따라 대시 무늬를 그린다. 무늬는 꺾은선 처음에서 시작하고, 꺾인 곳을 지나는 대시는 꺾인 채로 이어진다.
fn dash_chain(s: &mut Segments, pts: &[Vec2], dashes: &[f64], scale: f64, tol: f64, col: [u8; 4]) {
    if !dash_lines(pts, dashes, scale, tol, &mut |a, b| s.push(a, b, col)) {
        s.strip(pts, false, col);
    }
}

/// 선·곡선 모양 → 그릴 꺾은선들. 점선이면 폴리선은 꼭짓점마다 따로 (무늬를 새로 시작). 가리개 자르기도 쓴다 (§75)
pub(crate) fn line_chains(shape: &Shape, dashed: bool, tol: f64, pieces: &mut Vec<Vec<Vec2>>) {
    match shape {
        Shape::Line { a, b } => pieces.push(vec![*a, *b]),
        Shape::Polyline { points, closed } => {
            if dashed {
                // 꼭짓점마다 무늬를 새로 시작
                let n = points.len();
                let last = if *closed { n } else { n.saturating_sub(1) };
                for i in 0..last {
                    let (p, g) = points[i];
                    let q = points[(i + 1) % n].0;
                    let mut piece = Vec::new();
                    curve::bulge_points(p, q, g, tol, &mut piece);
                    piece.push(q);
                    pieces.push(piece);
                }
            } else {
                let mut pts = curve::polyline_points(points, *closed, tol);
                if *closed && pts.len() > 2 {
                    pts.push(pts[0]);
                }
                pieces.push(pts);
            }
        }
        Shape::Circle { center, radius } => {
            let mut pts = Vec::new();
            let (u, v) = (Vec2::new(*radius, 0.0), Vec2::new(0.0, *radius));
            curve::ellipse_points(*center, u, v, 0.0, TAU, tol, &mut pts);
            pieces.push(pts);
        }
        Shape::Arc {
            center,
            radius,
            start,
            end,
        } => {
            let mut pts = Vec::new();
            let (u, v) = (Vec2::new(*radius, 0.0), Vec2::new(0.0, *radius));
            let sweep = curve::ccw_sweep(*start, *end);
            curve::ellipse_points(*center, u, v, *start, sweep, tol, &mut pts);
            pieces.push(pts);
        }
        Shape::Ellipse {
            center,
            u,
            v,
            start,
            end,
        } => {
            let mut pts = Vec::new();
            let sweep = curve::ccw_sweep(*start, *end);
            curve::ellipse_points(*center, *u, *v, *start, sweep, tol, &mut pts);
            pieces.push(pts);
        }
        _ => {}
    }
}

/// 꺾은선 하나를 무늬대로 끊은 선분들을 `push` 로. 무늬가 너무 잘거나(허용 오차의 두 배 이하) 반복이 너무 많으면
/// 끊지 않고 거짓 (그러면 실선으로)
pub(crate) fn dash_lines(
    pts: &[Vec2],
    dashes: &[f64],
    scale: f64,
    tol: f64,
    push: &mut dyn FnMut(Vec2, Vec2),
) -> bool {
    let period: f64 = dashes.iter().map(|x| x.abs()).sum::<f64>() * scale;
    let len: f64 = pts.windows(2).map(|w| w[0].dist(w[1])).sum();
    if pts.len() < 2 || period <= tol * 2.0 || len / period > MAX_DASH_REPEATS {
        return false;
    }
    let dot = (tol * 0.5).max(period * 1e-3);
    // 무늬 상태: 지금 요소 번호, 남은 길이, 선인지
    let mut di = 0usize;
    let mut remain = dashes[0].abs() * scale;
    let mut on = dashes[0] >= 0.0;
    for w in pts.windows(2) {
        let (p, q) = (w[0], w[1]);
        let l = p.dist(q);
        if l <= 0.0 {
            continue;
        }
        let at = |t: f64| p.lerp(q, t / l);
        let mut t = 0.0;
        while t < l {
            if remain <= 0.0 {
                // 점(길이 0): 진행 방향으로 아주 짧게 찍고 다음 요소로
                if on {
                    push(at(t), at((t + dot).min(l)));
                }
            } else {
                let step = remain.min(l - t);
                if on {
                    push(at(t), at(t + step));
                }
                t += step;
                remain -= step;
                if remain > 1e-12 {
                    continue; // 이 선분이 끝났다 — 다음 선분에서 이어서
                }
            }
            di = (di + 1) % dashes.len();
            remain = dashes[di].abs() * scale;
            on = dashes[di] >= 0.0;
        }
    }
    true
}

/// 문자 표시 데이터 (`DST1`). 꺼진 레이어는 뺀다. origin 은 [`segments`] 와 같다.
pub fn texts(d: &Drawing) -> Vec<u8> {
    texts_at(d, d.bbox().center())
}

/// [`texts`] 를 이미 아는 원점으로 (도면 범위를 다시 재지 않으려고 — 실무 도면 7 ms, §74)
pub fn texts_at(d: &Drawing, origin: Vec2) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(TEXT_MAGIC);
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&origin.x.to_le_bytes());
    out.extend_from_slice(&origin.y.to_le_bytes());
    let mut k = 0u32;
    for e in &d.entities {
        let Shape::Text(t) = &e.shape else { continue };
        if t.value.trim().is_empty() || d.layers.get(e.layer as usize).is_some_and(|l| !l.visible) {
            continue;
        }
        let [r, g, b] = d.color_of(e);
        for v in [
            (t.at.x - origin.x) as f32,
            (t.at.y - origin.y) as f32,
            t.height as f32,
            t.rotation as f32,
            t.width_factor as f32,
            t.halign as f32,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        let va = match t.valign {
            VAlign::Baseline => 0u8,
            VAlign::Bottom => 1,
            VAlign::Middle => 2,
            VAlign::Top => 3,
        };
        out.extend_from_slice(&[va, r, g, b]);
        out.extend_from_slice(&(t.value.len() as u32).to_le_bytes());
        out.extend_from_slice(t.value.as_bytes());
        k += 1;
    }
    out[4..8].copy_from_slice(&k.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawing::{Drawing, Entity, Shape, TextShape};

    fn one(shape: Shape) -> Drawing {
        let mut d = Drawing::default();
        let layer = d.layer_index("0");
        d.entities.push(Entity {
            layer,
            aci: None,
            weight: None,
            linetype: None,
            color: None,
            shape,
        });
        d
    }

    #[test]
    fn line_is_relative_to_origin() {
        // TM 좌표처럼 큰 값도 origin 기준으로 작아진다
        let d = one(Shape::Line {
            a: Vec2::new(200_000.0, 550_000.0),
            b: Vec2::new(200_010.0, 550_000.0),
        });
        let s = segments(&d);
        assert_eq!(s.len(), 1);
        assert_eq!(s.origin, Vec2::new(200_005.0, 550_000.0));
        assert_eq!(&s.coords[..], &[-5.0, 0.0, 5.0, 0.0]);
    }

    #[test]
    fn bulge_half_circle_stays_on_radius() {
        // 불지 1 = 반원. (0,0)→(10,0), 중심 (5,0), 반지름 5
        let d = one(Shape::Polyline {
            points: vec![(Vec2::new(0.0, 0.0), 1.0), (Vec2::new(10.0, 0.0), 0.0)],
            closed: false,
        });
        let s = segments(&d);
        assert!(s.len() >= 8);
        let o = s.origin;
        for q in s.coords.chunks(2) {
            let p = Vec2::new(q[0] as f64 + o.x, q[1] as f64 + o.y);
            assert!((p.dist(Vec2::new(5.0, 0.0)) - 5.0).abs() < 1e-4, "{p:?}");
        }
        // 끝점까지 간다
        let last = &s.coords[s.coords.len() - 2..];
        assert!(((last[0] as f64 + o.x) - 10.0).abs() < 1e-4);
    }

    #[test]
    fn bytes_layout_with_fill_and_text() {
        let mut d = one(Shape::Circle {
            center: Vec2::new(0.0, 0.0),
            radius: 1.0,
        });
        d.entities.push(Entity {
            layer: 0,
            aci: None,
            weight: None,
            linetype: None,
            color: Some([1, 2, 3]),
            shape: Shape::Fill {
                loops: vec![vec![
                    Vec2::new(0.0, 0.0),
                    Vec2::new(1.0, 0.0),
                    Vec2::new(0.0, 1.0),
                ]],
            },
        });
        d.entities.push(Entity {
            layer: 0,
            aci: None,
            weight: None,
            linetype: None,
            color: None,
            shape: Shape::Text(TextShape {
                style: String::new(),
                at: Vec2::new(0.0, 0.0),
                height: 1.0,
                rotation: 0.0,
                width_factor: 1.0,
                halign: 0.0,
                valign: VAlign::Baseline,
                value: "거실".into(),
            }),
        });
        let s = segments(&d);
        let b = s.to_bytes();
        assert_eq!(&b[..4], MAGIC);
        let n = u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize;
        let m = u32::from_le_bytes(b[8..12].try_into().unwrap()) as usize;
        assert_eq!((n, m), (s.len(), 1));
        assert_eq!(b.len(), HEADER_LEN + n * 21 + m * 28);
        let t = texts(&d);
        assert_eq!(&t[..4], TEXT_MAGIC);
        assert_eq!(u32::from_le_bytes(t[4..8].try_into().unwrap()), 1);
        assert_eq!(&t[t.len() - 6..], "거실".as_bytes());
    }

    #[test]
    fn dashed_line_and_continuous_fallback() {
        use crate::drawing::{LinePattern, LineStyle};
        let mut d = one(Shape::Line {
            a: Vec2::new(0.0, 0.0),
            b: Vec2::new(100.0, 0.0),
        });
        // 대시 10, 빈칸 5 → 15 마다 하나: 0–10, 15–25, … 90–100 → 7 개
        d.linetypes.push(LinePattern {
            name: "DASHED".into(),
            dashes: vec![10.0, -5.0],
        });
        d.entities[0].linetype = Some(LineStyle {
            pattern: 0,
            scale: 1.0,
        });
        let s = segments(&d);
        assert_eq!(s.len(), 7, "{:?}", s.coords);
        let drawn: f64 = s.coords.chunks(4).map(|c| (c[2] - c[0]) as f64).sum();
        assert!((drawn - 70.0).abs() < 1e-3, "{drawn}");
        // 무늬가 너무 잘면 실선
        d.entities[0].linetype = Some(LineStyle {
            pattern: 0,
            scale: 1e-6,
        });
        assert_eq!(segments(&d).len(), 1);
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    fn same(a: &Segments, b: &Segments) -> bool {
        a.coords == b.coords
            && a.colors == b.colors
            && a.tris == b.tris
            && a.tri_colors == b.tri_colors
            && a.weights == b.weights
    }

    /// 캐시로 만든 표시 데이터가 늘 새로 만든 것과 같고, 바뀐 객체만 다시 펼친다 (§66)
    #[test]
    fn cached_matches_fresh() {
        let d0 = crate::archgen::shown(&crate::demo::demo_building().unwrap());
        let mut c = SegCache::default();
        assert!(same(&segments_cached(&d0, &mut c), &segments(&d0)));
        assert_eq!(c.rebuilt, d0.entities.len());
        // 그대로면 하나도 다시 안 펼침
        let _ = segments_cached(&d0, &mut c);
        assert_eq!(c.rebuilt, 0);
        // 범위 안에서 한 객체 색만 바꾸면 하나만
        let mut d1 = d0.clone();
        let i = d1
            .entities
            .iter()
            .position(|e| matches!(e.shape, Shape::Line { .. }))
            .unwrap();
        d1.entities[i].color = Some([1, 2, 3]);
        assert!(same(&segments_cached(&d1, &mut c), &segments(&d1)));
        assert_eq!(c.rebuilt, 1);
        // 레이어를 끄면 모두 다시
        let mut d2 = d1.clone();
        d2.layers[d2.entities[i].layer as usize].visible = false;
        assert!(same(&segments_cached(&d2, &mut c), &segments(&d2)));
        assert_eq!(c.rebuilt, d2.entities.len());
        // 부분 갱신 (§68): 색만 바꾼 객체 하나 → 범위 하나, 새로 다 펼친 뒤엔 없음
        assert!(c.patch_bytes().is_none());
        let mut d3 = d2.clone();
        d3.entities[i].color = Some([9, 9, 9]);
        let _ = segments_cached(&d2, &mut c);
        let p = update_bytes(&d3, &mut c);
        assert_eq!(&p[0..4], b"DSP1");
        let u = |k: usize| u32::from_le_bytes(p[k..k + 4].try_into().unwrap()) as usize;
        assert_eq!(u(4), 1);
        // 묶음의 선분이 새로 만든 전체의 그 자리와 같다
        let (lo, ln) = (u(8), u(12));
        let full = segments(&d3);
        let got: Vec<f32> = p[24..24 + ln * 16]
            .chunks(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        assert_eq!(got, full.coords[lo * 4..(lo + ln) * 4]);
        assert_eq!(
            p[24 + ln * 16..24 + ln * 20],
            full.colors[lo * 4..(lo + ln) * 4]
        );
        assert!(same(&segments_cached(&d3, &mut c), &full));
        // 객체가 늘면 부분 갱신 없음
        // 문자 데이터는 캐시 원점으로 만들어도 같다 (§74)
        assert_eq!(texts_at(&d3, c.origin().unwrap()), texts(&d3));
        let mut d4 = d3.clone();
        d4.entities.push(d4.entities[i].clone());
        assert!(same(&segments_cached(&d4, &mut c), &segments(&d4)));
        assert!(c.patch_bytes().is_none());
        // 가운데 지우기·넣기 (§73): 뒤 번호가 밀려도 뒤 객체는 다시 안 펼침
        let mut d5 = d4.clone();
        d5.entities.remove(i);
        assert!(same(&segments_cached(&d5, &mut c), &segments(&d5)));
        assert_eq!(c.rebuilt, 0);
        let mut d6 = d5.clone();
        d6.entities.insert(i, d4.entities[i].clone());
        d6.entities.insert(1, d4.entities[i].clone());
        assert!(same(&segments_cached(&d6, &mut c), &segments(&d6)));
        assert!(c.rebuilt <= i + 2, "{}", c.rebuilt);
        // 범위가 넓어지는 객체를 더하면 모두 다시 (그래도 같은 결과)
        let mut d7 = d6.clone();
        d7.entities.push(crate::drawing::Entity {
            shape: Shape::Line {
                a: Vec2::new(-1e6, -1e6),
                b: Vec2::new(1e6, 1e6),
            },
            ..d6.entities[i].clone()
        });
        assert!(same(&segments_cached(&d7, &mut c), &segments(&d7)));
        assert_eq!(c.rebuilt, d7.entities.len());
    }
}
