//! 곡선을 꼭짓점으로 펼치기 — 원호·불지·타원호·스플라인.
//!
//! 가져오기(해치 경계·스플라인)와 표시 데이터가 함께 쓴다.

use std::f64::consts::{PI, TAU};

use crate::geom::Vec2;

/// 원호를 나눌 조각 수: 화살 높이(현과 호 사이)가 `tol` 이하가 되게, 최대 512 조각.
/// 최소는 45° 마다 한 조각 (온 원 8 · 반원 4 · 사분원 2) — 전에는 작은 호도 늘 8 조각이라 단열재 표시처럼 작은 호가
/// 수백 개인 폴리선이 선분 수십만 개가 됐다 (구현 노트 §64)
pub fn arc_steps(radius: f64, sweep: f64, tol: f64) -> usize {
    let min = ((sweep.abs() / (PI / 4.0)).ceil() as usize).clamp(1, 8);
    if radius <= tol || tol <= 0.0 {
        return min;
    }
    let step = 2.0 * (1.0 - tol / radius).clamp(-1.0, 1.0).acos();
    ((sweep.abs() / step.max(1e-6)).ceil() as usize).clamp(min, 512)
}

/// `center + u·cos t + v·sin t` 를 t = start 부터 start + sweep 까지 (sweep 이 음수면 시계 방향).
/// 시작점을 포함해 `n + 1` 개 점을 `out` 에 넣는다.
pub fn ellipse_points(
    center: Vec2,
    u: Vec2,
    v: Vec2,
    start: f64,
    sweep: f64,
    tol: f64,
    out: &mut Vec<Vec2>,
) {
    let r = u.x.hypot(u.y).max(v.x.hypot(v.y));
    let n = arc_steps(r, sweep, tol);
    for i in 0..=n {
        let (s, c) = (start + sweep * i as f64 / n as f64).sin_cos();
        out.push(Vec2::new(
            center.x + u.x * c + v.x * s,
            center.y + u.y * c + v.y * s,
        ));
    }
}

/// 반시계 방향 호의 사이각 (end ≤ start 면 한 바퀴 더해 양수로)
pub fn ccw_sweep(start: f64, end: f64) -> f64 {
    let mut s = end - start;
    while s <= 0.0 {
        s += TAU;
    }
    while s > TAU {
        s -= TAU;
    }
    s
}

/// 불지 호: 두 점과 불지(= tan(사이각/4))로 중심·반지름·시작각·사이각을 구한다. 직선이면 None.
pub fn bulge_arc(a: Vec2, b: Vec2, bulge: f64) -> Option<(Vec2, f64, f64, f64)> {
    let chord = a.dist(b);
    if bulge.abs() < 1e-9 || chord < 1e-12 {
        return None;
    }
    let sweep = 4.0 * bulge.atan();
    let r = chord / (2.0 * (sweep / 2.0).sin().abs());
    let m = a.lerp(b, 0.5);
    let h = (r * r - (chord / 2.0).powi(2)).max(0.0).sqrt();
    let (ux, uy) = ((b.x - a.x) / chord, (b.y - a.y) / chord);
    // 반원보다 작은 호는 중심이 진행 방향 왼쪽(불지 양수일 때), 큰 호는 반대쪽
    let side = if (bulge > 0.0) == (sweep.abs() < PI) {
        1.0
    } else {
        -1.0
    };
    let c = Vec2::new(m.x - uy * h * side, m.y + ux * h * side);
    let start = (a.y - c.y).atan2(a.x - c.x);
    Some((c, r, start, sweep))
}

/// 불지 폴리선 한 변을 점으로 (시작점 포함, 끝점 `b` 제외)
pub fn bulge_points(a: Vec2, b: Vec2, bulge: f64, tol: f64, out: &mut Vec<Vec2>) {
    match bulge_arc(a, b, bulge) {
        None => out.push(a),
        Some((c, r, start, sweep)) => {
            let u = Vec2::new(r, 0.0);
            let v = Vec2::new(0.0, r);
            ellipse_points(c, u, v, start, sweep, tol, out);
            out.pop();
        }
    }
}

