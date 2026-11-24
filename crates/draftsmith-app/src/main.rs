//! DraftSmith 데스크톱 앱 (Tauri).
//!
//! 화면은 `ui/` 의 HTML·JavaScript 이고, 도면 읽기·표시 데이터는 이 Rust 명령들이
//! `draftsmith_core` 로 처리한다. BEATS 와 같은 구조다.
//!
//! BEATS 와 다른 점: 도면은 클 수 있어서 열린 도면을 앱(Rust)이 들고 있고,
//! 화면에는 표시 데이터만 바이너리로 넘긴다 (JSON 변환 없음 — 구현 노트 §4).

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use draftsmith_core::{Drawing, display, import};
use serde::Serialize;
use tauri::State;
use tauri::ipc::Response;

/// 열린 도면
#[derive(Default)]
struct Open {
    drawing: Option<Drawing>,
    path: Option<PathBuf>,
}

type AppState = Mutex<Open>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[tauri::command]
fn app_version() -> &'static str {
    env!("DRAFTSMITH_VERSION")
}

/// 파일 고르기 대화상자. 취소하면 null
#[tauri::command]
fn pick_drawing() -> Option<String> {
    rfd::FileDialog::new()
        .set_title("도면 열기")
        .add_filter("도면 (DWG, DXF)", &["dwg", "dxf", "DWG", "DXF"])
        .pick_file()
        .map(|p| p.display().to_string())
}

#[derive(Serialize)]
struct LayerInfo {
    name: String,
    color: [u8; 3],
    visible: bool,
    count: usize,
}

#[derive(Serialize)]
struct Summary {
    path: String,
    code_page: String,
    entities: usize,
    inserts: usize,
    layers: Vec<LayerInfo>,
    skipped: Vec<(String, usize)>,
    read_ms: f64,
}

fn summary(d: &Drawing, path: String, read_ms: f64) -> Summary {
    let mut counts = vec![0usize; d.layers.len()];
    for e in &d.entities {
        if let Some(c) = counts.get_mut(e.layer as usize) {
            *c += 1;
        }
    }
    Summary {
        path,
        code_page: d.code_page.clone(),
        entities: d.entities.len(),
        inserts: d.inserts,
        layers: d
            .layers
            .iter()
            .zip(counts)
            .map(|(l, count)| LayerInfo {
                name: l.name.clone(),
                color: l.color,
                visible: l.visible,
                count,
            })
            .collect(),
        skipped: d.skipped.iter().map(|(k, n)| (k.clone(), *n)).collect(),
        read_ms,
    }
}

/// 도면을 읽어 앱에 들고, 요약을 돌려준다
#[tauri::command]
async fn open_drawing(path: String, state: State<'_, AppState>) -> Result<Summary, String> {
    let t = Instant::now();
    let p = PathBuf::from(&path);
    // 읽기는 시간이 걸릴 수 있어 작업 스레드에서
    let d = tauri::async_runtime::spawn_blocking(move || import::read_file(&p))
        .await
        .map_err(err)?
        .map_err(err)?;
    let read_ms = t.elapsed().as_secs_f64() * 1000.0;
    let s = summary(&d, path.clone(), read_ms);
    let mut open = state.lock().map_err(err)?;
    open.drawing = Some(d);
    open.path = Some(PathBuf::from(path));
    Ok(s)
}

/// 레이어 켜기·끄기
#[tauri::command]
fn set_layer_visible(
    index: usize,
    visible: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut open = state.lock().map_err(err)?;
    let d = open.drawing.as_mut().ok_or("열린 도면이 없습니다")?;
    let l = d
        .layers
        .get_mut(index)
        .ok_or("레이어 번호가 잘못되었습니다")?;
    l.visible = visible;
    Ok(())
}

/// 표시 데이터 (바이너리, 형식은 `draftsmith_core::display`)
#[tauri::command]
fn display_data(state: State<'_, AppState>) -> Result<Response, String> {
    let open = state.lock().map_err(err)?;
    let d = open.drawing.as_ref().ok_or("열린 도면이 없습니다")?;
    Ok(Response::new(display::segments(d).to_bytes()))
}

fn main() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            app_version,
            pick_drawing,
            open_drawing,
            set_layer_visible,
            display_data
        ])
        .run(tauri::generate_context!())
        .expect("앱을 시작하지 못했습니다");
}
