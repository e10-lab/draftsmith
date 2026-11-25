//! IFC 내보내기 (M6, 구현 노트 §39): 건축 객체를 IFC4 (STEP 글) 로.
//!
//! 프로젝트 → 대지 → 건물 → 층(IfcBuildingStorey) 아래에 벽(IfcWall, 두께×길이 직사각형을 높이만큼 돌출),
//! 바닥판(IfcSlab, 실 경계 150 두께), 실(IfcSpace, 이름·층고 돌출), 기둥(IfcColumn), 문·창(IfcDoor·IfcWindow, 벽 두께
//! 상자 — 벽에 구멍(IfcOpeningElement)을 내는 것은 다음 단계). 단위는 mm. 한글 이름은 STEP 의 \X2\ 로 쓴다.

use std::fmt::Write as _;

use crate::arch::{ArchModel, OpeningKind};
use crate::geom::Vec2;

/// IFC 글로벌 ID (22 자, IFC base64) — 객체 종류·번호에서 늘 같게
fn guid(kind: &str, id: u64) -> String {
    const ABC: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz_$";
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in kind.bytes().chain(id.to_le_bytes()) {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    // splitmix 로 128 비트
    let mix = |mut z: u64| {
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    };
    let v: u128 = ((mix(h) as u128) << 64) | mix(h.wrapping_add(0x9e37_79b9_7f4a_7c15)) as u128;
    let mut out = String::with_capacity(22);
    out.push(ABC[(v >> 126) as usize & 3] as char);
    for i in (0..21).rev() {
        out.push(ABC[((v >> (i * 6)) & 63) as usize] as char);
    }
    out
}

/// STEP 문자열 ('…'): 따옴표·역슬래시는 겹치고, ASCII 밖은 \X2\(UTF-16 16진)\X0\
fn s(text: &str) -> String {
    let mut out = String::from("'");
    let mut wide = String::new();
    let flush = |out: &mut String, wide: &mut String| {
        if !wide.is_empty() {
            out.push_str("\\X2\\");
            out.push_str(wide);
            out.push_str("\\X0\\");
            wide.clear();
        }
    };
    for ch in text.chars() {
        if ch.is_ascii() && !ch.is_ascii_control() {
            flush(&mut out, &mut wide);
            match ch {
                '\'' => out.push_str("''"),
                '\\' => out.push_str("\\\\"),
                c => out.push(c),
            }
        } else {
            let mut buf = [0u16; 2];
            for u in ch.encode_utf16(&mut buf) {
                let _ = write!(wide, "{:04X}", u);
            }
        }
    }
    flush(&mut out, &mut wide);
    out.push('\'');
    out
}

/// 실수 (STEP 은 소수점이 꼭 있어야)
fn f(v: f64) -> String {
    let t = format!("{v:.6}");
    t.trim_end_matches('0').to_string()
}

struct W {
    lines: Vec<String>,
}

impl W {
    fn add(&mut self, body: String) -> usize {
        self.lines.push(body);
        self.lines.len()
    }
    fn pt3(&mut self, x: f64, y: f64, z: f64) -> usize {
        self.add(format!("IFCCARTESIANPOINT(({},{},{}))", f(x), f(y), f(z)))
    }
    fn pt2(&mut self, x: f64, y: f64) -> usize {
        self.add(format!("IFCCARTESIANPOINT(({},{}))", f(x), f(y)))
    }
    fn dir3(&mut self, x: f64, y: f64, z: f64) -> usize {
        self.add(format!("IFCDIRECTION(({},{},{}))", f(x), f(y), f(z)))
    }
    /// 원점 p, x 축 방향 (평면)
    fn place3(&mut self, p: Vec2, z: f64, xdir: Vec2) -> usize {
        let o = self.pt3(p.x, p.y, z);
        let zz = self.dir3(0.0, 0.0, 1.0);
        let xx = self.dir3(xdir.x, xdir.y, 0.0);
        self.add(format!("IFCAXIS2PLACEMENT3D(#{o},#{zz},#{xx})"))
    }
    fn local(&mut self, rel: Option<usize>, axis: usize) -> usize {
        let r = rel.map(|r| format!("#{r}")).unwrap_or_else(|| "$".into());
        self.add(format!("IFCLOCALPLACEMENT({r},#{axis})"))
    }
    /// 프로필을 높이만큼 위로 돌출한 몸체 표현
    fn body(&mut self, ctx: usize, profile: usize, height: f64) -> usize {
        let pos = self.place3(Vec2::default(), 0.0, Vec2::new(1.0, 0.0));
        let up = self.dir3(0.0, 0.0, 1.0);
        let solid = self.add(format!(
            "IFCEXTRUDEDAREASOLID(#{profile},#{pos},#{up},{})",
            f(height.max(1.0))
        ));
        let rep = self.add(format!(
            "IFCSHAPEREPRESENTATION(#{ctx},'Body','SweptSolid',(#{solid}))"
        ));
        self.add(format!("IFCPRODUCTDEFINITIONSHAPE($,$,(#{rep}))"))
    }
    /// 가운데가 (cx, cy) 인 w×d 직사각형 프로필
    fn rect(&mut self, cx: f64, cy: f64, w: f64, d: f64) -> usize {
        let c = self.pt2(cx, cy);
        let ax = self.add(format!("IFCAXIS2PLACEMENT2D(#{c},$)"));
        self.add(format!(
            "IFCRECTANGLEPROFILEDEF(.AREA.,$,#{ax},{},{})",
            f(w.max(1.0)),
            f(d.max(1.0))
        ))
    }
    fn polygon(&mut self, pts: &[Vec2]) -> usize {
        let mut ids: Vec<usize> = pts.iter().map(|p| self.pt2(p.x, p.y)).collect();
        ids.push(ids[0]);
        let list = ids
            .iter()
            .map(|i| format!("#{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let pl = self.add(format!("IFCPOLYLINE(({list}))"));
        self.add(format!("IFCARBITRARYCLOSEDPROFILEDEF(.AREA.,$,#{pl})"))
    }
}

/// 건축 객체 → IFC4 STEP 글
pub fn write(m: &ArchModel, project: &str) -> String {
    let mut w = W { lines: Vec::new() };
    // 단위·좌표계
    let len = w.add("IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.)".into());
    let area = w.add("IFCSIUNIT(*,.AREAUNIT.,$,.SQUARE_METRE.)".into());
    let vol = w.add("IFCSIUNIT(*,.VOLUMEUNIT.,$,.CUBIC_METRE.)".into());
    let ang = w.add("IFCSIUNIT(*,.PLANEANGLEUNIT.,$,.RADIAN.)".into());
    let units = w.add(format!("IFCUNITASSIGNMENT((#{len},#{area},#{vol},#{ang}))"));
    let world = w.place3(Vec2::default(), 0.0, Vec2::new(1.0, 0.0));
    let ctx = w.add(format!(
        "IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#{world},$)"
    ));
    let proj = w.add(format!(
        "IFCPROJECT('{}',$,{},$,$,$,$,(#{ctx}),#{units})",
        guid("project", 0),
        s(project)
    ));
    let site_pl = {
        let a = w.place3(Vec2::default(), 0.0, Vec2::new(1.0, 0.0));
        w.local(None, a)
    };
    let site = w.add(format!(
        "IFCSITE('{}',$,{},$,$,#{site_pl},$,$,.ELEMENT.,$,$,$,$,$)",
        guid("site", 0),
        s("대지")
    ));
    let bld_pl = {
        let a = w.place3(Vec2::default(), 0.0, Vec2::new(1.0, 0.0));
        w.local(Some(site_pl), a)
    };
    let bld = w.add(format!(
        "IFCBUILDING('{}',$,{},$,$,#{bld_pl},$,$,.ELEMENT.,$,$,$)",
        guid("building", 0),
        s(project)
    ));
    w.add(format!(
        "IFCRELAGGREGATES('{}',$,$,$,#{proj},(#{site}))",
        guid("agg-site", 0)
    ));
    w.add(format!(
        "IFCRELAGGREGATES('{}',$,$,$,#{site},(#{bld}))",
        guid("agg-bld", 0)
    ));

    let mut storeys = Vec::new();
    for l in &m.levels {
        let a = w.place3(Vec2::default(), l.elevation, Vec2::new(1.0, 0.0));
        let pl = w.local(Some(bld_pl), a);
        let st = w.add(format!(
            "IFCBUILDINGSTOREY('{}',$,{},$,$,#{pl},$,$,.ELEMENT.,{})",
            guid("storey", l.id),
            s(&l.name),
            f(l.elevation)
        ));
        storeys.push(st);
        let mut items: Vec<usize> = Vec::new();
        // 벽
        for wl in m.walls.iter().filter(|x| x.level == l.id) {
            let length = wl.length();
            if length < 1e-6 {
                continue;
            }
            let ax = w.place3(wl.a, 0.0, wl.dir());
            let lp = w.local(Some(pl), ax);
            let prof = w.rect(length / 2.0, 0.0, length, wl.thickness);
            let shape = w.body(ctx, prof, wl.height.unwrap_or(l.height));
            let name = format!("{} {}", wl.kind.label(), wl.id);
            items.push(w.add(format!(
                "IFCWALL('{}',$,{},$,$,#{lp},#{shape},$,.STANDARD.)",
                guid("wall", wl.id),
                s(&name)
            )));
            // 문·창: 벽 안 상자
            for o in m.openings.iter().filter(|o| o.wall == wl.id) {
                let (z, kind) = match o.kind {
                    OpeningKind::Door => (0.0, "door"),
                    OpeningKind::Window => (o.sill, "window"),
                };
                let at = Vec2::new(
                    wl.a.x + wl.dir().x * o.offset,
                    wl.a.y + wl.dir().y * o.offset,
                );
                let ax = w.place3(at, z, wl.dir());
                let lp = w.local(Some(pl), ax);
                let prof = w.rect(0.0, 0.0, o.width, wl.thickness);
                let shape = w.body(ctx, prof, o.height);
                let line = if kind == "door" {
                    format!(
                        "IFCDOOR('{}',$,{},$,$,#{lp},#{shape},$,{},{},.DOOR.,$,$)",
                        guid("door", o.id),
                        s("문"),
                        f(o.height),
                        f(o.width)
                    )
                } else {
                    format!(
                        "IFCWINDOW('{}',$,{},$,$,#{lp},#{shape},$,{},{},.WINDOW.,$,$)",
                        guid("window", o.id),
                        s("창"),
                        f(o.height),
                        f(o.width)
                    )
                };
                items.push(w.add(line));
            }
        }
        // 실·바닥판
        for r in m
            .rooms
            .iter()
            .filter(|x| x.level == l.id && x.boundary.len() >= 3)
        {
            let ax = w.place3(Vec2::default(), 0.0, Vec2::new(1.0, 0.0));
            let lp = w.local(Some(pl), ax);
            let prof = w.polygon(&r.boundary);
            let shape = w.body(ctx, prof, l.height);
            items.push(w.add(format!(
                "IFCSPACE('{}',$,{},$,$,#{lp},#{shape},{},.ELEMENT.,.INTERNAL.,$)",
                guid("space", r.id),
                s(&r.name),
                s(&format!("{:.2} m2", r.area() * 1e-6))
            )));
            let ax = w.place3(Vec2::default(), -150.0, Vec2::new(1.0, 0.0));
            let lp = w.local(Some(pl), ax);
            let prof = w.polygon(&r.boundary);
            let shape = w.body(ctx, prof, 150.0);
            items.push(w.add(format!(
                "IFCSLAB('{}',$,{},$,$,#{lp},#{shape},$,.FLOOR.)",
                guid("slab", r.id),
                s("바닥판")
            )));
        }
        // 기둥
        for c in m.columns.iter().filter(|x| x.level == l.id) {
            let ax = w.place3(c.at, 0.0, Vec2::new(c.angle.cos(), c.angle.sin()));
            let lp = w.local(Some(pl), ax);
            let prof = w.rect(0.0, 0.0, c.width, c.depth);
            let shape = w.body(ctx, prof, l.height);
            items.push(w.add(format!(
                "IFCCOLUMN('{}',$,{},$,$,#{lp},#{shape},$,.COLUMN.)",
                guid("column", c.id),
                s("기둥")
            )));
        }
        if !items.is_empty() {
            let list = items
                .iter()
                .map(|i| format!("#{i}"))
                .collect::<Vec<_>>()
                .join(",");
            w.add(format!(
                "IFCRELCONTAINEDINSPATIALSTRUCTURE('{}',$,$,$,({list}),#{st})",
                guid("contain", l.id)
            ));
        }
    }
    if !storeys.is_empty() {
        let list = storeys
            .iter()
            .map(|i| format!("#{i}"))
            .collect::<Vec<_>>()
            .join(",");
        w.add(format!(
            "IFCRELAGGREGATES('{}',$,$,$,#{bld},({list}))",
            guid("agg-storeys", 0)
        ));
    }

    let mut out = String::new();
    out.push_str("ISO-10303-21;\nHEADER;\n");
    out.push_str("FILE_DESCRIPTION(('ViewDefinition [DesignTransferView]'),'2;1');\n");
    let _ = writeln!(
        out,
        "FILE_NAME({},'{}',(''),(''),'DraftSmith','DraftSmith','');",
        s(project),
        crate::sheet::today().replace('.', "-")
    );
    out.push_str("FILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n");
    for (i, l) in w.lines.iter().enumerate() {
        let _ = writeln!(out, "#{}={};", i + 1, l);
    }
    out.push_str("ENDSEC;\nEND-ISO-10303-21;\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::WallKind;
    use crate::archgen::{ArchReq, apply, new_drawing};
    use crate::edit::Editor;

    #[test]
    fn writes_storey_walls_spaces() {
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
                at: [3000.0, 0.0],
                opening: OpeningKind::Door,
                width: 900.0,
                height: 2100.0,
                sill: 0.0,
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
        let t = write(&ed.d.arch, "시험 주택");
        assert!(t.starts_with("ISO-10303-21;") && t.trim_end().ends_with("END-ISO-10303-21;"));
        assert_eq!(t.matches("IFCWALL(").count(), 4);
        assert_eq!(t.matches("IFCDOOR(").count(), 1);
        assert_eq!(t.matches("IFCSPACE(").count(), 1);
        assert!(t.contains("IFCBUILDINGSTOREY("));
        // 「거실」 = U+AC70 U+C2E4
        assert!(t.contains("\\X2\\AC70C2E4\\X0\\"), "{t}");
        // 글로벌 ID 22 자, 서로 다름
        assert_eq!(guid("wall", 1).len(), 22);
        assert_ne!(guid("wall", 1), guid("wall", 2));
        // 모든 참조가 앞에 정의된 줄을 가리킨다
        let n = t.lines().filter(|l| l.starts_with('#')).count();
        for cap in t.split('#').skip(1) {
            let num: String = cap.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(k) = num.parse::<usize>() {
                assert!(k >= 1 && k <= n, "#{k}");
            }
        }
    }
}
