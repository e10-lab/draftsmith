//! 검증용 예제 건물 (구현 노트 §44): 2 층 주택 — 그리드·외벽·내벽·문·창·실·기둥·계단·치수·단면·시트.
//! `dsmith demo` 와 시험이 함께 쓴다. 모든 객체 종류를 한 번씩 거치게 짰다.

use crate::arch::{OpeningKind, WallKind};
use crate::archgen::{ArchReq, apply, new_drawing};
use crate::drawing::Drawing;
use crate::edit::Editor;
use crate::sheet::{self, SheetReq};

fn walls(
    ed: &mut Editor,
    pts: &[[f64; 2]],
    closed: bool,
    t: f64,
    wall: WallKind,
) -> Result<(), String> {
    apply(
        ed,
        ArchReq::Walls {
            points: pts.to_vec(),
            closed,
            thickness: t,
            wall,
        },
    )
}

/// 한 층 평면 (외벽 12×9 m, 내벽, 문·창, 실, 기둥, 치수)
fn floor(ed: &mut Editor, first: bool) -> Result<(), String> {
    walls(
        ed,
        &[[0.0, 0.0], [12000.0, 0.0], [12000.0, 9000.0], [0.0, 9000.0]],
        true,
        200.0,
        WallKind::Exterior,
    )?;
    walls(
        ed,
        &[[6000.0, 0.0], [6000.0, 9000.0]],
        false,
        120.0,
        WallKind::Interior,
    )?;
    walls(
        ed,
        &[[6000.0, 5000.0], [12000.0, 5000.0]],
        false,
        120.0,
        WallKind::Interior,
    )?;
    let door = |at: [f64; 2]| ArchReq::Opening {
        at,
        opening: OpeningKind::Door,
        width: 900.0,
        height: 2100.0,
        sill: 0.0,
    };
    let win = |at: [f64; 2], w: f64| ArchReq::Opening {
        at,
        opening: OpeningKind::Window,
        width: w,
        height: 1200.0,
        sill: 900.0,
    };
    if first {
        apply(ed, door([3000.0, 0.0]))?;
    }
    apply(ed, door([6000.0, 2500.0]))?;
    apply(ed, door([9000.0, 5000.0]))?;
    apply(ed, win([3000.0, 9000.0], 1800.0))?;
    apply(ed, win([9000.0, 9000.0], 1800.0))?;
    apply(ed, win([12000.0, 2500.0], 1200.0))?;
    for at in [
        [0.0, 0.0],
        [6000.0, 0.0],
        [12000.0, 0.0],
        [0.0, 9000.0],
        [6000.0, 9000.0],
        [12000.0, 9000.0],
    ] {
        apply(
            ed,
            ArchReq::Column {
                at,
                width: 400.0,
                depth: 400.0,
            },
        )?;
    }
    let names = if first {
        ["거실", "주방", "현관"]
    } else {
        ["침실 1", "침실 2", "욕실"]
    };
    for (at, name) in [[3000.0, 2500.0], [9000.0, 2500.0], [9000.0, 7000.0]]
        .into_iter()
        .zip(names)
    {
        apply(
            ed,
            ArchReq::RoomAt {
                at,
                name: name.into(),
            },
        )?;
    }
    apply(
        ed,
        ArchReq::Dim {
            a: [0.0, 0.0],
            b: [6000.0, 0.0],
            at: [3000.0, -2500.0],
            text_height: 250.0,
            arrow: false,
            plain: false,
            precision: 0,
        },
    )?;
    apply(
        ed,
        ArchReq::Dim {
            a: [6000.0, 0.0],
            b: [12000.0, 0.0],
            at: [9000.0, -2500.0],
            text_height: 250.0,
            arrow: false,
            plain: false,
            precision: 0,
        },
    )?;
    Ok(())
}

/// 예제 건물 도면
pub fn demo_building() -> Result<Drawing, String> {
    let mut ed = Editor::new(new_drawing());
    apply(
        &mut ed,
        ArchReq::GridSet {
            origin: [0.0, 0.0],
            xs: vec![6000.0, 6000.0],
            ys: vec![5000.0, 4000.0],
            extend: 1500.0,
        },
    )?;
    apply(
        &mut ed,
        ArchReq::Site {
            boundary: vec![
                [-4000.0, -6000.0],
                [18000.0, -6000.0],
                [18000.0, 14000.0],
                [-4000.0, 14000.0],
            ],
            zone: "제2종일반주거지역".into(),
            coverage_limit: 60.0,
            far_limit: 200.0,
        },
    )?;
    floor(&mut ed, true)?;
    apply(
        &mut ed,
        ArchReq::Stair {
            a: [1000.0, 6000.0],
            b: [1000.0, 7700.0],
            width: 1000.0,
            risers: 18,
            mid: Some([4200.0, 6000.0]),
            spiral: None,
        },
    )?;
    // 2 층
    apply(
        &mut ed,
        ArchReq::Level {
            name: "2층".into(),
            elevation: 3300.0,
            height: 3000.0,
        },
    )?;
    floor(&mut ed, false)?;
    // 단면 (1 층으로 돌아와서)
    let first = ed.d.arch.levels[0].id;
    apply(&mut ed, ArchReq::ActiveLevel { id: first })?;
    apply(
        &mut ed,
        ArchReq::Section {
            a: [3000.0, -1500.0],
            b: [3000.0, 10500.0],
            at: [18000.0, -12000.0],
            name: String::new(),
        },
    )?;
    // 시트: 도면 목록, 층별 평면
    let mut d = ed.d.clone();
    let levels: Vec<(u64, String)> = d
        .arch
        .levels
        .iter()
        .map(|l| (l.id, l.name.clone()))
        .collect();
    let mut reqs = vec![SheetReq {
        title: "도면 목록".into(),
        number: "A-000".into(),
        paper: "A3".into(),
        scale: 100.0,
        level: None,
        project: "예제 주택".into(),
        list: true,
    }];
    for (i, (id, name)) in levels.iter().enumerate() {
        reqs.push(SheetReq {
            title: format!("{name} 평면도"),
            number: format!("A-10{}", i + 1),
            paper: "A3".into(),
            scale: 100.0,
            level: Some(*id),
            project: "예제 주택".into(),
            list: false,
        });
    }
    for r in reqs {
        let l = sheet::make(&mut d, &r)?;
        d.layouts.push(l);
    }
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_round_trips_everywhere() {
        let d = demo_building().unwrap();
        let m = &d.arch;
        assert_eq!((m.levels.len(), m.walls.len(), m.rooms.len()), (2, 12, 6));
        assert!(m.rooms.iter().all(|r| r.area() > 1.0e6), "실 면적");
        // .dsmith 왕복이 같다
        let back = crate::docfile::from_bytes(&crate::docfile::to_bytes(&d).unwrap()).unwrap();
        assert_eq!(back.arch, d.arch);
        assert_eq!(back.layouts.len(), 3);
        // 시트가 모두 합쳐진다
        for i in 0..d.layouts.len() {
            assert!(
                crate::sheet::compose(&d, i).is_some_and(|c| !c.entities.is_empty()),
                "시트 {i}"
            );
        }
        // IFC·3D
        let ifc = crate::ifc::write(m, "예제 주택");
        assert_eq!(ifc.matches("IFCWALL(").count(), 12);
        assert!(!crate::model3d::mesh(m).tris.is_empty());
        // 표: 면적표 연면적
        let t = crate::archgen::tables(&d);
        assert!(
            t.tables
                .iter()
                .any(|x| x.name == "면적표" && x.rows.iter().any(|r| r[1] == "연면적"))
        );
    }
}
