# DraftSmith

**DraftSmith** — 한글에 최적화된 가벼운 건축 설계 CAD (macOS · Windows, 오픈소스).

건축설계 업무 순서(대지·법규 검토 → 배치·매스 → 평면 → 기본설계 → 실시설계 → 허가·납품)를 그대로 따르는 작업공간을 목표로 한다.
도면은 선이 아니라 건축 객체(벽·문·창·실)로 저장하고, 평면도·면적표·일람표는 그 결과로 만든다. 2D 를 먼저 만들고 3D 로 넓힌다.

명령행 실행 파일은 `dsmith`, 앱은 `DraftSmith.app`. 개발 환경은 [BEATS](https://github.com/e10-lab/beats) 와 같다.

## 현재 상태 — M0 기술 검증 시작 (0.1.0-dev)

| 할 수 있는 것 | 비고 |
|---|---|
| DXF·DWG 열기 — 선·폴리선·원·호·타원·스플라인·솔리드·해치(채움·무늬)·치수·문자, 블록 펼치기 | acadrust |
| 도면 보기 — 이동·확대·전체 보기, 레이어 켜기·끄기, 한글 문자 | WebGL2 + 글꼴 층 |
| 거리 재기 (D) — 끝점·중간점·중심·사분점 스냅 | 스냅은 Rust 코어(R*-tree) |
| 객체 고르기 — 강조 표시, 속성(길이·면적 ㎡·평 등) | |
| PDF 저장 — 벡터, 한글 글꼴 부분 삽입, 축척·용지·흑백 | krilla, `dsmith pdf` 로도 |
| 배치(종이 공간)·뷰포트 보기 — 도면 아래 탭 | |
| 최근 파일, 끌어 놓아 열기, `draftsmith-app 도면.dwg` 로 열기 | |
| `dsmith info / sample / bench / display` | 도면 정보, 측정용 도면 만들기, 시간 측정 |

### 조작

| 동작 | 방법 |
|---|---|
| 열기 | 「열기」 또는 ⌘O / Ctrl+O, 파일을 창에 끌어 놓기 |
| 화면 이동 | 끌기 (왼쪽·가운데 버튼), 트랙패드 두 손가락 |
| 확대·축소 | 휠, 트랙패드 오므리기 — 커서 위치 기준 |
| 전체 보기 | F, 「전체 보기」, 두 번 누르기 |
| 흰 배경 | B 또는 「흰 배경」 — 흰 선은 검게 |
| PDF 저장 | ⌘P / Ctrl+P 또는 「PDF」 — 용지(A4–A0)·방향·축척(맞춤·1:50–1:1000)·범위(전체·지금 화면)·컬러/흑백 |
| 객체 고르기 | 누르기 → 왼쪽에 속성. Esc 로 풀기 |
| 거리 재기 | D 또는 「측정」 → 두 점 누르기, Esc 로 끝 |

계획 전체는 [docs/ROADMAP.md](docs/ROADMAP.md), 결정과 근거는 [docs/implementation-notes.md](docs/implementation-notes.md).

## 구조

```
crates/draftsmith-core   도면 모델, DXF·DWG 가져오기, 화면용 표시 데이터 (화면 코드 없음)
crates/draftsmith-cli    dsmith 명령행
crates/draftsmith-app    데스크톱 앱 (Tauri 2) — 화면은 ui/ 의 HTML·CSS·JavaScript 모듈 (npm·번들러 없음)
build/version.rs         화면·CLI 에 보이는 판 (릴리스가 아니면 「-dev (커밋)」)
tools/                   앱 묶음 만들기 등
```

## 내려받아 실행 (빌드 없이)

`main` 에 올릴 때마다 GitHub Actions 가 macOS·Windows 앱을 만든다.
저장소의 **Actions → build → 맨 위 실행 → Artifacts** 에서 내려받는다 (GitHub 로그인 필요).

- **macOS** (`DraftSmith-macos`): 압축을 풀면 `DraftSmith-macos.zip` → 다시 풀어 `DraftSmith.app`.
  아직 Apple 공증 전이라 처음 한 번은 막힌다. 터미널에서 `xattr -cr DraftSmith.app` 을 한 번 실행하거나,
  「시스템 설정 → 개인정보 보호 및 보안」 에서 「그래도 열기」 를 누른다.
- **Windows** (`DraftSmith-windows`): `DraftSmith.exe` 를 실행한다. 서명 전이라 SmartScreen 이 경고하면
  「추가 정보 → 실행」. Windows 10·11 에 기본으로 있는 WebView2 를 쓴다.

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
