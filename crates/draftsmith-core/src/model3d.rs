//! 3D 보기용 삼각형 (M6, 구현 노트 §24): 층 높이·벽 높이로 벽을 세우고, 문·창 자리는 비우고(창대 아래·인방 위는
//! 벽), 창에 유리면, 실 경계로 바닥 슬래브. 좌표는 mm, Z 가 위.
//!
//! ## 형식 `DSM4` (리틀 엔디언)
//! ```text
//! 0   [u8; 4] "DSM4"
//! 4   u32     삼각형 수 n
//! 8   f32 × 4 가운데 x, y, z, 반지름 (보기 맞춤용)
//! 24  삼각형 n 개: f32 × 9 꼭짓점, f32 × 9 꼭짓점 법선, u8 × 4 색 (76 바이트)
//! ```
//! 꼭짓점 법선 (§79): 같은 자리에서 만나는 면 중 방향이 30° 안쪽인 면들의 법선을 넓이로 평균 — 곡선 벽·돌음 계단의
//! 조각 면은 이어진 곡면으로 보이고, 벽 모서리처럼 꺾인 곳은 그대로 날카롭다.

use crate::arch::{ArchModel, OpeningKind, WallKind};
use crate::fill;
use crate::geom::Vec2;

type P3 = [f64; 3];

/// 삼각형 모음
#[derive(Debug, Default)]
pub struct Mesh {
    pub tris: Vec<([P3; 3], [u8; 4])>,
    /// 곡선(곡선 벽·돌음 계단)에서 나온 삼각형 범위와 그 호 중심 — 옆면 법선을 중심에서 뻗는 방향으로 (§79)
    pub radial: Vec<(std::ops::Range<usize>, Vec2)>,
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

    /// 바이너리 (`DSM4`)
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
        let normals = self.vertex_normals();
        let mut out = Vec::with_capacity(24 + self.tris.len() * 76);
        out.extend_from_slice(b"DSM4");
        out.extend_from_slice(&(self.tris.len() as u32).to_le_bytes());
        for v in [c[0], c[1], c[2], r] {
            out.extend_from_slice(&(v as f32).to_le_bytes());
        }
        for ((t, col), ns) in self.tris.iter().zip(&normals) {
            // 가운데를 빼서 f32 정밀도 지킴
            for p in t {
                for k in 0..3 {
                    out.extend_from_slice(&((p[k] - c[k]) as f32).to_le_bytes());
                }
            }
            for n in ns {
                for v in n {
                    out.extend_from_slice(&(*v as f32).to_le_bytes());
                }
            }
            out.extend_from_slice(col);
        }
        out
    }

    /// 꼭짓점 법선 (§79): 같은 자리(1 mm 격자)에서 만나는 면 중 방향이 30° 안쪽인 면들의 법선을 넓이로 평균
    pub fn vertex_normals(&self) -> Vec<[P3; 3]> {
        let face: Vec<(P3, f64)> = self
            .tris
            .iter()
            .map(|(t, _)| {
                let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
                let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                if l < 1e-12 {
                    ([0.0, 0.0, 1.0], 0.0)
                } else {
                    ([n[0] / l, n[1] / l, n[2] / l], l / 2.0)
                }
            })
            .collect();
        let key = |p: &P3| {
            (
                p[0].round() as i64,
                p[1].round() as i64,
                p[2].round() as i64,
            )
        };
        let mut at: std::collections::HashMap<(i64, i64, i64), Vec<usize>> =
            std::collections::HashMap::new();
        for (i, (t, _)) in self.tris.iter().enumerate() {
            for p in t {
                at.entry(key(p)).or_default().push(i);
            }
        }
        let cos = (30f64).to_radians().cos();
        let dot = |a: P3, b: P3| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let mut center: Vec<Option<Vec2>> = vec![None; self.tris.len()];
        for (r, c) in &self.radial {
            for x in &mut center[r.clone()] {
                *x = Some(*c);
            }
        }
        self.tris
            .iter()
            .enumerate()
            .map(|(i, (t, _))| {
                let ni = face[i].0;
                // 곡선의 옆면 (수평 법선이 중심에서 뻗는 방향과 거의 같은 면): 꼭짓점마다 그 자리의 반지름 방향
                if let Some(c) = center[i]
                    && ni[2].abs() < 0.1
                {
                    let m = [
                        (t[0][0] + t[1][0] + t[2][0]) / 3.0 - c.x,
                        (t[0][1] + t[1][1] + t[2][1]) / 3.0 - c.y,
                        0.0,
                    ];
                    let ml = dot(m, m).sqrt();
                    let k = if ml > 1e-9 { dot(ni, m) / ml } else { 0.0 };
                    if k.abs() > 0.9 {
                        return t.map(|p| {
                            let r = [p[0] - c.x, p[1] - c.y, 0.0];
                            let rl = dot(r, r).sqrt().max(1e-12) * k.signum();
                            [r[0] / rl, r[1] / rl, 0.0]
                        });
                    }
                }
                t.map(|p| {
                    let mut s = [0.0; 3];
                    for &j in &at[&key(&p)] {
                        let (nj, a) = face[j];
                        if a > 0.0 && dot(ni, nj) > cos {
                            for k in 0..3 {
                                s[k] += nj[k] * a;
                            }
                        }
                    }
                    let l = dot(s, s).sqrt();
                    if l < 1e-12 {
                        ni
                    } else {
                        [s[0] / l, s[1] / l, s[2] / l]
                    }
                })
            })
            .collect()
    }
}

