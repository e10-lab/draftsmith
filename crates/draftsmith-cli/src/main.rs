//! `dsmith` — DraftSmith 명령행.
//!
//! - `dsmith info 도면.dxf|dwg` : 객체·레이어 개수, 문자 코드, 범위, 가져오지 못한 객체
//! - `dsmith sample 개수 출력.dxf` : M0 측정용 격자 평면 도면 만들기
//! - `dsmith bench 도면.dxf|dwg` : 읽기·표시 데이터 만들기 시간 (M0 ①·②)
//! - `dsmith display 도면 출력.bin` : 표시 데이터를 파일로 (화면 시험용)

use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use clap::{Parser, Subcommand};
use draftsmith_core::{display, import, sample};

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
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Info { file } => {
            let d = import::read_file(&file)?;
            let b = d.bbox();
            println!("파일      {}", file.display());
            println!(
                "문자 코드 {}",
                if d.code_page.is_empty() {
                    "-"
                } else {
                    &d.code_page
                }
            );
            println!("레이어    {} 개", d.layers.len());
            println!("객체      {} 개", d.entities.len());
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
        }
        Cmd::Sample { count, out } => {
            sample::write_plan_grid(count, &out).map_err(anyhow::Error::msg)?;
            println!("{} 에 약 {count} 개 객체 도면을 썼습니다", out.display());
        }
        Cmd::Bench { file } => {
            let t0 = Instant::now();
            let d = import::read_file(&file)?;
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
        Cmd::Display { file, out } => {
            let d = import::read_file(&file)?;
            std::fs::write(&out, display::segments(&d).to_bytes())?;
        }
    }
    Ok(())
}
