//! `dsmith` — DraftSmith 명령행.
//!
//! - `dsmith info 도면.dxf|dwg` : 객체·레이어 개수, 문자 코드, 범위, 가져오지 못한 객체
//! - `dsmith sample 개수 출력.dxf` : M0 측정용 격자 평면 도면 만들기
//! - `dsmith bench 도면.dxf|dwg` : 읽기·표시 데이터 만들기 시간 (M0 ①·②)
//! - `dsmith display 도면 출력.bin` : 표시 데이터를 파일로 (화면 시험용)
//! - `dsmith ifc 도면.dsmith 출력.ifc` : 건축 객체를 IFC4 로
//! - `dsmith demo 출력.dsmith` : 검증용 예제 건물
//! - `dsmith export 도면 출력.dxf|dwg` : DXF·DWG 로 내보내기
//! - `dsmith pdf 도면 출력.pdf [--paper A3] [--portrait] [--scale 100] [--mono] [--layout 이름]` : PDF 출력

use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use clap::{Parser, Subcommand};
use draftsmith_core::{Drawing, archgen, display, import, pdf, sample, sheet};

/// 보이는 도면: 블록 참조와 건축 객체(.dsmith)를 펼친 것
fn shown(d: Drawing) -> Drawing {
    archgen::with_arch(&d).unwrap_or(d)
}

