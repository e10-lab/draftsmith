//! `dsmith` — DraftSmith 명령행.
//!
//! - `dsmith info 도면.dxf|dwg` : 객체·레이어 개수, 문자 코드, 범위, 가져오지 못한 객체
//! - `dsmith sample 개수 출력.dxf` : M0 측정용 격자 평면 도면 만들기
//! - `dsmith bench 도면.dxf|dwg` : 읽기·표시 데이터 만들기 시간 (M0 ①·②)
//! - `dsmith display 도면 출력.bin` : 표시 데이터를 파일로 (화면 시험용)
//! - `dsmith ifc 도면.dsmith 출력.ifc` : 건축 객체를 IFC4 로
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
            println!(
                "객체      {} 개 (블록 참조 {} 개 펼침)",
                d.entities.len(),
                d.inserts
            );
            for (k, n) in d.counts() {
                println!("  {k:<8} {n}");
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
            let t0 = Instant::now();
            let d = shown(import::read_file(&file)?);
            let t1 = Instant::now();
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
            println!("읽기        {:8.1} ms", ms(t0, t1));
            println!("선분 펼치기 {:8.1} ms", ms(t1, t2));
            println!("바이너리    {:8.1} ms", ms(t2, t3));
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
        } => {
            let model = import::read_file(&file)?;
            let d = match &lay {
                None => shown(model),
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
            std::fs::write(&out, pdf::export(&d, &o).map_err(anyhow::Error::msg)?)?;
            println!("{} 에 썼습니다", out.display());
        }
        Cmd::Display { file, out } => {
            let d = shown(import::read_file(&file)?);
            std::fs::write(&out, display::segments(&d).to_bytes())?;
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
