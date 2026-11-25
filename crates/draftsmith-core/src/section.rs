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
    // 곡선 벽은 곧은 조각으로 (§54)
    let straight = m.straightened().0;
    let m = &straight;
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
    // 가림: 자른 면과 앞에 보이는 벽이 뒤를 가린다 (t 구간 × 높이) — §45
    let occ: std::cell::RefCell<Vec<[f64; 4]>> = std::cell::RefCell::new(Vec::new());
    let cut = |out: &mut Vec<(&'static str, Shape)>, t0: f64, t1: f64, z0: f64, z1: f64| {
        if z1 - z0 > 1e-6 && t1 - t0 > 1e-6 {
            occ.borrow_mut().push([t0, t1, z0, z1]);
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
    // 보는 쪽(자르는 선 왼쪽)과 자르는 선 위 거리
    let nv = Vec2::new(-u.y, u.x);
    let side = |q: Vec2| (q.x - s.a.x) * nv.x + (q.y - s.a.y) * nv.y;
    let along = |q: Vec2| (q.x - s.a.x) * u.x + (q.y - s.a.y) * u.y;
    // 보이는 외곽 (가린 구간은 빼고): 위·아래 변은 보이는 구간만, 세로 변은 그 끝이 보일 때만
    let outline = |out: &mut Vec<(&'static str, Shape)>,
                   occ: &[[f64; 4]],
                   t0: f64,
                   t1: f64,
                   z0: f64,
                   z1: f64| {
        let (t0, t1) = (t0.max(0.0), t1.min(len));
        if t1 - t0 <= 1.0 || z1 - z0 <= 1.0 {
            return;
        }
        // 높이를 다 덮는 앞 객체의 t 구간을 뺀다
        let mut vis = vec![(t0, t1)];
        for o in occ
            .iter()
            .filter(|o| o[2] <= z0 + 1e-6 && o[3] >= z1 - 1e-6)
        {
            vis = vis
                .into_iter()
                .flat_map(|(a, b)| {
                    let mut r = Vec::new();
                    if o[0] > a {
                        r.push((a, o[0].min(b)));
                    }
                    if o[1] < b {
                        r.push((o[1].max(a), b));
                    }
                    r.into_iter().filter(|(x, y)| y - x > 1e-6)
                })
                .collect();
        }
        let line = |a: Vec2, b: Vec2| ("A-SECT-VIEW", Shape::Line { a, b });
        for (a, b) in &vis {
            out.push(line(p(*a, z0), p(*b, z0)));
            out.push(line(p(*a, z1), p(*b, z1)));
        }
        for t in [t0, t1] {
            if vis.iter().any(|(a, b)| t >= a - 1e-6 && t <= b + 1e-6) {
                out.push(line(p(t, z0), p(t, z1)));
            }
        }
    };
    // 보이는 벽: (보는 쪽 거리, 벽) — 층을 다 돈 뒤 가까운 것부터 그린다
    let mut seen: Vec<(f64, u64, f64, f64)> = Vec::new();
    // 보이는 기둥·계단 줄기: (보는 쪽 거리, t0, t1, z0, z1, 계단 단 꼭짓점들 — 기둥이면 빔)  §46
    let mut others: Vec<(f64, f64, f64, f64, f64, Vec<Vec2>)> = Vec::new();
    let top_level = m
        .levels
        .iter()
        .max_by(|a, b| a.elevation.total_cmp(&b.elevation))
        .map(|l| l.id);
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
        // 보이는 벽 (자르지 않은 벽 중 보는 쪽에 있는 것): 입면 외곽과 창 (§41) — 가림 처리는 층을 다 돈 뒤
        for w in m.walls.iter().filter(|w| w.level == l.id) {
            if cross(w.a, w.b).is_some() || side(w.a) <= 0.0 || side(w.b) <= 0.0 {
                continue;
            }
            seen.push((
                side(w.a).min(side(w.b)),
                w.id,
                z0,
                z0 + w.height.unwrap_or(l.height),
            ));
        }
        // 바닥판: 실 경계와 자르는 선이 만나는 점들을 짝지어 안쪽 구간 (맨 위층은 지붕판도)
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
                    // 반자(천장) 선 (§55)
                    if let Some(c) = r.ceiling {
                        out.push((
                            "A-CLNG",
                            Shape::Line {
                                a: p(*t0, z0 + c),
                                b: p(*t1, z0 + c),
                            },
                        ));
                    }
                    if Some(l.id) == top_level {
                        let zt = z0 + l.height;
                        cut(&mut out, *t0, *t1, zt, zt + 200.0);
                    }
                }
            }
        }
        // 기둥: 자르는 선이 지나면 자른 면, 보는 쪽에 있으면 보이는 외곽 (§46)
        for c in m.columns.iter().filter(|c| c.level == l.id) {
            let k = c.corners();
            let top = z0 + l.height;
            let ts: Vec<f64> = (0..4)
                .filter_map(|i| cross(k[i], k[(i + 1) % 4]).map(|(t, _)| t))
                .collect();
            if ts.len() >= 2 {
                let (t0, t1) = ts
                    .iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), t| {
                        (a.min(*t), b.max(*t))
                    });
                cut(&mut out, t0, t1, z0, top);
            } else if k.iter().all(|q| side(*q) > 0.0) {
                let (t0, t1) = k
                    .iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), q| {
                        (a.min(along(*q)), b.max(along(*q)))
                    });
                let depth = k.iter().map(|q| side(*q)).fold(f64::INFINITY, f64::min);
                others.push((depth, t0, t1, z0, top, Vec::new()));
            }
        }
        // 보: 기둥과 같게 — 자르면 천장 밑 자른 면, 보는 쪽이면 보이는 외곽 (§52)
        for x in m.beams.iter().filter(|x| x.level == l.id) {
            let k = x.corners();
            let top = z0 + l.height;
            let ts: Vec<f64> = (0..4)
                .filter_map(|i| cross(k[i], k[(i + 1) % 4]).map(|(t, _)| t))
                .collect();
            if ts.len() >= 2 {
                let (t0, t1) = ts
                    .iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), t| {
                        (a.min(*t), b.max(*t))
                    });
                cut(&mut out, t0, t1, top - x.depth, top);
            } else if k.iter().all(|q| side(*q) > 0.0) {
                let (t0, t1) = k
                    .iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), q| {
                        (a.min(along(*q)), b.max(along(*q)))
                    });
                let depth = k.iter().map(|q| side(*q)).fold(f64::INFINITY, f64::min);
                others.push((depth, t0, t1, top - x.depth, top, Vec::new()));
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
                    // 자르지 않은 나란한 줄기가 보는 쪽에 있으면 입면 (단 모양)
                    let sd = w.x * -u.y + w.y * u.x;
                    if par.abs() >= 0.9 && sd - st.width / 2.0 > 0.0 {
                        let t0 = w.x * u.x + w.y * u.y;
                        let step = f.len / f.risers.max(1) as f64 * par.signum();
                        let mut pts = vec![Vec2::new(t0, z0 + rise * f.first as f64)];
                        for k in 0..f.risers {
                            let z = z0 + rise * (f.first + k + 1) as f64;
                            let ta = t0 + step * k as f64;
                            pts.push(Vec2::new(ta, z));
                            pts.push(Vec2::new(ta + step, z));
                        }
                        let (a, b) = (
                            t0.min(t0 + step * f.risers as f64),
                            t0.max(t0 + step * f.risers as f64),
                        );
                        others.push((
                            sd - st.width / 2.0,
                            a,
                            b,
                            z0 + rise * f.first as f64,
                            z0 + rise * (f.first + f.risers) as f64,
                            pts,
                        ));
                    }
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
    // 보이는 기둥·계단 (벽보다 먼저 — 대개 벽 앞): 가까운 것부터, 기둥은 뒤를 가린다
    others.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, t0, t1, z0, z1, steps) in others {
        let snapshot = occ.borrow().clone();
        if steps.is_empty() {
            outline(&mut out, &snapshot, t0, t1, z0, z1);
            occ.borrow_mut().push([t0, t1, z0, z1]);
        } else {
            // 계단 입면은 앞이 통째로 가리지 않을 때만 (단 모양 그대로)
            let hidden = snapshot
                .iter()
                .any(|o| o[0] <= t0 && o[1] >= t1 && o[2] <= z0 && o[3] >= z1);
            if !hidden {
                out.push((
                    "A-SECT-VIEW",
                    Shape::Polyline {
                        points: steps.into_iter().map(|q| (p(q.x, q.y), 0.0)).collect(),
                        closed: false,
                    },
                ));
            }
        }
    }
    // 보이는 벽을 가까운 것부터: 앞의 자른 면·벽이 가린 구간은 빼고 그리고, 그 벽도 뒤를 가린다
    seen.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, id, z0, top) in seen {
        let Some(w) = m.wall(id) else { continue };
        let (ta, tb) = (along(w.a), along(w.b));
        let h = w.thickness / 2.0;
        let snapshot = occ.borrow().clone();
        outline(&mut out, &snapshot, ta.min(tb) - h, ta.max(tb) + h, z0, top);
        let dir = w.dir();
        for op in m
            .openings
            .iter()
            .filter(|op| op.wall == w.id && op.kind == OpeningKind::Window)
        {
            let c = Vec2::new(w.a.x + dir.x * op.offset, w.a.y + dir.y * op.offset);
            let half = op.width / 2.0 * (dir.x * u.x + dir.y * u.y).abs();
            let tc = along(c);
            outline(
                &mut out,
                &snapshot,
                tc - half,
                tc + half,
                z0 + op.sill,
                (z0 + op.sill + op.height).min(top),
            );
        }
        occ.borrow_mut()
            .push([ta.min(tb) - h, ta.max(tb) + h, z0, top]);
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
        // 아래 벽 하나 + 창 있는 위 벽 둘(창대 아래·인방 위) + 바닥판 + 지붕판
        assert_eq!(fills, 5, "{shapes:?}");
        // 보는 쪽(−x) 의 x = 0 벽이 입면선으로 보인다
        assert!(shapes.iter().any(|(l, _)| *l == "A-SECT-VIEW"));
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

    #[test]
    fn nearer_wall_hides_farther() {
        // 자르는 선 y = 0 (x 0→10000, 보는 쪽 +y). 앞 벽 y = 2000 (x 2000~4000), 뒤 벽 y = 6000 (x 0~10000)
        let mut ed = Editor::new(new_drawing());
        apply(
            &mut ed,
            ArchReq::Walls {
                points: vec![[2000.0, 2000.0], [4000.0, 2000.0]],
                closed: false,
                thickness: 200.0,
                wall: WallKind::Interior,
            },
        )
        .unwrap();
        apply(
            &mut ed,
            ArchReq::Walls {
                points: vec![[0.0, 6000.0], [10000.0, 6000.0]],
                closed: false,
                thickness: 200.0,
                wall: WallKind::Exterior,
            },
        )
        .unwrap();
        let s = Section {
            id: 1,
            name: "A".into(),
            a: Vec2::new(0.0, 0.0),
            b: Vec2::new(10000.0, 0.0),
            at: Vec2::default(),
        };
        let shapes = draw(&ed.d.arch, &s);
        // 뒤 벽 아래 변은 앞 벽(1900~4100) 자리에서 끊겨 두 토막
        let bottoms: Vec<(f64, f64)> = shapes
            .iter()
            .filter_map(|(l, s)| match s {
                Shape::Line { a, b } if *l == "A-SECT-VIEW" && a.y == 0.0 && b.y == 0.0 => {
                    Some((a.x, b.x))
                }
                _ => None,
            })
            .collect();
        assert!(
            bottoms.contains(&(0.0, 1900.0)) && bottoms.contains(&(4100.0, 10000.0)),
            "{bottoms:?}"
        );
        assert!(
            !bottoms
                .iter()
                .any(|(a, b)| *a < 3000.0 && *b > 3000.0 && *b - *a > 5000.0),
            "{bottoms:?}"
        );
    }

    #[test]
    fn columns_cut_and_seen() {
        let mut ed = Editor::new(new_drawing());
        apply(
            &mut ed,
            ArchReq::Column {
                at: [1000.0, 0.0],
                width: 400.0,
                depth: 400.0,
            },
        )
        .unwrap();
        apply(
            &mut ed,
            ArchReq::Column {
                at: [3000.0, 3000.0],
                width: 400.0,
                depth: 400.0,
            },
        )
        .unwrap();
        let s = Section {
            id: 1,
            name: "A".into(),
            a: Vec2::new(0.0, 0.0),
            b: Vec2::new(5000.0, 0.0),
            at: Vec2::default(),
        };
        let shapes = draw(&ed.d.arch, &s);
        // 첫 기둥은 잘림 (800~1200), 둘째는 보이는 외곽 (2800~3200)
        assert!(shapes.iter().any(|(l, s)| *l == "A-SECT-PATT"
            && matches!(s, Shape::Fill { loops } if (loops[0][0].x - 800.0).abs() < 1e-6)));
        assert!(shapes.iter().any(|(l, s)| *l == "A-SECT-VIEW" && matches!(s, Shape::Line { a, b } if (a.x - 2800.0).abs() < 1e-6 && (b.x - 3200.0).abs() < 1e-6)));
    }
}
