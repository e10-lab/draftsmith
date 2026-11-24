//! 색 기준 출력 스타일(CTB). 설계사무소가 쓰는 `.ctb` 파일을 읽어 색 번호(ACI)마다
//! 출력 색·선 굵기를 정한다 (구현 노트 §16).
//!
//! 파일: 머리 60 바이트 뒤가 zlib 압축 글. 글 안에
//! `plot_style{ 0{ name="Color_1 … color=… lineweight=… screen=… } … }` 와
//! `lineweight{ 0=0.0 1=0.05 … }` 표가 있다. 번호 0 이 색 1, 254 가 색 255.

use std::io::Read;

/// 색 번호 하나의 출력 펜
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pen {
    /// 출력 색. None 이면 객체 색 그대로
    pub color: Option<[u8; 3]>,
    /// 출력 선 굵기 (mm). None 이면 객체 굵기 그대로
    pub lineweight_mm: Option<f64>,
    /// 농도 (0–100 %). 100 이 그대로, 낮을수록 흰색 쪽으로
    pub screen: u8,
    /// 회색조로
    pub grayscale: bool,
}

impl Default for Pen {
    fn default() -> Self {
        Pen {
            color: None,
            lineweight_mm: None,
            screen: 100,
            grayscale: false,
        }
    }
}

/// 출력 스타일 표: 색 번호 1–255 의 펜
#[derive(Debug, Clone, PartialEq)]
pub struct PenTable {
    pub name: String,
    pens: Vec<Pen>,
}

/// 「객체 색 그대로」 를 뜻하는 색 값 (AutoCAD 가 두 가지로 쓴다)
const OBJECT_COLORS: [i64; 2] = [-1, -1_006_632_961];
/// 「객체 굵기 그대로」 를 뜻하는 굵기 번호
const OBJECT_LINEWEIGHT: usize = 255;

impl PenTable {
    /// 색 번호(1–255)의 펜. 범위 밖이면 기본(객체 그대로)
    pub fn pen(&self, aci: u8) -> Pen {
        if aci == 0 {
            return Pen::default();
        }
        self.pens.get(aci as usize - 1).copied().unwrap_or_default()
    }

    /// 객체 색에 펜을 적용한 출력 색
    pub fn apply_color(&self, aci: u8, object: [u8; 3]) -> [u8; 3] {
        let p = self.pen(aci);
        let mut c = p.color.unwrap_or(object);
        if p.grayscale {
            let g = (0.299 * c[0] as f64 + 0.587 * c[1] as f64 + 0.114 * c[2] as f64).round() as u8;
            c = [g, g, g];
        }
        if p.screen < 100 {
            let k = p.screen as f64 / 100.0;
            c = c.map(|v| (255.0 - (255.0 - v as f64) * k).round() as u8);
        }
        c
    }

    /// `.ctb` 파일 바이트를 읽는다
    pub fn parse(bytes: &[u8], name: &str) -> Result<PenTable, String> {
        if bytes.len() <= 60 {
            return Err("CTB 파일이 너무 짧습니다".into());
        }
        let mut text = String::new();
        flate2::read::ZlibDecoder::new(&bytes[60..])
            .read_to_string(&mut text)
            .map_err(|e| format!("CTB 압축을 풀지 못했습니다: {e}"))?;
        Self::parse_text(&text, name)
    }

