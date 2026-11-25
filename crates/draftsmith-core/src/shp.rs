//! 연속지적도 SHP 읽기 (구현 노트 §58): `.shp`(필지 모양) + `.dbf`(지번 등 속성) + `.cpg`(글자 인코딩).
//!
//! 국가공간정보포털·브이월드에서 받는 연속지적도는 보통 TM 좌표(EPSG:5174·5186, 미터) 다각형이고 속성에 PNU·JIBUN 이
//! 있다. 미터를 mm 로 바꿔(×1000) 레이어 「지적」 의 닫힌 폴리선으로 넣고, 필지마다 지번 글자를 안쪽 점에 둔다 — 그러면
//! 대지 도구의 필지 고르기(§56)가 그대로 지번까지 읽는다.
//! 글자 인코딩은 `.cpg` 를 따르고, 없으면 UTF-8 로 읽히면 UTF-8, 아니면 CP949(EUC-KR 확장).

use std::path::{Path, PathBuf};

use crate::drawing::{Drawing, Entity, Shape, TextShape, VAlign};
use crate::geom::Vec2;

/// 미터 → mm
const K: f64 = 1000.0;

fn be_i32(b: &[u8], i: usize) -> Option<i32> {
    Some(i32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?))
}
fn le_i32(b: &[u8], i: usize) -> Option<i32> {
    Some(i32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}
fn le_f64(b: &[u8], i: usize) -> Option<f64> {
    Some(f64::from_le_bytes(b.get(i..i + 8)?.try_into().ok()?))
}

/// 모양 하나: 고리(또는 선)들, 닫혔는가
struct Rec {
    parts: Vec<Vec<Vec2>>,
    closed: bool,
}

/// .shp 내용 → 모양들 (점·빈 모양은 None 으로 자리만)
fn parse_shp(b: &[u8]) -> Result<Vec<Option<Rec>>, String> {
    if be_i32(b, 0) != Some(9994) {
        return Err("SHP 파일이 아닙니다 (파일 코드가 9994 가 아님)".into());
    }
    let mut out = Vec::new();
    let mut i = 100;
    while i + 8 <= b.len() {
        let words = be_i32(b, i + 4).ok_or("SHP 기록 머리가 잘렸습니다")?;
        let start = i + 8;
        let end = start + (words.max(0) as usize) * 2;
        if end > b.len() {
            break;
        }
        let ty = le_i32(b, start).unwrap_or(0);
        // 3·13·23 선, 5·15·25 다각형 (Z·M 값은 버린다)
        let rec = match ty {
            3 | 13 | 23 | 5 | 15 | 25 => {
                let np = le_i32(b, start + 36).unwrap_or(0).max(0) as usize;
                let nv = le_i32(b, start + 40).unwrap_or(0).max(0) as usize;
                let parts: Vec<usize> = (0..np)
                    .filter_map(|k| le_i32(b, start + 44 + 4 * k).map(|v| v.max(0) as usize))
                    .collect();
                let pts0 = start + 44 + 4 * np;
                let pts: Vec<Vec2> = (0..nv)
                    .filter_map(|k| {
                        Some(Vec2::new(
                            le_f64(b, pts0 + 16 * k)? * K,
                            le_f64(b, pts0 + 16 * k + 8)? * K,
                        ))
                    })
                    .collect();
                let closed = matches!(ty, 5 | 15 | 25);
                let mut rings = Vec::new();
                for (k, &s) in parts.iter().enumerate() {
                    let e = parts
                        .get(k + 1)
                        .copied()
                        .unwrap_or(pts.len())
                        .min(pts.len());
                    if s >= e {
                        continue;
                    }
                    let mut r = pts[s..e].to_vec();
                    if closed && r.len() > 1 && r[0].dist(*r.last().unwrap()) < 1e-6 {
                        r.pop();
                    }
                    if r.len() >= 2 {
                        rings.push(r);
                    }
                }
                (!rings.is_empty()).then_some(Rec {
                    parts: rings,
                    closed,
                })
            }
            _ => None,
        };
        out.push(rec);
        i = end;
    }
    Ok(out)
}

/// .dbf → (필드 이름들, 기록마다 값들)
fn parse_dbf(
    b: &[u8],
    decode: &dyn Fn(&[u8]) -> String,
) -> Option<(Vec<String>, Vec<Vec<String>>)> {
    let n = u32::from_le_bytes(b.get(4..8)?.try_into().ok()?) as usize;
    let head = u16::from_le_bytes(b.get(8..10)?.try_into().ok()?) as usize;
    let len = u16::from_le_bytes(b.get(10..12)?.try_into().ok()?) as usize;
    let mut fields: Vec<(String, usize)> = Vec::new();
    let mut i = 32;
    while i + 32 <= head && b.get(i) != Some(&0x0d) {
        let raw = &b[i..i + 11];
        let name = decode(&raw[..raw.iter().position(|&c| c == 0).unwrap_or(11)]);
        fields.push((name.trim().to_uppercase(), b[i + 16] as usize));
        i += 32;
    }
    let mut rows = Vec::with_capacity(n);
    for r in 0..n {
        let s = head + r * len;
        let rec = b.get(s..s + len)?;
        let mut off = 1; // 지움 표시
        let mut row = Vec::with_capacity(fields.len());
        for (_, w) in &fields {
            let v = rec.get(off..off + w).unwrap_or(&[]);
            row.push(decode(v).trim().to_string());
            off += w;
        }
        rows.push(row);
    }
    Some((fields.into_iter().map(|f| f.0).collect(), rows))
}

/// 같은 이름·다른 확장자 (대소문자 무시)
fn sibling(path: &Path, ext: &str) -> Option<PathBuf> {
    let stem = path.file_stem()?.to_string_lossy().to_string();
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_stem().is_some_and(|s| s.to_string_lossy() == stem)
                && p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext))
        })
}

