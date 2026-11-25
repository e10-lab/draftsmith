# DraftSmith

**DraftSmith** — 한글에 최적화된 가벼운 건축 설계 CAD (macOS · Windows, 오픈소스).

건축설계 업무 순서(대지·법규 검토 → 배치·매스 → 평면 → 기본설계 → 실시설계 → 허가·납품)를 그대로 따르는 작업공간을 목표로 한다.
도면은 선이 아니라 건축 객체(벽·문·창·실)로 저장하고, 평면도·면적표·일람표는 그 결과로 만든다. 2D 를 먼저 만들고 3D 로 넓힌다.

명령행 실행 파일은 `dsmith`, 앱은 `DraftSmith.app`. 개발 환경은 [BEATS](https://github.com/e10-lab/beats) 와 같다.

## 현재 상태 — 0.1.0-dev (M1 뷰어 + M2~M6 첫 판)

| 할 수 있는 것 | 비고 |
|---|---|
| DXF·DWG 열기 — 선·폴리선·원·호·타원·스플라인·솔리드·해치·치수·문자, 블록 펼치기 | acadrust |
| 도면 보기 — 이동·확대, 레이어, 한글 문자, 선 굵기, 흰 배경, 배치·뷰포트 | WebGL2 + 글꼴 층 |
| 작도·수정 — 선·폴리선·사각형·원·호·글자·치수·해치, 이동·복사·회전·대칭·크기·간격 띄우기·자르기·연장·모깎기·모따기·지우기, 그립 편집, 블록 만들기·넣기·펼치기, 되돌리기 | 명령줄·⌘K |
| 건축 객체 — 벽·문·창·실(면적 자동)·그리드·기둥·계단(꺾인 계단 포함)·층·대지 | 평면 선은 객체에서 자동 생성 |
| 표 — 면적표·창호일람표·실별 마감표·법규 검토(건폐율·용적률)·도면 목록·레이어 검사, CSV | |
| 도면 시트 — 표제란·틀·층별 뷰포트, 도면 목록 시트, 도면 세트 PDF | |
| 3D 보기 — 벽·창·바닥판·기둥·계단, 단면 자동, IFC4 내보내기 | |
| 저장 `.dsmith`, DXF·DWG 내보내기, PDF(벡터·한글 글꼴·CTB) | |
| 변경 기록 내보내기·합치기 (파일로 공동작업) | |

### 조작

| 동작 | 방법 |
|---|---|
| 명령 | 아래 명령줄에 단축(L·PL·REC·C·A·T·DIM·DRA·DDI·DAN·DST·H·M·CO·RO·MI·SC·O·TR·EX·F·CHA·B·I·X·E·WA·DO·WI·RM·GR·GRS·COL·ST·SEC·SH·SHL·TB·3D·IFC) 또는 한글 이름(선·이동·벽 …) + Enter/Space |
| 명령 찾기 | ⌘K / Ctrl+K — 이름·단축·초성(ㄱㄸ) |
| 점 입력 | 누르기(스냅) 또는 `x,y` · `@dx,dy` · `@거리<각` · `거리` |
| 마치기·되풀이 | 빈 Enter·Space·오른쪽 누르기 (도구가 없으면 지난 명령 되풀이), Esc 취소 |
| 스냅·직교 | F3 · F8 (Shift 누른 동안 직교) |
| 고르기 | 누르기(Shift 더하기·빼기), 왼→오른 끌기 창 고르기, 오른→왼 걸침 |
| 그립 | 고른 객체의 파란 네모를 누르고 옮길 곳을 누르기 (`@dx,dy` 도 됨, Esc 취소) |
| 화면 이동·확대 | 가운데 버튼·Space+끌기·트랙패드 두 손가락 / 휠·오므리기, 「Z」 전체 보기 |
| 되돌리기 | ⌘Z / Ctrl+Z, 다시 ⇧⌘Z / Ctrl+Y |
| 저장 | ⌘S / Ctrl+S (`.dsmith`), ⇧⌘S 다른 이름 — DWG·DXF 는 「내보내기」 |
| 레이어 | 옆 칸 — 켜기·끄기, 색 칸 눌러 색, 이름 두 번 눌러 현재 레이어, ＋ 더하기 |
| 건축 객체 고치기 | 눌러 고르면 옆 칸에서 두께·폭·이름·마감 등 바로 고침 |
| 층 | 옆 칸 「층」 — 눌러 바꾸기, ＋ 더하기, ⧉ 지금 층 복사 |

검증 결과는 [docs/verification.md](docs/verification.md). 계획 전체는 [docs/ROADMAP.md](docs/ROADMAP.md), 결정과 근거는 [docs/implementation-notes.md](docs/implementation-notes.md).

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
