//! PDF 출력 (krilla). 선·채움은 화면용 표시 데이터([`crate::display::segments`])를 그대로 쓰고
//! (선 종류·곡선 펼치기가 같게 나온다), 문자는 내장 Pretendard 를 부분 삽입해 글자 그대로 쓴다.
//!
//! 용지·축척·범위·색(컬러/흑백)을 고른다. 축척이 없으면 범위를 용지에 맞춘다.

use krilla::Document;
use krilla::color::rgb;
use krilla::geom::{PathBuilder, Point, Rect, Transform};
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::{Fill, FillRule, LineCap, LineJoin, Stroke};
use krilla::text::{Font, TextDirection};

use crate::display;
use crate::drawing::{Drawing, Shape, VAlign};
use crate::geom::{Bbox, Vec2};

/// 내장 글꼴 (SIL OFL 1.1, `fonts/Pretendard-LICENSE.txt`)
static FONT: &[u8] = include_bytes!("../fonts/Pretendard-Regular.otf");

/// 1 mm 가 몇 pt 인가
const PT_PER_MM: f64 = 72.0 / 25.4;
/// 도면 글자 높이(대문자·한글 높이)에 맞추려면 글꼴 크기를 키운다 (화면과 같은 값)
const EM_PER_HEIGHT: f64 = 1.35;
/// MTEXT 줄 간격 (글자 높이의 5/3)
const LINE_GAP: f64 = 5.0 / 3.0;

/// 용지 이름 → (가로, 세로) mm, 세로 방향 기준
pub fn paper_size(name: &str) -> Option<(f64, f64)> {
    Some(match name.to_ascii_uppercase().as_str() {
        "A4" => (210.0, 297.0),
        "A3" => (297.0, 420.0),
        "A2" => (420.0, 594.0),
        "A1" => (594.0, 841.0),
        "A0" => (841.0, 1189.0),
        _ => return None,
    })
}

/// PDF 출력 설정
#[derive(Debug, Clone)]
pub struct PdfOptions {
    /// 용지 가로·세로 (mm)
    pub paper_mm: (f64, f64),
    /// 여백 (mm)
    pub margin_mm: f64,
    /// 축척 분모 (1:100 이면 100) — 도면 단위가 mm 일 때 기준. None 이면 용지에 맞춤
    pub scale: Option<f64>,
    /// 출력할 범위 (도면 좌표). None 이면 도면 전체
    pub area: Option<Bbox>,
    /// 흑백 (모든 선·글자·채움을 검정·회색으로)
    pub mono: bool,
    /// 선 굵기 (mm)
    pub line_mm: f64,
}

impl Default for PdfOptions {
    fn default() -> Self {
        PdfOptions {
            paper_mm: (420.0, 297.0),
            margin_mm: 10.0,
            scale: None,
            area: None,
            mono: false,
            line_mm: 0.18,
        }
    }
}

/// 흰 종이용 색: 흰색은 검정, 밝은 색은 어둡게 (화면 흰 배경과 같은 규칙). 흑백이면 검정 (채움은 회색 유지).
fn paper_color(c: [u8; 3], mono: bool, fill: bool) -> rgb::Color {
    if mono {
        if fill {
            // 채움은 밝기를 회색으로 (솔리드 해치가 새까맣게 덮지 않게)
            let lum = (0.299 * c[0] as f64 + 0.587 * c[1] as f64 + 0.114 * c[2] as f64) / 255.0;
            let g = (255.0 * (0.35 + 0.5 * (1.0 - lum).min(1.0))).round() as u8;
            return rgb::Color::new(g, g, g);
        }
        return rgb::Color::new(0, 0, 0);
    }
    if c.iter().all(|&v| v > 235) {
        return rgb::Color::new(0, 0, 0);
    }
    let lum = (0.299 * c[0] as f64 + 0.587 * c[1] as f64 + 0.114 * c[2] as f64) / 255.0;
    let k = if lum > 0.5 { 0.45 / lum } else { 1.0 };
    let f = |v: u8| (v as f64 * k).round() as u8;
    rgb::Color::new(f(c[0]), f(c[1]), f(c[2]))
}