/// 지번으로 쓸 필드: JIBUN·지번 → 없으면 숫자와 '-' 가 든 첫 글자 필드
fn label_field(names: &[String], rows: &[Vec<String>]) -> Option<usize> {
    if let Some(i) = names
        .iter()
        .position(|n| n == "JIBUN" || n == "지번" || n == "JIBUN_NM")
    {
        return Some(i);
    }
    (0..names.len()).find(|&k| {
        rows.iter().take(20).any(|r| {
            r.get(k).is_some_and(|v| {
                v.contains('-') && v.chars().any(|c| c.is_ascii_digit()) && v.len() < 20
            })
        })
    })
}

/// 다각형 안쪽 점: 무게중심이 안에 있으면 그것, 아니면 가운데 높이 가로선의 가장 넓은 안쪽 구간 가운데
fn inner_point(ring: &[Vec2]) -> Vec2 {
    let n = ring.len();
    let (mut a, mut cx, mut cy) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let (p, q) = (ring[i], ring[(i + 1) % n]);
        let c = p.x * q.y - q.x * p.y;
        a += c;
        cx += (p.x + q.x) * c;
        cy += (p.y + q.y) * c;
    }
    if a.abs() > 1e-12 {
        let c = Vec2::new(cx / (3.0 * a), cy / (3.0 * a));
        if crate::fill::inside(c, ring) {
            return c;
        }
    }
    let (y0, y1) = ring
        .iter()
        .fold((f64::MAX, f64::MIN), |(l, h), p| (l.min(p.y), h.max(p.y)));
    let y = (y0 + y1) / 2.0;
    let mut xs: Vec<f64> = (0..n)
        .filter_map(|i| {
            let (p, q) = (ring[i], ring[(i + 1) % n]);
            ((p.y <= y) != (q.y <= y)).then(|| p.x + (y - p.y) / (q.y - p.y) * (q.x - p.x))
        })
        .collect();
    xs.sort_by(f64::total_cmp);
    xs.chunks(2)
        .filter(|c| c.len() == 2)
        .max_by(|a, b| (a[1] - a[0]).total_cmp(&(b[1] - b[0])))
        .map(|c| Vec2::new((c[0] + c[1]) / 2.0, y))
        .unwrap_or(ring[0])
}

