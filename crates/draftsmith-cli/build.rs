include!("../../build/version.rs");

fn main() {
    // 화면·CLI 에 보이는 판 — 릴리스가 아니면 「-dev (커밋)」 (구현 노트 §2)
    emit(&["src"]);
}
