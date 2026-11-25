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
        Shape::Line { a, b } => pieces.push(vec![*a, *b]),
        Shape::Polyline { points, closed } => {
            if dash.is_some() {
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
#[derive(Default)]
pub struct SegCache {
    key: Option<(
        Bbox,
        Vec<crate::drawing::Layer>,
        Vec<crate::drawing::LinePattern>,
    )>,
    items: Vec<(Entity, Segments)>,
    /// 마지막에 다시 펼친 객체 수 (측정용)
    pub rebuilt: usize,
    /// 마지막 결과에서 객체마다 (선분 시작, 선분 수, 삼각형 시작, 삼각형 수)
    layout: Vec<(usize, usize, usize, usize)>,
    /// 바로 앞 결과와 비교해 부분 갱신이 되면 바뀐 객체 번호들 (§68)
    patch: Option<Vec<usize>>,
}

impl SegCache {
    /// 바로 앞 [`segments_cached`] 결과 `s` 를 화면에 부분 갱신으로 보낼 수 있으면 그 바이너리 (`DSP1`).
    /// 형식: 매직 · 범위 수 u32 · 범위마다 (선분 시작·수·삼각형 시작·수 u32 ×4, 좌표 f32, 색 u8×4, 굵기 u8, 삼각형 f32, 색 u8×4)
    pub fn patch_bytes(&self, s: &Segments) -> Option<Vec<u8>> {
        let ids = self.patch.as_ref()?;
        let mut out = Vec::new();
        out.extend_from_slice(b"DSP1");
        out.extend_from_slice(&(ids.len() as u32).to_le_bytes());
        for &i in ids {
            let (lo, ln, to, tn) = self.layout[i];
            for v in [lo, ln, to, tn] {
                out.extend_from_slice(&(v as u32).to_le_bytes());
            }
            for v in &s.coords[lo * 4..(lo + ln) * 4] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.extend_from_slice(&s.colors[lo * 4..(lo + ln) * 4]);
            out.extend(s.weights[lo..lo + ln].iter().map(|&w| w.min(255) as u8));
            for v in &s.tris[to * 6..(to + tn) * 6] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.extend_from_slice(&s.tri_colors[to * 4..(to + tn) * 4]);
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

/// [`segments`] 와 같은 결과를, 바뀌지 않은 객체는 캐시에서
pub fn segments_cached(d: &Drawing, cache: &mut SegCache) -> Segments {
    let bbox = d.bbox();
    let diag = bbox.width().hypot(bbox.height()).max(1e-9);
    let tol = diag / 20_000.0;
    let origin = bbox.center();
    let same = cache
        .key
        .as_ref()
        .is_some_and(|(b, l, t)| *b == bbox && *l == d.layers && *t == d.linetypes);
    // 부분 갱신은 범위·레이어·객체 수가 같고, 다시 펼친 객체의 선분·삼각형 수가 그대로일 때만 (§68)
    let mut patch =
        (same && cache.items.len() == d.entities.len() && cache.layout.len() == d.entities.len())
            .then(Vec::new);
    if !same {
        cache.items.clear();
        cache.key = Some((bbox, d.layers.clone(), d.linetypes.clone()));
    }
    let mut s = Segments {
        origin,
        bbox,
        ..Default::default()
    };
    let mut pieces: Vec<Vec<Vec2>> = Vec::new();
    cache.rebuilt = 0;
    let mut layout = Vec::with_capacity(d.entities.len());
    for (i, e) in d.entities.iter().enumerate() {
        let hit = cache.items.get(i).is_some_and(|(c, _)| c == e);
        if !hit {
            if let Some(p) = patch.as_mut() {
                p.push(i);
            }
            let mut p = Segments {
                origin,
                bbox,
                ..Default::default()
            };
            emit(d, e, tol, &mut p, &mut pieces);
            if i < cache.items.len() {
                cache.items[i] = (e.clone(), p);
            } else {
                cache.items.push((e.clone(), p));
            }
            cache.rebuilt += 1;
        }
        let (ln, tn) = (s.len(), s.tri_count());
        s.append(&cache.items[i].1);
        layout.push((ln, s.len() - ln, tn, s.tri_count() - tn));
    }
    cache.items.truncate(d.entities.len());
    // 다시 펼친 객체의 자리가 그대로인지
    if let Some(p) = &patch
        && p.iter().any(|&i| cache.layout.get(i) != layout.get(i))
    {
        patch = None;
    }
    cache.patch = patch;
    cache.layout = layout;
    s
}

/// 한 조각에 들어갈 수 있는 무늬 반복 수. 넘으면 (화면에서 어차피 실선처럼 보이므로) 실선으로 그린다.
pub const MAX_DASH_REPEATS: f64 = 2_000.0;

/// 꺾은선을 따라 대시 무늬를 그린다. 무늬는 꺾은선 처음에서 시작하고, 꺾인 곳을 지나는 대시는 꺾인 채로 이어진다.
fn dash_chain(s: &mut Segments, pts: &[Vec2], dashes: &[f64], scale: f64, tol: f64, col: [u8; 4]) {
    let period: f64 = dashes.iter().map(|x| x.abs()).sum::<f64>() * scale;
    let len: f64 = pts.windows(2).map(|w| w[0].dist(w[1])).sum();
    if pts.len() < 2 || period <= tol * 2.0 || len / period > MAX_DASH_REPEATS {
        s.strip(pts, false, col);
        return;
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
                    s.push(at(t), at((t + dot).min(l)), col);
                }
            } else {
                let step = remain.min(l - t);
                if on {
                    s.push(at(t), at(t + step), col);
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
}

/// 문자 표시 데이터 (`DST1`). 꺼진 레이어는 뺀다. origin 은 [`segments`] 와 같다.
pub fn texts(d: &Drawing) -> Vec<u8> {
    let origin = d.bbox().center();
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
        assert!(c.patch_bytes(&segments(&d2)).is_none());
        let mut d3 = d2.clone();
        d3.entities[i].color = Some([9, 9, 9]);
        let s3 = segments_cached(&d3, &mut c);
        let p = c.patch_bytes(&s3).unwrap();
        assert_eq!(&p[0..4], b"DSP1");
        assert_eq!(u32::from_le_bytes(p[4..8].try_into().unwrap()), 1);
        // 객체가 늘면 부분 갱신 없음
        let mut d4 = d3.clone();
        d4.entities.push(d4.entities[i].clone());
        let s4 = segments_cached(&d4, &mut c);
        assert!(c.patch_bytes(&s4).is_none());
    }
}
