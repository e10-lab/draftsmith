//! 채움 영역: 짝홀(even-odd) 규칙 삼각형 분할과 해치 무늬 선 만들기.
//!
//! 해치 경계는 고리(닫힌 꼭짓점 목록) 여러 개다. 짝홀 규칙 — 점을 감싸는 고리 수가 홀수면 안쪽.
//! AutoCAD 의 기본(Normal) 해치 스타일과 같은 결과다.

use earcut::Earcut;

use crate::geom::Vec2;

/// 점이 고리 안에 있나 (짝홀 반직선 검사)
pub fn inside(p: Vec2, ring: &[Vec2]) -> bool {
    let mut c = false;
    let n = ring.len();
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            c = !c;
        }
    }
    c
}

fn area(ring: &[Vec2]) -> f64 {
    let n = ring.len();
    (0..n)
        .map(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            a.x * b.y - b.x * a.y
        })
        .sum::<f64>()
        / 2.0
}

/// 짝홀 규칙으로 삼각형 분할. 결과는 삼각형 꼭짓점 목록 (3 개씩).
///
/// 고리마다 다른 고리 몇 개 안에 있는지(깊이)를 세어, 깊이가 짝수면 바깥 고리, 홀수면 구멍이다.
/// 구멍은 자기를 감싸는 바깥 고리 중 가장 작은 것에 붙여 earcut 으로 나눈다.
pub fn triangulate(loops: &[Vec<Vec2>]) -> Vec<Vec2> {
    let rings: Vec<&Vec<Vec2>> = loops.iter().filter(|r| r.len() >= 3).collect();
    let depth: Vec<usize> = rings
        .iter()
        .enumerate()
        .map(|(i, r)| {
            rings
                .iter()
                .enumerate()
                .filter(|&(j, o)| j != i && inside(r[0], o))
                .count()
        })
        .collect();
    let areas: Vec<f64> = rings.iter().map(|r| area(r).abs()).collect();
    let mut holes: Vec<Vec<usize>> = vec![Vec::new(); rings.len()];
    for (i, r) in rings.iter().enumerate() {
        if depth[i] % 2 == 1 {
            // 감싸는 바깥 고리(깊이 = 내 깊이 − 1) 중 가장 작은 것
            let host = (0..rings.len())
                .filter(|&j| depth[j] + 1 == depth[i] && inside(r[0], rings[j]))
                .min_by(|&a, &b| areas[a].total_cmp(&areas[b]));
            if let Some(h) = host {
                holes[h].push(i);
            }
        }
    }
    let mut out = Vec::new();
    let mut ec = Earcut::new();
    let mut idx: Vec<u32> = Vec::new();
    for (i, r) in rings.iter().enumerate() {
        if depth[i] % 2 == 1 {
            continue;
        }
        let mut pts: Vec<[f64; 2]> = r.iter().map(|p| [p.x, p.y]).collect();
        let mut hole_starts: Vec<u32> = Vec::new();
        for &h in &holes[i] {
            hole_starts.push(pts.len() as u32);
            pts.extend(rings[h].iter().map(|p| [p.x, p.y]));
        }
        ec.earcut(pts.iter().copied(), &hole_starts, &mut idx);
        out.extend(idx.iter().map(|&k| {
            let [x, y] = pts[k as usize];
            Vec2::new(x, y)
        }));
    }
    out
}

/// 해치 무늬 선 한 묶음 (DXF 에 저장된 값 — 축척·각도가 이미 적용됨)
#[derive(Debug, Clone, PartialEq)]
pub struct PatternLine {
    /// 선 방향 (라디안)
    pub angle: f64,
    /// 기준점
    pub base: Vec2,
    /// 다음 선까지의 이동 (선 방향 성분은 엇갈림, 수직 성분은 간격)
    pub offset: Vec2,
    /// 대시 길이: 양수 = 선, 음수 = 빈칸, 0 = 점
    pub dashes: Vec<f64>,
}

