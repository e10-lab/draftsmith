//! DraftSmith 도면 파일 `.dsmith` (구현 노트 §17).
//!
//! ZIP 묶음:
//! ```text
//! manifest.json   {"format":"draftsmith","version":1,"app":"0.1.0","units":4}
//! drawing.json    도면 모델 (레이어·객체·배치·건축 객체 …)
//! ```
//! document.loro  변경 이력 (Loro, §85 — 있으면. 같은 파일에서 갈라진 사본을 합칠 때 쓴다)
//! ```

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
    to_bytes_with(d, None)
}

/// 도면과 변경 이력(있으면)을 .dsmith 바이트로 (§85)
pub fn to_bytes_with(d: &Drawing, loro: Option<&[u8]>) -> Result<Vec<u8>, String> {
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
        // 한 번에 만들어 통째로 쓴다 — 압축 스트림에 잘게 쓰면 실무 도면(블록 정의 JSON 54 MB)에서 2 초.
        // 압축 수준 3: 기본(6)과 크기는 거의 같고(13.6 / 13.3 MB) 시간은 반 (§80)
        let json = serde_json::to_vec(d).map_err(|e| e.to_string())?;
        z.start_file("drawing.json", opt.compression_level(Some(3)))
            .map_err(|e| e.to_string())?;
        z.write_all(&json).map_err(|e| e.to_string())?;
        if let Some(b) = loro {
            z.start_file("document.loro", opt.compression_level(Some(3)))
                .map_err(|e| e.to_string())?;
            z.write_all(b).map_err(|e| e.to_string())?;
        }
        z.finish().map_err(|e| e.to_string())?;
    }
    Ok(buf.into_inner())
}

/// .dsmith 바이트에서 도면
pub fn from_bytes(bytes: &[u8]) -> Result<Drawing, String> {
    from_bytes_with(bytes).map(|x| x.0)
}

/// .dsmith 바이트에서 도면과 변경 이력 (없으면 None, §85)
pub fn from_bytes_with(bytes: &[u8]) -> Result<(Drawing, Option<Vec<u8>>), String> {
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
    let d: Drawing = {
        let mut f = z
            .by_name("drawing.json")
            .map_err(|_| "drawing.json 이 없습니다".to_string())?;
        let mut s = String::new();
        f.read_to_string(&mut s).map_err(|e| e.to_string())?;
        serde_json::from_str(&s).map_err(|e| format!("도면 데이터를 읽지 못했습니다: {e}"))?
    };
    let loro = match z.by_name("document.loro") {
        Ok(mut f) => {
            let mut b = Vec::new();
            f.read_to_end(&mut b).map_err(|e| e.to_string())?;
            Some(b)
        }
        Err(_) => None,
    };
    Ok((d, loro))
}

/// 파일로 저장
pub fn save(d: &Drawing, path: &Path) -> Result<(), String> {
    save_with(d, None, path)
}

/// 도면과 변경 이력(있으면)을 파일로 (§85)
pub fn save_with(d: &Drawing, loro: Option<&[u8]>, path: &Path) -> Result<(), String> {
    let bytes = to_bytes_with(d, loro)?;
    // 쓰다가 실패해도 원래 파일이 남게: 옆에 쓴 뒤 바꿔치기
    let tmp = path.with_extension("dsmith.tmp");
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// 파일에서 읽기
pub fn load(path: &Path) -> Result<Drawing, String> {
    from_bytes(&std::fs::read(path).map_err(|e| e.to_string())?)
}

/// 파일에서 도면과 변경 이력 (§85)
pub fn load_with(path: &Path) -> Result<(Drawing, Option<Vec<u8>>), String> {
    from_bytes_with(&std::fs::read(path).map_err(|e| e.to_string())?)
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
