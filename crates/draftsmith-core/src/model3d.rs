//! 3D 보기용 삼각형 (M6, 구현 노트 §24): 층 높이·벽 높이로 벽을 세우고, 문·창 자리는 비우고(창대 아래·인방 위는
//! 벽), 창에 유리면, 실 경계로 바닥 슬래브. 좌표는 mm, Z 가 위.
//!
//! ## 형식 `DSM3` (리틀 엔디언)
//! ```text
//! 0   [u8; 4] "DSM3"
//! 4   u32     삼각형 수 n
//! 8   f32 × 4 가운데 x, y, z, 반지름 (보기 맞춤용)
//! 24  삼각형 n 개: f32 × 9 꼭짓점, f32 × 3 법선, u8 × 4 색 (52 바이트)
//! ```

use crate::arch::{ArchModel, OpeningKind, WallKind};
use crate::fill;
use crate::geom::Vec2;

type P3 = [f64; 3];

/// 삼각형 모음
#[derive(Debug, Default)]
pub struct Mesh {
    pub tris: Vec<([P3; 3], [u8; 4])>,
}

fn sub(a: P3, b: P3) -> P3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: P3, b: P3) -> P3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

impl Mesh {
    fn tri(&mut self, a: P3, b: P3, c: P3, col: [u8; 4]) {
        self.tris.push(([a, b, c], col));
    }

    fn quad(&mut self, a: P3, b: P3, c: P3, d: P3, col: [u8; 4]) {
        self.tri(a, b, c, col);
        self.tri(a, c, d, col);
    }

    /// 바닥 네 점(반시계)과 높이 z0..z1 의 상자
    fn prism(&mut self, base: [Vec2; 4], z0: f64, z1: f64, col: [u8; 4]) {
        if z1 - z0 <= 1e-6 {
            return;
        }
        let lo = |p: Vec2| [p.x, p.y, z0];
        let hi = |p: Vec2| [p.x, p.y, z1];
        for i in 0..4 {
            let (p, q) = (base[i], base[(i + 1) % 4]);
            self.quad(lo(p), lo(q), hi(q), hi(p), col);
        }
        self.quad(hi(base[0]), hi(base[1]), hi(base[2]), hi(base[3]), col);
        self.quad(lo(base[3]), lo(base[2]), lo(base[1]), lo(base[0]), col);
    }

    /// 바이너리 (`DSM3`)
    pub fn to_bytes(&self) -> Vec<u8> {
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for (t, _) in &self.tris {
            for p in t {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
        }
        let c = if self.tris.is_empty() {
            [0.0; 3]
        } else {
            [
                (lo[0] + hi[0]) / 2.0,
                (lo[1] + hi[1]) / 2.0,
                (lo[2] + hi[2]) / 2.0,
            ]
        };
        let r = if self.tris.is_empty() {
            1.0
        } else {
            (sub(hi, lo)[0].hypot(sub(hi, lo)[1]).hypot(sub(hi, lo)[2]) / 2.0).max(1.0)
        };
        let mut out = Vec::with_capacity(24 + self.tris.len() * 52);
        out.extend_from_slice(b"DSM3");
        out.extend_from_slice(&(self.tris.len() as u32).to_le_bytes());
        for v in [c[0], c[1], c[2], r] {
            out.extend_from_slice(&(v as f32).to_le_bytes());
        }
        for (t, col) in &self.tris {
            // 가운데를 빼서 f32 정밀도 지킴
            for p in t {
                for k in 0..3 {
                    out.extend_from_slice(&((p[k] - c[k]) as f32).to_le_bytes());
                }
            }
            let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-300);
            for v in n {
                out.extend_from_slice(&((v / l) as f32).to_le_bytes());
            }
            out.extend_from_slice(col);
        }
        out
    }
}

const WALL_EXT: [u8; 4] = [214, 210, 202, 255];
const WALL_INT: [u8; 4] = [236, 234, 230, 255];
const SLAB: [u8; 4] = [196, 182, 160, 255];
const GLASS: [u8; 4] = [120, 170, 210, 140];
const DOOR: [u8; 4] = [150, 110, 80, 255];
const COLUMN: [u8; 4] = [190, 186, 178, 255];
const STAIR: [u8; 4] = [205, 196, 182, 255];