    /// 압축을 푼 CTB 글을 읽는다
    pub fn parse_text(text: &str, name: &str) -> Result<PenTable, String> {
        // 굵기 표: lineweight{ 0=0.0 1=0.05 … }
        let mut weights: Vec<f64> = Vec::new();
        if let Some(block) = section(text, "lineweight{") {
            for line in block.lines() {
                if let Some((k, v)) = line.trim().split_once('=')
                    && let (Ok(k), Ok(v)) = (k.trim().parse::<usize>(), v.trim().parse::<f64>())
                {
                    if weights.len() <= k {
                        weights.resize(k + 1, 0.0);
                    }
                    weights[k] = v;
                }
            }
        }
        let styles = section(text, "plot_style{").ok_or("CTB 에 plot_style 표가 없습니다")?;
        let mut pens = vec![Pen::default(); 255];
        // 각 펜: " 12{ … }" — 번호와 중괄호 안 key=value
        let mut rest = styles;
        while let Some(open) = rest.find('{') {
            let head = rest[..open].trim();
            let Some(close) = rest[open..].find('}') else {
                break;
            };
            let body = &rest[open + 1..open + close];
            rest = &rest[open + close + 1..];
            let Ok(idx) = head
                .rsplit(|c: char| c.is_whitespace())
                .next()
                .unwrap_or("")
                .parse::<usize>()
            else {
                continue;
            };
            if idx >= 255 {
                continue;
            }
            let mut pen = Pen::default();
            for line in body.lines() {
                let Some((k, v)) = line.trim().split_once('=') else {
                    continue;
                };
                match k.trim() {
                    "color" => {
                        if let Ok(c) = v.trim().parse::<i64>()
                            && !OBJECT_COLORS.contains(&c)
                        {
                            let c = c as u32;
                            pen.color = Some([(c >> 16) as u8, (c >> 8) as u8, c as u8]);
                        }
                    }
                    "lineweight" => {
                        if let Ok(i) = v.trim().parse::<usize>()
                            && i != OBJECT_LINEWEIGHT
                        {
                            pen.lineweight_mm = weights.get(i).copied();
                        }
                    }
                    "screen" => pen.screen = v.trim().parse::<u8>().unwrap_or(100).min(100),
                    "color_policy" => pen.grayscale = v.trim().parse::<u32>().unwrap_or(0) & 2 != 0,
                    _ => {}
                }
            }
            pens[idx] = pen;
        }
        Ok(PenTable {
            name: name.to_string(),
            pens,
        })
    }
}

/// `키{` 로 시작하는 표의 안쪽 (짝 맞는 `}` 까지)
fn section<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let start = text.find(key)? + key.len();
    let mut depth = 1;
    for (i, ch) in text[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..start + i]);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// 시험용 CTB 글: 색 1(빨강) → 검정 0.35 mm, 색 2 → 객체 색·객체 굵기, 색 8 → 농도 50 %
    pub fn sample_text() -> String {
        let mut s = String::from("description=\"시험\"\naci_table_available=TRUE\nplot_style{\n");
        for i in 0..255 {
            let (color, lw, screen) = match i {
                0 => ("-16777216", "8", "100"), // 0xFF000000 → 검정, 굵기 번호 8
                1 => ("-1006632961", "255", "100"),
                7 => ("-1006632961", "255", "50"),
                _ => ("-1", "255", "100"),
            };
            s += &format!(
                " {i}{{\n  name=\"Color_{}\n  color={color}\n  color_policy=1\n  screen={screen}\n  lineweight={lw}\n }}\n",
                i + 1
            );
        }
        s += "}\nlineweight{\n 0=0.0\n 1=0.05\n 2=0.09\n 3=0.1\n 4=0.13\n 5=0.15\n 6=0.18\n 7=0.2\n 8=0.35\n}\n";
        s
    }

    /// 시험용 CTB 파일 바이트 (머리 60 바이트 + zlib)
    pub fn sample_bytes() -> Vec<u8> {
        use std::io::Write;
        let mut head = b"PIAFILEVERSION_2.0,CTBVER1,compress\r\npmzlibcodec".to_vec();
        head.resize(60, 0);
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(sample_text().as_bytes()).unwrap();
        head.extend(z.finish().unwrap());
        head
    }

    #[test]
    fn parses_pens() {
        let t = PenTable::parse(&sample_bytes(), "시험.ctb").unwrap();
        let p1 = t.pen(1);
        assert_eq!(p1.color, Some([0, 0, 0]));
        assert_eq!(p1.lineweight_mm, Some(0.35));
        assert_eq!(
            t.pen(2),
            Pen {
                color: None,
                lineweight_mm: None,
                screen: 100,
                grayscale: false
            }
        );
        assert_eq!(t.apply_color(1, [255, 0, 0]), [0, 0, 0]);
        assert_eq!(t.apply_color(2, [255, 255, 0]), [255, 255, 0]);
        // 농도 50 %: 검정 → 회색
        assert_eq!(t.apply_color(8, [0, 0, 0]), [128, 128, 128]);
    }
}
