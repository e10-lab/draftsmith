//! 화면용 표시 데이터.
//!
//! 도면을 선분·삼각형 목록으로 펼쳐 바이너리 하나로 만든다. 앱은 이것을 Tauri 명령의 바이너리 응답으로
//! 화면에 넘기고, 화면은 그대로 WebGL2 버퍼에 올린다 (JSON 변환 없음 — 구현 노트 §4).
//! 문자는 따로 [`texts`] 가 만든다 (화면이 글꼴로 그림).
//!
//! 좌표는 도면 중심(`origin`)을 뺀 f32 로 넣는다. 도면 좌표가 TM 좌표처럼 커도 GPU 의
//! f32 정밀도에서 떨리지 않게 하기 위해서다 (원점 재설정).
//!
//! ## 선·채움 형식 `DSD2` (리틀 엔디언)
//! ```text
//! 0   [u8; 4]  "DSD2"
//! 4   u32      선분 개수 n
//! 8   u32      삼각형 개수 m
//! 12  f64 × 2  origin x, y            (도면 좌표)
//! 28  f32 × 4  범위 min x, min y, max x, max y (origin 기준)
//! 44  f32 × 4n 선분 x0, y0, x1, y1
//! ..  u8 × 4n  선분 색 r, g, b, a
//! ..  f32 × 6m 삼각형 x0, y0, x1, y1, x2, y2
//! ..  u8 × 4m  삼각형 색
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
use crate::drawing::{Drawing, Shape, VAlign};
use crate::fill;
use crate::geom::{Bbox, Vec2};

pub const MAGIC: &[u8; 4] = b"DSD2";
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
    }

    /// 형식에 맞춘 바이너리
    pub fn to_bytes(&self) -> Vec<u8> {
        let (n, m) = (self.len(), self.tri_count());
        let mut out = Vec::with_capacity(HEADER_LEN + n * 20 + m * 28);
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
    let mut pts = Vec::new();

    for e in &d.entities {
        if d.layers.get(e.layer as usize).is_some_and(|l| !l.visible) {
            continue;
        }
        let [r, g, b] = d.color_of(e);
        let col = [r, g, b, 255];
        match &e.shape {
            Shape::Line { a, b } => s.push(*a, *b, col),
            Shape::Polyline { points, closed } => {
                pts = curve::polyline_points(points, *closed, tol);
                s.strip(&pts, *closed, col);
            }
            Shape::Circle { center, radius } => {
                pts.clear();
                let (u, v) = (Vec2::new(*radius, 0.0), Vec2::new(0.0, *radius));
                curve::ellipse_points(*center, u, v, 0.0, TAU, tol, &mut pts);
                s.strip(&pts, false, col);
            }
            Shape::Arc {
                center,
                radius,
                start,
                end,
            } => {
                pts.clear();
                let (u, v) = (Vec2::new(*radius, 0.0), Vec2::new(0.0, *radius));
                let sweep = curve::ccw_sweep(*start, *end);
                curve::ellipse_points(*center, u, v, *start, sweep, tol, &mut pts);
                s.strip(&pts, false, col);
            }
            Shape::Ellipse {
                center,
                u,
                v,
                start,
                end,
            } => {
                pts.clear();
                let sweep = curve::ccw_sweep(*start, *end);
                curve::ellipse_points(*center, *u, *v, *start, sweep, tol, &mut pts);
                s.strip(&pts, false, col);
            }
            Shape::Fill { loops } => {
                let t = fill::triangulate(loops);
                for tri in t.chunks_exact(3) {
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
    }
    s
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
            color: None,
            shape: Shape::Text(TextShape {
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
        assert_eq!(b.len(), HEADER_LEN + n * 20 + m * 28);
        let t = texts(&d);
        assert_eq!(&t[..4], TEXT_MAGIC);
        assert_eq!(u32::from_le_bytes(t[4..8].try_into().unwrap()), 1);
        assert_eq!(&t[t.len() - 6..], "거실".as_bytes());
    }
}