/// 불지 폴리선 전체를 점으로. 닫혀 있으면 첫 점으로 돌아오는 변까지 (마지막 점 = 첫 점은 넣지 않음).
pub fn polyline_points(points: &[(Vec2, f64)], closed: bool, tol: f64) -> Vec<Vec2> {
    let n = points.len();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let (p, g) = points[i];
        if i + 1 < n {
            bulge_points(p, points[i + 1].0, g, tol, &mut out);
        } else if closed {
            bulge_points(p, points[0].0, g, tol, &mut out);
        } else {
            out.push(p);
        }
    }
    out
}

/// NURBS 스플라인 위의 점들 (de Boor). 매듭·가중치가 맞지 않으면 조정점을 그대로 잇는다.
/// `steps_per_span` 은 매듭 구간마다 나눌 조각 수.
pub fn spline_points(
    degree: usize,
    knots: &[f64],
    ctrl: &[Vec2],
    weights: &[f64],
    steps_per_span: usize,
) -> Vec<Vec2> {
    let n = ctrl.len();
    if degree == 0 || n <= degree || knots.len() != n + degree + 1 {
        return ctrl.to_vec();
    }
    let w = |i: usize| if weights.len() == n { weights[i] } else { 1.0 };
    let (lo, hi) = (knots[degree], knots[n]);
    if hi.partial_cmp(&lo) != Some(std::cmp::Ordering::Greater) {
        return ctrl.to_vec();
    }
    let spans = (degree..n)
        .filter(|&i| knots[i + 1] > knots[i])
        .count()
        .max(1);
    let total = spans * steps_per_span.max(1);
    let mut out = Vec::with_capacity(total + 1);
    for s in 0..=total {
        let t = lo + (hi - lo) * s as f64 / total as f64;
        // 매듭 구간 k: knots[k] ≤ t < knots[k+1] (끝은 마지막 구간)
        let mut k = degree;
        while k + 1 < n && t >= knots[k + 1] {
            k += 1;
        }
        // 동차 좌표 (x·w, y·w, w)
        let mut d: Vec<[f64; 3]> = (0..=degree)
            .map(|j| {
                let i = k - degree + j;
                let wi = w(i);
                [ctrl[i].x * wi, ctrl[i].y * wi, wi]
            })
            .collect();
        for r in 1..=degree {
            for j in (r..=degree).rev() {
                let i = k - degree + j;
                let den = knots[i + degree + 1 - r] - knots[i];
                let a = if den.abs() < 1e-300 {
                    0.0
                } else {
                    (t - knots[i]) / den
                };
                let prev = d[j - 1];
                for (v, p) in d[j].iter_mut().zip(prev) {
                    *v = (1.0 - a) * p + a * *v;
                }
            }
        }
        let [x, y, ww] = d[degree];
        if ww.abs() > 1e-300 {
            out.push(Vec2::new(x / ww, y / ww));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quadratic_bezier_as_spline() {
        // 2 차, 매듭 [0,0,0,1,1,1] = 베지어. 중간점 = (0.25 p0 + 0.5 p1 + 0.25 p2)
        let c = [
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 2.0),
            Vec2::new(2.0, 0.0),
        ];
        let pts = spline_points(2, &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0], &c, &[], 2);
        assert_eq!(pts.len(), 3);
        assert!(
            (pts[1].x - 1.0).abs() < 1e-12 && (pts[1].y - 1.0).abs() < 1e-12,
            "{pts:?}"
        );
        assert_eq!(pts[2], Vec2::new(2.0, 0.0));
    }

    #[test]
    fn rational_quarter_circle() {
        // 가중치 [1, √½, 1] 인 2 차 스플라인 = 정확한 사분원
        let c = [
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
        ];
        let w = [1.0, std::f64::consts::FRAC_1_SQRT_2, 1.0];
        for p in spline_points(2, &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0], &c, &w, 16) {
            assert!((p.x.hypot(p.y) - 1.0).abs() < 1e-12, "{p:?}");
        }
    }
}
