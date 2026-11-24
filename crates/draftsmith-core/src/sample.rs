//! M0 측정용 도면 만들기.
//!
//! 격자 모양 평면(벽선·기둥 원·실 이름 문자)을 원하는 개수만큼 만들어 DXF 로 쓴다.
//! 실 이름은 한글이라 DXF 문자 코드 처리도 함께 시험된다.

use acadrust::entities::{Circle, Line, LwPolyline, Text};
use acadrust::types::{Vector2, Vector3};
use acadrust::{CadDocument, DxfWriter, EntityType};

const ROOMS: [&str; 6] = ["거실", "주방", "침실", "욕실", "현관", "다용도실"];

/// 대략 `n` 개 객체로 된 도면. 한 칸(6 m × 4.8 m)에 객체 10 개.
pub fn plan_grid(n: usize) -> CadDocument {
    let mut doc = CadDocument::new();
    let cells = n.div_ceil(10).max(1);
    let cols = (cells as f64).sqrt().ceil() as usize;
    let (w, h) = (6000.0, 4800.0);
    for k in 0..cells {
        let (i, j) = ((k % cols) as f64, (k / cols) as f64);
        let (x, y) = (i * w, j * h);
        // 벽: 바깥선 4 개 + 안쪽 폴리선 1 개
        for (a, b) in [
            ((x, y), (x + w, y)),
            ((x + w, y), (x + w, y + h)),
            ((x + w, y + h), (x, y + h)),
            ((x, y + h), (x, y)),
        ] {
            let _ = doc.add_entity(EntityType::Line(Line::from_coords(
                a.0, a.1, 0.0, b.0, b.1, 0.0,
            )));
        }
        let inner = LwPolyline::from_points(vec![
            Vector2::new(x + 200.0, y + 200.0),
            Vector2::new(x + w - 200.0, y + 200.0),
            Vector2::new(x + w - 200.0, y + h - 200.0),
            Vector2::new(x + 200.0, y + h - 200.0),
        ]);
        let mut inner = inner;
        inner.is_closed = true;
        let _ = doc.add_entity(EntityType::LwPolyline(inner));
        // 기둥 4 개
        for (cx, cy) in [(x, y), (x + w, y), (x + w, y + h), (x, y + h)] {
            let _ = doc.add_entity(EntityType::Circle(Circle::from_coords(cx, cy, 0.0, 150.0)));
        }
        // 실 이름
        let mut t = Text::with_value(
            ROOMS[k % ROOMS.len()],
            Vector3::new(x + 600.0, y + 600.0, 0.0),
        );
        t.height = 300.0;
        let _ = doc.add_entity(EntityType::Text(t));
    }
    doc
}

/// [`plan_grid`] 를 DXF 파일로 쓴다.
pub fn write_plan_grid(n: usize, path: impl AsRef<std::path::Path>) -> Result<(), String> {
    let doc = plan_grid(n);
    DxfWriter::new(&doc)
        .write_to_file(path)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::from_document;

    #[test]
    fn grid_round_trips_through_dxf_with_hangul() {
        let dir = std::env::temp_dir().join(format!("dsmith-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("grid.dxf");
        write_plan_grid(100, &path).unwrap();
        let d = crate::import::read_file(&path).unwrap();
        let direct = from_document(&plan_grid(100));
        assert_eq!(d.entities.len(), direct.entities.len());
        assert_eq!(d.entities.len(), 100);
        let texts: Vec<_> = d
            .entities
            .iter()
            .filter_map(|e| match &e.shape {
                crate::Shape::Text { value, .. } => Some(value.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            texts.contains(&"거실") && texts.contains(&"다용도실"),
            "{texts:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
