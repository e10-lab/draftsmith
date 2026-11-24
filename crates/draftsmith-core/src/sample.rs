//! M0 측정용 도면 만들기.
//!
//! 격자 모양 평면(벽선·기둥 원·실 이름 문자·문 블록)을 원하는 개수만큼 만들어 DXF 나 DWG 로 쓴다.
//! 실 이름은 한글이라 문자 코드 처리가, 문은 블록 참조(대칭 포함)라 블록 펼치기가 함께 시험된다.

use std::path::Path;

use acadrust::entities::{
    Arc, BoundaryEdge, BoundaryPath, Circle, Dimension, DimensionLinear, Hatch, HatchPattern,
    HatchPatternLine, Insert, Line, LwPolyline, MText, PolylineEdge, Solid, Text,
};
use acadrust::tables::{BlockRecord, Layer, LineType, LineTypeElement};
use acadrust::types::{Color, Vector2, Vector3};
use acadrust::{CadDocument, DwgWriter, DxfWriter, EntityType};

const ROOMS: [&str; 6] = ["거실", "주방", "침실", "욕실", "현관", "다용도실"];
/// 문 블록 이름 (한글 블록 이름도 시험)
pub const DOOR: &str = "문-900";

fn on(mut e: EntityType, layer: &str, color: Color) -> EntityType {
    e.common_mut().layer = layer.to_string();
    e.common_mut().color = color;
    e
}

/// 문 블록 정의: 문짝 선 + 여닫이 호 (레이어 「0」, 색 ByBlock — 참조의 레이어·색을 따른다)
fn define_door(doc: &mut CadDocument) {
    let mut br = BlockRecord::new(DOOR);
    br.handle = doc.allocate_handle();
    let owner = br.handle;
    let _ = doc.block_records.add(br);
    let mut arc = Arc::new();
    arc.radius = 900.0;
    arc.start_angle = 0.0;
    arc.end_angle = std::f64::consts::FRAC_PI_2;
    for e in [
        EntityType::Line(Line::from_coords(0.0, 0.0, 0.0, 0.0, 900.0, 0.0)),
        EntityType::Arc(arc),
    ] {
        let mut e = on(e, "0", Color::ByBlock);
        e.common_mut().owner_handle = owner;
        let _ = doc.add_entity(e);
    }
}

/// 한 칸에 파일 객체 수
pub const FILE_PER_CELL: usize = 13;
/// 한 칸을 펼친 객체 수: 벽선 4, 폴리선 1, 기둥 3, 실 이름 1, 문 2, 해치 1, 치수 6, 중심선 1
pub const EXPANDED_PER_CELL: usize = 19;

/// 치수 블록(*D) 하나: 보조선 2, 치수선 1, 화살표(SOLID) 2, 치수 문자 1 — 세계 좌표
fn define_dimension_block(
    doc: &mut CadDocument,
    name: &str,
    p1: (f64, f64),
    p2: (f64, f64),
    y: f64,
) {
    let mut br = BlockRecord::new(name);
    br.handle = doc.allocate_handle();
    let owner = br.handle;
    let _ = doc.block_records.add(br);
    let v = |x: f64, y: f64| Vector3::new(x, y, 0.0);
    let arrow = |tip: f64, dir: f64| {
        let mut s = Solid::new(
            v(tip, y),
            v(tip + dir * 250.0, y + 60.0),
            v(tip + dir * 250.0, y - 60.0),
            v(tip + dir * 250.0, y - 60.0),
        );
        s.common.color = Color::ByBlock;
        EntityType::Solid(s)
    };
    let mut text = MText::new();
    text.value = format!("{{\\fMalgun Gothic|b0;{}}}", (p2.0 - p1.0).round());
    text.insertion_point = v((p1.0 + p2.0) / 2.0, y + 150.0);
    text.height = 250.0;
    text.attachment_point = acadrust::entities::AttachmentPoint::BottomCenter;
    for e in [
        EntityType::Line(Line::from_coords(p1.0, p1.1, 0.0, p1.0, y, 0.0)),
        EntityType::Line(Line::from_coords(p2.0, p2.1, 0.0, p2.0, y, 0.0)),
        EntityType::Line(Line::from_coords(p1.0, y, 0.0, p2.0, y, 0.0)),
        arrow(p1.0, 1.0),
        arrow(p2.0, -1.0),
        EntityType::MText(Box::new(text)),
    ] {
        let mut e = on(e, "0", Color::ByBlock);
        e.common_mut().owner_handle = owner;
        let _ = doc.add_entity(e);
    }
}

