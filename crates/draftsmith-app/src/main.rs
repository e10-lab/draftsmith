// Windows 릴리스 빌드에서 콘솔 창이 함께 뜨지 않게
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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

use draftsmith_core::pick::{self, PickIndex};
use draftsmith_core::snap::SnapIndex;
use draftsmith_core::{Drawing, Vec2, display, import};
use serde::Serialize;
use tauri::ipc::Response;
use tauri::{AppHandle, Manager, State};

/// 열린 도면
#[derive(Default)]
struct Open {
    drawing: Option<Drawing>,
    path: Option<PathBuf>,
    /// 스냅 점 색인 (켜진 레이어만)
    snaps: Option<SnapIndex>,
    /// 객체 고르기 색인 (켜진 레이어만)
    picks: Option<PickIndex>,
}

/// 최근 파일 목록 최대 개수
const RECENT_MAX: usize = 12;

/// 최근 파일 목록 파일 (앱 설정 폴더의 recent.txt, 한 줄에 경로 하나)
fn recent_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|d| d.join("recent.txt"))
}

fn read_recent(app: &AppHandle) -> Vec<String> {
    recent_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| {
            s.lines()
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// 최근 파일 목록 (있는 파일만)
#[tauri::command]
fn recent_files(app: AppHandle) -> Vec<String> {
    read_recent(&app)
        .into_iter()
        .filter(|p| std::path::Path::new(p).is_file())
        .collect()
}

fn remember(app: &AppHandle, path: &str) {
    let mut list = read_recent(app);
    list.retain(|p| p != path);
    list.insert(0, path.to_string());
    list.truncate(RECENT_MAX);
    if let Some(p) = recent_path(app) {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, list.join("\n") + "\n");
    }
}

type AppState = Mutex<Open>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[tauri::command]
fn app_version() -> &'static str {
    env!("DRAFTSMITH_VERSION")
}

/// 실행할 때 넘긴 도면 경로 (`draftsmith-app 도면.dwg`, 파일 연결로 열기). 없으면 null
#[tauri::command]
fn startup_file() -> Option<String> {
    std::env::args()
        .skip(1)
        .find(|a| !a.starts_with('-'))
        .filter(|a| std::path::Path::new(a).is_file())
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
async fn open_drawing(
    path: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Summary, String> {
    let t = Instant::now();
    let p = PathBuf::from(&path);
    // 읽기·스냅 색인은 시간이 걸릴 수 있어 작업 스레드에서
    let (d, snaps, picks) = tauri::async_runtime::spawn_blocking(move || {
        import::read_file(&p).map(|d| {
            let s = SnapIndex::build(&d);
            let k = PickIndex::build(&d);
            (d, s, k)
        })
    })
    .await
    .map_err(err)?
    .map_err(err)?;
    let read_ms = t.elapsed().as_secs_f64() * 1000.0;
    let s = summary(&d, path.clone(), read_ms);
    remember(&app, &path);
    let mut open = state.lock().map_err(err)?;
    open.drawing = Some(d);
    open.snaps = Some(snaps);
    open.picks = Some(picks);
    open.path = Some(PathBuf::from(path));
    Ok(s)
}

#[derive(Serialize)]
struct SnapOut {
    x: f64,
    y: f64,
    /// 0 끝점, 1 중간점, 2 중심, 3 사분점, 4 삽입점
    kind: u8,
}

/// 도면 좌표 (x, y) 에서 반지름 r 안의 가장 가까운 스냅 점. 없으면 null
#[tauri::command]
fn snap(x: f64, y: f64, r: f64, state: State<'_, AppState>) -> Result<Option<SnapOut>, String> {
    let open = state.lock().map_err(err)?;
    Ok(open
        .snaps
        .as_ref()
        .and_then(|i| i.nearest(Vec2::new(x, y), r))
        .map(|s| SnapOut {
            x: s.at.x,
            y: s.at.y,
            kind: s.kind as u8,
        }))
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
    // 꺼진 레이어에는 스냅하지 않게 색인을 다시 만든다
    let snaps = SnapIndex::build(d);
    let picks = PickIndex::build(d);
    open.snaps = Some(snaps);
    open.picks = Some(picks);
    Ok(())
}

#[derive(Serialize)]
struct PickOut {
    index: usize,
    /// (이름, 값) — 속성 칸에 그대로
    props: Vec<(String, String)>,
    /// 강조 표시 선분 [x0, y0, x1, y1] (도면 좌표)
    outline: Vec<[f64; 4]>,
}

/// 도면 좌표 (x, y) 에서 반지름 r 안의 가장 가까운 객체와 그 속성. 없으면 null
#[tauri::command]
fn pick_at(x: f64, y: f64, r: f64, state: State<'_, AppState>) -> Result<Option<PickOut>, String> {
    let open = state.lock().map_err(err)?;
    let (Some(d), Some(idx)) = (open.drawing.as_ref(), open.picks.as_ref()) else {
        return Ok(None);
    };
    Ok(idx.pick(d, Vec2::new(x, y), r).map(|i| PickOut {
        index: i,
        props: pick::properties(d, i),
        outline: pick::outline(&d.entities[i].shape, r / 4.0, 20_000)
            .into_iter()
            .map(|(a, b)| [a.x, a.y, b.x, b.y])
            .collect(),
    }))
}

/// 표시 데이터 (바이너리, 형식은 `draftsmith_core::display`)
#[tauri::command]
fn display_data(state: State<'_, AppState>) -> Result<Response, String> {
    let open = state.lock().map_err(err)?;
    let d = open.drawing.as_ref().ok_or("열린 도면이 없습니다")?;
    Ok(Response::new(display::segments(d).to_bytes()))
}

/// 문자 표시 데이터 (바이너리, 형식은 `draftsmith_core::display::texts`)
#[tauri::command]
fn text_data(state: State<'_, AppState>) -> Result<Response, String> {
    let open = state.lock().map_err(err)?;
    let d = open.drawing.as_ref().ok_or("열린 도면이 없습니다")?;
    Ok(Response::new(display::texts(d)))
}

fn main() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            app_version,
            startup_file,
            pick_drawing,
            recent_files,
            open_drawing,
            snap,
            pick_at,
            set_layer_visible,
            display_data,
            text_data
        ])
        .run(tauri::generate_context!())
        .expect("앱을 시작하지 못했습니다");
}