/// 건축 객체 → 3D 삼각형
pub fn mesh(m: &ArchModel) -> Mesh {
    let mut out = Mesh::default();
    for l in &m.levels {
        let z0 = l.elevation;
        // 바닥 슬래브 (두께 150, 바닥 높이 아래로)
        for r in m.rooms.iter().filter(|r| r.level == l.id) {
            let t = fill::triangulate(std::slice::from_ref(&r.boundary));
            for c in t.chunks(3) {
                if c.len() == 3 {
                    out.tri(
                        [c[0].x, c[0].y, z0],
                        [c[1].x, c[1].y, z0],
                        [c[2].x, c[2].y, z0],
                        SLAB,
                    );
                    out.tri(
                        [c[2].x, c[2].y, z0 - 150.0],
                        [c[1].x, c[1].y, z0 - 150.0],
                        [c[0].x, c[0].y, z0 - 150.0],
                        SLAB,
                    );
                }
            }
        }
        // 기둥 (층고만큼)
        for c in m.columns.iter().filter(|c| c.level == l.id) {
            out.prism(c.corners(), z0, z0 + l.height, COLUMN);
        }
        // 보 (윗면이 천장, §52)
        for x in m.beams.iter().filter(|x| x.level == l.id) {
            let top = z0 + l.height;
            out.prism(x.corners(), top - x.depth, top, COLUMN);
        }
        // 계단: 단마다 바닥에서 그 단 높이까지 상자, 참은 첫 줄기 끝 높이까지 (§34)
        for st in m.stairs.iter().filter(|s| s.level == l.id) {
            // 돌음 계단: 단마다 부채꼴(네 모서리) 상자 (§43)
            if let Some((ri, ro, steps)) = st.spiral_steps() {
                let rise = l.height / st.risers.max(1) as f64;
                let at = |r: f64, t: f64| Vec2::new(st.a.x + r * t.cos(), st.a.y + r * t.sin());
                for (k, (ta, tb)) in steps.iter().enumerate() {
                    let q = if tb >= ta {
                        [at(ri, *ta), at(ro, *ta), at(ro, *tb), at(ri, *tb)]
                    } else {
                        [at(ri, *tb), at(ro, *tb), at(ro, *ta), at(ri, *ta)]
                    };
                    out.prism(q, z0, z0 + rise * (k + 1) as f64, STAIR);
                }
                continue;
            }
            let Some((flights, landing)) = st.flights() else {
                continue;
            };
            let rise = l.height / st.risers.max(1) as f64;
            for f in &flights {
                let n = Vec2::new(-f.u.y * st.width / 2.0, f.u.x * st.width / 2.0);
                let q = |t: f64, k: f64| {
                    Vec2::new(
                        f.start.x + f.u.x * t + n.x * k,
                        f.start.y + f.u.y * t + n.y * k,
                    )
                };
                let r = f.risers.max(1) as f64;
                for k in 0..f.risers {
                    let (t0, t1) = (f.len * k as f64 / r, f.len * (k + 1) as f64 / r);
                    out.prism(
                        [q(t0, -1.0), q(t1, -1.0), q(t1, 1.0), q(t0, 1.0)],
                        z0,
                        z0 + rise * (f.first + k + 1) as f64,
                        STAIR,
                    );
                }
            }
            if let Some(sq) = landing {
                out.prism(sq, z0, z0 + rise * flights[0].risers as f64, STAIR);
            }
        }
        for w in m.walls.iter().filter(|w| w.level == l.id) {
            let len = w.length();
            if len < 1e-6 {
                continue;
            }
            let top = z0 + w.height.unwrap_or(l.height);
            let dir = w.dir();
            let n = Vec2::new(-dir.y, dir.x);
            let h = w.thickness / 2.0;
            let col = if w.kind == WallKind::Interior {
                WALL_INT
            } else {
                WALL_EXT
            };
            let base = |t0: f64, t1: f64| -> [Vec2; 4] {
                let p = |t: f64, k: f64| {
                    Vec2::new(
                        w.a.x + dir.x * t + n.x * k * h,
                        w.a.y + dir.y * t + n.y * k * h,
                    )
                };
                [p(t0, -1.0), p(t1, -1.0), p(t1, 1.0), p(t0, 1.0)]
            };
            let mut gaps: Vec<_> = m
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
            let mut cur = 0.0;
            for (s, e, o) in &gaps {
                out.prism(base(cur, *s), z0, top, col);
                let (bottom, head) = match o.kind {
                    OpeningKind::Door => (z0, z0 + o.height),
                    OpeningKind::Window => (z0 + o.sill, z0 + o.sill + o.height),
                };
                // 창대 아래, 인방 위
                out.prism(base(*s, *e), z0, bottom.min(top), col);
                out.prism(base(*s, *e), head.min(top), top, col);
                // 유리·문짝 (벽 가운데 얇은 판)
                let p = |t: f64| Vec2::new(w.a.x + dir.x * t, w.a.y + dir.y * t);
                let (a, b) = (p(*s), p(*e));
                let fill_col = if o.kind == OpeningKind::Window {
                    GLASS
                } else {
                    DOOR
                };
                if o.kind == OpeningKind::Window {
                    out.quad(
                        [a.x, a.y, bottom],
                        [b.x, b.y, bottom],
                        [b.x, b.y, head.min(top)],
                        [a.x, a.y, head.min(top)],
                        fill_col,
                    );
                    out.quad(
                        [a.x, a.y, head.min(top)],
                        [b.x, b.y, head.min(top)],
                        [b.x, b.y, bottom],
                        [a.x, a.y, bottom],
                        fill_col,
                    );
                }
                cur = *e;
            }
            out.prism(base(cur, len), z0, top, col);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archgen::{ArchReq, apply, new_drawing};
    use crate::edit::Editor;

    #[test]
    fn walls_with_window_become_triangles() {
        let mut ed = Editor::new(new_drawing());
        apply(
            &mut ed,
            ArchReq::Walls {
                points: vec![[0.0, 0.0], [4000.0, 0.0]],
                closed: false,
                thickness: 200.0,
                wall: WallKind::Exterior,
            },
        )
        .unwrap();
        apply(
            &mut ed,
            ArchReq::Opening {
                at: [2000.0, 0.0],
                opening: OpeningKind::Window,
                width: 1000.0,
                height: 1200.0,
                sill: 900.0,
            },
        )
        .unwrap();
        let mm = mesh(&ed.d.arch);
        // 상자 4 개(좌·창대 아래·인방 위·우) × 12 + 유리 4
        assert_eq!(mm.tris.len(), 4 * 12 + 4);
        let b = mm.to_bytes();
        assert_eq!(&b[..4], b"DSM3");
        assert_eq!(b.len(), 24 + mm.tris.len() * 52);
    }
}
