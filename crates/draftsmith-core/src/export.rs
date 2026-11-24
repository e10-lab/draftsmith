//! DXF·DWG 내보내기 (도면 모델 → acadrust 문서). 구현 노트 §20.
//!
//! 모형 공간의 보이는 객체를 그대로 쓴다 (블록은 이미 펼쳐져 있어 펼친 채로). 레이어·색 번호·선 종류·선 굵기를 살린다.
//! 채움은 솔리드 해치, 해치 무늬 선분은 선으로. 배치는 아직 쓰지 않는다.

use std::path::Path;

use acadrust::entities::{
    Arc, BoundaryEdge, BoundaryPath, Circle, Ellipse, Hatch, Line, LwPolyline, LwVertex, MText,
    PolylineEdge, Text, TextHorizontalAlignment, TextVerticalAlignment,
};
use acadrust::tables::{Layer as CadLayer, LineType, LineTypeElement};
use acadrust::types::{Color, LineWeight, Vector2, Vector3};
use acadrust::{CadDocument, DwgWriter, DxfWriter, EntityType};

use crate::drawing::{Drawing, Entity, Shape, VAlign};
use crate::geom::Vec2;

fn v3(p: Vec2) -> Vector3 {
    Vector3::new(p.x, p.y, 0.0)
}

/// RGB 가 표준 색 번호 색과 같으면 색 번호로, 아니면 트루컬러로
fn cad_color(rgb: [u8; 3]) -> Color {
    let tc = Color::Rgb {
        r: rgb[0],
        g: rgb[1],
        b: rgb[2],
    };
    let i = tc.approximate_index();
    if (1..=255).contains(&i) {
        let ix = Color::from_index(i);
        if ix.rgb() == Some((rgb[0], rgb[1], rgb[2])) {
            return ix;
        }
    }
    tc
}

/// 켤레 반지름 (u, v) → 주축 (긴 축 벡터, 짧은/긴 비, 매개변수 이동, 돌아가는 방향이 시계인가)
fn principal_axes(u: Vec2, v: Vec2) -> (Vec2, f64, f64, bool) {
    let uu = u.x * u.x + u.y * u.y;
    let vv = v.x * v.x + v.y * v.y;
    let uv = u.x * v.x + u.y * v.y;
    let mut t0 = 0.5 * (2.0 * uv).atan2(uu - vv);
    let at = |t: f64| Vec2::new(u.x * t.cos() + v.x * t.sin(), u.y * t.cos() + v.y * t.sin());
    let mut major = at(t0);
    let mut minor = at(t0 + std::f64::consts::FRAC_PI_2);
    if minor.x.hypot(minor.y) > major.x.hypot(major.y) {
        t0 += std::f64::consts::FRAC_PI_2;
        std::mem::swap(&mut major, &mut minor);
        minor = Vec2::new(-minor.x, -minor.y);
    }
    let cw = major.x * minor.y - major.y * minor.x < 0.0;
    let ratio = minor.x.hypot(minor.y) / major.x.hypot(major.y).max(1e-300);
    (major, ratio.min(1.0), t0, cw)
}

/// 객체 하나 → acadrust 객체들
fn to_cad(e: &Entity) -> Vec<EntityType> {
    match &e.shape {
        Shape::Line { a, b } => vec![EntityType::Line(Line::from_coords(
            a.x, a.y, 0.0, b.x, b.y, 0.0,
        ))],
        Shape::Polyline { points, closed } => {
            let mut p = LwPolyline::new();
            p.vertices = points
                .iter()
                .map(|(q, g)| {
                    let mut v = LwVertex::new(Vector2::new(q.x, q.y));
                    v.bulge = *g;
                    v
                })
                .collect();
            p.is_closed = *closed;
            vec![EntityType::LwPolyline(p)]
        }
        Shape::Circle { center, radius } => vec![EntityType::Circle(Circle::from_coords(
            center.x, center.y, 0.0, *radius,
        ))],
        Shape::Arc {
            center,
            radius,
            start,
            end,
        } => {
            let mut a = Arc::new();
            a.center = v3(*center);
            a.radius = *radius;
            a.start_angle = *start;
            a.end_angle = *end;
            vec![EntityType::Arc(a)]
        }
        Shape::Ellipse {
            center,
            u,
            v,
            start,
            end,
        } => {
            let (major, ratio, t0, cw) = principal_axes(*u, *v);
            let mut el = Ellipse::from_center_axes(v3(*center), v3(major), ratio);
            // 시계 방향이면 돌출 방향을 −Z 로 (읽을 때와 같은 규칙)
            let (s, t) = if cw {
                (-(end - t0), -(start - t0))
            } else {
                (start - t0, end - t0)
            };
            if cw {
                el.normal = Vector3::new(0.0, 0.0, -1.0);
                // −Z 에서는 짧은 축 방향이 뒤집혀 매개변수도 거울상
                el.start_parameter = -t;
                el.end_parameter = -s;
            } else {
                el.start_parameter = s;
                el.end_parameter = t;
            }
            vec![EntityType::Ellipse(el)]
        }
        Shape::Fill { loops } => {
            let mut h = Hatch::solid();
            for r in loops {
                let mut path = BoundaryPath::new();
                path.add_edge(BoundaryEdge::Polyline(PolylineEdge::new(
                    r.iter().map(|p| Vector2::new(p.x, p.y)).collect(),
                    true,
                )));
                h.add_path(path);
            }
            vec![EntityType::Hatch(Box::new(h))]
        }
        Shape::Segments { lines } => lines
            .iter()
            .map(|(a, b)| EntityType::Line(Line::from_coords(a.x, a.y, 0.0, b.x, b.y, 0.0)))
            .collect(),
        Shape::Text(t) if t.value.contains('\n') => {
            let mut m = MText::new();
            m.value = t.value.replace('\n', "\\P");
            m.insertion_point = v3(t.at);
            m.height = t.height;
            m.rotation = t.rotation;
            let row = match t.valign {
                VAlign::Top => 0,
                VAlign::Middle => 1,
                _ => 2,
            };
            let col = (t.halign * 2.0).round() as i32;
            m.attachment_point = match row * 3 + col {
                0 => acadrust::entities::AttachmentPoint::TopLeft,
                1 => acadrust::entities::AttachmentPoint::TopCenter,
                2 => acadrust::entities::AttachmentPoint::TopRight,
                3 => acadrust::entities::AttachmentPoint::MiddleLeft,
                4 => acadrust::entities::AttachmentPoint::MiddleCenter,
                5 => acadrust::entities::AttachmentPoint::MiddleRight,
                6 => acadrust::entities::AttachmentPoint::BottomLeft,
                7 => acadrust::entities::AttachmentPoint::BottomCenter,
                _ => acadrust::entities::AttachmentPoint::BottomRight,
            };
            vec![EntityType::MText(Box::new(m))]
        }
        Shape::Text(t) => {
            let mut x = Text::with_value(t.value.clone(), v3(t.at));
            x.height = t.height;
            x.rotation = t.rotation;
            x.width_factor = t.width_factor;
            x.horizontal_alignment = if t.halign >= 0.75 {
                TextHorizontalAlignment::Right
            } else if t.halign >= 0.25 {
                TextHorizontalAlignment::Center
            } else {
                TextHorizontalAlignment::Left
            };
            x.vertical_alignment = match t.valign {
                VAlign::Baseline => TextVerticalAlignment::Baseline,
                VAlign::Bottom => TextVerticalAlignment::Bottom,
                VAlign::Middle => TextVerticalAlignment::Middle,
                VAlign::Top => TextVerticalAlignment::Top,
            };
            if !(t.halign < 0.25 && t.valign == VAlign::Baseline) {
                x.alignment_point = Some(v3(t.at));
            }
            vec![EntityType::Text(x)]
        }
    }
}