/// 사각형 경계 해치. `pattern` 이 true 면 45° 사선(간격 150), 아니면 솔리드
fn square_hatch(x: f64, y: f64, w: f64, h: f64, pattern: bool) -> Hatch {
    let mut hatch = if pattern {
        let mut p = HatchPattern::new("ANSI31");
        let a = std::f64::consts::FRAC_PI_4;
        // 간격 150 의 수직 이동 = (−sin a, cos a)·150
        p.add_line(HatchPatternLine {
            angle: a,
            base_point: Vector2::new(0.0, 0.0),
            offset: Vector2::new(-a.sin() * 150.0, a.cos() * 150.0),
            dash_lengths: Vec::new(),
        });
        Hatch::with_pattern(p)
    } else {
        Hatch::solid()
    };
    let mut path = BoundaryPath::new();
    path.add_edge(BoundaryEdge::Polyline(PolylineEdge::new(
        vec![
            Vector2::new(x, y),
            Vector2::new(x + w, y),
            Vector2::new(x + w, y + h),
            Vector2::new(x, y + h),
        ],
        true,
    )));
    hatch.add_path(path);
    hatch
}

/// 대략 `n` 개 파일 객체로 된 도면. 한 칸(6 m × 4.8 m)에 파일 객체 [`FILE_PER_CELL`] 개
/// (벽선 4, 안쪽 폴리선 1, 기둥 3, 실 이름 1, 문 블록 참조 1, 바닥 해치 1, 폭 치수 1).
pub fn plan_grid(n: usize) -> CadDocument {
    let mut doc = CadDocument::new();
    // 중심선(1점 쇄선): 긴 선 1200, 빈칸 300, 짧은 선 300, 빈칸 300 (mm 도면, LTSCALE 1)
    let mut center = LineType::new("CENTER");
    for e in [
        LineTypeElement::dash(1200.0),
        LineTypeElement::space(300.0),
        LineTypeElement::dash(300.0),
        LineTypeElement::space(300.0),
    ] {
        center.elements.push(e);
    }
    center.pattern_length = 2100.0;
    let _ = doc.line_types.add(center);
    for (name, color) in [
        ("A-WALL", 7),
        ("A-COLS", 4),
        ("A-ANNO", 2),
        ("A-DOOR", 1),
        ("A-HATC", 8),
        ("A-DIMS", 3),
        ("A-GRID", 1),
    ] {
        let mut layer = Layer::with_color(name, Color::Index(color));
        if name == "A-GRID" {
            layer.line_type = "CENTER".into();
        }
        // 선 굵기 (1/100 mm): 벽 0.5, 치수 0.13
        match name {
            "A-WALL" => layer.line_weight = acadrust::types::LineWeight::Value(50),
            "A-DIMS" => layer.line_weight = acadrust::types::LineWeight::Value(13),
            _ => {}
        }
        let _ = doc.layers.add(layer);
    }
    define_door(&mut doc);
    let cells = n.div_ceil(FILE_PER_CELL).max(1);
    let cols = (cells as f64).sqrt().ceil() as usize;
    let (w, h) = (6000.0, 4800.0);
    let add = |doc: &mut CadDocument, e: EntityType, layer: &str| {
        let _ = doc.add_entity(on(e, layer, Color::ByLayer));
    };
    for k in 0..cells {
        let (i, j) = ((k % cols) as f64, (k / cols) as f64);
        let (x, y) = (i * w, j * h);
        for (a, b) in [
            ((x, y), (x + w, y)),
            ((x + w, y), (x + w, y + h)),
            ((x + w, y + h), (x, y + h)),
            ((x, y + h), (x, y)),
        ] {
            let l = Line::from_coords(a.0, a.1, 0.0, b.0, b.1, 0.0);
            add(&mut doc, EntityType::Line(l), "A-WALL");
        }
        let mut inner = LwPolyline::from_points(vec![
            Vector2::new(x + 200.0, y + 200.0),
            Vector2::new(x + w - 200.0, y + 200.0),
            Vector2::new(x + w - 200.0, y + h - 200.0),
            Vector2::new(x + 200.0, y + h - 200.0),
        ]);
        inner.is_closed = true;
        add(&mut doc, EntityType::LwPolyline(inner), "A-WALL");
        for (cx, cy) in [(x, y), (x + w, y), (x + w, y + h)] {
            let c = Circle::from_coords(cx, cy, 0.0, 150.0);
            add(&mut doc, EntityType::Circle(c), "A-COLS");
        }
        let mut t = Text::with_value(
            ROOMS[k % ROOMS.len()],
            Vector3::new(x + 600.0, y + 600.0, 0.0),
        );
        t.height = 300.0;
        add(&mut doc, EntityType::Text(t), "A-ANNO");
        // 문: 칸마다 번갈아 대칭
        let sx = if k % 2 == 0 { 1.0 } else { -1.0 };
        let door =
            Insert::new(DOOR, Vector3::new(x + 3000.0, y + 200.0, 0.0)).with_scale(sx, 1.0, 1.0);
        add(&mut doc, EntityType::Insert(door), "A-DOOR");
        // 중심선: 칸 가운데 세로 (A-GRID 레이어의 CENTER 선 종류)
        let g = Line::from_coords(x + w / 2.0, y - 600.0, 0.0, x + w / 2.0, y + h + 600.0, 0.0);
        add(&mut doc, EntityType::Line(g), "A-GRID");
        // 바닥 해치: 칸마다 번갈아 사선 무늬·솔리드
        let hatch = square_hatch(x + 4000.0, y + 2600.0, 1200.0, 1200.0, k % 2 == 0);
        add(&mut doc, EntityType::Hatch(Box::new(hatch)), "A-HATC");
        // 폭 치수: 치수 블록 *D{k} 를 함께 만든다
        let name = format!("*D{k}");
        let dy = y + h - 700.0;
        define_dimension_block(&mut doc, &name, (x, y + h), (x + w, y + h), dy);
        let mut dim = DimensionLinear::horizontal(
            Vector3::new(x, y + h, 0.0),
            Vector3::new(x + w, y + h, 0.0),
        );
        dim.definition_point = Vector3::new(x + w, dy, 0.0);
        dim.base.block_name = name;
        dim.base.actual_measurement = w;
        add(
            &mut doc,
            EntityType::Dimension(Box::new(Dimension::Linear(dim))),
            "A-DIMS",
        );
    }
    add_sheet(&mut doc, cols, cells, w, h);
    doc
}

