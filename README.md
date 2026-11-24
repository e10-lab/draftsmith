# DraftSmith

**DraftSmith** — 한글에 최적화된 가벼운 건축 설계 CAD (macOS · Windows, 오픈소스).

건축설계 업무 순서(대지·법규 검토 → 배치·매스 → 평면 → 기본설계 → 실시설계 → 허가·납품)를 그대로 따르는 작업공간을 목표로 한다.
도면은 선이 아니라 건축 객체(벽·문·창·실)로 저장하고, 평면도·면적표·일람표는 그 결과로 만든다. 2D 를 먼저 만들고 3D 로 넓힌다.

명령행 실행 파일은 `dsmith`, 앱은 `DraftSmith.app`. 개발 환경은 [BEATS](https://github.com/e10-lab/beats) 와 같다.

## 현재 상태 — M0 기술 검증 시작 (0.1.0-dev)

| 할 수 있는 것 | 비고 |
|---|---|
| DXF·DWG 열기 (선·폴리선·원·호·문자) | acadrust. 블록·해치·치수는 아직 개수만 알림 |
| 도면 보기 — 이동·확대·전체 보기, 레이어 켜기·끄기 | WebGL2. 문자는 아직 밑줄로 자리만 표시 |
| `dsmith info / sample / bench / display` | 도면 정보, 측정용 도면 만들기, 시간 측정 |

계획 전체는 [docs/ROADMAP.md](docs/ROADMAP.md), 결정과 근거는 [docs/implementation-notes.md](docs/implementation-notes.md).

## 구조

```
crates/draftsmith-core   도면 모델, DXF·DWG 가져오기, 화면용 표시 데이터 (화면 코드 없음)
crates/draftsmith-cli    dsmith 명령행
crates/draftsmith-app    데스크톱 앱 (Tauri 2) — 화면은 ui/ 의 HTML·CSS·JavaScript 모듈 (npm·번들러 없음)
build/version.rs         화면·CLI 에 보이는 판 (릴리스가 아니면 「-dev (커밋)」)
tools/                   앱 묶음 만들기 등
```

## 빌드·실행

```sh
cargo test                                  # 코어·CLI 시험
cargo run --release -p draftsmith-cli -- info 도면.dwg
cargo run --release -p draftsmith-app       # 앱 (macOS·Windows)
sh tools/make_macos_app.sh                  # target/DraftSmith.app
```

M0 측정용 큰 도면: `dsmith sample 100000 examples/bench-100k.dxf` 뒤 앱에서 열면 상태 막대에 읽기·전달·올리기 시간이 나온다.

## 판

판은 `Cargo.toml` 한 곳에만 적는다. 릴리스: 시험 → `Cargo.toml` 판 → 이 기록 → 커밋 → 태그 `v0.X.0` → 푸시.

## 라이선스

MIT 또는 Apache-2.0 (BEATS 와 같음). 의존 라이브러리 acadrust 는 MPL-2.0.