#[derive(Parser)]
#[command(name = "dsmith", version = env!("DRAFTSMITH_VERSION"), about = "DraftSmith 명령행")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 도면 정보
    Info { file: PathBuf },
    /// 측정용 격자 평면 도면(DXF) 만들기
    Sample {
        /// 대략의 객체 개수
        count: usize,
        out: PathBuf,
    },
    /// 읽기·표시 데이터 시간 측정
    Bench { file: PathBuf },
    /// 표시 데이터(바이너리)를 파일로 쓰기 — 화면 시험·디버깅용
    Display { file: PathBuf, out: PathBuf },
    /// 건축 객체를 IFC4 로 (.dsmith)
    Ifc { file: PathBuf, out: PathBuf },
    /// 검증용 예제 건물(2 층 주택)을 .dsmith 로
    Demo { out: PathBuf },
    /// DXF·DWG 로 내보내기 (확장자로 고름) — 앱 「내보내기」 와 같음
    Export {
        file: PathBuf,
        out: PathBuf,
        /// 건축 객체를 펼칠 층 이름 (없으면 지금 층)
        #[arg(long)]
        level: Option<String>,
        /// 모든 층을 나란히 한 파일에
        #[arg(long)]
        all_levels: bool,
    },
    /// PDF 로 출력
    Pdf {
        file: PathBuf,
        out: PathBuf,
        /// 용지 (A4, A3, A2, A1, A0)
        #[arg(long, default_value = "A3")]
        paper: String,
        /// 세로 방향 (기본은 가로)
        #[arg(long)]
        portrait: bool,
        /// 축척 분모 (예: 100 → 1:100). 없으면 용지에 맞춤
        #[arg(long)]
        scale: Option<f64>,
        /// 흑백
        #[arg(long)]
        mono: bool,
        /// 배치 이름 (없으면 모형). 배치는 종이 단위라 보통 --scale 1
        #[arg(long)]
        layout: Option<String>,
        /// 색 기준 출력 스타일 (.ctb 파일)
        #[arg(long)]
        ctb: Option<PathBuf>,
        /// 모형 공간의 도면 틀마다 한 쪽 (용지·축척은 틀 크기로 — 구현 노트 §63)
        #[arg(long)]
        frames: bool,
    },
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Info { file } => {
            let d = import::read_file(&file)?;
            let b = d.bbox();
            println!("파일      {}", file.display());
            if let Some(e) = &d.recovered {
                println!("복구      깨진 곳이 있어 복구 모드로 읽었습니다 ({e})");
            }
            println!(
                "문자 코드 {}",
                if d.code_page.is_empty() {
                    "-"
                } else {
                    &d.code_page
                }
            );
            println!("레이어    {} 개", d.layers.len());
            if !d.arch.is_empty() {
                let a = draftsmith_core::archgen::summary(&d.arch);
                println!(
                    "건축 객체 층 {} · 벽 {} · 문·창 {} · 실 {} · 기둥 {} · 보 {} · 계단 {} · 치수 {} · 그리드 {}",
                    a.levels.len(),
                    a.walls,
                    a.openings,
                    a.rooms,
                    a.columns,
                    a.beams,
                    a.stairs,
                    a.dims,
                    a.grids
                );
            }
            println!(
                "객체      {} 개 (블록 참조 {} 개 펼침)",
                d.entities.len(),
                d.inserts
            );
            for (k, n) in d.counts() {
                println!("  {k:<8} {n}");
            }
            // 글자 스타일별 글꼴 (§57) — SHX 는 화면·PDF 에서 폴더에 있으면 그 모양으로 (§59)
            let used: Vec<_> = d.fonts.iter().filter(|f| f.count > 0).collect();
            if !used.is_empty() {
                println!("글꼴");
                for f in used {
                    println!(
                        "  {:<16} {:<14} {:<14} {}",
                        f.style, f.font, f.big_font, f.count
                    );
                }
            }
            if !d.skipped.is_empty() {
                println!("가져오지 않은 객체");
                for (k, n) in &d.skipped {
                    println!("  {k:<18} {n}");
                }
            }
            if !b.is_empty() {
                println!("범위      {:.0} × {:.0}", b.width(), b.height());
            }
            for l in &d.layouts {
                println!(
                    "배치      {} (종이 객체 {}, 뷰포트 {})",
                    l.name,
                    l.paper.len(),
                    l.viewports.len()
                );
            }
        }
        Cmd::Sample { count, out } => {
            sample::write_plan_grid(count, &out).map_err(anyhow::Error::msg)?;
            println!("{} 에 약 {count} 개 객체 도면을 썼습니다", out.display());
        }
        Cmd::Bench { file } => {
            // SHX 폴더 (환경 변수 DSMITH_SHX) — 앱과 같은 보이는 도면 단계를 재려고
            if let Ok(dir) = std::env::var("DSMITH_SHX") {
                draftsmith_core::shx::set_folder(Some(dir.into()));
            }
            let t0 = Instant::now();
            let model = import::read_file(&file)?;
            let tr = Instant::now();
            let d = shown(model.clone());
            let t1 = Instant::now();
            // 편집 한 번마다 앱이 하는 일: 보이는 도면 다시 만들기 (평균 3 번)
            for _ in 0..3 {
                let _ = draftsmith_core::archgen::with_arch_owned(&model);
            }
            let tv = Instant::now();
            let s = display::segments(&d);
            let t2 = Instant::now();
            let bytes = s.to_bytes();
            let t3 = Instant::now();
            let ms = |a: Instant, b: Instant| (b - a).as_secs_f64() * 1000.0;
            println!(
                "객체 {} 개 → 선분 {} 개, 표시 데이터 {:.1} MB",
                d.entities.len(),
                s.len(),
                bytes.len() as f64 / 1e6
            );
            println!("읽기        {:8.1} ms", ms(t0, tr));
            // 보이는 도면 캐시 (§72): 한 객체를 바꾼 뒤 바뀐 몫만
            let mut shc = draftsmith_core::shown::ShownCache::default();
            let first = shc.update(&model, 100.0, None);
            let mut m2 = model.clone();
            let k = m2.entities.len() / 2;
            if let Some(e) = m2.entities.get_mut(k) {
                e.color = Some([1, 2, 3]);
            }
            let tc = Instant::now();
            let second = shc.update(&m2, 100.0, first);
            let tc2 = Instant::now();
            // 앱은 결과를 다음 편집에 다시 쓰므로 버리는 시간은 재지 않는다
            drop(second);
            println!(
                "보이는 도면 {:8.1} ms (편집마다 다시 — {:.1} ms · 캐시 {:.1} ms, 다시 처리한 모형 객체 {})",
                ms(tr, t1),
                ms(t1, tv) / 3.0,
                ms(tc, tc2),
                shc.changed
            );
            println!("선분 펼치기 {:8.1} ms", ms(t1, t2));
            println!("바이너리    {:8.1} ms", ms(t2, t3));
            // 캐시 (§66): 처음 한 번 채우고, 한 객체를 바꾼 뒤 다시
            let mut cache = display::SegCache::default();
            let _ = display::segments_cached(&d, &mut cache);
            let mut d2 = d.clone();
            if let Some(e) = d2.entities.first_mut() {
                e.color = Some([1, 2, 3]);
            }
            let t4 = Instant::now();
            let _ = display::segments_cached(&d2, &mut cache);
            let t5 = Instant::now();
            println!(
                "편집 뒤 선분 (캐시) {:8.1} ms — 다시 펼친 객체 {}",
                ms(t4, t5),
                cache.rebuilt
            );
            // 앱이 편집 뒤 보내는 것 (§73): 부분 갱신이면 바뀐 객체 것만
            let mut d2b = d2.clone();
            if let Some(e) = d2b.entities.get_mut(1) {
                e.color = Some([4, 5, 6]);
            }
            let tu = Instant::now();
            let ub = display::update_bytes(&d2b, &mut cache);
            let tu2 = Instant::now();
            println!(
                "편집 뒤 보낼 데이터 {:8.1} ms ({} 바이트{})",
                ms(tu, tu2),
                ub.len(),
                if ub.starts_with(b"DSP1") {
                    ", 부분 갱신"
                } else {
                    ""
                }
            );
            // 가운데 객체 하나 지우기 (뒤 객체 번호가 밀린다)
            let mut d3 = d2.clone();
            d3.entities.remove(d3.entities.len() / 2);
            let tb = Instant::now();
            let _ = d3.bbox();
            let tb2 = Instant::now();
            let _ = display::segments_cached(&d3, &mut cache);
            let tb3 = Instant::now();
            println!(
                "가운데 지운 뒤 선분 (캐시) {:8.1} ms — 다시 펼친 객체 {} (범위 계산 {:.1} ms)",
                ms(tb2, tb3),
                cache.rebuilt,
                ms(tb, tb2)
            );
            // 스냅 색인 (§69): 새로 만들기와, 한 객체를 바꾼 뒤 고치기
            let mut sc = draftsmith_core::snap::SnapCache::default();
            let t6 = Instant::now();
            let n = sc.update(&d).len();
            let t7 = Instant::now();
            let _ = sc.update(&d2);
            let t8 = Instant::now();
            println!(
                "스냅 색인   {:8.1} ms (점 {n}) · 편집 뒤 {:.1} ms",
                ms(t6, t7),
                ms(t7, t8)
            );
            // 고르기 색인 (§71): 새로 만들기와, 스냅 캐시가 맞춘 범위로 고치기
            let t9 = Instant::now();
            let mut pi = draftsmith_core::pick::PickIndex::build(&d);
            let t10 = Instant::now();
            let _ = sc.update(&d);
            if let Some((pre, o, n)) = sc.diff {
                let t11 = Instant::now();
                pi.update(&d, pre, o, n);
                let t12 = Instant::now();
                println!(
                    "고르기 색인 {:8.1} ms · 편집 뒤 {:.1} ms",
                    ms(t9, t10),
                    ms(t11, t12)
                );
            }
        }
        Cmd::Pdf {
            file,
            out,
            paper,
            portrait,
            scale,
            mono,
            layout: lay,
            ctb,
            frames,
        } => {
            let model = import::read_file(&file)?;
            let d = match &lay {
                None => shown(model.clone()),
                Some(name) => {
                    let i = model
                        .layouts
                        .iter()
                        .position(|l| l.name == *name)
                        .ok_or_else(|| anyhow::anyhow!("배치 「{name}」 가 없습니다"))?;
                    sheet::compose(&model, i)
                        .ok_or_else(|| anyhow::anyhow!("배치를 합치지 못했습니다"))?
                }
            };
            let (w, h) =
                pdf::paper_size(&paper).ok_or_else(|| anyhow::anyhow!("모르는 용지: {paper}"))?;
            let paper_mm = if portrait { (w, h) } else { (h, w) };
            let pens = match &ctb {
                Some(p) => Some(
                    draftsmith_core::ctb::PenTable::parse(
                        &std::fs::read(p)?,
                        &p.display().to_string(),
                    )
                    .map_err(anyhow::Error::msg)?,
                ),
                None => None,
            };
            let o = pdf::PdfOptions {
                pens,
                paper_mm,
                scale,
                mono,
                ..Default::default()
            };
            if frames {
                let fs = draftsmith_core::frames::find(&model);
                let (bytes, n) = pdf::export_frames(&model, &o).map_err(anyhow::Error::msg)?;
                std::fs::write(&out, bytes)?;
                println!("{} 에 썼습니다 (도면 틀 {n} 장)", out.display());
                for (i, f) in fs.iter().enumerate() {
                    println!(
                        "  {:>2}  {} 1:{}  ({:.0}, {:.0})",
                        i + 1,
                        f.paper,
                        f.scale,
                        f.area.min.x,
                        f.area.min.y
                    );
                }
                return Ok(());
            }
            std::fs::write(&out, pdf::export(&d, &o).map_err(anyhow::Error::msg)?)?;
            println!("{} 에 썼습니다", out.display());
        }
        Cmd::Display { file, out } => {
            let d = shown(import::read_file(&file)?);
            std::fs::write(&out, display::segments(&d).to_bytes())?;
        }
        Cmd::Demo { out } => {
            let d = draftsmith_core::demo::demo_building().map_err(anyhow::Error::msg)?;
            draftsmith_core::docfile::save(&d, &out).map_err(anyhow::Error::msg)?;
            println!(
                "{} 에 썼습니다 (층 {} · 벽 {} · 실 {} · 시트 {})",
                out.display(),
                d.arch.levels.len(),
                d.arch.walls.len(),
                d.arch.rooms.len(),
                d.layouts.len()
            );
        }
        Cmd::Export {
            file,
            out,
            level,
            all_levels,
        } => {
            let mut d = import::read_file(&file)?;
            if let Some(name) = level {
                let l = d
                    .arch
                    .levels
                    .iter()
                    .find(|l| l.name == name)
                    .ok_or_else(|| anyhow::anyhow!("층 「{name}」 이 없습니다"))?;
                d.arch.active_level = l.id;
            }
            if all_levels {
                draftsmith_core::export::write_all_levels(&d, &out).map_err(anyhow::Error::msg)?;
            } else {
                draftsmith_core::export::write(&d, &out).map_err(anyhow::Error::msg)?;
            }
            println!("{} 에 썼습니다", out.display());
        }
        Cmd::Ifc { file, out } => {
            let d = import::read_file(&file)?;
            let name = file
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            std::fs::write(&out, draftsmith_core::ifc::write(&d.arch, &name))?;
            println!(
                "{} 에 썼습니다 (벽 {} · 실 {} · 기둥 {})",
                out.display(),
                d.arch.walls.len(),
                d.arch.rooms.len(),
                d.arch.columns.len()
            );
        }
    }
    Ok(())
}
