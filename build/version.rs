// 화면·CLI 에 보이는 판 (구현 노트 §2) — 앱(draftsmith-app)·CLI 의 build.rs 가 함께 쓴다.
// 릴리스(HEAD 에 태그 `v{판}` 이 있고 커밋 안 한 변경이 없음)면 「0.1.0」, 그 밖의 개발 빌드는
// 「0.1.0-dev (52dfa00)」, 커밋 안 한 변경이 있으면 「0.1.0-dev (52dfa00*)」. git 이 없으면 「0.1.0-dev」.
use std::path::Path;
use std::process::Command;

pub fn emit(watch: &[&str]) {
    let v = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let root = Path::new(&dir).join("../..");
    let git = |args: &[&str]| -> Option<String> {
        let o = Command::new("git").arg("-C").arg(&root).args(args).output().ok()?;
        o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let shown = match git(&["rev-parse", "--short", "HEAD"]) {
        None => format!("{v}-dev"),
        Some(hash) => {
            let tagged = git(&["tag", "--points-at", "HEAD"])
                .is_some_and(|t| t.lines().any(|l| l == format!("v{v}")));
            let dirty = git(&["status", "--porcelain", "--untracked-files=no"])
                .is_some_and(|s| !s.is_empty());
            if tagged && !dirty {
                v.clone()
            } else {
                format!("{v}-dev ({hash}{})", if dirty { "*" } else { "" })
            }
        }
    };
    println!("cargo:rustc-env=DRAFTSMITH_VERSION={shown}");
    // 커밋·태그·스테이지가 바뀌면 다시 (판 표시만 바뀌는 가벼운 다시 빌드)
    for p in ["HEAD", "index", "refs/heads", "refs/tags", "packed-refs"] {
        println!("cargo:rerun-if-changed={}", root.join(".git").join(p).display());
    }
    // 고친 파일(「*」 표시)도 따라가게 — 이 crate·core 의 소스
    for p in watch {
        println!("cargo:rerun-if-changed={p}");
    }
    println!("cargo:rerun-if-changed=../draftsmith-core/src");
}
