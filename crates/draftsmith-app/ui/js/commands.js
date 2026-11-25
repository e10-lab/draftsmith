// 명령 표·명령줄·명령 팔레트 (구현 노트 §25).
//
// 명령은 AutoCAD 단축 이름(L, PL, M …)과 한글 이름(선, 이동 …) 둘 다로 부른다. ⌘K / Ctrl+K 팔레트는
// 이름·단축·초성(ㅅ → 선, ㄱㄸ → 간격띄우기)으로 찾는다.

/** [id, 보이는 이름, 단축·별명들, 묶음] */
export const COMMANDS = [
  ["line", "선", ["L", "LINE", "선"], "그리기"],
  ["pline", "폴리선", ["PL", "PLINE", "폴리선"], "그리기"],
  ["rect", "사각형", ["REC", "RECTANG", "사각형", "네모"], "그리기"],
  ["circle", "원", ["C", "CIRCLE", "원"], "그리기"],
  ["arc", "호 (세 점)", ["A", "ARC", "호"], "그리기"],
  ["text", "글자", ["T", "DT", "TEXT", "글자", "문자"], "그리기"],
  ["dimrad", "반지름 치수", ["DRA", "DIMRAD", "반지름치수"], "그리기"],
  ["dimdia", "지름 치수", ["DDI", "DIMDIA", "지름치수"], "그리기"],
  ["dimang", "각도 치수", ["DAN", "DIMANG", "각도치수"], "그리기"],
  ["dimstyle", "치수 스타일", ["DST", "DIMSTY", "DIMSTYLE", "치수스타일"], "그리기"],
  ["dim", "치수 (정렬)", ["DIM", "DLI", "DAL", "치수"], "그리기"],
  ["move", "이동", ["M", "MOVE", "이동"], "수정"],
  ["copy", "복사", ["CO", "CP", "COPY", "복사"], "수정"],
  ["rotate", "회전", ["RO", "ROTATE", "회전"], "수정"],
  ["mirror", "대칭", ["MI", "MIRROR", "대칭"], "수정"],
  ["scale", "크기 (배율)", ["SC", "SCALE", "크기", "배율"], "수정"],
  ["offset", "간격 띄우기", ["O", "OFFSET", "간격띄우기", "오프셋"], "수정"],
  ["trim", "자르기", ["TR", "TRIM", "자르기"], "수정"],
  ["extend", "연장", ["EX", "EXTEND", "연장"], "수정"],
  ["fillet", "모깎기", ["F", "FILLET", "모깎기"], "수정"],
  ["chamfer", "모따기", ["CHA", "CHAMFER", "모따기"], "수정"],
  ["hatch", "해치", ["H", "BH", "HATCH", "해치"], "그리기"],
  ["block", "블록 만들기", ["B", "BLOCK", "블록"], "수정"],
  ["insert", "블록 넣기", ["I", "INSERT", "넣기", "블록넣기"], "그리기"],
  ["explode", "펼치기 (블록 풀기)", ["X", "EXPLODE", "펼치기", "분해"], "수정"],
  ["erase", "지우기", ["E", "ERASE", "DEL", "지우기", "삭제"], "수정"],
  ["undo", "되돌리기", ["U", "UNDO", "되돌리기", "취소"], "수정"],
  ["redo", "다시 하기", ["REDO", "다시하기"], "수정"],
  ["wall", "벽", ["WA", "WALL", "벽"], "건축"],
  ["door", "문", ["DO", "DOOR", "문"], "건축"],
  ["window", "창", ["WI", "WIN", "WINDOW", "창"], "건축"],
  ["room", "실 (면적)", ["RM", "ROOM", "실", "면적"], "건축"],
  ["grid", "그리드", ["GR", "GRID", "그리드", "통심"], "건축"],
  ["gridset", "그리드 세트", ["GRS", "GRIDSET", "그리드세트"], "건축"],
  ["column", "기둥", ["COL", "COLUMN", "기둥"], "건축"],
  ["beam", "보", ["BM", "BEAM", "보"], "건축"],
  ["stair", "계단", ["ST", "STAIR", "계단"], "건축"],
  ["site", "대지 경계", ["SITE", "대지"], "건축"],
  ["level", "층 더하기", ["LV", "LEVEL", "층추가", "층"], "건축"],
  ["copylevel", "층 복사", ["CL", "COPYLEVEL", "층복사"], "건축"],
  ["tables", "면적표·일람표", ["TB", "TABLES", "표", "면적표", "일람표"], "건축"],
  ["section", "단면 만들기", ["SEC", "SECTION", "단면", "단면도"], "건축"],
  ["view3d", "3D 보기", ["3D", "3DV", "VIEW3D", "입체"], "건축"],
  ["sheet", "도면 시트 만들기", ["SH", "SHEET", "시트", "도면시트"], "도면화"],
  ["sheetlist", "도면 목록 시트 만들기", ["SHL", "SHEETLIST", "목록시트", "도면목록시트"], "도면화"],
  ["sheetspdf", "도면 세트 PDF", ["SPDF", "PUBLISH", "일괄출력"], "도면화"],
  ["measure", "거리 재기", ["DI", "DIST", "거리", "측정"], "보기"],
  ["fit", "전체 보기", ["Z", "ZE", "FIT", "전체보기"], "보기"],
  ["paper", "흰 배경", ["BG", "PAPER", "흰배경"], "보기"],
  ["lwt", "선 굵기 보기", ["LWT", "LW", "선굵기"], "보기"],
  ["snap", "객체 스냅 (F3)", ["OS", "OSNAP", "스냅"], "보기"],
  ["ortho", "직교 (F8)", ["OR", "ORTHO", "직교"], "보기"],
  ["layer", "레이어 더하기", ["LA", "LAYER", "레이어"], "보기"],
  ["new", "새 도면", ["NEW", "새도면"], "파일"],
  ["open", "열기", ["OPEN", "열기"], "파일"],
  ["save", "저장", ["SAVE", "QSAVE", "저장"], "파일"],
  ["saveas", "다른 이름으로 저장", ["SAVEAS", "다른이름"], "파일"],
  ["dxf", "DXF 로 내보내기", ["DXFOUT", "DXF"], "파일"],
  ["dwg", "DWG 로 내보내기", ["DWGOUT", "DWG"], "파일"],
  ["ifc", "IFC 로 내보내기 (BIM)", ["IFC", "IFCOUT"], "파일"],
  ["pdf", "PDF 저장", ["PDF", "PLOT", "출력"], "파일"],
  ["journal", "변경 기록 저장", ["JOURNAL", "변경기록"], "협업"],
  ["merge", "변경 기록 합치기", ["MERGE", "합치기"], "협업"],
];