/// 배치(Layout1)에 A3 도곽·표제란과 1:100 뷰포트를 넣는다 (종이 단위 mm)
fn add_sheet(doc: &mut CadDocument, cols: usize, cells: usize, w: f64, h: f64) {
    let paper = |doc: &mut CadDocument, e: EntityType| {
        let _ = doc.add_paper_space_entity(on(e, "A-ANNO", Color::ByLayer));
    };
    // 도곽 (A3 가로 420 × 297, 여백 10)
    let mut frame = LwPolyline::from_points(vec![
        Vector2::new(10.0, 10.0),
        Vector2::new(410.0, 10.0),
        Vector2::new(410.0, 287.0),
        Vector2::new(10.0, 287.0),
    ]);
    frame.is_closed = true;
    paper(doc, EntityType::LwPolyline(frame));
    // 표제란
    paper(
        doc,
        EntityType::Line(Line::from_coords(290.0, 10.0, 0.0, 290.0, 40.0, 0.0)),
    );
    paper(
        doc,
        EntityType::Line(Line::from_coords(290.0, 40.0, 0.0, 410.0, 40.0, 0.0)),
    );
    let mut title = Text::with_value("평면도  1:100", Vector3::new(300.0, 22.0, 0.0));
    title.height = 5.0;
    paper(doc, EntityType::Text(title));
    // 종이 전체 뷰포트 (AutoCAD 는 배치마다 이것을 첫 뷰포트로 둔다 — DWG 는 번호를 저장하지 않아
    // 읽을 때 순서대로 번호가 붙으므로, 이것이 먼저 와야 실제 뷰포트가 2 번이 된다. 구현 노트 §13)
    let mut overall = acadrust::entities::Viewport::new();
    overall.id = 1;
    overall.center = Vector3::new(210.0, 148.5, 0.0);
    overall.width = 462.0;
    overall.height = 326.7;
    overall.view_center = Vector3::new(210.0, 148.5, 0.0);
    overall.view_height = 326.7;
    overall.view_direction = Vector3::new(0.0, 0.0, 1.0);
    overall.status.is_on = true;
    paper(doc, EntityType::Viewport(Box::new(overall)));
    // 뷰포트: 도곽 안쪽 위, 모형 왼쪽 아래 칸들을 1:100 으로
    let rows = cells.div_ceil(cols.max(1));
    let (mw, mh) = (cols as f64 * w, rows as f64 * h);
    let mut vp = acadrust::entities::Viewport::new();
    vp.id = 2;
    vp.center = Vector3::new(210.0, 160.0, 0.0);
    vp.width = 380.0;
    vp.height = 220.0;
    vp.view_height = 220.0 * 100.0;
    vp.view_center = Vector3::new((mw / 2.0).min(19000.0), (mh / 2.0).min(11000.0), 0.0);
    vp.view_direction = Vector3::new(0.0, 0.0, 1.0);
    vp.status.is_on = true;
    paper(doc, EntityType::Viewport(Box::new(vp)));
}

