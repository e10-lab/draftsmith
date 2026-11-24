//! M0 측정용 도면 만들기.
//!
//! 격자 모양 평면(벽선·기둥 원·실 이름 문자·문 블록)을 원하는 개수만큼 만들어 DXF 나 DWG 로 쓴다.
//! 실 이름은 한글이라 문자 코드 처리가, 문은 블록 참조(대칭 포함)라 블록 펼치기가 함께 시험된다.

use std::path::Path;

use acadrust::entities::{Arc, Circle, Insert, Line, LwPolyline, Text};
use acadrust::tables::{BlockRecord, Layer};
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

/// 대략 `n` 개 객체로 된 도면. 한 칸(6 m × 4.8 m)에 파일 객체 10 개
/// (벽선 4, 안쪽 폴리선 1, 기둥 3, 실 이름 1, 문 블록 참조 1 — 문은 펼치면 2 개).
pub fn plan_grid(n: usize) -> CadDocument {
    let mut doc = CadDocument::new();
    for (name, color) in [("A-WALL", 7), ("A-COLS", 4), ("A-ANNO", 2), ("A-DOOR", 1)] {
        let _ = doc.layers.add(Layer::with_color(name, Color::Index(color)));
    }
    define_door(&mut doc);
    let cells = n.div_ceil(10).max(1);
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
    }
    doc
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
                Shape::Text { value, .. } => Some(value.as_str()),
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
        // 10 칸 × (파일 객체 9 + 문 블록 펼친 2)
        assert_eq!(direct.entities.len(), 110);
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
        assert_eq!(n, 20, "{ext}");
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