const CHO = "ㄱㄲㄴㄷㄸㄹㅁㅂㅃㅅㅆㅇㅈㅉㅊㅋㅌㅍㅎ";
/** 한글 → 초성 (ㄱ … ㅎ), 한글 아닌 글자는 그대로 */
export function chosung(s) {
  let out = "";
  for (const ch of s) {
    const c = ch.charCodeAt(0) - 0xac00;
    out += c >= 0 && c < 11172 ? CHO[Math.floor(c / 588)] : ch;
  }
  return out;
}

const norm = (s) => s.replace(/\s+/g, "").toUpperCase();

/** 명령줄에 친 낱말 → 명령 id */
export function lookup(word) {
  const w = norm(word);
  if (!w) return null;
  for (const [id, , names] of COMMANDS) if (names.some((n) => norm(n) === w)) return id;
  return null;
}

/** 팔레트 찾기: 이름·단축·초성 부분 일치 */
export function search(q) {
  const w = norm(q);
  if (!w) return COMMANDS;
  const onlyCho = [...w].every((c) => CHO.includes(c));
  const score = ([, title, names]) => {
    const keys = [title, ...names].map(norm);
    if (keys.some((k) => k === w)) return 0;
    if (keys.some((k) => k.startsWith(w))) return 1;
    if (keys.some((k) => k.includes(w))) return 2;
    if (onlyCho && keys.some((k) => chosung(k).includes(w))) return 3;
    return 9;
  };
  return COMMANDS.map((c) => [score(c), c]).filter(([s]) => s < 9).sort((a, b) => a[0] - b[0]).map(([, c]) => c);
}

/** ⌘K 팔레트 */
export class Palette {
  constructor(root, run) {
    this.root = root;
    this.run = run;
    this.input = root.querySelector("input");
    this.list = root.querySelector("ul");
    this.at = 0;
    this.items = [];
    this.input.addEventListener("input", () => this.fill());
    this.input.addEventListener("keydown", (e) => {
      e.stopPropagation();
      if (e.isComposing) return;
      if (e.key === "ArrowDown") { this.at = Math.min(this.at + 1, this.items.length - 1); this.mark(); e.preventDefault(); }
      else if (e.key === "ArrowUp") { this.at = Math.max(this.at - 1, 0); this.mark(); e.preventDefault(); }
      else if (e.key === "Enter") { const c = this.items[this.at]; this.hide(); if (c) this.run(c[0]); }
      else if (e.key === "Escape") this.hide();
    });
    root.addEventListener("pointerdown", (e) => { if (e.target === root) this.hide(); });
  }
  get open() { return !this.root.hidden; }
  show() {
    this.root.hidden = false;
    this.input.value = "";
    this.fill();
    this.input.focus();
  }
  hide() {
    this.root.hidden = true;
    document.getElementById("cmd")?.focus();
  }
  fill() {
    this.items = search(this.input.value).slice(0, 40);
    this.at = 0;
    this.list.replaceChildren(
      ...this.items.map(([id, title, names, group], i) => {
        const li = document.createElement("li");
        const t = document.createElement("span");
        t.textContent = title;
        const k = document.createElement("span");
        k.className = "muted small";
        k.textContent = `${group} · ${names[0]}`;
        li.append(t, k);
        li.addEventListener("pointerenter", () => { this.at = i; this.mark(); });
        li.addEventListener("click", () => { this.hide(); this.run(id); });
        return li;
      }),
    );
    this.mark();
  }
  mark() {
    [...this.list.children].forEach((li, i) => li.classList.toggle("on", i === this.at));
    this.list.children[this.at]?.scrollIntoView({ block: "nearest" });
  }
}
