//! 자르기·연장·모깎기·해치 영역 (M2, 구현 노트 §27).
//!
//! 선·호·원·폴리선(불지 포함)을 「조각」(선분 또는 호) 열로 보고, 조각 k 의 매개변수 t∈[0,1] 을 이어 붙인
//! 전체 매개변수 T∈[0,n] 위에서 자르고 늘린다. 경계는 화면에 보이는 모든 객체(건축 객체가 만든 선 포함)
//! — AutoCAD 의 「빠른 자르기」 와 같다.

use std::f64::consts::{PI, TAU};

use crate::curve::{bulge_arc, ccw_sweep, ellipse_points, polyline_points};
use crate::drawing::{Drawing, Shape};
use crate::fill::{self, PatternLine};
use crate::geom::Vec2;

const EPS: f64 = 1e-9;

fn sub(a: Vec2, b: Vec2) -> Vec2 {
    Vec2::new(a.x - b.x, a.y - b.y)
}
fn add(a: Vec2, b: Vec2) -> Vec2 {
    Vec2::new(a.x + b.x, a.y + b.y)
}
fn mul(a: Vec2, k: f64) -> Vec2 {
    Vec2::new(a.x * k, a.y * k)
}
fn dot(a: Vec2, b: Vec2) -> f64 {
    a.x * b.x + a.y * b.y
}
fn cross(a: Vec2, b: Vec2) -> f64 {
    a.x * b.y - a.y * b.x
}
fn norm(a: Vec2) -> Vec2 {
    let l = a.x.hypot(a.y);
    if l < 1e-300 { a } else { mul(a, 1.0 / l) }
}
fn wrap(a: f64) -> f64 {
    a.rem_euclid(TAU)
}

/// 조각: 선분 또는 호 (호는 시작각 s 에서 부호 있는 사이각 sw — 양수 반시계)
#[derive(Debug, Clone, Copy)]
enum Piece {
    Seg(Vec2, Vec2),
    Arc { c: Vec2, r: f64, s: f64, sw: f64 },
}

impl Piece {
    fn at(&self, t: f64) -> Vec2 {
        match *self {
            Piece::Seg(a, b) => a.lerp(b, t),
            Piece::Arc { c, r, s, sw } => {
                let a = s + sw * t;
                Vec2::new(c.x + r * a.cos(), c.y + r * a.sin())
            }
        }
    }

    /// 조각 위 (또는 늘린 선·원 위) 점의 매개변수들. 호는 한 바퀴 앞뒤 둘 다
    fn params(&self, p: Vec2) -> Vec<f64> {
        match *self {
            Piece::Seg(a, b) => {
                let d = sub(b, a);
                let l2 = dot(d, d);
                if l2 < 1e-300 {
                    vec![0.0]
                } else {
                    vec![dot(sub(p, a), d) / l2]
                }
            }
            Piece::Arc { c, s, sw, .. } => {
                if sw.abs() < 1e-300 {
                    return vec![0.0];
                }
                let ang = (p.y - c.y).atan2(p.x - c.x);
                let d = if sw > 0.0 {
                    wrap(ang - s)
                } else {
                    wrap(s - ang)
                };
                let t = d / sw.abs();
                vec![t, t - TAU / sw.abs()]
            }
        }
    }

    /// 조각 위 가장 가까운 점의 매개변수 (0..1 로 자름)와 거리
    fn closest(&self, p: Vec2) -> (f64, f64) {
        let mut best = (0.0, f64::INFINITY);
        let mut try_t = |t: f64| {
            let t = t.clamp(0.0, 1.0);
            let d = self.at(t).dist(p);
            if d < best.1 {
                best = (t, d);
            }
        };
        for t in self.params(p) {
            try_t(t);
        }
        try_t(0.0);
        try_t(1.0);
        best
    }

    /// 매개변수 구간 [t0, t1] 의 부분 조각 (t 는 0..1 밖이어도 된다 — 늘리기)
    fn part(&self, t0: f64, t1: f64) -> Piece {
        match *self {
            Piece::Seg(..) => Piece::Seg(self.at(t0), self.at(t1)),
            Piece::Arc { c, r, s, sw } => Piece::Arc {
                c,
                r,
                s: s + sw * t0,
                sw: sw * (t1 - t0),
            },
        }
    }
}