/// 글자 폭 (글꼴 단위 → 글꼴 크기 1 기준). 없는 글자는 1 em 으로.
fn text_width(face: &ttf_parser::Face, s: &str) -> f64 {
    let upem = face.units_per_em() as f64;
    s.chars()
        .map(|ch| {
            face.glyph_index(ch)
                .and_then(|g| face.glyph_hor_advance(g))
                .map(|a| a as f64 / upem)
                .unwrap_or(1.0)
        })
        .sum()
}

/// 도면을 PDF 한 쪽으로
pub fn export(d: &Drawing, o: &PdfOptions) -> Result<Vec<u8>, String> {
    let area = o.area.unwrap_or_else(|| d.bbox());
    if area.is_empty() {
        return Err("출력할 것이 없습니다".into());
    }
    let (pw, ph) = o.paper_mm;
    let (aw, ah) = (pw - 2.0 * o.margin_mm, ph - 2.0 * o.margin_mm);
    if aw <= 0.0 || ah <= 0.0 {
        return Err("여백이 용지보다 큽니다".into());
    }
    // 도면 단위 1 이 용지에서 몇 mm 인가
    let (m_per_unit, _) = d.meters_per_unit();
    let mm_per_unit = match o.scale {
        Some(s) if s > 0.0 => m_per_unit * 1000.0 / s,
        _ => (aw / area.width().max(1e-9)).min(ah / area.height().max(1e-9)),
    };
    // 범위를 용지 가운데에
    let k = mm_per_unit * PT_PER_MM; // 도면 단위 → pt
    let (page_w, page_h) = (pw * PT_PER_MM, ph * PT_PER_MM);
    let c = area.center();
    let to_page = |x: f64, y: f64| -> (f32, f32) {
        (
            (page_w / 2.0 + (x - c.x) * k) as f32,
            (page_h / 2.0 - (y - c.y) * k) as f32,
        )
    };

    let font_data = FONT.to_vec();
    let face = ttf_parser::Face::parse(FONT, 0).map_err(|e| e.to_string())?;
    let font = Font::new(font_data.into(), 0).ok_or("글꼴을 읽지 못했습니다")?;

    let mut doc = Document::new();
    let settings =
        PageSettings::from_wh(page_w as f32, page_h as f32).ok_or("용지 크기가 잘못되었습니다")?;
    let mut page = doc.start_page_with(settings);
    let mut surface = page.surface();

    // 출력 범위로 자르기
    let (x0, y0) = to_page(area.min.x, area.max.y);
    let (x1, y1) = to_page(area.max.x, area.min.y);
    if let Some(r) = Rect::from_ltrb(x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)) {
        let mut pb = PathBuilder::new();
        pb.push_rect(r);
        if let Some(clip) = pb.finish() {
            surface.push_clip_path(&clip, &FillRule::NonZero);
        }
    }

    // 선·채움: 색마다 경로 하나로 묶는다
    let s = display::segments(d);
    let o_x = s.origin.x;
    let o_y = s.origin.y;
    let mut by_color: std::collections::BTreeMap<[u8; 3], PathBuilder> = Default::default();
    for (i, q) in s.tri_colors.chunks_exact(4).enumerate() {
        let t = &s.tris[i * 6..i * 6 + 6];
        let pb = by_color.entry([q[0], q[1], q[2]]).or_default();
        let p = |j: usize| to_page(t[j * 2] as f64 + o_x, t[j * 2 + 1] as f64 + o_y);
        let (a, b, cc) = (p(0), p(1), p(2));
        pb.move_to(a.0, a.1);
        pb.line_to(b.0, b.1);
        pb.line_to(cc.0, cc.1);
        pb.close();
    }
    for (col, pb) in std::mem::take(&mut by_color) {
        if let Some(path) = pb.finish() {
            surface.set_stroke(None);
            surface.set_fill(Some(Fill {
                paint: paper_color(col, o.mono, true).into(),
                opacity: NormalizedF32::ONE,
                rule: FillRule::NonZero,
            }));
            surface.draw_path(&path);
        }
    }
    for (i, q) in s.colors.chunks_exact(4).enumerate() {
        let t = &s.coords[i * 4..i * 4 + 4];
        let pb = by_color.entry([q[0], q[1], q[2]]).or_default();
        let a = to_page(t[0] as f64 + o_x, t[1] as f64 + o_y);
        let b = to_page(t[2] as f64 + o_x, t[3] as f64 + o_y);
        pb.move_to(a.0, a.1);
        pb.line_to(b.0, b.1);
    }
    let width = (o.line_mm * PT_PER_MM) as f32;
    surface.set_fill(None);
    for (col, pb) in by_color {
        if let Some(path) = pb.finish() {
            surface.set_stroke(Some(Stroke {
                paint: paper_color(col, o.mono, false).into(),
                width,
                miter_limit: 4.0,
                line_cap: LineCap::Round,
                line_join: LineJoin::Round,
                opacity: NormalizedF32::ONE,
                dash: None,
            }));
            surface.draw_path(&path);
        }
    }
    surface.set_stroke(None);

    // 문자
    for e in &d.entities {
        let Shape::Text(t) = &e.shape else { continue };
        if t.value.trim().is_empty() || d.layers.get(e.layer as usize).is_some_and(|l| !l.visible) {
            continue;
        }
        let size = t.height * k * EM_PER_HEIGHT; // pt
        if size < 0.5 || !area_hits(&area, t.at, t.height * 50.0) {
            continue;
        }
        surface.set_fill(Some(Fill {
            paint: paper_color(d.color_of(e), o.mono, false).into(),
            opacity: NormalizedF32::ONE,
            rule: FillRule::NonZero,
        }));
        let (px, py) = to_page(t.at.x, t.at.y);
        let (sn, cs) = t.rotation.sin_cos();
        let wf = if t.width_factor > 0.0 {
            t.width_factor
        } else {
            1.0
        };
        // 글자 좌표(오른쪽 u, 아래 v, pt) → 용지 (y 아래로): 도면 반시계 회전은 용지에서 시계 반대 그대로
        let tf = Transform::from_row(
            (cs * wf) as f32,
            (-sn * wf) as f32,
            sn as f32,
            cs as f32,
            px,
            py,
        );
        surface.push_transform(&tf);
        let lines: Vec<&str> = t.value.split('\n').collect();
        let h = t.height * k;
        let gap = h * LINE_GAP;
        let n = lines.len() as f64;
        // 첫 줄 바탕선이 0 일 때 글자 덩어리 [−h, (n−1)·gap] (아래가 +)
        let dy = match t.valign {
            VAlign::Baseline => 0.0,
            VAlign::Bottom => -(n - 1.0) * gap,
            VAlign::Middle => -((n - 1.0) * gap - h) / 2.0,
            VAlign::Top => h,
        };
        for (i, line) in lines.iter().enumerate() {
            let w = text_width(&face, line) * size;
            let x = -w * t.halign;
            let y = dy + i as f64 * gap;
            surface.draw_text(
                Point::from_xy(x as f32, y as f32),
                font.clone(),
                size as f32,
                line,
                false,
                TextDirection::Auto,
            );
        }
        surface.pop();
    }
    surface.pop(); // 자르기
    surface.finish();
    page.finish();
    doc.finish()
        .map_err(|e| format!("PDF 를 만들지 못했습니다: {e:?}"))
}

/// 점이 범위(여유 `pad`)에 드나
fn area_hits(a: &Bbox, p: Vec2, pad: f64) -> bool {
    p.x >= a.min.x - pad && p.x <= a.max.x + pad && p.y >= a.min.y - pad && p.y <= a.max.y + pad
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_plan_to_pdf() {
        let d = crate::import::from_document(&crate::sample::plan_grid(60));
        let pdf = export(&d, &PdfOptions::default()).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
        // 1:100 A3 세로 흑백도 된다
        let o = PdfOptions {
            paper_mm: (297.0, 420.0),
            scale: Some(100.0),
            mono: true,
            ..Default::default()
        };
        assert!(export(&d, &o).unwrap().len() > 1000);
        // 한글 글꼴이 부분 삽입되어 원본(1.6 MB)보다 훨씬 작다
        assert!(pdf.len() < 600_000, "{}", pdf.len());
    }
}
