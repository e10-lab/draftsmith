//! 화면용 표시 데이터.
//!
//! 도면을 선분 목록으로 펼쳐 바이너리 하나로 만든다. 앱은 이것을 Tauri 명령의 바이너리 응답으로
//! 화면에 넘기고, 화면은 그대로 WebGL2 버퍼에 올린다 (JSON 변환 없음 — M0 ① 측정 대상).
//!
//! 좌표는 도면 중심(`origin`)을 뺀 f32 로 넣는다. 도면 좌표가 TM 좌표처럼 커도 GPU 의
//! f32 정밀도에서 떨리지 않게 하기 위해서다 (원점 재설정).
//!
//! ## 형식 (리틀 엔디언)
//! ```text
//! 0   [u8; 4]  "DSD1"
//! 4   u32      선분 개수 n
//! 8   f64 × 2  origin x, y            (도면 좌표)
//! 24  f32 × 4  범위 min x, min y, max x, max y (origin 기준)
//! 40  f32 × 4n 선분 x0, y0, x1, y1    (origin 기준)
//! ..  u8 × 4n  선분 색 r, g, b, a
//! ```
//! 문자는 아직 글꼴 처리 전이라 글자 상자(밑줄 사각형)로 그린다 — M0 ⑥.

use std::f64::consts::TAU;

use crate::drawing::{Drawing, Shape};
use crate::geom::{Bbox, Vec2};

pub const MAGIC: &[u8; 4] = b"DSD1";
pub const HEADER_LEN: usize = 40;

/// 펼친 선분 목록
#[derive(Debug, Default)]
pub struct Segments {
    pub origin: Vec2,
    pub bbox: Bbox,
    /// x0, y0, x1, y1 (origin 기준)
    pub coords: Vec<f32>,
    /// r, g, b, a
    pub colors: Vec<u8>,
}

impl Segments {
    pub fn len(&self) -> usize {
        self.coords.len() / 4
    }

    pub fn is_empty(&self) -> bool {
        self.coords.is_empty()
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
    }

    /// 형식에 맞춘 바이너리
    pub fn to_bytes(&self) -> Vec<u8> {
        let n = self.len();
        let mut out = Vec::with_capacity(HEADER_LEN + n * 20);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(n as u32).to_le_bytes());
        out.extend_from_slice(&self.origin.x.to_le_bytes());
        out.extend_from_slice(&self.origin.y.to_le_bytes());
        let o = self.origin;
        let (lo, hi) = if self.bbox.is_empty() {
            (Vec2::default(), Vec2::default())
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
        out
    }
}

/// 원호를 나눌 조각 수: 화살 높이(현과 호 사이)가 `tol` 이하가 되게, 8–512 조각
fn arc_steps(radius: f64, sweep: f64, tol: f64) -> usize {
    if radius <= tol {
        return 8;
    }
    let step = 2.0 * (1.0 - tol / radius).clamp(-1.0, 1.0).acos();
    ((sweep.abs() / step.max(1e-6)).ceil() as usize).clamp(8, 512)
}

fn push_arc(s: &mut Segments, c: Vec2, r: f64, start: f64, sweep: f64, tol: f64, col: [u8; 4]) {
    let n = arc_steps(r, sweep, tol);
    let at = |t: f64| Vec2::new(c.x + r * t.cos(), c.y + r * t.sin());
    let mut prev = at(start);
    for i in 1..=n {
        let p = at(start + sweep * i as f64 / n as f64);
        s.push(prev, p, col);
        prev = p;
    }
}

/// 불지로 두 점 사이 호를 그린다 (불지 = tan(사이각/4))
fn push_bulge(s: &mut Segments, a: Vec2, b: Vec2, bulge: f64, tol: f64, col: [u8; 4]) {
    let chord = a.dist(b);
    if bulge.abs() < 1e-9 || chord < 1e-12 {
        s.push(a, b, col);
        return;
    }
    let sweep = 4.0 * bulge.atan();
    let r = chord / (2.0 * (sweep / 2.0).sin().abs());
    // 현의 중점에서 수직으로 중심까지
    let m = a.lerp(b, 0.5);
    let d = (r * r - (chord / 2.0).powi(2)).max(0.0).sqrt();
    let (ux, uy) = ((b.x - a.x) / chord, (b.y - a.y) / chord);
    let side = if (bulge > 0.0) == (sweep.abs() < std::f64::consts::PI) {
        1.0
    } else {
        -1.0
    };
    let c = Vec2::new(m.x - uy * d * side, m.y + ux * d * side);
    let start = (a.y - c.y).atan2(a.x - c.x);
    push_arc(s, c, r, start, sweep, tol, col);
}

/// 도면을 선분으로 펼친다. 꺼진 레이어는 뺀다.
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

    for e in &d.entities {
        if d.layers.get(e.layer as usize).is_some_and(|l| !l.visible) {
            continue;
        }
        let [r, g, b] = d.color_of(e);
        let col = [r, g, b, 255];
        match &e.shape {
            Shape::Line { a, b } => s.push(*a, *b, col),
            Shape::Polyline { points, closed } => {
                let n = points.len();
                let last = if *closed { n } else { n.saturating_sub(1) };
                for i in 0..last {
                    let (p, bulge) = points[i];
                    let (q, _) = points[(i + 1) % n];
                    push_bulge(&mut s, p, q, bulge, tol, col);
                }
            }
            Shape::Circle { center, radius } => {
                push_arc(&mut s, *center, *radius, 0.0, TAU, tol, col)
            }
            Shape::Arc {
                center,
                radius,
                start,
                end,
            } => {
                let mut sweep = end - start;
                if sweep <= 0.0 {
                    sweep += TAU;
                }
                push_arc(&mut s, *center, *radius, *start, sweep, tol, col);
            }
            Shape::Ellipse {
                center,
                u,
                v,
                start,
                end,
            } => {
                let mut sweep = end - start;
                if sweep <= 0.0 {
                    sweep += TAU;
                }
                // 조각 수는 긴 쪽 반지름으로
                let r = u.x.hypot(u.y).max(v.x.hypot(v.y));
                let n = arc_steps(r, sweep, tol);
                let at = |t: f64| {
                    let (sn, cs) = t.sin_cos();
                    Vec2::new(
                        center.x + u.x * cs + v.x * sn,
                        center.y + u.y * cs + v.y * sn,
                    )
                };
                let mut prev = at(*start);
                for i in 1..=n {
                    let p = at(start + sweep * i as f64 / n as f64);
                    s.push(prev, p, col);
                    prev = p;
                }
            }
            Shape::Text {
                at,
                height,
                rotation,
                value,
            } => {
                // 글자 상자 밑줄: 글꼴 처리 전 자리 표시 (M0 ⑥)
                let w = value.chars().count() as f64 * height;
                let (cs, sn) = (rotation.cos(), rotation.sin());
                let end = Vec2::new(at.x + w * cs, at.y + w * sn);
                s.push(*at, end, col);
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawing::{Drawing, Entity, Shape};

    fn one(shape: Shape) -> Drawing {
        let mut d = Drawing::default();
        let layer = d.layer_index("0");
        d.entities.push(Entity {
            layer,
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
    }

    #[test]
    fn bytes_layout() {
        let d = one(Shape::Circle {
            center: Vec2::new(0.0, 0.0),
            radius: 1.0,
        });
        let s = segments(&d);
        let b = s.to_bytes();
        assert_eq!(&b[..4], MAGIC);
        let n = u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize;
        assert_eq!(n, s.len());
        assert_eq!(b.len(), HEADER_LEN + n * 20);
    }
}
