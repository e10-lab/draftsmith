include!("../../build/version.rs");

fn main() {
    // 아이콘은 실행 파일 안에 들어가므로, 바뀌면 다시 빌드한다.
    println!("cargo:rerun-if-changed=icons");
    // 화면·CLI 에 보이는 판 — 릴리스가 아니면 「-dev (커밋)」 (구현 노트 §2)
    emit(&["src", "ui"]);
    tauri_build::build()
}