const WALL_EXT: [u8; 4] = [214, 210, 202, 255];
const WALL_INT: [u8; 4] = [236, 234, 230, 255];
const SLAB: [u8; 4] = [196, 182, 160, 255];
const CEILING: [u8; 4] = [244, 242, 236, 255];
const GLASS: [u8; 4] = [120, 170, 210, 140];
const DOOR: [u8; 4] = [150, 110, 80, 255];
const COLUMN: [u8; 4] = [190, 186, 178, 255];
const STAIR: [u8; 4] = [205, 196, 182, 255];

/// 건축 객체 → 3D 삼각형
pub fn mesh(m: &ArchModel) -> Mesh {
    // 곡선 벽은 곧은 조각으로 (§54). 조각 → 원래 벽으로 호 중심을 찾는다 (§79)
    let orig = m;
    let (straight, seg_of) = m.straightened();
    let m = &straight;
    let arc_center = |id: u64| {
        let o = seg_of.get(&id).copied().unwrap_or(id);
        orig.walls
            .iter()
            .find(|w| w.id == o)
            .and_then(|w| w.arc())
            .map(|a| a.0)
    };
    let mut out = Mesh::default();
    for l in &m.levels {
        let z0 = l.elevation;
        // 바닥 슬래브 (두께 150, 바닥 높이 아래로)
        for r in m.rooms.iter().filter(|r| r.level == l.id) {
            let t = fill::triangulate(std::slice::from_ref(&r.boundary));
            for c in t.chunks(3) {
                // 반자(천장): 아래를 보는 면 하나 (§55)
                if let (3, Some(h)) = (c.len(), r.ceiling) {
                    let z = z0 + h;
                    out.tri(
                        [c[2].x, c[2].y, z],
                        [c[1].x, c[1].y, z],
                        [c[0].x, c[0].y, z],
                        CEILING,
                    );
                }
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
                let start = out.tris.len();
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
                out.radial.push((start..out.tris.len(), st.a));
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
            let start = out.tris.len();
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
            // 곡선 벽 조각은 모서리를 호 중심에서 뻗는 방향으로 두께/2 — 이웃 조각과 꼭짓점이 겹쳐 틈이 없다 (§79)
            let center = arc_center(w.id);
            let base = |t0: f64, t1: f64| -> [Vec2; 4] {
                let p = |t: f64, k: f64| {
                    let q = Vec2::new(w.a.x + dir.x * t, w.a.y + dir.y * t);
                    match center {
                        Some(c) => {
                            let r = Vec2::new(q.x - c.x, q.y - c.y);
                            let rl = r.x.hypot(r.y).max(1e-9);
                            let s = if r.x * n.x + r.y * n.y >= 0.0 {
                                1.0
                            } else {
                                -1.0
                            };
                            Vec2::new(q.x + r.x / rl * k * h * s, q.y + r.y / rl * k * h * s)
                        }
                        None => Vec2::new(q.x + n.x * k * h, q.y + n.y * k * h),
                    }
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
            if let Some(c) = center {
                out.radial.push((start..out.tris.len(), c));
            }
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
        assert_eq!(&b[..4], b"DSM4");
        assert_eq!(b.len(), 24 + mm.tris.len() * 76);
    }

    /// 꼭짓점 법선 (§79): 곡선 벽 옆면은 중심에서 뻗는 방향으로 이어지고, 곧은 벽 모서리는 면 법선 그대로
    #[test]
    fn smooth_normals_on_curved_walls_only() {
        let mut m = crate::arch::ArchModel::default();
        let lv = m.ensure_level();
        let wid = m.new_id();
        m.walls.push(crate::arch::Wall {
            id: wid,
            level: lv,
            a: Vec2::new(0.0, 0.0),
            b: Vec2::new(10000.0, 0.0),
            thickness: 200.0,
            height: None,
            kind: WallKind::Exterior,
            bulge: -1.0,
        });
        let c = m.walls[0].arc().unwrap().0;
        let me = mesh(&m);
        let ns = me.vertex_normals();
        let mut side = 0;
        for ((t, _), n) in me.tris.iter().zip(&ns) {
            let f = cross(sub(t[1], t[0]), sub(t[2], t[0]));
            let fl = (f[0] * f[0] + f[1] * f[1] + f[2] * f[2]).sqrt();
            if fl < 1e-9 || (f[2] / fl).abs() > 0.1 {
                continue; // 윗면·아랫면
            }
            for (p, v) in t.iter().zip(n) {
                let r = Vec2::new(p[0] - c.x, p[1] - c.y);
                let rl = r.x.hypot(r.y);
                // 그 자리의 반지름 방향 그대로 (끝 마구리 면은 방향이 90° 달라 빼고)
                let cosr = (v[0] * r.x + v[1] * r.y).abs() / rl;
                let cosf = (f[0] * r.x + f[1] * r.y).abs() / (fl * rl);
                if cosf > 0.9 {
                    side += 1;
                    assert!(cosr > 1.0 - 1e-9, "{v:?} at {p:?}");
                    assert!(v[2].abs() < 1e-9);
                }
            }
        }
        assert!(side > 100, "{side}");
        // 곧은 벽 네 개 상자: 모든 꼭짓점 법선이 제 면 법선 그대로
        let mut ed = Editor::new(new_drawing());
        apply(
            &mut ed,
            ArchReq::Walls {
                points: vec![[0.0, 0.0], [5000.0, 0.0], [5000.0, 4000.0], [0.0, 4000.0]],
                closed: true,
                thickness: 200.0,
                wall: WallKind::Exterior,
            },
        )
        .unwrap();
        let mm = mesh(&ed.d.arch);
        for ((t, _), n) in mm.tris.iter().zip(mm.vertex_normals()) {
            let f = cross(sub(t[1], t[0]), sub(t[2], t[0]));
            let fl = (f[0] * f[0] + f[1] * f[1] + f[2] * f[2]).sqrt();
            if fl < 1e-9 {
                continue;
            }
            for v in n {
                let d = (v[0] * f[0] + v[1] * f[1] + v[2] * f[2]) / fl;
                assert!(d > 0.999_999, "{d}");
            }
        }
    }
}