/// 밑바탕 곡선 (무한 직선 또는 온전한 원)의 교점
fn raw_hits(p: &Piece, q: &Piece) -> Vec<Vec2> {
    match (*p, *q) {
        (Piece::Seg(a, b), Piece::Seg(c, d)) => {
            let (r, s) = (sub(b, a), sub(d, c));
            let den = cross(r, s);
            if den.abs() < 1e-12 * (r.x.hypot(r.y) * s.x.hypot(s.y)).max(1e-300) {
                return Vec::new();
            }
            let t = cross(sub(c, a), s) / den;
            vec![add(a, mul(r, t))]
        }
        (Piece::Seg(a, b), Piece::Arc { c, r, .. })
        | (Piece::Arc { c, r, .. }, Piece::Seg(a, b)) => line_circle(a, b, c, r),
        (Piece::Arc { c: c1, r: r1, .. }, Piece::Arc { c: c2, r: r2, .. }) => {
            circle_circle(c1, r1, c2, r2)
        }
    }
}

fn line_circle(a: Vec2, b: Vec2, c: Vec2, r: f64) -> Vec<Vec2> {
    let d = sub(b, a);
    let f = sub(a, c);
    let (aa, bb, cc) = (dot(d, d), 2.0 * dot(f, d), dot(f, f) - r * r);
    if aa < 1e-300 {
        return Vec::new();
    }
    let disc = bb * bb - 4.0 * aa * cc;
    if disc < -1e-9 * bb.abs().max(1.0) {
        return Vec::new();
    }
    let sq = disc.max(0.0).sqrt();
    let mut v = vec![add(a, mul(d, (-bb - sq) / (2.0 * aa)))];
    if sq > 1e-12 {
        v.push(add(a, mul(d, (-bb + sq) / (2.0 * aa))));
    }
    v
}

fn circle_circle(c1: Vec2, r1: f64, c2: Vec2, r2: f64) -> Vec<Vec2> {
    let d = c1.dist(c2);
    if d < 1e-12 || d > r1 + r2 + 1e-9 || d < (r1 - r2).abs() - 1e-9 {
        return Vec::new();
    }
    let a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
    let h = (r1 * r1 - a * a).max(0.0).sqrt();
    let u = mul(sub(c2, c1), 1.0 / d);
    let m = add(c1, mul(u, a));
    let n = Vec2::new(-u.y, u.x);
    if h < 1e-12 {
        vec![m]
    } else {
        vec![add(m, mul(n, h)), sub(m, mul(n, h))]
    }
}

/// 점이 조각(경계 있는) 위에 있나
fn on_piece(q: &Piece, p: Vec2) -> bool {
    let tol = 1e-7;
    q.params(p).iter().any(|&t| t >= -tol && t <= 1.0 + tol) && {
        let (t, _) = q.closest(p);
        q.at(t).dist(p) < 1e-6 * (1.0 + p.x.abs().max(p.y.abs()))
    }
}

/// 모양 → 조각들, 닫혔나
fn pieces(s: &Shape) -> Option<(Vec<Piece>, bool)> {
    Some(match s {
        Shape::Line { a, b } => (vec![Piece::Seg(*a, *b)], false),
        Shape::Arc {
            center,
            radius,
            start,
            end,
        } => (
            vec![Piece::Arc {
                c: *center,
                r: *radius,
                s: *start,
                sw: ccw_sweep(*start, *end),
            }],
            false,
        ),
        Shape::Circle { center, radius } => (
            vec![Piece::Arc {
                c: *center,
                r: *radius,
                s: 0.0,
                sw: TAU,
            }],
            true,
        ),
        Shape::Polyline { points, closed } => {
            let n = points.len();
            if n < 2 {
                return None;
            }
            let m = if *closed { n } else { n - 1 };
            let v = (0..m)
                .map(|i| {
                    let (a, bulge) = points[i];
                    let b = points[(i + 1) % n].0;
                    match bulge_arc(a, b, bulge) {
                        Some((c, r, s, sw)) => Piece::Arc { c, r, s, sw },
                        None => Piece::Seg(a, b),
                    }
                })
                .collect();
            (v, *closed)
        }
        _ => return None,
    })
}