/// 도면 → acadrust 문서
pub fn to_document(d: &Drawing) -> CadDocument {
    let mut doc = CadDocument::new();
    for p in &d.linetypes {
        let mut lt = LineType::new(p.name.clone());
        for &x in &p.dashes {
            lt.elements.push(if x > 0.0 {
                LineTypeElement::dash(x)
            } else if x < 0.0 {
                LineTypeElement::space(-x)
            } else {
                LineTypeElement::dot()
            });
        }
        lt.pattern_length = p.dashes.iter().map(|x| x.abs()).sum();
        let _ = doc.line_types.add(lt);
    }
    for l in &d.layers {
        if l.name == "0" {
            if let Some(z) = doc.layers.iter_mut().find(|z| z.name == "0") {
                z.color = cad_color(l.color);
            }
            continue;
        }
        let _ = doc
            .layers
            .add(CadLayer::with_color(l.name.clone(), cad_color(l.color)));
    }
    for e in &d.entities {
        let layer = d
            .layers
            .get(e.layer as usize)
            .map(|l| l.name.clone())
            .unwrap_or_else(|| "0".into());
        for mut c in to_cad(e) {
            let common = c.common_mut();
            common.layer = layer.clone();
            common.color = match (e.aci, e.color) {
                (Some(i), _) => Color::from_index(i as i16),
                (None, Some(rgb)) => cad_color(rgb),
                (None, None) => Color::ByLayer,
            };
            if let Some(lt) = e.linetype
                && let Some(p) = d.linetypes.get(lt.pattern as usize)
                && !matches!(e.shape, Shape::Fill { .. } | Shape::Text(_))
            {
                common.linetype = p.name.clone();
                common.linetype_scale = lt.scale;
            }
            if let Some(w) = e.weight {
                common.line_weight = LineWeight::Value(w as i16);
            }
            let _ = doc.add_entity(c);
        }
    }
    doc
}

/// 확장자로 DXF·DWG 를 골라 쓴다
pub fn write(d: &Drawing, path: &Path) -> Result<(), String> {
    let doc = to_document(d);
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

    #[test]
    fn export_and_read_back() {
        let d = crate::import::from_document(&crate::sample::plan_grid(60));
        for ext in ["dxf", "dwg"] {
            let dir =
                std::env::temp_dir().join(format!("dsmith-export-{}-{ext}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let p = dir.join(format!("out.{ext}"));
            write(&d, &p).unwrap();
            let back = crate::import::read_file(&p).unwrap();
            // 펼친 객체 수는 같거나(해치 무늬 선분이 선 여러 개가 되어) 더 많다
            assert!(
                back.entities.len() >= d.entities.len(),
                "{ext}: {} < {}",
                back.entities.len(),
                d.entities.len()
            );
            let (a, b) = (d.bbox(), back.bbox());
            assert!(
                (a.width() - b.width()).abs() < 1e-6 && (a.height() - b.height()).abs() < 1e-6,
                "{ext}"
            );
            assert!(back.layers.iter().any(|l| l.name == "A-WALL"), "{ext}");
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    #[test]
    fn conjugate_to_principal() {
        // 원 반지름 2 를 (1,0)/(1,1) 같은 비스듬한 켤레로 써도 주축 길이는 같아야
        let (major, ratio, _, _) = principal_axes(Vec2::new(3.0, 0.0), Vec2::new(0.0, 1.0));
        assert!((major.x.hypot(major.y) - 3.0).abs() < 1e-9 && (ratio - 1.0 / 3.0).abs() < 1e-9);
    }
}
