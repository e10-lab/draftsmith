// Windows 릴리스 빌드에서 콘솔 창이 함께 뜨지 않게
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! DraftSmith 데스크톱 앱 (Tauri).
//!
//! 화면은 `ui/` 의 HTML·JavaScript 이고, 도면 읽기·편집·표시 데이터는 이 Rust 명령들이
//! `draftsmith_core` 로 처리한다. BEATS 와 같은 구조다.
//!
//! 열린 도면은 앱(Rust)이 편집 엔진(`Editor`)으로 들고, 화면에는 표시 데이터만 바이너리로 넘긴다
//! (JSON 변환 없음 — 구현 노트 §4·§18). 편집은 모두 `edit` 명령 하나로 들어온다.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use draftsmith_core::edit::{Editor, JOURNAL_EXT, Journal, Op};
use draftsmith_core::pdf::{self, PdfOptions};
use draftsmith_core::pick::{self, PickIndex};
use draftsmith_core::snap::SnapIndex;
use draftsmith_core::{
    Affine, Bbox, Drawing, Shape, Vec2, archgen, display, docfile, export, import, modify, sheet,
};
use serde::{Deserialize, Serialize};
use tauri::ipc::Response;
use tauri::{AppHandle, Manager, State};

/// 열린 도면
#[derive(Default)]
struct Open {
    /// 편집 중인 모형 도면 (배치·건축 객체 포함)
    ed: Option<Editor>,
    /// 지금 보는 배치 번호. None 이면 모형
    sheet: Option<usize>,
    /// 화면에 보이는 도면 (모형 + 건축 객체가 만든 선, 또는 배치를 합친 종이 도면). None 이면 모형 그대로
    view: Option<Drawing>,
    /// 원본 경로 (DWG·DXF·dsmith)
    path: Option<PathBuf>,
    /// 저장 경로 (.dsmith). DWG·DXF 를 열었으면 처음 저장 때 묻는다
    save_path: Option<PathBuf>,
    /// 스냅 점 색인 (켜진 레이어만)
    snaps: Option<SnapIndex>,
    /// 객체 고르기 색인 (켜진 레이어만)
    picks: Option<PickIndex>,
    /// 변경 기록으로 이미 내보낸 작업 수 (M5)
    journal_mark: usize,
    /// 모형 객체 뒤에 덧붙인 객체(블록 펼친 것·건축 객체 선)의 주인 (종류, 번호)
    owners: Vec<(String, u64)>,
}

impl Open {
    /// 지금 화면에 보이는 도면
    fn shown(&self) -> Option<&Drawing> {
        self.view.as_ref().or(self.ed.as_ref().map(|e| &e.d))
    }

    /// 편집한 뒤: 보이는 도면을 다시 만들고 스냅·고르기 색인도 다시
    fn refresh(&mut self) {
        self.owners.clear();
        self.view = match (&self.ed, self.sheet) {
            (None, _) => None,
            (Some(e), Some(i)) => sheet::compose(&e.d, i),
            (Some(e), None) => archgen::with_arch_owned(&e.d).map(|(v, o)| {
                self.owners = o;
                v
            }),
        };
        let (snaps, picks) = match self.shown() {
            Some(d) => (Some(SnapIndex::build(d)), Some(PickIndex::build(d))),
            None => (None, None),
        };
        self.snaps = snaps;
        self.picks = picks;
    }

    /// 화면 객체 번호 → (모형 객체 번호, 건축 객체 주인). 블록을 펼친 객체는 그 참조 번호로
    fn owner_of(&self, i: usize) -> (Option<usize>, Option<(String, u64)>) {
        if self.sheet.is_some() {
            return (None, None);
        }
        let base = self.ed.as_ref().map(|e| e.d.entities.len()).unwrap_or(0);
        if i < base {
            return (Some(i), None);
        }
        match self.owners.get(i - base) {
            Some((k, id)) if k == "block" => (Some(*id as usize), None),
            Some(o) => (None, Some(o.clone())),
            None => (None, None),
        }
    }

    /// 모형 객체 i 의 강조선 (블록 참조면 펼친 객체들)
    fn outline_model(&self, i: usize, tol: f64) -> Vec<[f64; 4]> {
        let Some(d) = self.shown() else {
            return Vec::new();
        };
        let base = self.ed.as_ref().map(|e| e.d.entities.len()).unwrap_or(0);
        let mut out = if i < d.entities.len() {
            outline_of(d, i, tol)
        } else {
            Vec::new()
        };
        for (k, (kind, id)) in self.owners.iter().enumerate() {
            if kind == "block" && *id as usize == i && out.len() < 60_000 {
                out.extend(outline_of(d, base + k, tol));
            }
        }
        out
    }

    fn editor(&mut self) -> Result<&mut Editor, String> {
        if self.sheet.is_some() {
            return Err("배치에서는 고칠 수 없습니다 — 「모형」 탭에서 고치세요".into());
        }
        self.ed
            .as_mut()
            .ok_or_else(|| "열린 도면이 없습니다".into())
    }
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

/// 화면 설정 파일 (앱 설정 폴더의 settings.txt, 한 줄에 `이름=값`)
fn settings_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|d| d.join("settings.txt"))
}