/// 경계로 쓰는 조각들 (해치 무늬·채움 테두리·타원은 선분으로)
fn boundary_pieces(s: &Shape) -> Vec<Piece> {
    if let Some((p, _)) = pieces(s) {
        return p;
    }
    let segs = |pts: &[Vec2], closed: bool| -> Vec<Piece> {
        let n = pts.len();
        let m = if closed { n } else { n.saturating_sub(1) };
        (0..m)
            .map(|i| Piece::Seg(pts[i], pts[(i + 1) % n]))
            .collect()
    };
    match s {
        Shape::Segments { lines } => lines.iter().map(|(a, b)| Piece::Seg(*a, *b)).collect(),
        Shape::Fill { loops } => loops.iter().flat_map(|l| segs(l, true)).collect(),
        Shape::Ellipse {
            center,
            u,
            v,
            start,
            end,
        } => {
            let mut pts = Vec::new();
            ellipse_points(
                *center,
                *u,
                *v,
                *start,
                ccw_sweep(*start, *end),
                0.001 * u.x.hypot(u.y),
                &mut pts,
            );
            segs(&pts, false)
        }
        _ => Vec::new(),
    }
}

/// 조각 열의 구간 [t0, t1] (전체 매개변수, 닫힌 것은 n 을 넘어 한 바퀴 돌 수 있음)
fn span(ps: &[Piece], t0: f64, t1: f64) -> Vec<Piece> {
    let n = ps.len() as f64;
    let mut out = Vec::new();
    let mut a = t0;
    while a < t1 - EPS {
        let k = a.floor();
        let b = (k + 1.0).min(t1);
        let idx = (k.rem_euclid(n)) as usize;
        out.push(ps[idx].part(a - k, b - k));
        a = b;
    }
    out
}

/// 조각 열 → 모양 (원래 모양 꼴을 따라)
fn rebuild(orig: &Shape, ps: &[Piece]) -> Option<Shape> {
    if ps.is_empty() {
        return None;
    }
    let arc_shape = |p: &Piece| match *p {
        Piece::Arc { c, r, s, sw } => {
            let (st, en) = if sw >= 0.0 { (s, s + sw) } else { (s + sw, s) };
            Some(Shape::Arc {
                center: c,
                radius: r,
                start: wrap(st),
                end: wrap(en),
            })
        }
        Piece::Seg(a, b) => Some(Shape::Line { a, b }),
    };
    match orig {
        Shape::Line { .. } | Shape::Arc { .. } | Shape::Circle { .. } if ps.len() == 1 => {
            arc_shape(&ps[0])
        }
        _ => {
            let mut points: Vec<(Vec2, f64)> = ps
                .iter()
                .map(|p| {
                    let bulge = match *p {
                        Piece::Seg(..) => 0.0,
                        Piece::Arc { sw, .. } => (sw / 4.0).tan(),
                    };
                    (p.at(0.0), bulge)
                })
                .collect();
            points.push((ps.last()?.at(1.0), 0.0));
            if ps.len() == 1 && matches!(ps[0], Piece::Seg(..)) {
                return Some(Shape::Line {
                    a: points[0].0,
                    b: points[1].0,
                });
            }
            Some(Shape::Polyline {
                points,
                closed: false,
            })
        }
    }
}