/// [`plan_grid`] 를 파일로 쓴다. 확장자가 `.dwg` 면 DWG, 아니면 DXF.
pub fn write_plan_grid(n: usize, path: impl AsRef<Path>) -> Result<(), String> {
    let path = path.as_ref();
    let doc = plan_grid(n);
    let dwg = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("dwg"));
    if dwg {
        DwgWriter::write_to_file(path, &doc).map_err(|e| e.to_string())
    } else {
        DxfWriter::new(&doc)
            .write_to_file(path)
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::{from_document, read_file};
    use crate::{Drawing, Shape};

    fn texts(d: &Drawing) -> Vec<&str> {
        d.entities
            .iter()
            .filter_map(|e| match &e.shape {
                Shape::Text(t) => Some(t.value.as_str()),
                _ => None,
            })
            .collect()
    }

    fn round_trip(ext: &str) {
        let dir = std::env::temp_dir().join(format!("dsmith-test-{}-{ext}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("grid.{ext}"));
        write_plan_grid(100, &path).unwrap();
        let d = read_file(&path).unwrap();
        let direct = from_document(&plan_grid(100));
        let cells = 100usize.div_ceil(FILE_PER_CELL);
        assert_eq!(
            direct.entities.len(),
            cells * EXPANDED_PER_CELL,
            "{:?}",
            direct.skipped
        );
        assert!(direct.skipped.is_empty(), "{:?}", direct.skipped);
        assert_eq!(
            d.entities.len(),
            direct.entities.len(),
            "{ext}: {:?}",
            d.skipped
        );
        assert_eq!(d.bbox(), direct.bbox(), "{ext}");
        let t = texts(&d);
        assert!(
            t.contains(&"거실") && t.contains(&"다용도실"),
            "{ext}: {t:?}"
        );
        // 문 선분은 A-DOOR 레이어·빨강(레이어 색)으로
        let door = d.layers.iter().position(|l| l.name == "A-DOOR").unwrap() as u32;
        let n = d.entities.iter().filter(|e| e.layer == door).count();
        assert_eq!(n, cells * 2, "{ext}");
        // 치수 문자 = 칸 폭, 서식 코드 없이
        assert!(t.contains(&"6000"), "{ext}: {t:?}");
        // 중심선 레이어 객체는 CENTER 무늬
        let grid = d.layers.iter().position(|l| l.name == "A-GRID").unwrap() as u32;
        let lt = d
            .entities
            .iter()
            .find(|e| e.layer == grid)
            .and_then(|e| e.linetype)
            .expect("선 종류");
        assert_eq!(d.linetypes[lt.pattern as usize].name, "CENTER", "{ext}");
        // 레이어 선 굵기: 벽 0.5 mm, 치수 블록 안(「0」 레이어 → 치수 레이어) 0.13 mm
        let wall = d.layers.iter().position(|l| l.name == "A-WALL").unwrap() as u32;
        let dims = d.layers.iter().position(|l| l.name == "A-DIMS").unwrap() as u32;
        let w_of = |layer: u32| {
            d.entities
                .iter()
                .find(|e| {
                    e.layer == layer && !matches!(e.shape, Shape::Text(_) | Shape::Fill { .. })
                })
                .and_then(|e| e.weight)
        };
        assert_eq!(w_of(wall), Some(50), "{ext}");
        assert_eq!(w_of(dims), Some(13), "{ext}");
        // 배치: 도곽·표제란(한글)과 뷰포트 하나, 1:100
        let lay = d.layouts.first().expect("배치");
        assert_eq!(lay.viewports.len(), 1, "{ext}");
        assert!(
            (lay.viewports[0].view_height / lay.viewports[0].height - 100.0).abs() < 1e-9,
            "{ext}"
        );
        let sheet = crate::layout::compose(&d, 0).unwrap();
        assert!(texts(&sheet).contains(&"평면도  1:100"), "{ext}");
        assert!(
            sheet.entities.len() > lay.paper.len(),
            "모형이 뷰포트에 들어와야 함 ({ext})"
        );
        // 해치: 무늬(선분 묶음)와 솔리드(채움)가 모두 있다
        let kinds = d.counts();
        assert!(
            kinds.contains_key("해치 무늬") && kinds.contains_key("채움"),
            "{ext}: {kinds:?}"
        );
        assert_eq!(d.layers[door as usize].color, [255, 0, 0], "{ext}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn grid_round_trips_through_dxf() {
        round_trip("dxf");
    }

    #[test]
    fn grid_round_trips_through_dwg() {
        round_trip("dwg");
    }
}
