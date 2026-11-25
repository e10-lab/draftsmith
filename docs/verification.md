# 최초 버전 검증 (2026-11-25)

대상: `main` 의 0.1.0-dev (M1~M6 첫 판). 이 문서는 개발 환경(Linux)에서 할 수 있는 검증의 결과와, Mac·Windows
실제 기기에서 남은 확인을 적는다. 다시 할 때는 아래 「다시 하기」 의 명령을 그대로 쓴다.

## 1. 자동 시험

| 항목 | 결과 |
|---|---|
| `sh tools/check.sh` (fmt·clippy 경고 금지·시험) | 통과 — 코어 시험 72 개 |
| CI (macOS 유니버설·Windows x64 앱 빌드) | 통과 |

## 2. 예제 건물로 파일 왕복 (`dsmith demo`)

예제: 2 층 주택 — 그리드 6, 벽 12, 문·창 11, 실 6, 기둥 12, ㄱ자 계단, 치수 4, 단면 1, 대지, 시트 3 (목록·1층·2층).

| 경로 | 결과 |
|---|---|
| `.dsmith` 저장 → 열기 | 건축 객체가 비트까지 같다 (시험) |
| DXF 내보내기 → ezdxf 1.4.5 감사 (`recover`) | **오류 0** — 처음엔 블록 BLOCK·ENDBLK 핸들 0 으로 거부됨 → 고침 (§44) |
| DXF·DWG 다시 읽기 | 객체 수 같음, 치수는 치수 객체로, 블록은 블록으로 |
| 층별 내보내기 (`--level 1층`·`2층`) | 각 층 실 이름만 정확히 들어감 |
| IFC4 → IfcOpenShell 0.9 | **스키마 문제 0**, 형상 58 개 모두 만들어짐, 층 2·벽 12·문 5·창 6·구멍 11·실 6·바닥판 6·기둥 12, 한글 이름 정상 |
| 시트 PDF 3 장 | Pretendard 부분 삽입(CID), 한글 글자 추출 정상, 도면 목록 표 내용 맞음 |
| 면적표 | 층별 소계 102.08 ㎡, 외벽 중심선 바닥면적 108.00 ㎡, 연면적 216.00 ㎡ (12×9 m × 2 층과 맞음) |

## 3. 실제 AutoCAD 파일 (acadrust 시험 자료 21 개 — R12~2018 DXF·DWG, 동적 블록·주석·뷰포트 등)

| 항목 | 결과 |
|---|---|
| 읽기 | 21 개 모두 성공, 패닉 0 |
| 표시 데이터·PDF·DXF 내보내기 | 16 개 모두 성공. 나머지 5 개는 객체가 없는 파일이라 PDF 만 「출력할 것이 없습니다」 (정상) |
| `dsmith sample` DWG → DXF (블록 참조·치수 포함) | ezdxf 감사 오류 0, INSERT 5·DIMENSION 5 유지 |

## 4. 성능 (Linux, 릴리스 빌드, 객체 15 만)

| 항목 | 시간 |
|---|---|
| DWG 읽기 | 457 ms |
| DXF 읽기 | 819 ms |
| 표시 데이터 만들기 (선분 43 만) | 25 ms |
| PDF 저장 (2.6 MB) | 1.4 s |
| DWG 내보내기 | 1.5 s |

## 5. 앱 화면 (Linux Xvfb, WebKitGTK)

`.dsmith` 열기·층 전환·표 서랍·시트·3D·단면·작도·고르기·손잡이·뷰포트 편집을 실제 앱에서 눌러 확인 (§25~§43 의 화면 사진).

## 6. 이번 검증에서 찾아 고친 것

1. **DXF·DWG 의 블록 정의(BLOCK·ENDBLK) 핸들이 0** — ezdxf 가 파일을 거부. AutoCAD 도 거부할 수 있는 결함. 핸들을 준다.
2. **새 시트가 건물이 아닌 전체(대지·멀리 놓은 단면도 포함) 가운데를 봄** — 그 층 벽·실·기둥·그리드 범위로.
3. **DXF·DWG 내보내기는 지금 층만 펼침** — 층이 여럿이면 파일 이름에 층 이름을 붙이고(`도면-1층.dwg`), CLI 에 `--level`.
4. **건축 객체만 있는 도면에서 「객체 0 개」 로 보임** — `dsmith info`·앱 상태 줄에 건축 객체 수.

## 7. 알려진 한계 (이번 판)

검증 뒤 고침 (§45): 무늬 해치가 HATCH 로 오감, 모든 층을 한 DWG 에 나란히, 가져온 반지름·각도 치수도 치수 객체로,
단면도 가림 처리. 남은 것:
- 단면 가림은 벽 단위 근사 (벽 일부 높이만 가리는 경우·기둥·계단 입면은 아직).
- 치수 글꼴·DIMSTYLE 세부(화살표 크기·글자 위치)는 DraftSmith 식.

## 8. 실제 기기에서 남은 확인 (사용자)

- macOS·Windows 에서 실행 (서명 전 — macOS 는 `xattr -cr DraftSmith.app`, Windows 는 SmartScreen 「추가 정보 → 실행」).
- 한글 입력(실 이름·글자 도구), 파일 대화상자(저장·내보내기·PDF), Retina·고해상도 화면에서 선 굵기.
- 실무 DWG 로 열기·고치기·다시 저장 뒤 AutoCAD 에서 열기.
- 큰 도면 화면 속도(WebGL — Linux 개발 환경은 소프트웨어 렌더라 측정하지 않음).

## 다시 하기

```sh
cargo build --release -p draftsmith-cli
B=target/release/dsmith
$B demo /tmp/v/demo.dsmith && $B info /tmp/v/demo.dsmith
$B export /tmp/v/demo.dsmith /tmp/v/demo-1층.dxf --level 1층
$B ifc /tmp/v/demo.dsmith /tmp/v/demo.ifc
$B pdf /tmp/v/demo.dsmith /tmp/v/A-101.pdf --layout "A-101 1층 평면도" --scale 1
# 검사: python -m pip install ezdxf ifcopenshell 뒤 ezdxf.recover.readfile·ifcopenshell.validate
```