fn read_settings(app: &AppHandle) -> Vec<(String, String)> {
    settings_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| {
            s.lines()
                .filter_map(|l| l.split_once('='))
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// 화면 설정 하나 읽기 (없으면 null)
#[tauri::command]
fn get_setting(key: String, app: AppHandle) -> Option<String> {
    read_settings(&app)
        .into_iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v)
}

/// 화면 설정 하나 쓰기
#[tauri::command]
fn set_setting(key: String, value: String, app: AppHandle) -> Result<(), String> {
    let mut all = read_settings(&app);
    all.retain(|(k, _)| *k != key);
    all.push((key, value.replace('\n', " ")));
    let p = settings_path(&app).ok_or("설정 폴더를 찾지 못했습니다")?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(err)?;
    }
    let text: String = all.iter().map(|(k, v)| format!("{k}={v}\n")).collect();
    std::fs::write(p, text).map_err(err)
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
        .add_filter(
            "도면 (DraftSmith, DWG, DXF)",
            &["dsmith", "dwg", "dxf", "DSMITH", "DWG", "DXF"],
        )
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

/// 화면이 그리는 도면 상태 (열기·편집 뒤마다)
#[derive(Serialize)]
struct DocState {
    /// 창 제목에 보일 이름
    title: String,
    path: Option<String>,
    code_page: String,
    entities: usize,
    inserts: usize,
    layers: Vec<LayerInfo>,
    /// 블록 이름들
    blocks: Vec<String>,
    /// 모형 도면 레이어 수 (그 뒤는 건축 객체가 더한 레이어 — 고칠 수 없음)
    base_layers: usize,
    current_layer: u32,
    skipped: Vec<(String, usize)>,
    /// 배치 이름 (탭 순서)
    layouts: Vec<String>,
    /// 지금 보는 배치 (None = 모형)
    sheet: Option<usize>,
    /// 복구 모드로 읽었으면 처음 오류
    recovered: Option<String>,
    can_undo: bool,
    can_redo: bool,
    dirty: bool,
    /// 건축 객체 요약 (층·벽·문·창·실 수 등)
    arch: archgen::ArchSummary,
    /// 열기에 걸린 시간 (열 때만)
    read_ms: f64,
    /// 편집으로 새로 생긴 객체 번호 (고르기 표시용)
    added: Vec<usize>,
    /// 지금 시트의 뷰포트들 (모형이면 빈 목록)
    viewports: Vec<VpInfo>,
}

fn state_of(open: &Open, read_ms: f64, added: Vec<usize>) -> Result<DocState, String> {
    let e = open.ed.as_ref().ok_or("열린 도면이 없습니다")?;
    let d = &e.d;
    let mut counts = vec![0usize; d.layers.len()];
    for x in &d.entities {
        if let Some(c) = counts.get_mut(x.layer as usize) {
            *c += 1;
        }
    }
    // 건축 객체가 만든 선이 든 레이어도 보이게
    if let Some(v) = &open.view
        && open.sheet.is_none()
    {
        for x in v.entities.iter().skip(d.entities.len()) {
            if let Some(c) = counts.get_mut(x.layer as usize) {
                *c += 1;
            }
        }
    }
    let layers_src = open
        .view
        .as_ref()
        .filter(|_| open.sheet.is_none())
        .map(|v| &v.layers)
        .unwrap_or(&d.layers);
    counts.resize(layers_src.len(), 0);
    let title = open
        .path
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "새 도면".into());
    Ok(DocState {
        title,
        path: open.path.as_ref().map(|p| p.display().to_string()),
        code_page: d.code_page.clone(),
        entities: d.entities.len(),
        inserts: d.inserts,
        layers: layers_src
            .iter()
            .zip(counts)
            .map(|(l, count)| LayerInfo {
                name: l.name.clone(),
                color: l.color,
                visible: l.visible,
                count,
            })
            .collect(),
        base_layers: d.layers.len(),
        blocks: d.blocks.iter().map(|b| b.name.clone()).collect(),
        current_layer: d.current_layer,
        skipped: d.skipped.iter().map(|(k, n)| (k.clone(), *n)).collect(),
        layouts: d.layouts.iter().map(|l| l.name.clone()).collect(),
        sheet: open.sheet,
        recovered: d.recovered.clone(),
        can_undo: e.can_undo(),
        can_redo: e.can_redo(),
        dirty: e.dirty,
        arch: archgen::summary(&d.arch),
        read_ms,
        added,
        viewports: open
            .sheet
            .and_then(|i| d.layouts.get(i))
            .map(|l| {
                l.viewports
                    .iter()
                    .enumerate()
                    .map(|(k, v)| VpInfo {
                        index: k,
                        center: [v.center.x, v.center.y],
                        size: [v.width, v.height],
                        view_center: [v.view_center.x, v.view_center.y],
                        scale: if v.height > 0.0 {
                            v.view_height / v.height
                        } else {
                            0.0
                        },
                    })
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// 시트의 뷰포트 (§35)
#[derive(Serialize)]
struct VpInfo {
    index: usize,
    center: [f64; 2],
    size: [f64; 2],
    view_center: [f64; 2],
    scale: f64,
}

/// 새 도면 (기본 레이어)
#[tauri::command]
fn new_drawing(state: State<'_, AppState>) -> Result<DocState, String> {
    let mut open = state.lock().map_err(err)?;
    *open = Open {
        ed: Some(Editor::new(archgen::new_drawing())),
        ..Default::default()
    };
    open.refresh();
    state_of(&open, 0.0, Vec::new())
}

/// 도면을 읽어 앱에 들고, 상태를 돌려준다
#[tauri::command]
async fn open_drawing(
    path: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DocState, String> {
    let t = Instant::now();
    let p = PathBuf::from(&path);
    // 읽기는 시간이 걸릴 수 있어 작업 스레드에서
    let d = tauri::async_runtime::spawn_blocking(move || import::read_file(&p))
        .await
        .map_err(err)?
        .map_err(err)?;
    let read_ms = t.elapsed().as_secs_f64() * 1000.0;
    remember(&app, &path);
    let mut open = state.lock().map_err(err)?;
    let p = PathBuf::from(&path);
    *open = Open {
        ed: Some(Editor::new(d)),
        save_path: docfile::is_dsmith(&p).then(|| p.clone()),
        path: Some(p),
        ..Default::default()
    };
    open.refresh();
    state_of(&open, read_ms, Vec::new())
}

/// 지금 상태
#[tauri::command]
fn doc_state(state: State<'_, AppState>) -> Result<DocState, String> {
    let open = state.lock().map_err(err)?;
    state_of(&open, 0.0, Vec::new())
}

/// 저장 (.dsmith). `save_as` 거나 저장 경로가 없으면 대화상자. 취소하면 null
#[tauri::command]
fn save_drawing(
    save_as: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let mut open = state.lock().map_err(err)?;
    let target = match (&open.save_path, save_as) {
        (Some(p), false) => p.clone(),
        _ => {
            let stem = open
                .path
                .as_ref()
                .and_then(|p| p.file_stem())
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "새 도면".into());
            let mut dlg = rfd::FileDialog::new()
                .set_title("저장")
                .add_filter("DraftSmith 도면", &["dsmith"])
                .set_file_name(format!("{stem}.dsmith"));
            if let Some(dir) = open.path.as_ref().and_then(|p| p.parent()) {
                dlg = dlg.set_directory(dir);
            }
            let Some(mut p) = dlg.save_file() else {
                return Ok(None);
            };
            if !docfile::is_dsmith(&p) {
                p.set_extension(docfile::EXT);
            }
            p
        }
    };
    let ed = open.ed.as_mut().ok_or("열린 도면이 없습니다")?;
    docfile::save(&ed.d, &target)?;
    ed.dirty = false;
    open.save_path = Some(target.clone());
    open.path = Some(target.clone());
    let s = target.display().to_string();
    remember(&app, &s);
    Ok(Some(s))
}

/// DXF·DWG 로 내보내기 (`format`: "dxf" | "dwg"). 취소하면 null
#[tauri::command]
fn export_cad(format: String, state: State<'_, AppState>) -> Result<Option<String>, String> {
    let open = state.lock().map_err(err)?;
    let d = open.ed.as_ref().ok_or("열린 도면이 없습니다")?;
    // 모형을 넘기면 블록은 INSERT, 치수는 DIMENSION, 건축 객체 선은 펼친 선으로 쓴다 (§31)
    let full = d.d.clone();
    let ext = if format.eq_ignore_ascii_case("dwg") {
        "dwg"
    } else {
        "dxf"
    };
    let stem = open
        .path
        .as_ref()
        .and_then(|p| p.file_stem())
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "도면".into());
    let mut dlg = rfd::FileDialog::new()
        .set_title(format!("{} 로 내보내기", ext.to_uppercase()))
        .add_filter(ext.to_uppercase(), &[ext])
        .set_file_name(format!("{stem}.{ext}"));
    if let Some(dir) = open.path.as_ref().and_then(|p| p.parent()) {
        dlg = dlg.set_directory(dir);
    }
    let Some(path) = dlg.save_file() else {
        return Ok(None);
    };
    export::write(&full, &path)?;
    Ok(Some(path.display().to_string()))
}

/// 편집 요청 (화면 → 앱). 객체 번호는 모형 도면 기준
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum EditReq {
    Add {
        shapes: Vec<Shape>,
    },
    Delete {
        ids: Vec<usize>,
    },
    /// 아핀 변환 [a, b, c, d, e, f]: x' = a·x + c·y + e, y' = b·x + d·y + f
    Transform {
        ids: Vec<usize>,
        m: [f64; 6],
    },
    Copy {
        ids: Vec<usize>,
        m: [f64; 6],
    },
    SetLayer {
        ids: Vec<usize>,
        layer: u32,
    },
    /// 색 번호 (None = 레이어 색)
    SetColor {
        ids: Vec<usize>,
        aci: Option<u8>,
    },
    Offset {
        id: usize,
        dist: f64,
        side: [f64; 2],
    },
    AddLayer {
        name: String,
        color: [u8; 3],
    },
    EditLayer {
        index: u32,
        name: String,
        color: [u8; 3],
    },
    CurrentLayer {
        layer: u32,
    },
    /// 건축 객체 편집 (M3)
    Arch {
        op: archgen::ArchReq,
    },
    /// 자르기: 고른 점이 든 구간을 없앤다 (§27)
    Trim {
        id: usize,
        pick: [f64; 2],
    },
    /// 연장: 고른 점에 가까운 끝을 경계까지
    Extend {
        id: usize,
        pick: [f64; 2],
    },
    /// 모깎기 (r = 0 이면 모서리)
    Fillet {
        a: usize,
        pa: [f64; 2],
        b: usize,
        pb: [f64; 2],
        r: f64,
    },
    /// 모따기 (§33)
    Chamfer {
        a: usize,
        pa: [f64; 2],
        b: usize,
        pb: [f64; 2],
        d1: f64,
        d2: f64,
    },
    /// 그립 옮기기: 모형 객체 id 의 손잡이 k 를 to 로 (§33)
    Grip {
        id: usize,
        k: usize,
        to: [f64; 2],
    },
    /// 해치: 점을 둘러싼 경계에 무늬
    Hatch {
        at: [f64; 2],
        pattern: String,
        spacing: f64,
    },
    /// 블록 만들기 (§30)
    MakeBlock {
        name: String,
        ids: Vec<usize>,
        base: [f64; 2],
    },
    /// 블록 넣기
    InsertBlock {
        name: String,
        at: [f64; 2],
        scale: f64,
        rotation: f64,
    },
    /// 블록 펼치기
    Explode {
        ids: Vec<usize>,
    },
    /// 지금 시트의 뷰포트 고치기 (§35)
    Viewport {
        index: usize,
        #[serde(flatten)]
        edit: sheet::VpEdit,
    },
    /// 도면 시트 만들기 (M4)
    AddSheet(sheet::SheetReq),
    /// 도면 시트 지우기
    DeleteSheet {
        index: usize,
    },
    Undo,
    Redo,
}

fn affine(m: [f64; 6]) -> Affine {
    Affine {
        a: m[0],
        b: m[1],
        c: m[2],
        d: m[3],
        e: m[4],
        f: m[5],
    }
}

/// 편집 하나를 실행하고 새 상태를 돌려준다
#[tauri::command]
fn edit(req: EditReq, state: State<'_, AppState>) -> Result<DocState, String> {
    let mut open = state.lock().map_err(err)?;
    let mut added = Vec::new();
    // 시트에서도 되는 편집: 뷰포트 고치기, 되돌리기·다시 하기
    if open.sheet.is_some()
        && matches!(
            req,
            EditReq::Viewport { .. } | EditReq::Undo | EditReq::Redo
        )
    {
        let i = open.sheet.unwrap_or(0);
        let ed = open.ed.as_mut().ok_or("열린 도면이 없습니다")?;
        match &req {
            EditReq::Viewport { index, edit } => {
                let l = sheet::edit_viewport(&ed.d, i, *index, edit)?;
                ed.set_layout(i, l);
            }
            EditReq::Undo => {
                ed.undo();
            }
            _ => {
                ed.redo();
            }
        }
        // 되돌리다 시트가 없어졌으면 모형으로
        if open.ed.as_ref().is_some_and(|e| i >= e.d.layouts.len()) {
            open.sheet = None;
        }
        open.refresh();
        return state_of(&open, 0.0, Vec::new());
    }
    // 화면에 보이는 도면(건축 객체 선 포함)을 경계로 쓰는 편집은 먼저 계산한다
    let (mut reshape, mut hatch) = (None, None);
    if matches!(
        req,
        EditReq::Trim { .. }
            | EditReq::Extend { .. }
            | EditReq::Fillet { .. }
            | EditReq::Chamfer { .. }
            | EditReq::Grip { .. }
            | EditReq::Hatch { .. }
    ) {
        if open.sheet.is_some() {
            return Err("배치에서는 고칠 수 없습니다 — 「모형」 탭에서 고치세요".into());
        }
        let base = open.ed.as_ref().map(|e| e.d.entities.len()).unwrap_or(0);
        let v = open.shown().ok_or("열린 도면이 없습니다")?;
        let model = |i: usize| {
            if i < base {
                Ok(i)
            } else {
                Err("건축 객체가 만든 선은 고칠 수 없습니다 — 객체 속성을 고치세요".to_string())
            }
        };
        let pt = |p: [f64; 2]| Vec2::new(p[0], p[1]);
        match &req {
            EditReq::Trim { id, pick } => {
                reshape = Some(vec![(model(*id)?, modify::trim(v, *id, pt(*pick))?)])
            }
            EditReq::Extend { id, pick } => {
                reshape = Some(vec![(
                    model(*id)?,
                    vec![modify::extend(v, *id, pt(*pick))?],
                )])
            }
            EditReq::Fillet { a, pa, b, pb, r } => {
                let (sa, sb) = (&v.entities[model(*a)?].shape, &v.entities[model(*b)?].shape);
                if a == b {
                    // 한 폴리선의 두 변: 그 사이 꼭짓점
                    let s = modify::corner_polyline(
                        sa,
                        pt(*pa).lerp(pt(*pb), 0.5),
                        modify::Corner::Fillet(*r),
                    )
                    .map_err(|e| {
                        format!("{e} — 선 두 개를 고르거나 폴리선 모서리 양쪽 변을 누르세요")
                    })?;
                    reshape = Some(vec![(*a, vec![s])]);
                } else if matches!(sa, Shape::Line { .. })
                    && matches!(sb, Shape::Arc { .. } | Shape::Circle { .. })
                {
                    // 선과 호·원 (§37)
                    let (nl, nc, arc) = modify::fillet_line_arc(sa, pt(*pa), sb, pt(*pb), *r)?;
                    reshape = Some(vec![(*a, vec![nl, arc]), (*b, vec![nc])]);
                } else if matches!(sb, Shape::Line { .. })
                    && matches!(sa, Shape::Arc { .. } | Shape::Circle { .. })
                {
                    let (nl, nc, arc) = modify::fillet_line_arc(sb, pt(*pb), sa, pt(*pa), *r)?;
                    reshape = Some(vec![(*b, vec![nl, arc]), (*a, vec![nc])]);
                } else {
                    let (na, nb, arc) = modify::fillet(sa, pt(*pa), sb, pt(*pb), *r)?;
                    reshape = Some(vec![
                        (*a, std::iter::once(na).chain(arc).collect()),
                        (*b, vec![nb]),
                    ]);
                }
            }
            EditReq::Chamfer {
                a,
                pa,
                b,
                pb,
                d1,
                d2,
            } => {
                let (sa, sb) = (&v.entities[model(*a)?].shape, &v.entities[model(*b)?].shape);
                if a == b {
                    let s = modify::corner_polyline(
                        sa,
                        pt(*pa).lerp(pt(*pb), 0.5),
                        modify::Corner::Chamfer(*d1, *d2),
                    )?;
                    reshape = Some(vec![(*a, vec![s])]);
                } else {
                    let (na, nb, cut) = modify::chamfer(sa, pt(*pa), sb, pt(*pb), *d1, *d2)?;
                    reshape = Some(vec![
                        (*a, std::iter::once(na).chain(cut).collect()),
                        (*b, vec![nb]),
                    ]);
                }
            }
            EditReq::Grip { id, k, to } => {
                let s = &v.entities[model(*id)?].shape;
                let n = draftsmith_core::grip::move_grip(s, *k, pt(*to))
                    .ok_or("이 손잡이로는 고칠 수 없습니다")?;
                if draftsmith_core::grip::degenerate(&n) {
                    return Err("길이·반지름이 0 이 됩니다".into());
                }
                reshape = Some(vec![(*id, vec![n])]);
            }
            EditReq::Hatch {
                at,
                pattern,
                spacing,
            } => {
                let ring = modify::region_at(v, pt(*at))
                    .ok_or("누른 곳을 둘러싼 닫힌 경계를 찾지 못했습니다")?;
                hatch = Some(modify::hatch_shape(ring, pattern, *spacing)?);
            }
            _ => {}
        }
    }
    {
        let ed = open.editor()?;
        match req {
            EditReq::Add { shapes } => added = ed.add_shapes(shapes).collect(),
            EditReq::Delete { ids } => ed.remove(&ids),
            EditReq::Transform { ids, m } => ed.transform(&ids, &affine(m)),
            EditReq::Copy { ids, m } => added = ed.copy(&ids, &affine(m)).collect(),
            EditReq::SetLayer { ids, layer } => ed.set_layer(&ids, layer),
            EditReq::SetColor { ids, aci } => ed.set_color(&ids, aci.map(Drawing::aci_rgb), aci),
            EditReq::Offset { id, dist, side } => {
                added = ed
                    .offset(id, dist, Vec2::new(side[0], side[1]))
                    .into_iter()
                    .collect();
            }
            EditReq::AddLayer { name, color } => {
                let i = ed.add_layer(&name, color);
                ed.set_current_layer(i);
            }
            EditReq::EditLayer { index, name, color } => ed.edit_layer(index, &name, color),
            EditReq::CurrentLayer { layer } => ed.set_current_layer(layer),
            EditReq::Arch { op } => archgen::apply(ed, op)?,
            EditReq::AddSheet(req) => {
                let mut d = ed.d.clone();
                let l = sheet::make(&mut d, &req)?;
                // 표제란 레이어가 새로 생겼으면 레이어 표도 함께 (되돌리기 한 단계)
                if d.layers.len() != ed.d.layers.len() {
                    let (old, new) = (ed.d.layers.clone(), d.layers);
                    ed.run(Op::Layers { old, new });
                }
                ed.add_layout(l);
            }
            EditReq::DeleteSheet { index } => ed.remove_layout(index),
            EditReq::MakeBlock { name, ids, base } => {
                added = vec![ed.make_block(&name, &ids, Vec2::new(base[0], base[1]))?]
            }
            EditReq::InsertBlock {
                name,
                at,
                scale,
                rotation,
            } => {
                added = vec![ed.insert_block(
                    &name,
                    Vec2::new(at[0], at[1]),
                    scale,
                    rotation.to_radians(),
                )?];
            }
            EditReq::Explode { mut ids } => {
                // 뒤에서부터 (앞 번호가 밀리지 않게)
                ids.sort_unstable();
                for i in ids.into_iter().rev() {
                    ed.explode(i);
                }
            }
            EditReq::Trim { .. }
            | EditReq::Extend { .. }
            | EditReq::Fillet { .. }
            | EditReq::Chamfer { .. }
            | EditReq::Grip { .. } => {
                added = ed.reshape(reshape.take().unwrap_or_default()).collect();
            }
            EditReq::Hatch { .. } => {
                added = ed.add_shapes(hatch.take().into_iter().collect()).collect()
            }
            EditReq::Viewport { .. } => return Err("뷰포트는 시트에서 고칩니다".into()),
            EditReq::Undo => {
                ed.undo();
            }
            EditReq::Redo => {
                ed.redo();
            }
        }
    }
    open.refresh();
    state_of(&open, 0.0, added)
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

/// 레이어 켜기·끄기 (되돌리기에 넣지 않는 보기 설정)
#[tauri::command]
fn set_layer_visible(
    index: usize,
    visible: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut open = state.lock().map_err(err)?;
    let ed = open.ed.as_mut().ok_or("열린 도면이 없습니다")?;
    if let Some(l) = ed.d.layers.get_mut(index) {
        l.visible = visible;
    }
    open.refresh();
    Ok(())
}

/// 볼 공간 고르기: null 이면 모형, 번호면 그 배치
#[tauri::command]
fn set_space(index: Option<usize>, state: State<'_, AppState>) -> Result<DocState, String> {
    let mut open = state.lock().map_err(err)?;
    let n = open.ed.as_ref().map(|e| e.d.layouts.len()).unwrap_or(0);
    if index.is_some_and(|i| i >= n) {
        return Err("배치 번호가 잘못되었습니다".into());
    }
    open.sheet = index;
    open.refresh();
    state_of(&open, 0.0, Vec::new())
}

#[derive(Serialize)]
struct PickOut {
    index: usize,
    /// 모형 객체인가 (건축 객체가 만든 선·배치 종이 객체는 거짓)
    editable: bool,
    /// 건축 객체면 (종류, 번호)
    arch: Option<(String, u64)>,
    /// (이름, 값) — 속성 칸에 그대로
    props: Vec<(String, String)>,
    /// 강조 표시 선분 [x0, y0, x1, y1] (도면 좌표)
    outline: Vec<[f64; 4]>,
}

fn outline_of(d: &Drawing, i: usize, tol: f64) -> Vec<[f64; 4]> {
    pick::outline(&d.entities[i].shape, tol, 20_000)
        .into_iter()
        .map(|(a, b)| [a.x, a.y, b.x, b.y])
        .collect()
}

/// 도면 좌표 (x, y) 에서 반지름 r 안의 가장 가까운 객체와 그 속성. 없으면 null
#[tauri::command]
fn pick_at(x: f64, y: f64, r: f64, state: State<'_, AppState>) -> Result<Option<PickOut>, String> {
    let open = state.lock().map_err(err)?;
    let (Some(d), Some(idx)) = (open.shown(), open.picks.as_ref()) else {
        return Ok(None);
    };
    Ok(idx.pick(d, Vec2::new(x, y), r).map(|i| {
        let (model, arch) = open.owner_of(i);
        let mut props = pick::properties(d, model.unwrap_or(i));
        if let (Some((k, id)), Some(e)) = (&arch, open.ed.as_ref()) {
            props = archgen::properties(&e.d.arch, k, *id);
        }
        let outline = match model {
            Some(m) => open.outline_model(m, r / 4.0),
            None => outline_of(d, i, r / 4.0),
        };
        PickOut {
            index: model.unwrap_or(i),
            editable: model.is_some(),
            arch,
            props,
            outline,
        }
    }))
}

/// 상자 고르기 (도면 좌표). 걸침이면 상자에 걸친 것도. 모형 객체 번호만
#[tauri::command]
fn pick_box(
    rect: [f64; 4],
    crossing: bool,
    state: State<'_, AppState>,
) -> Result<Vec<usize>, String> {
    let open = state.lock().map_err(err)?;
    if open.sheet.is_some() {
        return Ok(Vec::new());
    }
    let (Some(d), Some(idx)) = (open.shown(), open.picks.as_ref()) else {
        return Ok(Vec::new());
    };
    let mut b = Bbox::EMPTY;
    b.add(Vec2::new(rect[0], rect[1]));
    b.add(Vec2::new(rect[2], rect[3]));
    let mut out: Vec<usize> = idx
        .in_box(d, &b, crossing)
        .into_iter()
        .filter_map(|i| open.owner_of(i).0)
        .collect();
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

/// 도면 좌표 근처 객체의 모양 (보이는 도면 — 치수 도구가 원·선 모양을 읽는다, §36)
#[tauri::command]
fn shape_at(x: f64, y: f64, r: f64, state: State<'_, AppState>) -> Result<Option<Shape>, String> {
    let open = state.lock().map_err(err)?;
    let (Some(d), Some(idx)) = (open.shown(), open.picks.as_ref()) else {
        return Ok(None);
    };
    Ok(idx
        .pick(d, Vec2::new(x, y), r)
        .map(|i| d.entities[i].shape.clone()))
}

/// 손잡이 하나 (화면이 들고 있다가 옮길 곳과 함께 돌려준다)
#[derive(Serialize)]
struct GripOut {
    x: f64,
    y: f64,
    /// 모형 객체 번호 (건축 객체면 null)
    id: Option<usize>,
    /// 건축 객체 (종류, 번호)
    arch: Option<(String, u64)>,
    k: usize,
}

/// 고른 객체들의 손잡이 (§33). 객체가 많으면 처음 50 개만
#[tauri::command]
fn grips(
    ids: Vec<usize>,
    arch: Option<(String, u64)>,
    state: State<'_, AppState>,
) -> Result<Vec<GripOut>, String> {
    let open = state.lock().map_err(err)?;
    if open.sheet.is_some() {
        return Ok(Vec::new());
    }
    let ed = open.ed.as_ref().ok_or("열린 도면이 없습니다")?;
    let mut out = Vec::new();
    for &i in ids.iter().take(50) {
        if let Some(e) = ed.d.entities.get(i) {
            for (k, p) in draftsmith_core::grip::grips(&e.shape)
                .into_iter()
                .enumerate()
            {
                out.push(GripOut {
                    x: p.x,
                    y: p.y,
                    id: Some(i),
                    arch: None,
                    k,
                });
            }
        }
    }
    if let Some((what, id)) = arch {
        for (k, p) in archgen::arch_grips(&ed.d.arch, &what, id)
            .into_iter()
            .enumerate()
        {
            out.push(GripOut {
                x: p.x,
                y: p.y,
                id: None,
                arch: Some((what.clone(), id)),
                k,
            });
        }
    }
    Ok(out)
}

/// 여러 객체의 강조 표시 선분과 요약 속성
#[derive(Serialize)]
struct SelOut {
    outline: Vec<[f64; 4]>,
    props: Vec<(String, String)>,
}

#[tauri::command]
fn selection_info(ids: Vec<usize>, tol: f64, state: State<'_, AppState>) -> Result<SelOut, String> {
    let open = state.lock().map_err(err)?;
    let d = open.shown().ok_or("열린 도면이 없습니다")?;
    let ids: Vec<usize> = ids.into_iter().filter(|&i| i < d.entities.len()).collect();
    let mut outline = Vec::new();
    for &i in &ids {
        if outline.len() > 60_000 {
            break;
        }
        outline.extend(open.outline_model(i, tol));
    }
    let props = if ids.len() == 1 {
        pick::properties(d, ids[0])
    } else {
        let mut layers: Vec<String> = ids
            .iter()
            .filter_map(|&i| d.layers.get(d.entities[i].layer as usize))
            .map(|l| l.name.clone())
            .collect();
        layers.sort();
        layers.dedup();
        vec![
            ("고른 객체".into(), format!("{} 개", ids.len())),
            (
                "레이어".into(),
                if layers.len() == 1 {
                    layers[0].clone()
                } else {
                    format!("{} 개 레이어", layers.len())
                },
            ),
        ]
    };
    Ok(SelOut { outline, props })
}

/// PDF 출력 설정 (화면에서)
#[derive(Deserialize)]
struct PdfRequest {
    /// A4 … A0
    paper: String,
    /// 가로 방향
    landscape: bool,
    /// 축척 분모. 없으면 용지에 맞춤
    scale: Option<f64>,
    /// 출력 범위 [x0, y0, x1, y1] (도면 좌표). 없으면 도면 전체
    area: Option<[f64; 4]>,
    mono: bool,
    /// 색 기준 출력 스타일 (.ctb 경로). 없으면 쓰지 않음
    ctb: Option<String>,
}

/// CTB 파일 고르기 대화상자. 취소하면 null
#[tauri::command]
fn pick_ctb() -> Option<String> {
    rfd::FileDialog::new()
        .set_title("출력 스타일 (CTB) 고르기")
        .add_filter("출력 스타일 (CTB)", &["ctb", "CTB"])
        .pick_file()
        .map(|p| p.display().to_string())
}

fn pdf_options(req: &PdfRequest) -> Result<PdfOptions, String> {
    let (w, h) = pdf::paper_size(&req.paper).ok_or("모르는 용지입니다")?;
    let area = req.area.map(|[x0, y0, x1, y1]| {
        let mut b = Bbox::EMPTY;
        b.add(Vec2::new(x0, y0));
        b.add(Vec2::new(x1, y1));
        b
    });
    let pens = match &req.ctb {
        Some(p) if !p.is_empty() => Some(
            draftsmith_core::ctb::PenTable::parse(&std::fs::read(p).map_err(err)?, p)
                .map_err(|e| format!("출력 스타일을 읽지 못했습니다: {e}"))?,
        ),
        _ => None,
    };
    Ok(PdfOptions {
        pens,
        paper_mm: if req.landscape { (h, w) } else { (w, h) },
        scale: req.scale,
        area,
        mono: req.mono,
        ..Default::default()
    })
}

/// PDF 저장: 저장 대화상자 → 파일 쓰기. 취소하면 null, 저장하면 경로
#[tauri::command]
fn export_pdf(req: PdfRequest, state: State<'_, AppState>) -> Result<Option<String>, String> {
    let o = pdf_options(&req)?;
    let (bytes, stem, dir) = {
        let open = state.lock().map_err(err)?;
        let d = open.shown().ok_or("열린 도면이 없습니다")?;
        let stem = open
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "도면".into());
        let dir = open
            .path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf());
        (pdf::export(d, &o)?, stem, dir)
    };
    let mut dlg = rfd::FileDialog::new()
        .set_title("PDF 저장")
        .add_filter("PDF", &["pdf"])
        .set_file_name(format!("{stem}.pdf"));
    if let Some(dir) = dir {
        dlg = dlg.set_directory(dir);
    }
    let Some(path) = dlg.save_file() else {
        return Ok(None);
    };
    std::fs::write(&path, bytes).map_err(err)?;
    Ok(Some(path.display().to_string()))
}

/// 모든 배치를 PDF 여러 쪽으로 (도면 세트). 취소하면 null
#[tauri::command]
fn export_sheets_pdf(
    req: PdfRequest,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let o = pdf_options(&req)?;
    let (bytes, stem, dir) = {
        let open = state.lock().map_err(err)?;
        let ed = open.ed.as_ref().ok_or("열린 도면이 없습니다")?;
        let sheets: Vec<Drawing> = (0..ed.d.layouts.len())
            .filter_map(|i| sheet::compose(&ed.d, i))
            .collect();
        if sheets.is_empty() {
            return Err("배치(도면 시트)가 없습니다".into());
        }
        let stem = open
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "도면".into());
        let dir = open
            .path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf());
        (
            pdf::export_pages(
                &sheets,
                &PdfOptions {
                    scale: Some(1.0),
                    ..o
                },
            )?,
            stem,
            dir,
        )
    };
    let mut dlg = rfd::FileDialog::new()
        .set_title("도면 세트 PDF 저장")
        .add_filter("PDF", &["pdf"])
        .set_file_name(format!("{stem}-도면세트.pdf"));
    if let Some(dir) = dir {
        dlg = dlg.set_directory(dir);
    }
    let Some(path) = dlg.save_file() else {
        return Ok(None);
    };
    std::fs::write(&path, bytes).map_err(err)?;
    Ok(Some(path.display().to_string()))
}

/// 표시 데이터 (바이너리, 형식은 `draftsmith_core::display`)
#[tauri::command]
fn display_data(state: State<'_, AppState>) -> Result<Response, String> {
    let open = state.lock().map_err(err)?;
    let d = open.shown().ok_or("열린 도면이 없습니다")?;
    Ok(Response::new(display::segments(d).to_bytes()))
}

/// 문자 표시 데이터 (바이너리, 형식은 `draftsmith_core::display::texts`)
#[tauri::command]
fn text_data(state: State<'_, AppState>) -> Result<Response, String> {
    let open = state.lock().map_err(err)?;
    let d = open.shown().ok_or("열린 도면이 없습니다")?;
    Ok(Response::new(display::texts(d)))
}

/// 표 (면적표·창호일람표·실별 마감표·법규 검토) — 건축 객체에서 만든다
#[tauri::command]
fn arch_tables(state: State<'_, AppState>) -> Result<archgen::Tables, String> {
    let open = state.lock().map_err(err)?;
    let ed = open.ed.as_ref().ok_or("열린 도면이 없습니다")?;
    Ok(archgen::tables(&ed.d))
}

/// 표를 CSV 로 저장 (엑셀에서 열림). 취소하면 null
#[tauri::command]
fn export_table_csv(name: String, state: State<'_, AppState>) -> Result<Option<String>, String> {
    let csv = {
        let open = state.lock().map_err(err)?;
        let ed = open.ed.as_ref().ok_or("열린 도면이 없습니다")?;
        archgen::table_csv(&archgen::tables(&ed.d), &name).ok_or("모르는 표입니다")?
    };
    let Some(path) = rfd::FileDialog::new()
        .set_title("표 저장 (CSV)")
        .add_filter("CSV", &["csv"])
        .set_file_name(format!("{name}.csv"))
        .save_file()
    else {
        return Ok(None);
    };
    // 엑셀이 한글을 바로 읽게 UTF-8 BOM
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(csv.as_bytes());
    std::fs::write(&path, bytes).map_err(err)?;
    Ok(Some(path.display().to_string()))
}

/// 3D 보기용 삼각형 (층·벽 높이로 돌출) — M6
#[tauri::command]
fn model_3d(state: State<'_, AppState>) -> Result<Response, String> {
    let open = state.lock().map_err(err)?;
    let ed = open.ed.as_ref().ok_or("열린 도면이 없습니다")?;
    Ok(Response::new(
        draftsmith_core::model3d::mesh(&ed.d.arch).to_bytes(),
    ))
}

/// 변경 기록 내보내기 (M5): 지난번 내보낸 뒤의 작업만 `.dsjournal` 로. 취소하면 null
#[tauri::command]
fn save_journal(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let mut open = state.lock().map_err(err)?;
    let ed = open.ed.as_ref().ok_or("열린 도면이 없습니다")?;
    let hist = ed.history();
    let from = open.journal_mark.min(hist.len());
    if from == hist.len() {
        return Err("내보낼 새 변경이 없습니다".into());
    }
    let bytes = Journal::new(hist[from..].to_vec()).to_bytes()?;
    let n = hist.len();
    let stem = open
        .path
        .as_ref()
        .and_then(|p| p.file_stem())
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "도면".into());
    let Some(path) = rfd::FileDialog::new()
        .set_title("변경 기록 저장")
        .add_filter("DraftSmith 변경 기록", &[JOURNAL_EXT])
        .set_file_name(format!("{stem}-변경.{JOURNAL_EXT}"))
        .save_file()
    else {
        return Ok(None);
    };
    std::fs::write(&path, bytes).map_err(err)?;
    open.journal_mark = n;
    Ok(Some(path.display().to_string()))
}

#[derive(Serialize)]
struct MergeOut {
    applied: usize,
    skipped: usize,
    state: DocState,
}

/// 변경 기록 합치기 (M5). 취소하면 null
#[tauri::command]
fn merge_journal(state: State<'_, AppState>) -> Result<Option<MergeOut>, String> {
    let Some(path) = rfd::FileDialog::new()
        .set_title("변경 기록 합치기")
        .add_filter("DraftSmith 변경 기록", &[JOURNAL_EXT])
        .pick_file()
    else {
        return Ok(None);
    };
    let j = Journal::from_bytes(&std::fs::read(&path).map_err(err)?)?;
    let mut open = state.lock().map_err(err)?;
    let (applied, skipped) = open.editor()?.merge(&j.ops);
    // 합친 작업은 다시 내보내지 않는다
    open.journal_mark = open.ed.as_ref().map(|e| e.history().len()).unwrap_or(0);
    open.refresh();
    Ok(Some(MergeOut {
        applied,
        skipped,
        state: state_of(&open, 0.0, Vec::new())?,
    }))
}

/// 저장하지 않은 변경이 있으면 버려도 되는지 묻는다. 변경이 없으면 묻지 않고 참
fn ask_discard(open: &Open) -> bool {
    if !open.ed.as_ref().is_some_and(|e| e.dirty) {
        return true;
    }
    rfd::MessageDialog::new()
        .set_title("저장하지 않은 변경")
        .set_description("저장하지 않은 변경이 있습니다. 버리고 계속할까요?")
        .set_buttons(rfd::MessageButtons::YesNo)
        .set_level(rfd::MessageLevel::Warning)
        .show()
        == rfd::MessageDialogResult::Yes
}

/// 새 도면·열기 전에: 버려도 되면 참
#[tauri::command]
fn confirm_discard(state: State<'_, AppState>) -> Result<bool, String> {
    let open = state.lock().map_err(err)?;
    Ok(ask_discard(&open))
}

fn main() {
    tauri::Builder::default()
        .manage(AppState::default())
        // 창을 닫을 때 저장하지 않은 변경이 있으면 묻는다
        .on_window_event(|w, e| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = e {
                let state = w.state::<AppState>();
                let keep = state.lock().map(|o| !ask_discard(&o)).unwrap_or(false);
                if keep {
                    api.prevent_close();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            app_version,
            confirm_discard,
            startup_file,
            pick_drawing,
            recent_files,
            get_setting,
            set_setting,
            new_drawing,
            open_drawing,
            doc_state,
            save_drawing,
            export_cad,
            edit,
            snap,
            pick_at,
            pick_box,
            grips,
            shape_at,
            selection_info,
            export_pdf,
            export_sheets_pdf,
            pick_ctb,
            set_layer_visible,
            set_space,
            display_data,
            text_data,
            arch_tables,
            export_table_csv,
            model_3d,
            save_journal,
            merge_journal
        ])
        .run(tauri::generate_context!())
        .expect("앱을 시작하지 못했습니다");
}
