//! DraftSmith 도면 파일 `.dsmith` (구현 노트 §17).
//!
//! ZIP 묶음:
//! ```text
//! manifest.json   {"format":"draftsmith","version":1,"app":"0.1.0","units":4}
//! drawing.json    도면 모델 (레이어·객체·배치·건축 객체 …)
//! ```
//! 계획의 Loro 변경 이력(document.loro)은 공동작업(M5) 단계에서 이 묶음에 더한다.

use std::io::{Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::drawing::Drawing;

/// 파일 확장자
pub const EXT: &str = "dsmith";
/// 형식 판
pub const VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    format: String,
    version: u32,
    app: String,
    units: i16,
}

/// 파일이 .dsmith 인가 (ZIP 머리 `PK` 와 확장자)
pub fn is_dsmith(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(EXT))
}

/// 도면을 .dsmith 바이트로
pub fn to_bytes(d: &Drawing) -> Result<Vec<u8>, String> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let opt = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let m = Manifest {
            format: "draftsmith".into(),
            version: VERSION,
            app: env!("CARGO_PKG_VERSION").into(),
            units: d.units,
        };
        z.start_file("manifest.json", opt)
            .map_err(|e| e.to_string())?;
        z.write_all(&serde_json::to_vec_pretty(&m).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        z.start_file("drawing.json", opt)
            .map_err(|e| e.to_string())?;
        serde_json::to_writer(&mut z, d).map_err(|e| e.to_string())?;
        z.finish().map_err(|e| e.to_string())?;
    }
    Ok(buf.into_inner())
}

/// .dsmith 바이트에서 도면
pub fn from_bytes(bytes: &[u8]) -> Result<Drawing, String> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("DraftSmith 파일이 아닙니다: {e}"))?;
    let m: Manifest = {
        let mut f = z
            .by_name("manifest.json")
            .map_err(|_| "manifest.json 이 없습니다".to_string())?;
        let mut s = String::new();
        f.read_to_string(&mut s).map_err(|e| e.to_string())?;
        serde_json::from_str(&s).map_err(|e| e.to_string())?
    };
    if m.format != "draftsmith" {
        return Err("DraftSmith 파일이 아닙니다".into());
    }
    if m.version > VERSION {
        return Err(format!(
            "더 새로운 DraftSmith 로 만든 파일입니다 (형식 {})",
            m.version
        ));
    }
    let mut f = z
        .by_name("drawing.json")
        .map_err(|_| "drawing.json 이 없습니다".to_string())?;
    let mut s = String::new();
    f.read_to_string(&mut s).map_err(|e| e.to_string())?;
    serde_json::from_str(&s).map_err(|e| format!("도면 데이터를 읽지 못했습니다: {e}"))
}

/// 파일로 저장
pub fn save(d: &Drawing, path: &Path) -> Result<(), String> {
    let bytes = to_bytes(d)?;
    // 쓰다가 실패해도 원래 파일이 남게: 옆에 쓴 뒤 바꿔치기
    let tmp = path.with_extension("dsmith.tmp");
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// 파일에서 읽기
pub fn load(path: &Path) -> Result<Drawing, String> {
    from_bytes(&std::fs::read(path).map_err(|e| e.to_string())?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_everything() {
        let d = crate::import::from_document(&crate::sample::plan_grid(60));
        let b = to_bytes(&d).unwrap();
        let e = from_bytes(&b).unwrap();
        assert_eq!(e.entities, d.entities);
        assert_eq!(e.layers, d.layers);
        assert_eq!(e.layouts, d.layouts);
        assert_eq!(e.linetypes, d.linetypes);
        assert!(b.len() < 200_000, "{}", b.len());
    }
}
