//! 도면 시트 (M4, 구현 노트 §21): 용지·도면 틀·표제란·뷰포트 하나를 갖춘 배치를 만든다.
//!
//! 용지는 가로 방향 KS A 계열(A3·A2·A1), 단위 mm. 틀은 왼쪽 철 여백 20, 나머지 10. 표제란은 오른쪽 아래
//! 180 × 48 (프로젝트·도면명·도면 번호·축척·날짜). 뷰포트는 틀 안 전체 (표제란과 겹치는 아래 띠 제외)에
//! 모형(또는 고른 층)의 가운데를 축척 1:N 으로 보인다.

use crate::archgen;
use crate::drawing::{Drawing, Entity, Shape, TextShape, VAlign};
use crate::geom::Vec2;
use crate::layout::{Layout, ViewportView};

/// 시트 요청
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SheetReq {
    /// 도면명 (배치 이름도 「번호 도면명」)
    pub title: String,
    pub number: String,
    /// A3 · A2 · A1
    pub paper: String,
    /// 축척 분모 (100 → 1:100)
    pub scale: f64,
    /// 보일 층 (건축 객체). None 이면 지금 층
    pub level: Option<u64>,
    pub project: String,
    /// 도면 목록 시트 (뷰포트 없이 목록 표)
    #[serde(default)]
    pub list: bool,
}

/// 가로 방향 용지 크기 (mm)
pub fn paper_size(name: &str) -> Option<(f64, f64)> {
    Some(match name.to_ascii_uppercase().as_str() {
        "A4" => (297.0, 210.0),
        "A3" => (420.0, 297.0),
        "A2" => (594.0, 420.0),
        "A1" => (841.0, 594.0),
        "A0" => (1189.0, 841.0),
        _ => return None,
    })
}

/// 오늘 날짜 (YYYY.MM.DD, UTC) — 외부 상자 없이 날 수에서 셈 (Howard Hinnant 의 civil_from_days)
pub fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y}.{m:02}.{d:02}")
}

/// 시트에 보일 모형: 그 층의 건축 객체를 펼친 도면
pub fn model_for(d: &Drawing, level: Option<u64>) -> Drawing {
    model_at_scale(d, level, 100.0)
}

/// 시트 축척에 맞춘 표현으로 (§42)
pub fn model_at_scale(d: &Drawing, level: Option<u64>, scale: f64) -> Drawing {
    let mut m = d.clone();
    if let Some(l) = level
        && m.arch.levels.iter().any(|x| x.id == l)
    {
        m.arch.active_level = l;
    }
    archgen::with_arch_scaled(&m, scale)
        .map(|(v, _)| v)
        .unwrap_or(m)
}

/// 배치 `i` 를 종이 도면으로 (층이 정해진 시트면 그 층으로 펼쳐서)
pub fn compose(d: &Drawing, i: usize) -> Option<Drawing> {
    let lay = d.layouts.get(i)?;
    let scale = scale_of(lay).unwrap_or(100.0);
    let mut out = crate::layout::compose(&model_at_scale(d, lay.level, scale), i)?;
    if lay.list {
        let frame = {
            let mut b = crate::geom::Bbox::EMPTY;
            for e in &lay.paper {
                b.merge(&e.shape.bbox());
            }
            b
        };
        let layer = out.layer_index("A-ANNO-TTLB");
        out.entities.extend(list_table(d, frame, layer));
    }
    Some(out)
}

/// 뷰포트 고치기 (§35). 없는 값은 그대로
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct VpEdit {
    /// 축척 분모 (100 → 1:100)
    pub scale: Option<f64>,
    /// 종이 위 가운데 (mm)
    pub center: Option<[f64; 2]>,
    /// 종이 위 폭·높이 (mm)
    pub size: Option<[f64; 2]>,
    /// 모형에서 보는 가운데
    pub view_center: Option<[f64; 2]>,
    /// 모형(그 층) 전체가 들어오게 가운데·축척을 맞춘다 (표준 축척으로)
    pub fit: bool,
}

/// 표준 축척 (분모)
pub const SCALES: [f64; 22] = [
    1.0, 2.0, 5.0, 10.0, 20.0, 25.0, 30.0, 50.0, 100.0, 150.0, 200.0, 250.0, 300.0, 400.0, 500.0,
    600.0, 1000.0, 1200.0, 1500.0, 2000.0, 3000.0, 5000.0,
];