/// 연속지적도 SHP 를 도면으로
pub fn read(path: &Path) -> Result<Drawing, String> {
    let shp = std::fs::read(path).map_err(|e| e.to_string())?;
    let recs = parse_shp(&shp)?;
    let cpg = sibling(path, "cpg")
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default()
        .to_uppercase();
    let utf8 = cpg.contains("UTF");
    let decode = move |b: &[u8]| -> String {
        if utf8 || (cpg.is_empty() && std::str::from_utf8(b).is_ok()) {
            String::from_utf8_lossy(b).into_owned()
        } else {
            encoding_rs::EUC_KR.decode(b).0.into_owned()
        }
    };
    let dbf = sibling(path, "dbf").and_then(|p| std::fs::read(p).ok());
    let table = dbf.as_deref().and_then(|b| parse_dbf(b, &decode));
    let field = table.as_ref().and_then(|(n, r)| label_field(n, r));
    // 경위도(도) 좌표면 mm 로 바꿔도 쓸 수 없다 — 알린다
    let geographic = sibling(path, "prj")
        .and_then(|p| std::fs::read_to_string(p).ok())
        .is_some_and(|t| t.trim_start().starts_with("GEOGCS"));

    let mut d = crate::archgen::new_drawing();
    let lay = d.layer_index("지적");
    let txt = d.layer_index("지적-지번");
    d.layers[txt as usize].color = [0, 255, 255];
    let ent = |layer: u32, shape: Shape| Entity {
        layer,
        color: None,
        aci: None,
        linetype: None,
        weight: None,
        shape,
    };
    let mut count = 0;
    for (k, r) in recs.iter().enumerate() {
        let Some(r) = r else { continue };
        count += 1;
        for part in &r.parts {
            d.entities.push(ent(
                lay,
                Shape::Polyline {
                    points: part.iter().map(|p| (*p, 0.0)).collect(),
                    closed: r.closed,
                },
            ));
        }
        // 지번: 가장 넓은 고리 안쪽에, 필지 크기에 맞춘 높이 (0.3 ~ 2 m)
        let label = field.and_then(|f| table.as_ref()?.1.get(k)?.get(f).cloned());
        if let (true, Some(label)) = (r.closed, label.filter(|s| !s.is_empty())) {
            let big = r
                .parts
                .iter()
                .max_by(|a, b| ring_area(a).total_cmp(&ring_area(b)))
                .unwrap();
            let h = (ring_area(big).sqrt() * 0.08).clamp(300.0, 2000.0);
            d.entities.push(ent(
                txt,
                Shape::Text(TextShape {
                    style: String::new(),
                    at: inner_point(big),
                    height: h,
                    rotation: 0.0,
                    width_factor: 1.0,
                    halign: 0.5,
                    valign: VAlign::Middle,
                    value: label,
                }),
            ));
        }
    }
    if count == 0 {
        return Err("SHP 에 선·다각형이 없습니다".into());
    }
    if geographic {
        d.skipped
            .insert("경위도 좌표(투영 좌표계로 받아 주세요)".into(), count);
    }
    Ok(d)
}