/// 객체 i 와 다른 모든 객체의 교점 (i 의 전체 매개변수). `extend` 면 i 의 양 끝 조각을 늘려서
fn cuts(view: &Drawing, i: usize, ps: &[Piece], extend: bool) -> Vec<f64> {
    let mut out = Vec::new();
    let tb = {
        let mut b = crate::geom::Bbox::EMPTY;
        b.merge(&view.entities[i].shape.bbox());
        b
    };
    for (j, e) in view.entities.iter().enumerate() {
        if j == i || !view.layers.get(e.layer as usize).is_none_or(|l| l.visible) {
            continue;
        }
        // 늘리기가 아니면 범위가 겹치는 것만
        if !extend {
            let b = e.shape.bbox();
            if b.is_empty()
                || b.max.x < tb.min.x - 1e-6
                || b.min.x > tb.max.x + 1e-6
                || b.max.y < tb.min.y - 1e-6
                || b.min.y > tb.max.y + 1e-6
            {
                continue;
            }
        }
        for q in boundary_pieces(&e.shape) {
            for (k, p) in ps.iter().enumerate() {
                for h in raw_hits(p, &q) {
                    if !on_piece(&q, h) {
                        continue;
                    }
                    for t in p.params(h) {
                        if p.at(t).dist(h) > 1e-6 * (1.0 + h.x.abs().max(h.y.abs())) {
                            continue;
                        }
                        if (0.0..=1.0).contains(&t) || extend {
                            out.push(k as f64 + t);
                        }
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| a.total_cmp(b));
    out.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    out
}

/// 고른 점에서 가장 가까운 전체 매개변수
fn pick_param(ps: &[Piece], pick: Vec2) -> f64 {
    let mut best = (0.0, f64::INFINITY);
    for (k, p) in ps.iter().enumerate() {
        let (t, d) = p.closest(pick);
        if d < best.1 {
            best = (k as f64 + t, d);
        }
    }
    best.0
}

/// 자르기: 객체 i 에서 고른 점이 든 구간(이웃 교점 사이)을 없앤다. 남는 모양들 (0~2 개)
pub fn trim(view: &Drawing, i: usize, pick: Vec2) -> Result<Vec<Shape>, String> {
    let e = view.entities.get(i).ok_or("객체가 없습니다")?;
    let (ps, closed) = pieces(&e.shape).ok_or("선·호·원·폴리선만 자를 수 있습니다")?;
    let n = ps.len() as f64;
    let c: Vec<f64> = cuts(view, i, &ps, false)
        .into_iter()
        .filter(|t| *t > EPS && *t < n - EPS || closed)
        .collect();
    let tp = pick_param(&ps, pick);
    if closed {
        let c: Vec<f64> = c.into_iter().map(|t| t.rem_euclid(n)).collect();
        if c.len() < 2 {
            return Err("닫힌 객체는 경계와 두 번 이상 만나야 자를 수 있습니다".into());
        }
        let lo = c
            .iter()
            .copied()
            .filter(|t| *t < tp)
            .fold(f64::NAN, f64::max);
        let lo = if lo.is_nan() {
            c.iter().copied().fold(f64::NEG_INFINITY, f64::max)
        } else {
            lo
        };
        let hi = c
            .iter()
            .copied()
            .filter(|t| *t > tp)
            .fold(f64::NAN, f64::min);
        let hi = if hi.is_nan() {
            c.iter().copied().fold(f64::INFINITY, f64::min)
        } else {
            hi
        };
        // 남는 길: hi → lo (한 바퀴 돌아)
        let end = if lo <= hi { lo + n } else { lo };
        return Ok(rebuild(&e.shape, &span(&ps, hi, end)).into_iter().collect());
    }
    if c.is_empty() {
        return Err("자를 경계가 없습니다 — 다른 객체와 만나는 곳이 있어야 합니다".into());
    }
    let lo = c
        .iter()
        .copied()
        .filter(|t| *t < tp)
        .fold(f64::NAN, f64::max);
    let hi = c
        .iter()
        .copied()
        .filter(|t| *t > tp)
        .fold(f64::NAN, f64::min);
    let mut out = Vec::new();
    if !lo.is_nan() {
        out.extend(rebuild(&e.shape, &span(&ps, 0.0, lo)));
    }
    if !hi.is_nan() {
        out.extend(rebuild(&e.shape, &span(&ps, hi, n)));
    }
    Ok(out)
}

/// 연장: 고른 점에 가까운 끝을 가장 가까운 경계까지 늘린다
pub fn extend(view: &Drawing, i: usize, pick: Vec2) -> Result<Shape, String> {
    let e = view.entities.get(i).ok_or("객체가 없습니다")?;
    let (ps, closed) = pieces(&e.shape).ok_or("선·호·폴리선만 연장할 수 있습니다")?;
    if closed {
        return Err("닫힌 객체는 연장할 수 없습니다".into());
    }
    let at_end = pick.dist(ps[ps.len() - 1].at(1.0)) < pick.dist(ps[0].at(0.0));
    // 끝 조각만 늘려 교점을 구한다
    let end_piece = if at_end { ps[ps.len() - 1] } else { ps[0] };
    let hits: Vec<f64> = cuts(view, i, &[end_piece], true);
    let t = if at_end {
        hits.into_iter()
            .filter(|t| *t > 1.0 + 1e-9)
            .fold(f64::NAN, f64::min)
    } else {
        hits.into_iter()
            .filter(|t| *t < -1e-9)
            .fold(f64::NAN, f64::max)
    };
    if t.is_nan() {
        return Err("늘려서 만나는 경계가 없습니다".into());
    }
    // 끝 조각을 늘린 조각 열
    let mut v = ps.clone();
    if at_end {
        let last = v.len() - 1;
        v[last] = ps[last].part(0.0, t);
    } else {
        v[0] = ps[0].part(t, 1.0);
    }
    rebuild(&e.shape, &v).ok_or_else(|| "연장하지 못했습니다".into())
}

/// 모깎기: 두 선(고른 점 쪽을 남김)을 반지름 r 호로 잇는다. r = 0 이면 모서리로 맞춘다.
/// (선 a 새 모양, 선 b 새 모양, 호)
pub fn fillet(
    a: &Shape,
    pa: Vec2,
    b: &Shape,
    pb: Vec2,
    r: f64,
) -> Result<(Shape, Shape, Option<Shape>), String> {
    let (Shape::Line { a: a0, b: a1 }, Shape::Line { a: b0, b: b1 }) = (a, b) else {
        return Err("모깎기는 지금은 선 두 개만 됩니다".into());
    };
    let x = raw_hits(&Piece::Seg(*a0, *a1), &Piece::Seg(*b0, *b1))
        .pop()
        .ok_or("평행한 선은 모깎기 할 수 없습니다")?;
    // 각 선에서 고른 점 쪽 방향과 그쪽 먼 끝
    let side = |p0: Vec2, p1: Vec2, pick: Vec2| -> (Vec2, Vec2) {
        let d = norm(sub(p1, p0));
        let s = dot(sub(pick, x), d);
        let u = if s >= 0.0 { d } else { mul(d, -1.0) };
        let far = if dot(sub(p0, x), u) >= dot(sub(p1, x), u) {
            p0
        } else {
            p1
        };
        (u, far)
    };
    let (ua, fa) = side(*a0, *a1, pa);
    let (ub, fb) = side(*b0, *b1, pb);
    if r <= 1e-9 {
        return Ok((
            Shape::Line { a: fa, b: x },
            Shape::Line { a: fb, b: x },
            None,
        ));
    }
    let phi = dot(ua, ub).clamp(-1.0, 1.0).acos();
    if phi < 1e-6 || (PI - phi) < 1e-6 {
        return Err("두 선이 한 줄이라 모깎기 할 수 없습니다".into());
    }
    let dt = r / (phi / 2.0).tan();
    if dt > x.dist(fa) + 1e-9 || dt > x.dist(fb) + 1e-9 {
        return Err("반지름이 선보다 큽니다".into());
    }
    let ta = add(x, mul(ua, dt));
    let tb = add(x, mul(ub, dt));
    let c = add(x, mul(norm(add(ua, ub)), r / (phi / 2.0).sin()));
    let (mut s, mut e) = (
        (ta.y - c.y).atan2(ta.x - c.x),
        (tb.y - c.y).atan2(tb.x - c.x),
    );
    if ccw_sweep(s, e) > PI {
        std::mem::swap(&mut s, &mut e);
    }
    Ok((
        Shape::Line { a: fa, b: ta },
        Shape::Line { a: fb, b: tb },
        Some(Shape::Arc {
            center: c,
            radius: r,
            start: wrap(s),
            end: wrap(e),
        }),
    ))
}

fn ring_area(r: &[Vec2]) -> f64 {
    let n = r.len();
    (0..n)
        .map(|i| cross(r[i], r[(i + 1) % n]))
        .sum::<f64>()
        .abs()
        / 2.0
}

/// 모따기: 두 선(고른 점 쪽을 남김)을 교점에서 d1·d2 떨어진 곳에서 잘라 선으로 잇는다 (§33)
pub fn chamfer(
    a: &Shape,
    pa: Vec2,
    b: &Shape,
    pb: Vec2,
    d1: f64,
    d2: f64,
) -> Result<(Shape, Shape, Option<Shape>), String> {
    let (Shape::Line { a: a0, b: a1 }, Shape::Line { a: b0, b: b1 }) = (a, b) else {
        return Err("모따기는 선 두 개, 또는 폴리선 모서리에 됩니다".into());
    };
    let x = raw_hits(&Piece::Seg(*a0, *a1), &Piece::Seg(*b0, *b1))
        .pop()
        .ok_or("평행한 선은 모따기 할 수 없습니다")?;
    let side = |p0: Vec2, p1: Vec2, pick: Vec2| -> (Vec2, Vec2) {
        let d = norm(sub(p1, p0));
        let u = if dot(sub(pick, x), d) >= 0.0 {
            d
        } else {
            mul(d, -1.0)
        };
        let far = if dot(sub(p0, x), u) >= dot(sub(p1, x), u) {
            p0
        } else {
            p1
        };
        (u, far)
    };
    let (ua, fa) = side(*a0, *a1, pa);
    let (ub, fb) = side(*b0, *b1, pb);
    if d1 <= 1e-9 && d2 <= 1e-9 {
        return Ok((
            Shape::Line { a: fa, b: x },
            Shape::Line { a: fb, b: x },
            None,
        ));
    }
    if d1 > x.dist(fa) + 1e-9 || d2 > x.dist(fb) + 1e-9 {
        return Err("모따기 거리가 선보다 깁니다".into());
    }
    let (ta, tb) = (add(x, mul(ua, d1)), add(x, mul(ub, d2)));
    Ok((
        Shape::Line { a: fa, b: ta },
        Shape::Line { a: fb, b: tb },
        Some(Shape::Line { a: ta, b: tb }),
    ))
}

/// 폴리선 모서리 처리
#[derive(Debug, Clone, Copy)]
pub enum Corner {
    /// 반지름
    Fillet(f64),
    /// 두 거리 (앞 변, 뒤 변)
    Chamfer(f64, f64),
}

/// 폴리선의 고른 점에서 가장 가까운 꼭짓점(앞뒤 변이 직선인 것)을 모깎기·모따기한 폴리선
pub fn corner_polyline(s: &Shape, pick: Vec2, c: Corner) -> Result<Shape, String> {
    let Shape::Polyline { points, closed } = s else {
        return Err("폴리선이 아닙니다".into());
    };
    let n = points.len();
    let ok = |i: usize| {
        let has_prev = *closed || i > 0;
        let has_next = *closed || i + 1 < n;
        has_prev && has_next && points[(i + n - 1) % n].1.abs() < 1e-12 && points[i].1.abs() < 1e-12
    };
    let i = (0..n)
        .filter(|&i| ok(i))
        .min_by(|&p, &q| points[p].0.dist(pick).total_cmp(&points[q].0.dist(pick)))
        .ok_or("모서리를 만들 꼭짓점이 없습니다 (앞뒤 변이 직선이어야)")?;
    let v = points[i].0;
    let (p, q) = (points[(i + n - 1) % n].0, points[(i + 1) % n].0);
    let (u1, u2) = (norm(sub(p, v)), norm(sub(q, v)));
    let (d1, d2, bulge) = match c {
        Corner::Chamfer(a, b) => (a, b, 0.0),
        Corner::Fillet(r) => {
            let phi = dot(u1, u2).clamp(-1.0, 1.0).acos();
            if phi < 1e-6 || (PI - phi) < 1e-6 {
                return Err("곧게 이어진 꼭짓점입니다".into());
            }
            let dt = r / (phi / 2.0).tan();
            // 왼쪽으로 꺾이면 반시계 호 (불지 양수)
            let turn = cross(sub(v, p), sub(q, v)).signum();
            (dt, dt, turn * ((PI - phi) / 4.0).tan())
        }
    };
    if d1 > v.dist(p) + 1e-9 || d2 > v.dist(q) + 1e-9 {
        return Err("반지름·거리가 변보다 깁니다".into());
    }
    if d1 <= 1e-9 && d2 <= 1e-9 {
        return Ok(s.clone());
    }
    let mut out = points.clone();
    out[i] = (add(v, mul(u1, d1)), bulge);
    out.insert(i + 1, (add(v, mul(u2, d2)), 0.0));
    Ok(Shape::Polyline {
        points: out,
        closed: *closed,
    })
}

/// 해치 경계 찾기: 점을 품은 가장 작은 닫힌 폴리선·원, 없으면 점에서 사방으로 쏜 반직선이 처음 닿는 선들로
pub fn region_at(view: &Drawing, p: Vec2) -> Option<Vec<Vec2>> {
    let tol = |b: &crate::geom::Bbox| (b.width().max(b.height()) * 0.002).max(1e-6);
    let mut best: Option<(f64, Vec<Vec2>)> = None;
    for e in &view.entities {
        if !view.layers.get(e.layer as usize).is_none_or(|l| l.visible) {
            continue;
        }
        let ring = match &e.shape {
            Shape::Polyline {
                points,
                closed: true,
            } => polyline_points(points, true, tol(&e.shape.bbox())),
            Shape::Circle { center, radius } => (0..96)
                .map(|k| {
                    Vec2::new(
                        center.x + radius * (k as f64 * TAU / 96.0).cos(),
                        center.y + radius * (k as f64 * TAU / 96.0).sin(),
                    )
                })
                .collect(),
            _ => continue,
        };
        if ring.len() >= 3 && fill::inside(p, &ring) {
            let a = ring_area(&ring);
            if best.as_ref().is_none_or(|(b, _)| a < *b) {
                best = Some((a, ring));
            }
        }
    }
    if let Some((_, r)) = best {
        return Some(r);
    }
    // 반직선: 모든 경계 조각과
    let all: Vec<Piece> = view
        .entities
        .iter()
        .filter(|e| view.layers.get(e.layer as usize).is_none_or(|l| l.visible))
        .filter(|e| !matches!(e.shape, Shape::Text(_)))
        .flat_map(|e| boundary_pieces(&e.shape))
        .collect();
    if all.is_empty() {
        return None;
    }
    let rays = 720;
    let mut pts = Vec::with_capacity(rays);
    for k in 0..rays {
        let a = TAU * k as f64 / rays as f64;
        let ray = Piece::Seg(p, add(p, Vec2::new(a.cos(), a.sin())));
        let mut tmin = f64::INFINITY;
        for q in &all {
            for h in raw_hits(&ray, q) {
                let t = ray.params(h)[0];
                if t > 1e-9 && t < tmin && on_piece(q, h) {
                    tmin = t;
                }
            }
        }
        if !tmin.is_finite() {
            return None; // 열린 곳이 있다
        }
        pts.push(ray.at(tmin));
    }
    // 같은 직선 위 점 줄이기
    let mut out: Vec<Vec2> = Vec::new();
    for (k, q) in pts.iter().enumerate() {
        let prev = pts[(k + rays - 1) % rays];
        let next = pts[(k + 1) % rays];
        let span = sub(next, prev);
        if cross(sub(*q, prev), sub(next, *q)).abs() > 1e-9 * dot(span, span) {
            out.push(*q);
        }
    }
    (out.len() >= 3).then_some(out)
}

/// 해치 무늬 이름들
pub const PATTERNS: [&str; 4] = ["SOLID", "사선", "격자", "콘크리트"];

/// 경계와 무늬로 해치 모양 (간격 = 도면 단위)
pub fn hatch_shape(ring: Vec<Vec2>, pattern: &str, spacing: f64) -> Result<Shape, String> {
    let s = spacing.max(1e-6);
    let line = |angle: f64, gap: f64, dashes: Vec<f64>| {
        let a = angle.to_radians();
        PatternLine {
            angle: a,
            base: Vec2::default(),
            offset: Vec2::new(-a.sin() * gap, a.cos() * gap),
            dashes,
        }
    };
    let lines = match pattern {
        "SOLID" => return Ok(Shape::Fill { loops: vec![ring] }),
        "사선" => vec![line(45.0, s, vec![])],
        "격자" => vec![line(0.0, s, vec![]), line(90.0, s, vec![])],
        "콘크리트" => vec![
            line(45.0, s * 2.0, vec![s * 1.2, -s * 0.8]),
            line(135.0, s * 2.0, vec![s * 0.6, -s * 1.4]),
        ],
        _ => return Err(format!("모르는 무늬: {pattern}")),
    };
    let segs = fill::pattern_segments(&[ring], &lines, 200_000)
        .ok_or("무늬가 너무 촘촘합니다 — 간격을 키우세요")?;
    Ok(Shape::Segments { lines: segs })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawing::{Entity, Layer};

    fn view(shapes: Vec<Shape>) -> Drawing {
        Drawing {
            layers: vec![Layer {
                name: "0".into(),
                color: [255; 3],
                visible: true,
            }],
            entities: shapes
                .into_iter()
                .map(|shape| Entity {
                    layer: 0,
                    color: None,
                    aci: None,
                    linetype: None,
                    weight: None,
                    shape,
                })
                .collect(),
            ..Default::default()
        }
    }
    fn l(ax: f64, ay: f64, bx: f64, by: f64) -> Shape {
        Shape::Line {
            a: Vec2::new(ax, ay),
            b: Vec2::new(bx, by),
        }
    }

    #[test]
    fn trim_middle_of_line() {
        // 가로선을 세로선 둘이 자른다 → 가운데를 누르면 두 토막
        let v = view(vec![
            l(0.0, 0.0, 10.0, 0.0),
            l(3.0, -1.0, 3.0, 1.0),
            l(7.0, -1.0, 7.0, 1.0),
        ]);
        let out = trim(&v, 0, Vec2::new(5.0, 0.1)).unwrap();
        assert_eq!(out, vec![l(0.0, 0.0, 3.0, 0.0), l(7.0, 0.0, 10.0, 0.0)]);
        // 끝 쪽을 누르면 한 토막
        let out = trim(&v, 0, Vec2::new(9.0, 0.0)).unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn trim_circle_makes_arc() {
        let v = view(vec![
            Shape::Circle {
                center: Vec2::default(),
                radius: 5.0,
            },
            l(-10.0, 0.0, 10.0, 0.0),
        ]);
        let out = trim(&v, 0, Vec2::new(0.0, 5.0)).unwrap();
        match &out[0] {
            Shape::Arc { start, end, .. } => {
                // 위쪽 반원을 지우면 아래쪽 반원(π → 2π)
                assert!(
                    (start - PI).abs() < 1e-6 && (end - 0.0).abs() < 1e-6
                        || (end - TAU).abs() < 1e-6,
                    "{start} {end}"
                );
            }
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn extend_to_boundary() {
        let v = view(vec![l(0.0, 0.0, 5.0, 0.0), l(8.0, -1.0, 8.0, 1.0)]);
        let s = extend(&v, 0, Vec2::new(4.5, 0.0)).unwrap();
        assert_eq!(s, l(0.0, 0.0, 8.0, 0.0));
    }

    #[test]
    fn fillet_corner_and_radius() {
        let a = l(0.0, 0.0, 10.0, 0.0);
        let b = l(12.0, 2.0, 12.0, 10.0);
        let (na, nb, arc) = fillet(&a, Vec2::new(2.0, 0.0), &b, Vec2::new(12.0, 8.0), 0.0).unwrap();
        assert_eq!(
            (na, nb, arc),
            (l(0.0, 0.0, 12.0, 0.0), l(12.0, 10.0, 12.0, 0.0), None)
        );
        let (na, _, arc) = fillet(&a, Vec2::new(2.0, 0.0), &b, Vec2::new(12.0, 8.0), 2.0).unwrap();
        assert_eq!(na, l(0.0, 0.0, 10.0, 0.0));
        match arc.unwrap() {
            Shape::Arc { center, radius, .. } => {
                assert!(center.dist(Vec2::new(10.0, 2.0)) < 1e-9 && radius == 2.0);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn region_from_lines() {
        let v = view(vec![
            l(0.0, 0.0, 10.0, 0.0),
            l(10.0, 0.0, 10.0, 5.0),
            l(10.0, 5.0, 0.0, 5.0),
            l(0.0, 5.0, 0.0, 0.0),
        ]);
        let r = region_at(&v, Vec2::new(3.0, 2.0)).unwrap();
        assert!((ring_area(&r) - 50.0).abs() < 0.5, "{}", ring_area(&r));
        let h = hatch_shape(r, "사선", 1.0).unwrap();
        assert!(matches!(h, Shape::Segments { ref lines } if !lines.is_empty()));
    }

    #[test]
    fn chamfer_and_polyline_corners() {
        let (na, nb, c) = chamfer(
            &l(0.0, 0.0, 10.0, 0.0),
            Vec2::new(2.0, 0.0),
            &l(10.0, 0.0, 10.0, 10.0),
            Vec2::new(10.0, 8.0),
            2.0,
            3.0,
        )
        .unwrap();
        assert_eq!(
            (na, nb, c),
            (
                l(0.0, 0.0, 8.0, 0.0),
                l(10.0, 10.0, 10.0, 3.0),
                Some(l(8.0, 0.0, 10.0, 3.0))
            )
        );
        // 사각형 (반시계) 모서리 (10,0) 을 반지름 2 로
        let sq = Shape::Polyline {
            points: [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]
                .iter()
                .map(|&(x, y)| (Vec2::new(x, y), 0.0))
                .collect(),
            closed: true,
        };
        match corner_polyline(&sq, Vec2::new(9.0, 1.0), Corner::Fillet(2.0)).unwrap() {
            Shape::Polyline { points, .. } => {
                assert_eq!(points.len(), 5);
                assert!(
                    points[1].0.dist(Vec2::new(8.0, 0.0)) < 1e-9
                        && points[2].0.dist(Vec2::new(10.0, 2.0)) < 1e-9
                );
                // 왼쪽(반시계)으로 꺾이는 90° → 불지 tan(22.5°)
                assert!(
                    (points[1].1 - (PI / 8.0).tan()).abs() < 1e-9,
                    "{}",
                    points[1].1
                );
            }
            s => panic!("{s:?}"),
        }
        match corner_polyline(&sq, Vec2::new(0.0, 9.0), Corner::Chamfer(1.0, 1.0)).unwrap() {
            Shape::Polyline { points, .. } => assert_eq!(points.len(), 5),
            s => panic!("{s:?}"),
        }
    }
}
