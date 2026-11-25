//! 단면 자동 (M6, 구현 노트 §38): 평면의 건축 객체를 자르는 선으로 잘라 2D 단면도를 만든다.
//!
//! 좌표: 단면도의 x 는 자르는 선 a 에서의 거리, y 는 높이(층 바닥 높이 기준 절대). `Section::at` 만큼 옮겨 놓는다.
//! - 벽: 자르는 선과 벽 중심선이 만나는 곳에 벽 두께(비스듬하면 두께/|sin|)만큼 칠한 단면. 그 자리에 문이 있으면
//!   인방 위만, 창이면 창대 아래·인방 위 벽과 유리선.
//! - 바닥판: 실 경계가 자르는 선과 겹치는 구간에 두께 150.
//! - 계단: 줄기가 자르는 선과 나란하고 선이 줄기 폭 안을 지나면 계단 단면(계단 모양 선).
//! - 층 바닥선(점선 대신 가는 선)과 층 이름·높이, 지반선, 제목.

use crate::arch::{ArchModel, OpeningKind, Section};
use crate::drawing::{Shape, TextShape, VAlign};
use crate::geom::Vec2;

/// 단면도 모양들: (레이어 이름, 모양)
pub fn draw(m: &ArchModel, s: &Section) -> Vec<(&'static str, Shape)> {
    let mut out: Vec<(&'static str, Shape)> = Vec::new();
    let len = s.a.dist(s.b);
    if len < 1e-6 {
        return out;
    }
    let u = Vec2::new((s.b.x - s.a.x) / len, (s.b.y - s.a.y) / len);
    let o = s.at;
    let p = |t: f64, z: f64| Vec2::new(o.x + t, o.y + z);
    let rect =
        |t0: f64, t1: f64, z0: f64, z1: f64| vec![p(t0, z0), p(t1, z0), p(t1, z1), p(t0, z1)];
    let cut = |out: &mut Vec<(&'static str, Shape)>, t0: f64, t1: f64, z0: f64, z1: f64| {
        if z1 - z0 > 1e-6 && t1 - t0 > 1e-6 {
            let r = rect(t0, t1, z0, z1);
            out.push((
                "A-SECT-PATT",
                Shape::Fill {
                    loops: vec![r.clone()],
                },
            ));
            out.push((
                "A-SECT",
                Shape::Polyline {
                    points: r.into_iter().map(|q| (q, 0.0)).collect(),
                    closed: true,
                },
            ));
        }
    };
    // 2D 선분 교차: 자르는 선 위 t (a 에서 거리), 상대 선분 위 s (0..1)
    let cross = |p0: Vec2, p1: Vec2| -> Option<(f64, f64)> {
        let d = Vec2::new(p1.x - p0.x, p1.y - p0.y);
        let den = u.x * d.y - u.y * d.x;
        if den.abs() < 1e-12 {
            return None;
        }
        let w = Vec2::new(p0.x - s.a.x, p0.y - s.a.y);
        let t = (w.x * d.y - w.y * d.x) / den;
        let k = (w.x * u.y - w.y * u.x) / den;
        (t >= -1e-9 && t <= len + 1e-9 && (-1e-9..=1.0 + 1e-9).contains(&k)).then_some((t, k))
    };
    let mut zmin = 0.0f64;
    for l in &m.levels {
        let z0 = l.elevation;
        zmin = zmin.min(z0 - 150.0);
        // 벽
        for w in m.walls.iter().filter(|w| w.level == l.id) {
            let Some((t, k)) = cross(w.a, w.b) else {
                continue;
            };
            let wl = w.length();
            let dir = w.dir();
            let sin = (u.x * dir.y - u.y * dir.x).abs().max(0.2);
            let half = w.thickness / 2.0 / sin;
            let top = z0 + w.height.unwrap_or(l.height);
            let along = k * wl;
            let hole = m
                .openings
                .iter()
                .find(|op| op.wall == w.id && (op.offset - along).abs() <= op.width / 2.0);
            match hole {
                None => cut(&mut out, t - half, t + half, z0, top),
                Some(op) => {
                    let (bottom, head) = match op.kind {
                        OpeningKind::Door => (z0, z0 + op.height),
                        OpeningKind::Window => (z0 + op.sill, z0 + op.sill + op.height),
                    };
                    cut(&mut out, t - half, t + half, z0, bottom.min(top));
                    cut(&mut out, t - half, t + half, head.min(top), top);
                    if op.kind == OpeningKind::Window {
                        out.push((
                            "A-GLAZ",
                            Shape::Line {
                                a: p(t, bottom),
                                b: p(t, head.min(top)),
                            },
                        ));
                    }
                }
            }
        }
        // 바닥판: 실 경계와 자르는 선이 만나는 점들을 짝지어 안쪽 구간
        for r in m.rooms.iter().filter(|r| r.level == l.id) {
            let n = r.boundary.len();
            let mut ts: Vec<f64> = (0..n)
                .filter_map(|i| cross(r.boundary[i], r.boundary[(i + 1) % n]).map(|(t, _)| t))
                .collect();
            ts.sort_by(|a, b| a.total_cmp(b));
            ts.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
            for pair in ts.chunks(2) {
                if let [t0, t1] = pair {
                    cut(&mut out, *t0, *t1, z0 - 150.0, z0);
                }
            }
        }
        // 계단
        for st in m.stairs.iter().filter(|x| x.level == l.id) {
            let Some((flights, _)) = st.flights() else {
                continue;
            };
            let rise = l.height / st.risers.max(1) as f64;
            for f in &flights {
                let par = f.u.x * u.x + f.u.y * u.y;
                // 줄기 중심선이 자르는 선에서 얼마나 떨어졌나
                let w = Vec2::new(f.start.x - s.a.x, f.start.y - s.a.y);
                let off = (w.x * -u.y + w.y * u.x).abs();
                if par.abs() < 0.9 || off > st.width / 2.0 {
                    continue;
                }
                let t0 = w.x * u.x + w.y * u.y;
                let step = f.len / f.risers.max(1) as f64 * par.signum();
                let mut pts = vec![(p(t0, z0 + rise * f.first as f64), 0.0)];
                for k in 0..f.risers {
                    let z = z0 + rise * (f.first + k + 1) as f64;
                    let ta = t0 + step * k as f64;
                    pts.push((p(ta, z), 0.0));
                    pts.push((p(ta + step, z), 0.0));
                }
                out.push((
                    "A-SECT",
                    Shape::Polyline {
                        points: pts,
                        closed: false,
                    },
                ));
            }
        }
    }
    // 층 바닥선과 이름 (왼쪽), 지반선, 제목
    let (x0, x1) = (-1500.0, len + 1500.0);
    let h = 250.0;
    for l in &m.levels {
        out.push((
            "A-SECT-IDEN",
            Shape::Line {
                a: p(x0, l.elevation),
                b: p(x1, l.elevation),
            },
        ));
        let label = format!(
            "{} FL{}{:.0}",
            l.name,
            if l.elevation >= 0.0 { "+" } else { "" },
            l.elevation
        );
        out.push((
            "A-SECT-IDEN",
            text(p(x0, l.elevation + h * 0.4), h, 0.0, label),
        ));
    }
    out.push((
        "A-SECT",
        Shape::Line {
            a: p(x0, -150.0),
            b: p(x1, -150.0),
        },
    ));
    out.push((
        "A-SECT-IDEN",
        text(
            p(len / 2.0, zmin - h * 4.0),
            h * 1.6,
            0.5,
            format!("단면 {}-{}'", s.name, s.name),
        ),
    ));
    out
}

/// 평면의 자르는 선 표시: 선, 양 끝 기호(이름), 보는 쪽 화살표
pub fn mark(s: &Section) -> Vec<(&'static str, Shape)> {
    let len = s.a.dist(s.b);
    if len < 1e-6 {
        return Vec::new();
    }
    let u = Vec2::new((s.b.x - s.a.x) / len, (s.b.y - s.a.y) / len);
    let n = Vec2::new(-u.y, u.x);
    let h = 350.0;
    let mut out = vec![("A-SECT-MARK", Shape::Line { a: s.a, b: s.b })];
    for (q, back) in [(s.a, -1.0), (s.b, 1.0)] {
        // 보는 방향(왼쪽 n) 화살표와 이름
        let tip = Vec2::new(q.x + n.x * h * 2.0, q.y + n.y * h * 2.0);
        out.push(("A-SECT-MARK", Shape::Line { a: q, b: tip }));
        let side = Vec2::new(u.x * h * 0.3, u.y * h * 0.3);
        let base = Vec2::new(tip.x - n.x * h * 0.8, tip.y - n.y * h * 0.8);
        out.push((
            "A-SECT-MARK",
            Shape::Fill {
                loops: vec![vec![
                    tip,
                    Vec2::new(base.x + side.x, base.y + side.y),
                    Vec2::new(base.x - side.x, base.y - side.y),
                ]],
            },
        ));
        let at = Vec2::new(q.x + u.x * back * h * 1.2, q.y + u.y * back * h * 1.2);
        let name = if back < 0.0 {
            s.name.clone()
        } else {
            format!("{}'", s.name)
        };
        out.push(("A-SECT-MARK", text(at, h, 0.5, name)));
    }
    out
}

fn text(at: Vec2, height: f64, halign: f64, value: String) -> Shape {
    Shape::Text(TextShape {
        at,
        height,
        rotation: 0.0,
        width_factor: 1.0,
        halign,
        valign: VAlign::Middle,
        value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::{OpeningKind, WallKind};
    use crate::archgen::{ArchReq, apply, new_drawing};
    use crate::edit::Editor;

    #[test]
    fn section_cuts_walls_slab_and_window() {
        let mut ed = Editor::new(new_drawing());
        apply(
            &mut ed,
            ArchReq::Walls {
                points: vec![[0.0, 0.0], [6000.0, 0.0], [6000.0, 4000.0], [0.0, 4000.0]],
                closed: true,
                thickness: 200.0,
                wall: WallKind::Exterior,
            },
        )
        .unwrap();
        apply(
            &mut ed,
            ArchReq::Opening {
                at: [3000.0, 4000.0],
                opening: OpeningKind::Window,
                width: 1200.0,
                height: 1200.0,
                sill: 900.0,
            },
        )
        .unwrap();
        apply(
            &mut ed,
            ArchReq::RoomAt {
                at: [3000.0, 2000.0],
                name: "거실".into(),
            },
        )
        .unwrap();
        let s = Section {
            id: 1,
            name: "A".into(),
            a: Vec2::new(3000.0, -1000.0),
            b: Vec2::new(3000.0, 5000.0),
            at: Vec2::new(0.0, -20000.0),
        };
        let shapes = draw(&ed.d.arch, &s);
        let fills = shapes
            .iter()
            .filter(|(l, s)| *l == "A-SECT-PATT" && matches!(s, Shape::Fill { .. }))
            .count();
        // 아래 벽 하나 + 창 있는 위 벽 둘(창대 아래·인방 위) + 바닥판 하나
        assert_eq!(fills, 4, "{shapes:?}");
        assert!(shapes.iter().any(|(l, _)| *l == "A-GLAZ"));
        assert!(
            shapes
                .iter()
                .any(|(_, s)| matches!(s, Shape::Text(t) if t.value == "단면 A-A'"))
        );
        assert_eq!(
            mark(&s)
                .iter()
                .filter(|(_, s)| matches!(s, Shape::Text(_)))
                .count(),
            2
        );
    }
}