/// 해치 무늬 선분을 만든다 (짝홀 규칙으로 경계 안만). 선분이 `limit` 개를 넘으면 None (너무 촘촘함).
pub fn pattern_segments(
    loops: &[Vec<Vec2>],
    lines: &[PatternLine],
    limit: usize,
) -> Option<Vec<(Vec2, Vec2)>> {
    let edges: Vec<(Vec2, Vec2)> = loops
        .iter()
        .filter(|r| r.len() >= 2)
        .flat_map(|r| (0..r.len()).map(move |i| (r[i], r[(i + 1) % r.len()])))
        .collect();
    if edges.is_empty() {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    for pl in lines {
        let d = Vec2::new(pl.angle.cos(), pl.angle.sin());
        let n = Vec2::new(-d.y, d.x);
        let gap = pl.offset.x * n.x + pl.offset.y * n.y; // 선 사이 수직 간격
        if gap.abs() < 1e-9 {
            continue;
        }
        // 경계 꼭짓점들을 수직 방향으로 투영해 필요한 선 번호 범위를 구한다
        let proj = |p: Vec2| (p.x - pl.base.x) * n.x + (p.y - pl.base.y) * n.y;
        let (mut smin, mut smax) = (f64::INFINITY, f64::NEG_INFINITY);
        for (a, _) in &edges {
            let s = proj(*a);
            smin = smin.min(s);
            smax = smax.max(s);
        }
        let (k0, k1) = {
            let (a, b) = (smin / gap, smax / gap);
            (a.min(b).ceil() as i64, a.max(b).floor() as i64)
        };
        if (k1 - k0) as usize > limit {
            return None;
        }
        let period: f64 = pl.dashes.iter().map(|x| x.abs()).sum();
        let mut ts = Vec::new();
        for k in k0..=k1 {
            let s = k as f64 * gap;
            // 이 선 위의 원점: base + k·offset  (선 방향 좌표 t 의 0)
            let o = Vec2::new(
                pl.base.x + pl.offset.x * k as f64,
                pl.base.y + pl.offset.y * k as f64,
            );
            ts.clear();
            for (a, b) in &edges {
                let (sa, sb) = (proj(*a) - s, proj(*b) - s);
                if (sa > 0.0) != (sb > 0.0) {
                    let p = a.lerp(*b, sa / (sa - sb));
                    ts.push((p.x - o.x) * d.x + (p.y - o.y) * d.y);
                }
            }
            ts.sort_by(f64::total_cmp);
            let at = |t: f64| Vec2::new(o.x + d.x * t, o.y + d.y * t);
            for span in ts.chunks_exact(2) {
                let (t0, t1) = (span[0], span[1]);
                if period < 1e-12 {
                    out.push((at(t0), at(t1)));
                } else {
                    // 대시 무늬는 선 원점 o 에서 시작한다 (선마다 엇갈림은 o = base + k·offset 에 이미 들어 있음)
                    let mut c = (t0 / period).floor() * period;
                    while c < t1 {
                        let mut pos = c;
                        for &len in &pl.dashes {
                            let (a, b) = (pos, pos + len.abs());
                            if len >= 0.0 {
                                // 0 길이 = 점: 아주 짧은 선으로
                                let b = if len == 0.0 { a + period * 1e-3 } else { b };
                                let (s0, s1) = (a.max(t0), b.min(t1));
                                if s1 > s0 {
                                    out.push((at(s0), at(s1)));
                                }
                            }
                            pos = b;
                        }
                        c += period;
                        if out.len() > limit {
                            return None;
                        }
                    }
                }
            }
            if out.len() > limit {
                return None;
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f64, y: f64, s: f64) -> Vec<Vec2> {
        vec![
            Vec2::new(x, y),
            Vec2::new(x + s, y),
            Vec2::new(x + s, y + s),
            Vec2::new(x, y + s),
        ]
    }

    fn tri_area(t: &[Vec2]) -> f64 {
        t.chunks(3)
            .map(|c| {
                ((c[1].x - c[0].x) * (c[2].y - c[0].y) - (c[2].x - c[0].x) * (c[1].y - c[0].y))
                    .abs()
                    / 2.0
            })
            .sum()
    }

    #[test]
    fn square_with_hole_and_island() {
        // 10×10 바깥, 6×6 구멍, 그 안에 2×2 섬 → 100 − 36 + 4 = 68
        let t = triangulate(&[
            square(0.0, 0.0, 10.0),
            square(2.0, 2.0, 6.0),
            square(4.0, 4.0, 2.0),
        ]);
        assert!((tri_area(&t) - 68.0).abs() < 1e-9, "{}", tri_area(&t));
    }

    #[test]
    fn horizontal_lines_clipped_to_square_with_hole() {
        let lines = [PatternLine {
            angle: 0.0,
            base: Vec2::default(),
            offset: Vec2::new(0.0, 1.0),
            dashes: vec![],
        }];
        let segs = pattern_segments(
            &[square(0.0, 0.0, 10.0), square(4.0, 4.0, 2.0)],
            &lines,
            10_000,
        )
        .unwrap();
        // 반열림 규칙: 경계 위의 선은 아래쪽 변(y=0, 구멍 y=4)에서만 걸린다 → y = 0..9 의 10 개 = 100,
        // 구멍(4..6)을 지나는 y=4·5 는 2 씩 빠짐 → 96. y=5 는 둘로 나뉨
        let total: f64 = segs.iter().map(|(a, b)| a.dist(*b)).sum();
        let at5: Vec<_> = segs
            .iter()
            .filter(|(a, _)| (a.y - 5.0).abs() < 1e-9)
            .collect();
        assert_eq!(at5.len(), 2);
        assert!((total - 96.0).abs() < 1e-9, "{total}");
    }

    #[test]
    fn dashes_and_limit() {
        let lines = [PatternLine {
            angle: 0.0,
            base: Vec2::default(),
            offset: Vec2::new(0.0, 1.0),
            dashes: vec![1.0, -1.0],
        }];
        let segs = pattern_segments(&[square(0.0, 0.5, 10.0)], &lines, 10_000).unwrap();
        // 선마다 길이 10 에 대시 1·빈칸 1 → 5 개
        assert_eq!(segs.len(), 10 * 5);
        assert!(pattern_segments(&[square(0.0, 0.5, 10.0)], &lines, 10).is_none());
    }
}