/// 시트 `i` 의 뷰포트 `k` 를 고친 배치
pub fn edit_viewport(d: &Drawing, i: usize, k: usize, e: &VpEdit) -> Result<Layout, String> {
    let mut lay = d.layouts.get(i).cloned().ok_or("시트가 없습니다")?;
    let level = lay.level;
    let v = lay.viewports.get_mut(k).ok_or("뷰포트가 없습니다")?;
    let scale_now = if v.height > 0.0 {
        v.view_height / v.height
    } else {
        100.0
    };
    if let Some([w, h]) = e.size {
        if w <= 1.0 || h <= 1.0 {
            return Err("뷰포트가 너무 작습니다".into());
        }
        v.width = w;
        v.height = h;
        v.view_height = h * scale_now;
    }
    if let Some([x, y]) = e.center {
        v.center = Vec2::new(x, y);
    }
    if let Some([x, y]) = e.view_center {
        v.view_center = Vec2::new(x, y);
    }
    if let Some(s) = e.scale {
        if s <= 0.0 {
            return Err("축척은 0 보다 커야 합니다".into());
        }
        v.view_height = v.height * s;
    }
    if e.fit {
        let bb = model_for(d, level).bbox();
        if bb.is_empty() {
            return Err("모형에 그린 것이 없습니다".into());
        }
        // 대상점·비틀기 없는 뷰포트 기준 (DraftSmith 시트)
        v.view_center = Vec2::new(
            (bb.min.x + bb.max.x) / 2.0 - v.target.x,
            (bb.min.y + bb.max.y) / 2.0 - v.target.y,
        );
        let need = (bb.width() / v.width).max(bb.height() / v.height) / 0.92;
        let s = SCALES
            .iter()
            .copied()
            .find(|s| *s >= need)
            .unwrap_or(need.ceil());
        v.view_height = v.height * s;
    }
    // 첫 뷰포트 축척이 바뀌면 표제란의 「1:N」 글자도
    let scale_new = lay
        .viewports
        .get(k)
        .map(|v| v.view_height / v.height.max(1e-9))
        .unwrap_or(scale_now);
    if k == 0 && (scale_new - scale_now).abs() > 1e-9 {
        let (old, new) = (
            format!("1:{}", scale_now.round()),
            format!("1:{}", scale_new.round()),
        );
        for e in &mut lay.paper {
            if let Shape::Text(t) = &mut e.shape
                && t.value == old
            {
                t.value = new.clone();
            }
        }
    }
    Ok(lay)
}

/// 뷰포트 하나 더하기 (§42): 종이 왼쪽 아래 1/3 크기, 첫 뷰포트와 같은 가운데·축척 (없으면 1:100)
pub fn add_viewport(d: &Drawing, i: usize) -> Result<Layout, String> {
    let mut lay = d.layouts.get(i).cloned().ok_or("시트가 없습니다")?;
    if lay.list {
        return Err("도면 목록 시트에는 뷰포트를 넣지 않습니다".into());
    }
    let mut frame = crate::geom::Bbox::EMPTY;
    for e in &lay.paper {
        frame.merge(&e.shape.bbox());
    }
    if frame.is_empty() {
        return Err("시트 틀이 없습니다".into());
    }
    let (w, h) = (frame.width() / 3.0, frame.height() / 3.0);
    let first = lay.viewports.first().cloned();
    let scale = first
        .as_ref()
        .map(|v| v.view_height / v.height.max(1e-9))
        .unwrap_or(100.0);
    let view_center = first.as_ref().map(|v| v.view_center).unwrap_or_else(|| {
        let bb = model_for(d, lay.level).bbox();
        if bb.is_empty() {
            Vec2::default()
        } else {
            bb.center()
        }
    });
    lay.viewports.push(ViewportView {
        center: Vec2::new(frame.min.x + 10.0 + w / 2.0, frame.min.y + 10.0 + h / 2.0),
        width: w,
        height: h,
        view_center,
        view_height: h * scale,
        twist: 0.0,
        target: Vec2::default(),
        frozen: Vec::new(),
    });
    Ok(lay)
}

/// 뷰포트 지우기
pub fn remove_viewport(d: &Drawing, i: usize, k: usize) -> Result<Layout, String> {
    let mut lay = d.layouts.get(i).cloned().ok_or("시트가 없습니다")?;
    if k >= lay.viewports.len() {
        return Err("뷰포트가 없습니다".into());
    }
    lay.viewports.remove(k);
    Ok(lay)
}

/// 배치의 축척 (첫 뷰포트에서)/// 배치의 축척 (첫 뷰포트에서). 없으면 None
pub fn scale_of(l: &Layout) -> Option<f64> {
    let v = l.viewports.first()?;
    (v.height > 0.0).then(|| v.view_height / v.height)
}

