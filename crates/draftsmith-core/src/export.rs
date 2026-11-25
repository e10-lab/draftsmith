//! DXF·DWG 내보내기 (도면 모델 → acadrust 문서). 구현 노트 §20.
//!
//! 모형 도면을 받아, 레이어·색 번호·선 종류·선 굵기를 살려 쓴다. DraftSmith 블록은 BLOCK 정의와 INSERT 로,
//! 치수 객체는 DIMENSION(정렬) 과 그 모양을 담은 익명 블록 *D 로, 그 밖의 건축 객체 선은 펼친 선으로 (§31).
//! 채움은 솔리드 해치, 해치 무늬 선분은 선으로. 배치는 아직 쓰지 않는다.

use std::path::Path;

use acadrust::entities::{
    Arc, AttributeDefinition, AttributeEntity, BoundaryEdge, BoundaryPath, Circle, Dimension,
    DimensionAligned, DimensionAngular3Pt, DimensionDiameter, DimensionLinear, DimensionRadius,
    Ellipse, Hatch, HatchPattern, HatchPatternLine, HorizontalAlignment, Insert, Line, LwPolyline,
    LwVertex, MText, PolylineEdge, Text, TextHorizontalAlignment, TextVerticalAlignment,
    VerticalAlignment,
};
use acadrust::tables::{BlockRecord, Layer as CadLayer, LineType, LineTypeElement, TextStyle};
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
        // 가리개 (§60): 다각형 WIPEOUT (닫는 꼭짓점까지)
        Shape::Wipeout { ring } => {
            let mut v: Vec<Vector2> = ring.iter().map(|p| Vector2::new(p.x, p.y)).collect();
            v.push(v[0]);
            vec![EntityType::Wipeout(acadrust::entities::Wipeout::polygonal(
                &v, 0.0,
            ))]
        }
        Shape::Hatch { loops, lines, name } => {
            // 무늬 해치는 HATCH (무늬 정의 그대로, §45)
            let mut pat = HatchPattern::new(if name.is_ascii() && !name.is_empty() {
                name.clone()
            } else {
                "DRAFTSMITH".into()
            });
            for l in lines {
                pat.add_line(HatchPatternLine {
                    angle: l.angle,
                    base_point: Vector2::new(l.base.x, l.base.y),
                    offset: Vector2::new(l.offset.x, l.offset.y),
                    dash_lengths: l.dashes.clone(),
                });
            }
            let mut h = Hatch::with_pattern(pat);
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
        Shape::Insert {
            block,
            at,
            scale,
            rotation,
            attrs,
        } => {
            let mut x = Insert::new(block.clone(), v3(*at))
                .with_scale(scale.x, scale.y, 1.0)
                .with_rotation(*rotation);
            // 속성값은 ATTRIB 로 (세계 좌표, §46)
            let t = crate::drawing::Block::transform(*at, *scale, *rotation);
            for a in attrs {
                if let Shape::Text(tx) = Shape::Text(a.text.clone()).transformed(&t) {
                    x.attributes.push(attrib(&a.tag, &tx));
                }
            }
            vec![EntityType::Insert(x)]
        }
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

/// 객체 하나를 문서에 넣는다 (레이어·색·선 종류·굵기). `owner` 가 있으면 그 블록 안에
/// DraftSmith 가 만든 글자의 스타일 (맑은 고딕 — 한글이 AutoCAD 에서 깨지지 않게, §60-1)
pub const DS_STYLE: &str = "DRAFTSMITH";

/// 치수 스타일 이름: 가져온 스타일이면 그대로, 아니면 DRAFTSMITH (§61)
fn dim_style_of(m: &crate::arch::ArchModel, s: &str) -> String {
    if !s.is_empty() && m.dim_styles.iter().any(|x| x.name.eq_ignore_ascii_case(s)) {
        s.to_string()
    } else {
        DS_STYLE.to_string()
    }
}

/// 치수 스타일 표 (§61): 가져온 스타일의 주요 값을 되쓰고, DraftSmith 치수용 DRAFTSMITH (글자 2.5 × 축척 100, 사선)
fn dim_styles(doc: &mut CadDocument, m: &crate::arch::ArchModel) {
    let mut list = m.dim_styles.clone();
    if !list.iter().any(|x| x.name.eq_ignore_ascii_case(DS_STYLE)) {
        list.push(crate::arch::DimStyle {
            name: DS_STYLE.into(),
            tick: 1.25,
            scale: 100.0,
            txsty: DS_STYLE.into(),
            ..Default::default()
        });
    }
    for s in list {
        let mut x = acadrust::tables::DimStyle::new(s.name.clone());
        x.dimtxt = s.text;
        x.dimasz = s.arrow;
        x.dimtsz = s.tick;
        x.dimexo = s.exo;
        x.dimexe = s.exe;
        x.dimgap = s.gap;
        x.dimscale = s.scale;
        x.dimlfac = s.lfac;
        x.dimdec = s.dec;
        x.dimzin = s.zin;
        x.dimlunit = s.lunit;
        x.dimtad = s.tad;
        x.dimjust = s.just;
        x.dimblk_name = s.blk.clone();
        x.dimtxsty = if doc.text_styles.get(&s.txsty).is_some() {
            s.txsty.clone()
        } else {
            DS_STYLE.into()
        };
        x.dimpost = s.post.clone();
        x.dimclrd = s.clrd;
        x.dimclre = s.clre;
        x.dimclrt = s.clrt;
        x.dimdle = s.dle;
        match doc.dim_styles.get_mut(&s.name) {
            Some(y) => {
                *y = acadrust::tables::DimStyle {
                    handle: y.handle,
                    ..x
                }
            }
            None => {
                let _ = doc.dim_styles.add(x);
            }
        }
    }
}

/// 글자 스타일 이름: 가져온 스타일이면 그대로, 아니면 DRAFTSMITH
fn style_of(d: &Drawing, s: &str) -> String {
    if !s.is_empty() && d.fonts.iter().any(|f| f.style.eq_ignore_ascii_case(s)) {
        s.to_string()
    } else {
        DS_STYLE.to_string()
    }
}

fn put(doc: &mut CadDocument, d: &Drawing, e: &Entity, owner: Option<acadrust::types::Handle>) {
    let layer = d
        .layers
        .get(e.layer as usize)
        .map(|l| l.name.clone())
        .unwrap_or_else(|| "0".into());
    for mut c in to_cad(e) {
        // 글자 스타일 (§60-1)
        match (&mut c, &e.shape) {
            (EntityType::Text(x), Shape::Text(t)) => x.style = style_of(d, &t.style),
            (EntityType::MText(x), Shape::Text(t)) => x.style = style_of(d, &t.style),
            (EntityType::Insert(x), Shape::Insert { attrs, .. }) => {
                for (a, t) in x.attributes.iter_mut().zip(attrs) {
                    a.text_style = style_of(d, &t.text.style);
                }
            }
            _ => {}
        }
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
        if let Some(h) = owner {
            common.owner_handle = h;
        }
        let _ = doc.add_entity(c);
    }
}

/// 블록 정의 하나 (기준점 원점)
fn block_record(doc: &mut CadDocument, name: &str) -> acadrust::types::Handle {
    let mut br = BlockRecord::new(name);
    br.handle = doc.allocate_handle();
    // BLOCK·ENDBLK 도 핸들이 있어야 한다 (없으면 0 — AutoCAD·ezdxf 가 거부, §44)
    br.block_entity_handle = doc.allocate_handle();
    br.block_end_handle = doc.allocate_handle();
    let h = br.handle;
    let _ = doc.block_records.add(br);
    h
}

/// 도면(모형) → acadrust 문서
pub fn to_document(model: &Drawing) -> CadDocument {
    // 건축 객체 선·블록 펼친 것까지 만든 도면과 그 주인
    let (d, owners) =
        crate::archgen::with_arch_owned(model).unwrap_or_else(|| (model.clone(), Vec::new()));
    let base = model.entities.len();
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
    // 글자 스타일 (§60-1): 가져온 스타일은 원래 글꼴 파일·트루타입·큰 글꼴 그대로, DraftSmith 글자는 맑은 고딕
    for f in &d.fonts {
        let mut st = TextStyle::new(f.style.clone());
        st.font_file = if f.file.is_empty() {
            f.font.clone()
        } else {
            f.file.clone()
        };
        st.big_font_file = f.big_font.clone();
        st.true_type_font = f.ttf.clone();
        match doc.text_styles.get_mut(&f.style) {
            Some(x) => {
                x.font_file = st.font_file;
                x.big_font_file = st.big_font_file;
                x.true_type_font = st.true_type_font;
            }
            None => {
                let _ = doc.text_styles.add(st);
            }
        }
    }
    if doc.text_styles.get(DS_STYLE).is_none() {
        let mut st = TextStyle::new(DS_STYLE);
        st.font_file = "malgun.ttf".into();
        st.true_type_font = "맑은 고딕".into();
        let _ = doc.text_styles.add(st);
    }
    dim_styles(&mut doc, &model.arch);
    // 블록 정의
    for b in &model.blocks {
        let h = block_record(&mut doc, &b.name);
        for e in &b.entities {
            put(&mut doc, &d, e, Some(h));
        }
        // 속성 정의 ATTDEF (§49)
        for a in &b.attdefs {
            let mut x =
                AttributeDefinition::new(a.tag.clone(), a.prompt.clone(), a.text.value.clone());
            x.insertion_point = v3(a.text.at);
            x.alignment_point = v3(a.text.at);
            x.height = a.text.height;
            x.rotation = a.text.rotation;
            x.width_factor = a.text.width_factor;
            (x.horizontal_alignment, x.vertical_alignment) = align(&a.text);
            x.common.layer = "0".into();
            x.common.owner_handle = h;
            x.text_style = style_of(&d, &a.text.style);
            let _ = doc.add_entity(EntityType::AttributeDefinition(x));
        }
    }
    // 모형 객체 (블록 참조는 INSERT 로)
    // 모형 객체는 모형에서 (무늬 해치를 HATCH 그대로, §45)
    for e in &model.entities {
        put(&mut doc, &d, e, None);
    }
    // 덧붙인 객체: 블록 펼친 것은 빼고, 치수는 치수 객체마다 모은다
    let mut dims: Vec<(u64, Vec<&Entity>)> = Vec::new();
    for (k, e) in d.entities[base..].iter().enumerate() {
        match owners.get(k).map(|(w, id)| (w.as_str(), *id)) {
            Some(("block", _)) => {}
            Some(("dim", id)) => match dims.iter_mut().find(|(i, _)| *i == id) {
                Some((_, v)) => v.push(e),
                None => dims.push((id, vec![e])),
            },
            _ => put(&mut doc, &d, e, None),
        }
    }
    // 치수: 모양은 익명 블록 *D 에 (다른 프로그램이 그대로 그린다), 값·정의점은 DIMENSION 에
    for (n, (id, parts)) in dims.into_iter().enumerate() {
        let Some(dm) = model.arch.dims.iter().find(|x| x.id == id) else {
            continue;
        };
        // 반지름·지름·각도 치수도 DIMENSION 과 그 모양 블록으로 (§43)
        if dm.kind != crate::arch::DimKind::Linear {
            use crate::arch::DimKind as K;
            let (a, b) = dm.ends(&model.arch);
            let name = format!("*D{}", n + 1);
            let h = block_record(&mut doc, &name);
            for e in &parts {
                put(&mut doc, &d, e, Some(h));
            }
            let mut dim = match dm.kind {
                K::Radius => Dimension::Radius(DimensionRadius::new(v3(a), v3(b))),
                K::Diameter => {
                    let far = Vec2::new(2.0 * a.x - b.x, 2.0 * a.y - b.y);
                    Dimension::Diameter(DimensionDiameter::new(v3(b), v3(far)))
                }
                _ => {
                    let Some(v) = dm.vertex else { continue };
                    let mut x = DimensionAngular3Pt::new(v3(v), v3(a), v3(b));
                    // 호가 지나는 점 (두 반직선 사이 가운데)
                    let (t0, t1) = ((a.y - v.y).atan2(a.x - v.x), (b.y - v.y).atan2(b.x - v.x));
                    let mut sw = crate::curve::ccw_sweep(t0, t1);
                    let mut s0 = t0;
                    if sw > std::f64::consts::PI {
                        s0 = t1;
                        sw = std::f64::consts::TAU - sw;
                    }
                    let mid = s0 + sw / 2.0;
                    let r = dm.offset.abs();
                    x.definition_point = v3(Vec2::new(v.x + r * mid.cos(), v.y + r * mid.sin()));
                    Dimension::Angular3Pt(x)
                }
            };
            let bb = dim.base_mut();
            bb.block_name = name;
            bb.style_name = dim_style_of(&model.arch, &dm.style);
            bb.actual_measurement = match dm.kind {
                K::Radius => a.dist(b),
                K::Diameter => 2.0 * a.dist(b),
                _ => bb.actual_measurement,
            };
            if !dm.text.trim().is_empty() {
                bb.user_text = Some(dm.text.clone());
            }
            let layer = parts
                .first()
                .and_then(|e| d.layers.get(e.layer as usize))
                .map(|l| l.name.clone())
                .unwrap_or_else(|| "0".into());
            let mut ent = EntityType::Dimension(Box::new(dim));
            ent.common_mut().layer = layer;
            let _ = doc.add_entity(ent);
            continue;
        }
        let Some(k) = dm.geom(&model.arch) else {
            continue;
        };
        let name = format!("*D{}", n + 1);
        let h = block_record(&mut doc, &name);
        for e in &parts {
            put(&mut doc, &d, e, Some(h));
        }
        let mut base = match dm.rotation {
            Some(r) => {
                let mut x = DimensionLinear::new(v3(k.a), v3(k.b));
                x.rotation = r;
                x.definition_point = v3(k.db);
                Dimension::Linear(x)
            }
            None => {
                let mut x = DimensionAligned::new(v3(k.a), v3(k.b));
                x.definition_point = v3(k.db);
                Dimension::Aligned(x)
            }
        };
        let bb = base.base_mut();
        bb.style_name = dim_style_of(&model.arch, &dm.style);
        bb.definition_point = v3(k.db);
        bb.text_middle_point = v3(k.da.lerp(k.db, 0.5));
        bb.actual_measurement = k.len;
        bb.block_name = name;
        if !dm.text.trim().is_empty() {
            bb.user_text = Some(dm.text.clone());
        }
        let layer = parts
            .first()
            .and_then(|e| d.layers.get(e.layer as usize))
            .map(|l| l.name.clone())
            .unwrap_or_else(|| "0".into());
        let mut ent = EntityType::Dimension(Box::new(base));
        ent.common_mut().layer = layer;
        let _ = doc.add_entity(ent);
    }
    doc
}

/// 모든 층을 한 도면에 (§45): 층마다 건물 폭 + 10 m 씩 오른쪽으로 옮겨 나란히 놓고, 층 이름 제목을 단다.
/// 층마다 그리드를 복사하고(번호 새로) 그 층 치수가 묶인 그리드도 바꿔 준다. 대지·단면은 첫 층 자리에 한 번만.
pub fn all_levels_model(d: &Drawing) -> Drawing {
    let m = &d.arch;
    if m.levels.len() < 2 {
        return d.clone();
    }
    let mut levels = m.levels.clone();
    levels.sort_by(|a, b| a.elevation.total_cmp(&b.elevation));
    let bb = levels
        .iter()
        .filter_map(|l| crate::sheet::building_bbox(d, Some(l.id)))
        .fold(crate::geom::Bbox::EMPTY, |mut acc, b| {
            acc.merge(&b);
            acc
        });
    let step = if bb.is_empty() {
        30000.0
    } else {
        bb.width() + 10000.0
    };
    let mut out = d.clone();
    let first = levels[0].id;
    let mut a = crate::arch::ArchModel {
        levels: vec![levels[0].clone()],
        active_level: first,
        site: m.site.clone(),
        sections: m.sections.clone(),
        dim_styles: m.dim_styles.clone(),
        next_id: m.next_id,
        ..Default::default()
    };
    let sh = |p: Vec2, dx: f64| Vec2::new(p.x + dx, p.y);
    for (i, l) in levels.iter().enumerate() {
        let dx = step * i as f64;
        // 그리드: 첫 층은 그대로, 다음 층은 새 번호로 복사
        let mut grid_map: Vec<(u64, u64)> = Vec::new();
        for g in &m.grids {
            let id = if i == 0 { g.id } else { a.new_id() };
            grid_map.push((g.id, id));
            a.grids.push(crate::arch::Grid {
                id,
                a: sh(g.a, dx),
                b: sh(g.b, dx),
                ..g.clone()
            });
        }
        for w in m.walls.iter().filter(|w| w.level == l.id) {
            a.walls.push(crate::arch::Wall {
                level: first,
                a: sh(w.a, dx),
                b: sh(w.b, dx),
                ..w.clone()
            });
        }
        let ids: Vec<u64> = m
            .walls
            .iter()
            .filter(|w| w.level == l.id)
            .map(|w| w.id)
            .collect();
        a.openings
            .extend(m.openings.iter().filter(|o| ids.contains(&o.wall)).cloned());
        for r in m.rooms.iter().filter(|r| r.level == l.id) {
            a.rooms.push(crate::arch::Room {
                level: first,
                boundary: r.boundary.iter().map(|p| sh(*p, dx)).collect(),
                ..r.clone()
            });
        }
        for c in m.columns.iter().filter(|c| c.level == l.id) {
            a.columns.push(crate::arch::Column {
                level: first,
                at: sh(c.at, dx),
                ..c.clone()
            });
        }
        for x in m.beams.iter().filter(|x| x.level == l.id) {
            a.beams.push(crate::arch::Beam {
                level: first,
                a: sh(x.a, dx),
                b: sh(x.b, dx),
                ..x.clone()
            });
        }
        for st in m.stairs.iter().filter(|s| s.level == l.id) {
            a.stairs.push(crate::arch::Stair {
                level: first,
                a: sh(st.a, dx),
                b: sh(st.b, dx),
                mid: st.mid.map(|p| sh(p, dx)),
                ..st.clone()
            });
        }
        for dm in m.dims.iter().filter(|x| x.level == l.id) {
            let remap = |k: &Option<crate::arch::Anchor>| {
                k.clone().map(|mut k| {
                    if k.what == "grid"
                        && let Some((_, n)) = grid_map.iter().find(|(o, _)| *o == k.id)
                    {
                        k.id = *n;
                    }
                    k
                })
            };
            a.dims.push(crate::arch::Dim {
                level: first,
                a: sh(dm.a, dx),
                b: sh(dm.b, dx),
                vertex: dm.vertex.map(|p| sh(p, dx)),
                ka: remap(&dm.ka),
                kb: remap(&dm.kb),
                ..dm.clone()
            });
        }
        // 층 제목
        if !bb.is_empty() {
            let layer = out.layer_index("A-ANNO-TEXT");
            out.entities.push(Entity {
                layer,
                color: None,
                aci: None,
                linetype: None,
                weight: None,
                shape: crate::drawing::Shape::Text(crate::drawing::TextShape {
                    style: String::new(),
                    at: Vec2::new(bb.min.x + dx, bb.min.y - 4000.0),
                    height: 500.0,
                    rotation: 0.0,
                    width_factor: 1.0,
                    halign: 0.0,
                    valign: VAlign::Baseline,
                    value: format!("{} 평면도", l.name),
                }),
            });
        }
    }
    out.arch = a;
    out
}

/// 모든 층을 나란히 놓아 DXF·DWG 로 (§45)
pub fn write_all_levels(d: &Drawing, path: &Path) -> Result<(), String> {
    write(&all_levels_model(d), path)
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

/// 속성값 글자 → ATTRIB (맞춤이 왼쪽·기준선이 아니면 맞춤점도 둔다)
fn attrib(tag: &str, t: &crate::drawing::TextShape) -> AttributeEntity {
    let mut a = AttributeEntity::new(tag.to_string(), t.value.clone());
    a.insertion_point = v3(t.at);
    a.alignment_point = v3(t.at);
    a.height = t.height;
    a.rotation = t.rotation;
    a.width_factor = t.width_factor;
    (a.horizontal_alignment, a.vertical_alignment) = align(t);
    a
}

/// 글자 맞춤 → ATTRIB·ATTDEF 맞춤
fn align(t: &crate::drawing::TextShape) -> (HorizontalAlignment, VerticalAlignment) {
    use crate::drawing::VAlign;
    let h = if t.halign > 0.75 {
        HorizontalAlignment::Right
    } else if t.halign > 0.25 {
        HorizontalAlignment::Center
    } else {
        HorizontalAlignment::Left
    };
    let v = match t.valign {
        VAlign::Bottom => VerticalAlignment::Bottom,
        VAlign::Middle => VerticalAlignment::Middle,
        VAlign::Top => VerticalAlignment::Top,
        _ => VerticalAlignment::Baseline,
    };
    (h, v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_and_read_back() {
        let d = crate::block::flatten(&crate::import::from_document(&crate::sample::plan_grid(60)));
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

    /// 치수 스타일 왕복 (§61): 스타일 이름·값이 돌아오고, 소수 자리는 스타일 DIMDEC 로
    #[test]
    fn dim_styles_round_trip() {
        use crate::archgen::{ArchReq, apply, new_drawing};
        let mut ed = crate::edit::Editor::new(new_drawing());
        apply(
            &mut ed,
            ArchReq::Dim {
                a: [0.0, 0.0],
                b: [3000.0, 0.0],
                at: [1500.0, -800.0],
                text_height: 200.0,
                arrow: false,
                plain: true,
                precision: 0,
            },
        )
        .unwrap();
        ed.d.arch.dim_styles.push(crate::arch::DimStyle {
            name: "SDIM-100".into(),
            text: 2.0,
            arrow: 0.8,
            exo: 1.0,
            exe: 1.0,
            gap: 1.0,
            scale: 100.0,
            dec: 1,
            ..Default::default()
        });
        ed.d.arch.dims[0].style = "SDIM-100".into();
        for ext in ["dxf", "dwg"] {
            let dir = std::env::temp_dir().join(format!("dsmith-dst-{}-{ext}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let p = dir.join(format!("d.{ext}"));
            write(&ed.d, &p).unwrap();
            let back = crate::import::read_file(&p).unwrap();
            assert_eq!(back.arch.dims[0].style, "SDIM-100", "{ext}");
            assert_eq!(back.arch.dims[0].precision, 1, "{ext}");
            let st = back
                .arch
                .dim_styles
                .iter()
                .find(|s| s.name == "SDIM-100")
                .unwrap();
            assert_eq!((st.scale, st.exe, st.arrow), (100.0, 1.0, 0.8), "{ext}");
            assert!(
                back.arch.dim_styles.iter().any(|s| s.name == DS_STYLE),
                "{ext}"
            );
            std::fs::remove_dir_all(&dir).ok();
        }
        // 그리기: 보조선이 치수선을 exe × 축척 = 100 넘는다
        let v = crate::archgen::with_arch(&ed.d).unwrap();
        let k = ed.d.arch.dims[0].geom(&ed.d.arch).unwrap();
        assert!(v.entities.iter().any(|e| match e.shape {
            Shape::Line { a, b } =>
                (a.dist(k.a) - 100.0).abs() < 1e-6 && (b.dist(k.da) - 100.0).abs() < 1e-6,
            _ => false,
        }));
    }

    /// 글자 스타일 왕복 (§60-1): 가져온 스타일은 그대로, DraftSmith 글자는 맑은 고딕 스타일
    #[test]
    fn text_styles_round_trip() {
        let mut d = crate::archgen::new_drawing();
        d.fonts.push(crate::drawing::FontUse {
            style: "KOR".into(),
            font: "txt.shx".into(),
            big_font: "whgtxt.shx".into(),
            count: 1,
            file: "txt.shx".into(),
            ttf: String::new(),
        });
        let t = |v: &str, style: &str| Entity {
            layer: 0,
            color: None,
            aci: None,
            linetype: None,
            weight: None,
            shape: Shape::Text(crate::drawing::TextShape {
                at: Vec2::default(),
                height: 250.0,
                rotation: 0.0,
                width_factor: 1.0,
                halign: 0.0,
                valign: crate::drawing::VAlign::Baseline,
                value: v.into(),
                style: style.into(),
            }),
        };
        d.entities = vec![t("가져온 글자", "KOR"), t("새 글자", "")];
        for ext in ["dxf", "dwg"] {
            let dir = std::env::temp_dir().join(format!("dsmith-sty-{}-{ext}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let p = dir.join(format!("s.{ext}"));
            write(&d, &p).unwrap();
            let back = crate::import::read_file(&p).unwrap();
            let kor = back.fonts.iter().find(|f| f.style == "KOR").unwrap();
            assert_eq!(
                (kor.big_font.as_str(), kor.count),
                ("whgtxt.shx", 1),
                "{ext}"
            );
            let ds = back.fonts.iter().find(|f| f.style == DS_STYLE).unwrap();
            assert!(ds.file.contains("malgun") && ds.count == 1, "{ext} {ds:?}");
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    #[test]
    fn blocks_and_dims_survive_round_trip() {
        use crate::archgen::{ArchReq, apply, new_drawing};
        let mut ed = crate::edit::Editor::new(new_drawing());
        let _ = ed.add_shapes(vec![
            Shape::Line {
                a: Vec2::new(0.0, 0.0),
                b: Vec2::new(500.0, 0.0),
            },
            Shape::Circle {
                center: Vec2::new(250.0, 300.0),
                radius: 100.0,
            },
        ]);
        ed.make_block("의자", &[0, 1], Vec2::default()).unwrap();
        let j = ed
            .insert_block("의자", Vec2::new(2000.0, 0.0), 1.0, 1.0)
            .unwrap();
        ed.add_attr(j, "NO", "A-01", 200.0).unwrap();
        ed.define_attrs(j).unwrap();
        let want = ed.d.entities[j].shape.clone();
        apply(
            &mut ed,
            ArchReq::Dim {
                a: [0.0, 0.0],
                b: [2000.0, 0.0],
                at: [1000.0, -800.0],
                text_height: 250.0,
                arrow: false,
                plain: false,
                precision: 0,
            },
        )
        .unwrap();
        let doc = to_document(&ed.d);
        assert!(doc.block_records.get("의자").is_some());
        let inserts = doc
            .entities()
            .filter(|e| matches!(e, EntityType::Insert(_)))
            .count();
        let dims = doc
            .entities()
            .filter(|e| matches!(e, EntityType::Dimension(_)))
            .count();
        assert_eq!((inserts, dims), (2, 1));
        // 다시 읽기: 블록은 블록으로 (§31), 치수는 *D 블록을 펼친 선·글자로
        for ext in ["dxf", "dwg"] {
            let dir = std::env::temp_dir().join(format!("dsmith-blk-{}-{ext}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let p = dir.join(format!("out.{ext}"));
            write(&ed.d, &p).unwrap();
            let back = crate::import::read_file(&p).unwrap();
            // 치수는 치수 객체로 돌아온다 (§32)
            assert_eq!(back.arch.dims.len(), 1, "{ext}");
            assert_eq!(back.arch.dims[0].rotation, None, "{ext}");
            // 속성값은 블록 안 좌표로 돌아온다 (§46)
            let got = back.entities.iter().find_map(|e| match &e.shape {
                Shape::Insert { attrs, .. } if !attrs.is_empty() => Some(attrs.clone()),
                _ => None,
            });
            let Shape::Insert { attrs: w, .. } = &want else {
                unreachable!()
            };
            let got = got.unwrap_or_else(|| panic!("{ext}: 속성 없음"));
            assert_eq!(
                (got[0].tag.as_str(), got[0].text.value.as_str()),
                ("NO", "A-01"),
                "{ext}"
            );
            assert!(
                got[0].text.at.dist(w[0].text.at) < 1e-6,
                "{ext} {:?} {:?}",
                got[0].text.at,
                w[0].text.at
            );
            assert!(
                (got[0].text.height - w[0].text.height).abs() < 1e-6,
                "{ext}"
            );
            // 블록의 속성 정의도 돌아온다 (§49)
            let bd = back.blocks.iter().find(|b| b.name == "의자").unwrap();
            assert_eq!(bd.attdefs.len(), 1, "{ext}");
            assert_eq!(bd.attdefs[0].tag, "NO", "{ext}");
            assert!(bd.attdefs[0].text.at.dist(w[0].text.at) < 1e-6, "{ext}");
            let back = crate::archgen::shown(&back);
            assert!(back.blocks.iter().any(|b| b.name == "의자"), "{ext}");
            assert_eq!(
                back.entities
                    .iter()
                    .filter(|e| matches!(e.shape, Shape::Insert { .. }))
                    .count(),
                2,
                "{ext}"
            );
            assert!(
                back.entities
                    .iter()
                    .any(|e| matches!(&e.shape, Shape::Text(t) if t.value == "2000")),
                "{ext}"
            );
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    #[test]
    fn radial_and_angle_dims_become_dimensions() {
        use crate::archgen::{ArchReq, apply, new_drawing};
        let mut ed = crate::edit::Editor::new(new_drawing());
        apply(
            &mut ed,
            ArchReq::DimRadial {
                center: [0.0, 0.0],
                point: [600.0, 0.0],
                diameter: false,
                text_height: 250.0,
                arrow: true,
                plain: false,
            },
        )
        .unwrap();
        apply(
            &mut ed,
            ArchReq::DimRadial {
                center: [0.0, 0.0],
                point: [0.0, 600.0],
                diameter: true,
                text_height: 250.0,
                arrow: true,
                plain: false,
            },
        )
        .unwrap();
        apply(
            &mut ed,
            ArchReq::DimAngle {
                vertex: [0.0, 0.0],
                a: [1000.0, 0.0],
                b: [1000.0, 1000.0],
                at: [800.0, 0.0],
                text_height: 250.0,
                arrow: true,
            },
        )
        .unwrap();
        let doc = to_document(&ed.d);
        let kinds: Vec<&str> = doc
            .entities()
            .filter_map(|e| match e {
                EntityType::Dimension(x) => Some(match x.as_ref() {
                    Dimension::Radius(_) => "R",
                    Dimension::Diameter(_) => "D",
                    Dimension::Angular3Pt(_) => "A",
                    _ => "?",
                }),
                _ => None,
            })
            .collect();
        assert_eq!(kinds, vec!["R", "D", "A"]);
        for ext in ["dxf", "dwg"] {
            let dir = std::env::temp_dir().join(format!("dsmith-rad-{}-{ext}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let p = dir.join(format!("r.{ext}"));
            write(&ed.d, &p).unwrap();
            let raw = crate::import::read_file(&p).unwrap();
            // 반지름·지름·각도 치수가 치수 객체로 돌아온다 (§45)
            let kinds: Vec<_> = raw.arch.dims.iter().map(|x| x.kind).collect();
            assert_eq!(
                kinds,
                vec![
                    crate::arch::DimKind::Radius,
                    crate::arch::DimKind::Diameter,
                    crate::arch::DimKind::Angle
                ],
                "{ext}"
            );
            let back = crate::archgen::shown(&raw);
            // 모양 블록을 펼친 글자가 돌아온다
            assert!(
                back.entities
                    .iter()
                    .any(|e| matches!(&e.shape, Shape::Text(t) if t.value == "R600")),
                "{ext}"
            );
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    #[test]
    fn all_levels_side_by_side() {
        let d = crate::demo::demo_building().unwrap();
        let all = all_levels_model(&d);
        assert_eq!(all.arch.levels.len(), 1);
        assert_eq!(all.arch.walls.len(), 12);
        assert_eq!(all.arch.rooms.len(), 6);
        assert_eq!(all.arch.grids.len(), 12);
        let doc = to_document(&all);
        let texts: Vec<String> = doc
            .entities()
            .filter_map(|e| match e {
                EntityType::Text(t) => Some(t.value.clone()),
                _ => None,
            })
            .collect();
        for want in ["거실", "침실 1", "1층 평면도", "2층 평면도"] {
            assert!(texts.iter().any(|t| t == want), "{want}");
        }
        // 2 층 치수도 묶인 그리드를 따라 6,000
        let dims = doc
            .entities()
            .filter(|e| matches!(e, EntityType::Dimension(_)))
            .count();
        assert_eq!(dims, 4);
    }
}
