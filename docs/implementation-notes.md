# 구현 노트

결정과 근거를 번호(§)로 남긴다. 코드 주석은 이 번호를 가리킨다.

## §1 개발 환경을 BEATS 와 같게 (2026-11-24)

Rust 작업공간에 코어·CLI·앱 세 crate. 앱은 Tauri 2, 화면은 HTML·CSS·순수 JS 모듈(npm·번들러 없음).
`cargo build/test` 기본 대상은 코어·CLI 이고 앱은 `-p draftsmith-app`.
검토했다가 접은 것: 네이티브 화면 위에 투명 웹 UI 겹치기, Rust 네이티브 UI(egui·iced·Slint), Qt, GPUI —
한글 입력 안정성과 개발 부담 때문 (결정 과정은 ROADMAP 2026-11-24 판 이전 논의).

## §2 판 표시

판은 `Cargo.toml` 한 곳. `build/version.rs` 가 git 상태를 보고 「0.1.0」·「0.1.0-dev (커밋)」·「…*」 를
`DRAFTSMITH_VERSION` 으로 넘긴다 (BEATS 구현 노트 §218 과 같은 방식).

## §3 DXF·DWG 는 acadrust

순수 Rust, MPL-2.0, R12~R2018 읽기·쓰기. DWG 코드는 ACadSharp(MIT)에서 옮긴 것이라 GPL 코드가 없다
(`THIRD_PARTY_NOTICES.md` 확인). 2007 이전 도면의 `ANSI_949`·조합형을 `encoding_rs` 로 푼다 (소스 확인).
DWG 여부는 파일 머리 `AC` + 숫자로 본다.

지금 가져오는 것: LINE, LWPOLYLINE, POLYLINE(2D), CIRCLE, ARC, TEXT, MTEXT(위치·크기만).
그 밖(INSERT·HATCH·DIMENSION 등)은 `Drawing::skipped` 에 개수를 남겨 화면·CLI 에 알린다 — 실무 도면은
블록이 많으므로 INSERT 펼치기가 M0 ② 의 첫 일이다.

## §4 화면에는 바이너리 표시 데이터

열린 도면은 앱(Rust)이 들고, 화면에는 `display_data` 명령이 선분 목록 바이너리(`DSD1`)를
`tauri::ipc::Response` 로 넘긴다. 형식은 `draftsmith_core::display` 머리 주석.

- 좌표는 도면 범위 중심(origin)을 뺀 f32 — GPU f32 정밀도 문제(TM 좌표) 회피.
- 원호는 화살 높이가 도면 대각선의 1/20000 이하가 되게 8–512 조각.
- 문자는 글꼴 처리 전이라 밑줄로 자리만 (M0 ⑥).

첫 측정 (2026-11-24, 클라우드 개발 컨테이너, 4 코어):
`dsmith sample 100000` (선 4 만·원 4 만·폴리선 1 만·한글 문자 1 만, DXF 16.6 MB) →
읽기 358 ms, 선분 41 만 개 펼치기 14 ms, 바이너리 3 ms (8.2 MB).
화면(Chromium, 소프트웨어 WebGL)에 올리기 18 ms. 실제 WKWebView·WebView2 와 Tauri 전달 시간은 실기 측정이 남았다.