/// 도면 목록 표: 틀 왼쪽 위에 순번·도면 번호·도면명·축척·층 (§34)
fn list_table(d: &Drawing, frame: crate::geom::Bbox, layer: u32) -> Vec<Entity> {
    let mut out = Vec::new();
    if frame.is_empty() {
        return out;
    }
    let mut put = |shape: Shape, weight: Option<u16>| {
        out.push(Entity {
            layer,
            color: None,
            aci: Some(7),
            linetype: None,
            weight,
            shape,
        });
    };
    let text = |x: f64, y: f64, h: f64, v: String| {
        Shape::Text(TextShape {
            at: Vec2::new(x, y),
            height: h,
            rotation: 0.0,
            width_factor: 1.0,
            halign: 0.0,
            valign: VAlign::Middle,
            value: v,
        })
    };
    let cols = [
        ("순번", 15.0),
        ("도면 번호", 32.0),
        ("도면명", 110.0),
        ("축척", 25.0),
        ("층", 28.0),
    ];
    let width: f64 = cols.iter().map(|c| c.1).sum();
    let (x0, top) = (frame.min.x + 15.0, frame.max.y - 18.0);
    put(text(x0, top, 7.0, "도면 목록".into()), None);
    let rh = 9.0;
    let y0 = top - 10.0;
    let mut rows: Vec<[String; 5]> = vec![cols.map(|c| c.0.to_string())];
    for (i, l) in d.layouts.iter().enumerate() {
        let (num, title) = match l.name.split_once(' ') {
            Some((a, b)) if a.chars().any(|c| c.is_ascii_digit()) => (a.to_string(), b.to_string()),
            _ => (String::new(), l.name.clone()),
        };
        let scale = scale_of(l)
            .map(|k| format!("1:{}", k.round()))
            .unwrap_or_else(|| "-".into());
        let level = l
            .level
            .and_then(|id| d.arch.level(id))
            .map(|x| x.name.clone())
            .unwrap_or_default();
        rows.push([(i + 1).to_string(), num, title, scale, level]);
    }
    let n = rows.len();
    for k in 0..=n {
        let y = y0 - rh * k as f64;
        put(
            Shape::Line {
                a: Vec2::new(x0, y),
                b: Vec2::new(x0 + width, y),
            },
            Some(if k <= 1 || k == n { 35 } else { 18 }),
        );
    }
    let mut x = x0;
    for c in 0..=cols.len() {
        put(
            Shape::Line {
                a: Vec2::new(x, y0),
                b: Vec2::new(x, y0 - rh * n as f64),
            },
            Some(if c == 0 || c == cols.len() { 35 } else { 18 }),
        );
        if c < cols.len() {
            x += cols[c].1;
        }
    }
    for (k, row) in rows.into_iter().enumerate() {
        let y = y0 - rh * (k as f64 + 0.5);
        let mut x = x0;
        for (c, v) in row.into_iter().enumerate() {
            put(text(x + 2.5, y, if k == 0 { 3.0 } else { 3.5 }, v), None);
            x += cols[c].1;
        }
    }
    out
}