fn ring_area(r: &[Vec2]) -> f64 {
    let n = r.len();
    (0..n)
        .map(|i| r[i].x * r[(i + 1) % n].y - r[(i + 1) % n].x * r[i].y)
        .sum::<f64>()
        .abs()
        / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 시험용 SHP·DBF·CPG 쓰기 (다각형, 미터)
    fn write(dir: &Path, polys: &[Vec<[f64; 2]>], jibun: &[&str], cp949: bool) -> PathBuf {
        let mut recs = Vec::new();
        for (k, p) in polys.iter().enumerate() {
            let mut c = Vec::new();
            c.extend(5i32.to_le_bytes());
            c.extend([0u8; 32]);
            c.extend(1i32.to_le_bytes());
            c.extend(((p.len() + 1) as i32).to_le_bytes());
            c.extend(0i32.to_le_bytes());
            for q in p.iter().chain(std::iter::once(&p[0])) {
                c.extend(q[0].to_le_bytes());
                c.extend(q[1].to_le_bytes());
            }
            let mut r = Vec::new();
            r.extend(((k + 1) as i32).to_be_bytes());
            r.extend(((c.len() / 2) as i32).to_be_bytes());
            r.extend(c);
            recs.push(r);
        }
        let body: Vec<u8> = recs.concat();
        let mut shp = vec![0u8; 100];
        shp[0..4].copy_from_slice(&9994i32.to_be_bytes());
        shp[24..28].copy_from_slice((((100 + body.len()) / 2) as i32).to_be_bytes().as_slice());
        shp[28..32].copy_from_slice(&1000i32.to_le_bytes());
        shp[32..36].copy_from_slice(&5i32.to_le_bytes());
        shp.extend(body);
        let p = dir.join("필지.shp");
        std::fs::write(&p, shp).unwrap();
        // DBF: PNU(C,19), JIBUN(C,20)
        let enc = |s: &str| -> Vec<u8> {
            if cp949 {
                encoding_rs::EUC_KR.encode(s).0.into_owned()
            } else {
                s.as_bytes().to_vec()
            }
        };
        let fields = [("PNU", 19u8), ("JIBUN", 20u8)];
        let head = 32 + 32 * fields.len() + 1;
        let len = 1 + fields.iter().map(|f| f.1 as usize).sum::<usize>();
        let mut dbf = vec![0u8; 32];
        dbf[0] = 3;
        dbf[4..8].copy_from_slice(&(jibun.len() as u32).to_le_bytes());
        dbf[8..10].copy_from_slice(&(head as u16).to_le_bytes());
        dbf[10..12].copy_from_slice(&(len as u16).to_le_bytes());
        for (name, w) in fields {
            let mut f = [0u8; 32];
            f[..name.len()].copy_from_slice(name.as_bytes());
            f[11] = b'C';
            f[16] = w;
            dbf.extend(f);
        }
        dbf.push(0x0d);
        for (k, j) in jibun.iter().enumerate() {
            dbf.push(b' ');
            for (v, w) in [
                (format!("11110101001000{k:05}"), 19usize),
                (j.to_string(), 20),
            ] {
                let mut b = enc(&v);
                b.resize(w, b' ');
                dbf.extend(b);
            }
        }
        dbf.push(0x1a);
        std::fs::write(dir.join("필지.dbf"), dbf).unwrap();
        if cp949 {
            std::fs::write(dir.join("필지.cpg"), "CP949").unwrap();
        }
        p
    }

    #[test]
    fn parcels_with_jibun_cp949() {
        let dir = std::env::temp_dir().join(format!("dsmith-shp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // TM 좌표(미터) 필지 둘: 15×20 m 사각형, ㄱ자
        let p = write(
            &dir,
            &[
                vec![
                    [200000.0, 550000.0],
                    [200015.0, 550000.0],
                    [200015.0, 550020.0],
                    [200000.0, 550020.0],
                ],
                vec![
                    [200015.0, 550000.0],
                    [200035.0, 550000.0],
                    [200035.0, 550008.0],
                    [200023.0, 550008.0],
                    [200023.0, 550020.0],
                    [200015.0, 550020.0],
                ],
            ],
            &["101-1 대", "102 전"],
            true,
        );
        let d = read(&p).unwrap();
        let polys: Vec<_> = d
            .entities
            .iter()
            .filter(|e| matches!(e.shape, Shape::Polyline { closed: true, .. }))
            .collect();
        assert_eq!(polys.len(), 2);
        let texts: Vec<String> = d
            .entities
            .iter()
            .filter_map(|e| match &e.shape {
                Shape::Text(t) => Some(t.value.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, ["101-1 대", "102 전"]);
        // 지번 글자는 필지 안 (ㄱ자도), 대지 도구의 필지 고르기로 지번까지
        let mut ed = crate::edit::Editor::new(d);
        crate::archgen::apply(
            &mut ed,
            crate::archgen::ArchReq::SiteAt {
                at: [200030.0 * K, 550004.0 * K],
            },
        )
        .unwrap();
        assert_eq!(ed.d.arch.site.parcel, "102 전");
        let a = ring_area(&ed.d.arch.site.boundary) * 1e-6;
        assert!((a - (20.0 * 8.0 + 8.0 * 12.0)).abs() < 1e-6, "{a}");
        // import::read_file 로도
        assert!(crate::import::read_file(&p).is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn not_shp() {
        assert!(parse_shp(&[0u8; 120]).is_err());
    }
}