/// 시트 하나를 만든다 (도면 `d` 의 레이어에 표제란 레이어를 더할 수 있다)
pub fn make(d: &mut Drawing, req: &SheetReq) -> Result<Layout, String> {
    let (pw, ph) = paper_size(&req.paper).ok_or_else(|| format!("모르는 용지: {}", req.paper))?;
    if req.scale <= 0.0 {
        return Err("축척은 0 보다 커야 합니다".into());
    }
    let layer = d.layer_index("A-ANNO-TTLB");
    let mut paper = Vec::new();
    let mut put = |shape: Shape, weight: Option<u16>| {
        paper.push(Entity {
            layer,
            color: None,
            aci: Some(7),
            linetype: None,
            weight,
            shape,
        });
    };
    let rect = |x0: f64, y0: f64, x1: f64, y1: f64| Shape::Polyline {
        points: vec![
            (Vec2::new(x0, y0), 0.0),
            (Vec2::new(x1, y0), 0.0),
            (Vec2::new(x1, y1), 0.0),
            (Vec2::new(x0, y1), 0.0),
        ],
        closed: true,
    };
    let line = |x0: f64, y0: f64, x1: f64, y1: f64| Shape::Line {
        a: Vec2::new(x0, y0),
        b: Vec2::new(x1, y1),
    };
    let text = |x: f64, y: f64, h: f64, v: &str| {
        Shape::Text(TextShape {
            at: Vec2::new(x, y),
            height: h,
            rotation: 0.0,
            width_factor: 1.0,
            halign: 0.0,
            valign: VAlign::Middle,
            value: v.to_string(),
        })
    };

    // 도면 틀
    let (l, r, b, t) = (20.0, pw - 10.0, 10.0, ph - 10.0);
    put(rect(l, b, r, t), Some(70));
    // 표제란 (오른쪽 아래)
    let (tw, th) = (180.0, 48.0);
    let (x0, y0) = (r - tw, b);
    put(rect(x0, y0, r, y0 + th), Some(50));
    let rows = 4.0;
    let rh = th / rows;
    for k in 1..4 {
        put(
            line(x0, y0 + rh * k as f64, r, y0 + rh * k as f64),
            Some(18),
        );
    }
    let lab = 28.0;
    put(line(x0 + lab, y0, x0 + lab, y0 + th), Some(18));
    // 아래 줄은 반으로 나눠 축척·날짜
    let half = x0 + lab + (tw - lab) / 2.0;
    put(line(half, y0, half, y0 + rh), Some(18));
    put(line(half + 18.0, y0, half + 18.0, y0 + rh), Some(18));
    let scale = if req.list {
        "-".to_string()
    } else {
        format!("1:{}", req.scale.round())
    };
    let date = today();
    let cells: [(&str, &str, f64); 3] = [
        ("프로젝트", &req.project, 3.0),
        ("도면명", &req.title, 4.5),
        ("도면 번호", &req.number, 4.0),
    ];
    for (k, (name, value, h)) in cells.iter().enumerate() {
        let y = y0 + th - rh * (k as f64 + 0.5);
        put(text(x0 + 2.5, y, 2.5, name), None);
        put(text(x0 + lab + 3.0, y, *h, value), None);
    }
    let y = y0 + rh * 0.5;
    put(text(x0 + 2.5, y, 2.5, "축척"), None);
    put(text(x0 + lab + 3.0, y, 3.0, &scale), None);
    put(text(half + 2.5, y, 2.5, "날짜"), None);
    put(text(half + 21.0, y, 3.0, &date), None);
    // 방위 표시 (왼쪽 아래): 원과 N
    let (nx, ny) = (l + 18.0, b + 18.0);
    put(
        Shape::Circle {
            center: Vec2::new(nx, ny),
            radius: 7.0,
        },
        Some(25),
    );
    put(
        Shape::Fill {
            loops: vec![vec![
                Vec2::new(nx, ny + 7.0),
                Vec2::new(nx - 2.5, ny - 3.0),
                Vec2::new(nx, ny - 1.5),
                Vec2::new(nx + 2.5, ny - 3.0),
            ]],
        },
        None,
    );
    put(
        Shape::Text(TextShape {
            at: Vec2::new(nx, ny + 9.5),
            height: 3.0,
            rotation: 0.0,
            width_factor: 1.0,
            halign: 0.5,
            valign: VAlign::Bottom,
            value: "N".into(),
        }),
        None,
    );

    // 뷰포트: 틀 안, 표제란 위 띠는 비움 (도면 목록 시트는 뷰포트 없음)
    let (vx0, vy0, vx1, vy1) = (l + 2.0, b + th + 2.0, r - 2.0, t - 2.0);
    let (vw, vh) = (vx1 - vx0, vy1 - vy0);
    let model = model_for(d, req.level);
    let bb = model.bbox();
    let view_center = if bb.is_empty() {
        Vec2::new(vw * req.scale / 2.0, vh * req.scale / 2.0)
    } else {
        Vec2::new((bb.min.x + bb.max.x) / 2.0, (bb.min.y + bb.max.y) / 2.0)
    };
    let vp = ViewportView {
        center: Vec2::new((vx0 + vx1) / 2.0, (vy0 + vy1) / 2.0),
        width: vw,
        height: vh,
        view_center,
        view_height: vh * req.scale,
        twist: 0.0,
        target: Vec2::default(),
        frozen: Vec::new(),
    };
    let name = if req.number.trim().is_empty() {
        req.title.clone()
    } else {
        format!("{} {}", req.number.trim(), req.title.trim())
    };
    Ok(Layout {
        name,
        order: d.layouts.iter().map(|l| l.order).max().unwrap_or(0) + 1,
        paper,
        viewports: if req.list { Vec::new() } else { vec![vp] },
        level: if req.list { None } else { req.level },
        list: req.list,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::WallKind;
    use crate::archgen::{ArchReq, apply, new_drawing};
    use crate::edit::Editor;

    #[test]
    fn sheet_shows_walls_at_scale() {
        let mut ed = Editor::new(new_drawing());
        apply(
            &mut ed,
            ArchReq::Walls {
                points: vec![[0.0, 0.0], [10000.0, 0.0], [10000.0, 8000.0], [0.0, 8000.0]],
                closed: true,
                thickness: 200.0,
                wall: WallKind::Exterior,
            },
        )
        .unwrap();
        let req = SheetReq {
            title: "1층 평면도".into(),
            number: "A-101".into(),
            paper: "A3".into(),
            scale: 100.0,
            level: None,
            project: "시험".into(),
            list: false,
        };
        let mut d = ed.d.clone();
        let l = make(&mut d, &req).unwrap();
        assert_eq!(l.name, "A-101 1층 평면도");
        d.layouts.push(l);
        let c = compose(&d, 0).unwrap();
        // 벽 선이 종이 위로 1:100 으로 들어온다: 10 m → 100 mm
        let bb = c.bbox();
        assert!(bb.max.x - bb.min.x >= 399.0 - 30.0, "틀 포함 폭 {:?}", bb);
        assert!(c.entities.len() > 20);
    }

    #[test]
    fn date_format() {
        let t = today();
        assert_eq!(t.len(), 10);
        assert_eq!(&t[4..5], ".");
    }

    #[test]
    fn list_sheet_lists_all_sheets() {
        let mut d = new_drawing();
        let mk = |title: &str, number: &str, list: bool| SheetReq {
            title: title.into(),
            number: number.into(),
            paper: "A3".into(),
            scale: 100.0,
            level: None,
            project: String::new(),
            list,
        };
        for (t, n, l) in [
            ("도면 목록", "A-000", true),
            ("1층 평면도", "A-101", false),
            ("2층 평면도", "A-102", false),
        ] {
            let lay = make(&mut d, &mk(t, n, l)).unwrap();
            d.layouts.push(lay);
        }
        let c = compose(&d, 0).unwrap();
        let texts: Vec<String> = c
            .entities
            .iter()
            .filter_map(|e| match &e.shape {
                Shape::Text(t) => Some(t.value.clone()),
                _ => None,
            })
            .collect();
        for want in ["도면 목록", "A-101", "2층 평면도", "1:100", "A-000"] {
            assert!(texts.iter().any(|t| t == want), "{want}: {texts:?}");
        }
        // 목록 시트 자신은 축척 없음
        assert!(texts.iter().any(|t| t == "-"));
    }

    #[test]
    fn viewport_scale_and_fit() {
        let mut ed = Editor::new(new_drawing());
        apply(
            &mut ed,
            ArchReq::Walls {
                points: vec![[0.0, 0.0], [30000.0, 0.0], [30000.0, 20000.0]],
                closed: false,
                thickness: 200.0,
                wall: WallKind::Exterior,
            },
        )
        .unwrap();
        let mut d = ed.d.clone();
        let req = SheetReq {
            title: "t".into(),
            number: "A-1".into(),
            paper: "A3".into(),
            scale: 50.0,
            level: None,
            project: String::new(),
            list: false,
        };
        let l = make(&mut d, &req).unwrap();
        d.layouts.push(l);
        let l = edit_viewport(
            &d,
            0,
            0,
            &VpEdit {
                scale: Some(200.0),
                ..Default::default()
            },
        )
        .unwrap();
        assert!((scale_of(&l).unwrap() - 200.0).abs() < 1e-9);
        // 표제란 축척 글자도 1:200
        assert!(
            l.paper
                .iter()
                .any(|e| matches!(&e.shape, Shape::Text(t) if t.value == "1:200"))
        );
        // A3 뷰포트 386×225 에 30 m×20 m: 20100/225/0.92 ≈ 97 → 표준 축척 1:100
        let l = edit_viewport(
            &d,
            0,
            0,
            &VpEdit {
                fit: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(scale_of(&l).unwrap(), 100.0);
    }

    #[test]
    fn add_and_remove_viewport() {
        let mut d = new_drawing();
        let req = SheetReq {
            title: "t".into(),
            number: "A-1".into(),
            paper: "A3".into(),
            scale: 50.0,
            level: None,
            project: String::new(),
            list: false,
        };
        let l = make(&mut d, &req).unwrap();
        d.layouts.push(l);
        let l = add_viewport(&d, 0).unwrap();
        assert_eq!(l.viewports.len(), 2);
        assert!((l.viewports[1].view_height / l.viewports[1].height - 50.0).abs() < 1e-9);
        d.layouts[0] = l;
        assert_eq!(remove_viewport(&d, 0, 0).unwrap().viewports.len(), 1);
    }
}
