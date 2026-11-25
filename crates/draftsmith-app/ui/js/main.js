// DraftSmith 화면 진입점. Rust 명령은 window.__TAURI__.core.invoke 로 부른다 (BEATS 와 같음).
//
// 도면은 Rust 가 들고 있고 (편집 엔진), 화면은 상태(DocState)와 표시 데이터만 받는다. 편집은 app.apply(req)
// 하나로 `edit` 명령에 보내고, 돌아온 상태로 옆 칸·탭·되돌리기 단추를 다시 그린다 (구현 노트 §18·§25).
import { Viewer } from "./viewer.js";
import { Snapper } from "./snapper.js";
import { SelectTool } from "./select.js";
import { TOOLS } from "./tools.js";
import { COMMANDS, lookup, Palette } from "./commands.js";
import { renderLayers, renderLevels, renderViewports, TablesDrawer } from "./panels.js";
import { View3D } from "./view3d.js";

const invoke = window.__TAURI__.core.invoke;
const $ = (id) => document.getElementById(id);
const fmt = (n) => n.toLocaleString("ko-KR");
const ms = (v) => `${Math.round(v)} ms`;
const base = (p) => p.split(/[\\/]/).pop();

function status(text) {
  $("status").textContent = text;
  $("status").title = text;
}
function prompt(text) {
  $("prompt").textContent = text;
  $("prompt").title = text;
}

// ── 앱 상태 (도구들이 함께 씀) ─────────────────────────────────────────────────
const app = {
  invoke,
  doc: null,
  baseLayers: 0,
  textHeight: 250,
  offsetDist: 100,
  wallThickness: 200,
  wallKind: "외벽",
  doorWidth: 900,
  windowWidth: 1200,
  filletRadius: 0,
  dimArrow: false,
  dimPlain: false,
  chamfer: [500, 500],
  hatchPattern: "사선",
  hatchSpacing: 200,
  colW: 600,
  colD: 600,
  stairWidth: 1200,
  stairRisers: 18,
  report: status,
  prompt,
};

const viewer = new Viewer($("view"), $("text"), $("overlay"), (x, y) => {
  const lat = app.snap.latency();
  $("cursor").textContent =
    `X ${fmt(Math.round(x))}  Y ${fmt(Math.round(y))}` + (lat != null ? `  · 스냅 ${lat.toFixed(1)} ms` : "");
});
app.v = viewer;
app.snap = new Snapper(viewer, invoke);
app.select = new SelectTool(app, $("props"));
const tables = new TablesDrawer($("tables"), app);

/** Tauri 바이너리 응답을 ArrayBuffer 로. 플랫폼에 따라 ArrayBuffer·Uint8Array·숫자 배열(JSON 대체 경로)로 온다.
 *  숫자 배열이면 느린 경로라 상태 막대에 알린다 (구현 노트 §6). */
let slowPath = false;
function toBuffer(v) {
  if (v instanceof ArrayBuffer) return v;
  if (ArrayBuffer.isView(v)) return v.buffer.slice(v.byteOffset, v.byteOffset + v.byteLength);
  if (Array.isArray(v)) {
    slowPath = true;
    return Uint8Array.from(v).buffer;
  }
  throw new Error(`표시 데이터를 받지 못했습니다 (${typeof v})`);
}
const view3d = new View3D($("view3d"), invoke, toBuffer);

// ── 도구 ─────────────────────────────────────────────────────────────────────
let active = null;      // 지금 도구 (없으면 고르기)
let activeId = null;
let lastCmd = null;

/** 화면(Viewer)이 부르는 도구: 지금 도구에 넘기고, 고른 것·스냅 표시를 함께 그린다 */
const host = {
  get t() { return active || app.select; },
  get drag() {
    const t = this.t;
    return t.drag ? (a, b, c) => t.drag(a, b, c) : undefined;
  },
  dragEnd(a, b, c, s) { this.t.dragEnd?.(a, b, c, s); },
  move(m, e) {
    app.snap.ortho = orthoOn || !!e?.shiftKey;
    if (this.t.snaps) app.snap.update(m);
    else app.snap.snap = null;
    this.t.move?.(m, e);
    viewer.drawOverlay();
  },
  async click(m, e) {
    try {
      await this.t.click?.(m, e);
    } catch (err) {
      status(String(err));
    }
    viewer.drawOverlay();
  },
  rightClick() {
    if (active) active.enter();
    else repeatLast();
  },
  dblclick() { return false; },
  paint(ctx, ts) {
    // 시트에서는 뷰포트 테두리를 점선으로
    for (const v of app.doc?.sheet != null ? app.doc.viewports ?? [] : []) {
      const [ax, ay] = ts(v.center[0] - v.size[0] / 2, v.center[1] - v.size[1] / 2);
      const [bx, by] = ts(v.center[0] + v.size[0] / 2, v.center[1] + v.size[1] / 2);
      ctx.strokeStyle = "rgba(74,168,255,0.7)";
      ctx.lineWidth = 1;
      ctx.setLineDash([5, 4]);
      ctx.strokeRect(Math.min(ax, bx), Math.min(ay, by), Math.abs(bx - ax), Math.abs(by - ay));
      ctx.setLineDash([]);
    }
    app.select.paint(ctx, ts);
    if (active) active.paint(ctx, ts);
    if (this.t.snaps) app.snap.paint(ctx, ts);
  },
};
viewer.tool = host;

function startTool(id) {
  if (!app.doc) return status("먼저 도면을 열거나 새 도면을 만드세요");
  if (app.doc.sheet != null && id !== "measure") return status("배치에서는 그릴 수 없습니다 — 「모형」 탭에서 그리세요");
  endTool(true);
  const T = TOOLS[id];
  active = new T(app);
  activeId = id;
  lastCmd = id;
  markTool();
  active.start();
  viewer.drawOverlay();
}

function endTool(quiet = false) {
  active?.stop();
  active = null;
  activeId = null;
  app.snap.snap = null;
  markTool();
  prompt("명령:");
  if (!quiet) viewer.drawOverlay();
}
app.done = () => endTool();

function markTool() {
  for (const b of document.querySelectorAll("[data-cmd]")) {
    if (TOOLS[b.dataset.cmd]) b.classList.toggle("tool-on", b.dataset.cmd === activeId);
  }
}

function repeatLast() {
  if (lastCmd) run(lastCmd);
}

// ── 편집·상태 ─────────────────────────────────────────────────────────────────
/** 편집 하나 (Rust `edit`). 실패하면 알리고 null — raise 면 던진다 */
app.apply = async (req, raise = false) => {
  try {
    const st = await invoke("edit", { req });
    await setDoc(st, true);
    return st;
  } catch (e) {
    if (raise) throw e;
    status(String(e));
    return null;
  }
};

app.setLayerVisible = async (i, v) => {
  await invoke("set_layer_visible", { index: i, visible: v });
  await redraw(true);
  await app.select.update();
};

app.onSelect = () => {
  const n = app.select.count;
  if (n && !active) status(app.select.arch ? "건축 객체를 골랐습니다 — 옆 칸에서 고치거나 Delete 로 지움" : `고른 객체 ${fmt(n)} 개 · M 이동 · CO 복사 · Delete 지우기`);
};

/** 표시 데이터(선·채움, 문자)를 받아 그린다. 전달·올리기 시간을 돌려준다 */
async function redraw(keepView = false) {
  const t = performance.now();
  const [buf, textBuf] = (await Promise.all([invoke("display_data"), invoke("text_data")])).map(toBuffer);
  const transfer = performance.now() - t;
  const upload = viewer.load(buf, textBuf, { keepView });
  return { transfer, upload, mb: (buf.byteLength + textBuf.byteLength) / 1e6 };
}

/** 새 상태 반영 */
async function setDoc(st, keepView) {
  const sheetChanged = app.doc?.sheet !== st.sheet;
  app.doc = st;
  app.baseLayers = st.base_layers;
  renderLayers(app, st);
  renderLevels(app, st);
  renderViewports(app, st);
  renderSpaces(st);
  $("undo").disabled = !st.can_undo;
  $("redo").disabled = !st.can_redo;
  document.title = `${st.dirty ? "● " : ""}${st.title} — DraftSmith`;
  const t = await redraw(keepView && !sheetChanged);
  tables.refresh();
  if (view3d.open) view3d.show();
  return t;
}

/** 모형·배치 탭 (+ 시트 만들기) */
function renderSpaces(st) {
  const nav = $("spaces");
  nav.replaceChildren();
  const tabs = [["모형", null], ...st.layouts.map((n, i) => [n, i])];
  for (const [name, index] of tabs) {
    const b = document.createElement("button");
    b.textContent = name;
    b.classList.toggle("on", index === st.sheet);
    b.addEventListener("click", () => showSpace(index));
    nav.append(b);
  }
  const add = document.createElement("button");
  add.className = "add";
  add.textContent = "＋ 시트";
  add.title = "도면 시트 만들기 (SH) — 표제란·도면 틀·뷰포트";
  add.addEventListener("click", () => run("sheet"));
  nav.append(add);
}

async function showSpace(index) {
  try {
    endTool(true);
    app.select.clear();
    const st = await invoke("set_space", { index });
    // 배치는 종이 단위(mm)라 PDF 는 1:1 이 알맞다
    $("pdf-scale").value = index === null ? "" : "1";
    await setDoc(st, false);
  } catch (e) {
    status(`배치를 보여 주지 못했습니다: ${e}`);
  }
}

// ── 파일 ─────────────────────────────────────────────────────────────────────
async function loadRecent() {
  const list = await invoke("recent_files");
  const sel = $("recent");
  sel.replaceChildren(new Option("최근 파일", ""));
  for (const p of list) sel.append(new Option(base(p), p));
  sel.disabled = list.length === 0;
}

async function discardOk() {
  return app.doc ? invoke("confirm_discard").catch(() => true) : true;
}

async function open(path) {
  if (!(await discardOk())) return;
  status("읽는 중…");
  try {
    endTool(true);
    const st = await invoke("open_drawing", { path });
    app.doc = null;
    app.select.clear();
    $("pdf-scale").value = "";
    const t = await setDoc(st, false);
    status(
      `${base(path)} · 객체 ${fmt(st.entities)} 개` +
        (st.arch.walls || st.arch.rooms ? ` · 건축 객체 층 ${st.arch.levels.length} · 벽 ${st.arch.walls} · 실 ${st.arch.rooms}` : "") +
        (st.inserts ? ` (블록 참조 ${fmt(st.inserts)} 개)` : "") +
        (st.recovered ? " · 깨진 곳이 있어 복구해서 읽음" : "") +
        ` · 읽기 ${ms(st.read_ms)} · ` +
        `전달 ${ms(t.transfer)} (${t.mb.toFixed(1)} MB${slowPath ? ", 느린 경로" : ""}) · 올리기 ${ms(t.upload)}`,
    );
    loadRecent();
  } catch (e) {
    status(`열지 못했습니다: ${e}`);
  }
}

async function newDrawing() {
  if (!(await discardOk())) return;
  endTool(true);
  app.select.clear();
  const st = await invoke("new_drawing");
  app.doc = null;
  await setDoc(st, false);
  status("새 도면 — 단위 mm. 벽(WA)·선(L) 등으로 그리세요. ⌘K 로 명령 찾기");
}

async function pickAndOpen() {
  const path = await invoke("pick_drawing");
  if (path) await open(path);
}

async function save(saveAs = false) {
  if (!app.doc) return;
  try {
    const p = await invoke("save_drawing", { saveAs });
    if (!p) return status("저장을 취소했습니다");
    const st = await invoke("doc_state");
    app.doc = st;
    document.title = `${st.title} — DraftSmith`;
    status(`저장: ${p}`);
    loadRecent();
  } catch (e) {
    status(`저장하지 못했습니다: ${e}`);
  }
}

async function exportCad(format) {
  if (!app.doc) return;
  try {
    const p = await invoke(format === "ifc" ? "export_ifc" : "export_cad", { format });
    status(p ? `${format.replace("-all", "").toUpperCase()} 내보내기: ${p}` : "내보내기를 취소했습니다");
  } catch (e) {
    status(`내보내지 못했습니다: ${e}`);
  }
}

// ── 묻기 창 (레이어·층·시트) ───────────────────────────────────────────────────
/** fields: [[key, 이름, 기본값, 고를 값들?]] → 값 객체 또는 null */
function ask(title, fields) {
  return new Promise((resolve) => {
    const root = $("ask"), form = root.querySelector("form");
    root.querySelector("h4").textContent = title;
    const box = root.querySelector(".ask-fields");
    box.replaceChildren();
    const inputs = {};
    for (const [key, label, value, choices] of fields) {
      const l = document.createElement("label");
      l.append(label);
      let i;
      if (choices) {
        i = document.createElement("select");
        for (const c of choices) i.append(new Option(c, c, false, c === value));
      } else {
        i = document.createElement("input");
        i.value = value;
      }
      i.addEventListener("keydown", (e) => e.stopPropagation());
      inputs[key] = i;
      l.append(i);
      box.append(l);
    }
    const done = (v) => {
      root.hidden = true;
      form.onsubmit = null;
      root.querySelector(".ask-cancel").onclick = null;
      $("cmd").focus();
      resolve(v);
    };
    form.onsubmit = (e) => {
      e.preventDefault();
      done(Object.fromEntries(Object.entries(inputs).map(([k, i]) => [k, i.value])));
    };
    root.querySelector(".ask-cancel").onclick = () => done(null);
    root.hidden = false;
    box.querySelector("input,select")?.focus();
    box.querySelector("input")?.select();
  });
}

/** 치수 스타일 (§36): 새 치수의 끝 모양·쉼표·글자 높이 — 설정 파일에 기억 */
async function dimStyle() {
  const v = await ask("치수 스타일 (새 치수에)", [
    ["arrow", "끝 모양", app.dimArrow ? "화살표" : "사선", ["사선", "화살표"]],
    ["plain", "쉼표", app.dimPlain ? "아니오" : "예", ["예", "아니오"]],
    ["h", "글자 높이", String(app.textHeight)],
    ["prec", "소수 자리", String(app.dimPrec ?? 0), ["0", "1", "2", "3"]],
  ]);
  if (!v) return;
  app.dimArrow = v.arrow === "화살표";
  app.dimPlain = v.plain === "아니오";
  if (Number(v.h) > 0) app.textHeight = Number(v.h);
  app.dimPrec = Number(v.prec) || 0;
  invoke("set_setting", { key: "dimstyle", value: JSON.stringify([app.dimArrow, app.dimPlain, app.textHeight, app.dimPrec]) }).catch(() => {});
  status(`치수 스타일: ${v.arrow} · 쉼표 ${v.plain} · 글자 ${app.textHeight}`);
}
invoke("get_setting", { key: "dimstyle" }).then((v) => {
  try {
    const [a, p, h, d] = JSON.parse(v);
    app.dimArrow = !!a;
    app.dimPlain = !!p;
    if (h > 0) app.textHeight = h;
    app.dimPrec = Number(d) || 0;
  } catch {
    /* 처음 */
  }
}, () => {});

async function addLayer() {
  const v = await ask("레이어 더하기", [["name", "이름", ""], ["color", "색", "#ffffff"]]);
  if (!v || !v.name.trim()) return;
  const c = /^#[0-9a-f]{6}$/i.test(v.color) ? [1, 3, 5].map((i) => parseInt(v.color.slice(i, i + 2), 16)) : [255, 255, 255];
  await app.apply({ kind: "add_layer", name: v.name.trim(), color: c });
}

async function addLevel() {
  const a = app.doc?.arch;
  if (!a) return;
  const n = a.levels.length;
  const v = await ask("층 더하기", [["name", "이름", `${n + 1}층`], ["elevation", "바닥 높이 (mm)", String(n * 3300)], ["height", "층고 (mm)", "3300"]]);
  if (!v) return;
  await app.apply({ kind: "arch", op: { kind: "level", name: v.name, elevation: Number(v.elevation) || 0, height: Number(v.height) || 3300 } });
}

async function copyLevel() {
  const a = app.doc?.arch;
  if (!a) return;
  const v = await ask("지금 층 복사 (벽·문·창·실)", [["name", "새 층 이름", `${a.levels.length + 1}층`]]);
  if (v) await app.apply({ kind: "arch", op: { kind: "copy_level", name: v.name } });
}

async function addSheet(listSheet = false) {
  if (!app.doc) return;
  // 지금 층을 먼저
  const act = app.doc.arch.levels.find(([id]) => id === app.doc.arch.active_level)?.[1];
  const levels = [...(act ? [act] : []), ...app.doc.arch.levels.map(([, n]) => n).filter((n) => n !== act)];
  const v = await ask("도면 시트 만들기", [
    ["kind", "종류", listSheet ? "도면 목록" : "평면 시트", ["평면 시트", "도면 목록"]],
    ["title", "도면명", listSheet ? "도면 목록" : levels.length ? `${levels[0]} 평면도` : "평면도"],
    ["number", "도면 번호", listSheet ? "A-000" : `A-${String(app.doc.layouts.length + 101)}`],
    ["paper", "용지", "A3", ["A3", "A2", "A1"]],
    ["scale", "축척 1:", "100"],
    ["level", "층 (건축 객체)", levels[0] ?? "", ["", ...levels]],
    ["project", "프로젝트", app.lastProject || ""],
  ]);
  if (!v) return;
  app.lastProject = v.project;
  const list = v.kind === "도면 목록";
  if (list && v.title.endsWith("평면도")) v.title = "도면 목록";
  const level = app.doc.arch.levels.find(([, n]) => n === v.level)?.[0] ?? null;
  const st = await app.apply({
    kind: "add_sheet",
    title: v.title,
    number: v.number,
    paper: v.paper,
    scale: Number(v.scale) || 100,
    level,
    project: v.project,
    list,
  });
  if (st) await showSpace(st.layouts.length - 1);
}

// ── 공동작업 (M5): 변경 기록 ───────────────────────────────────────────────────
async function saveJournal() {
  try {
    const p = await invoke("save_journal");
    status(p ? `변경 기록 저장: ${p} — 다른 사람이 「합치기」로 받습니다` : "취소했습니다");
  } catch (e) {
    status(`변경 기록을 저장하지 못했습니다: ${e}`);
  }
}
async function mergeJournal() {
  try {
    const r = await invoke("merge_journal");
    if (!r) return status("취소했습니다");
    await setDoc(r.state, true);
    status(`변경 기록 합침: 작업 ${fmt(r.applied)} 개` + (r.skipped ? ` · 맞지 않아 건너뜀 ${fmt(r.skipped)} 개` : ""));
  } catch (e) {
    status(`합치지 못했습니다: ${e}`);
  }
}

// ── 명령 실행 ─────────────────────────────────────────────────────────────────
let orthoOn = false;
function setToggle(id, on) {
  $(id)?.classList.toggle("on", on);
}

async function run(id) {
  if (TOOLS[id]) return startTool(id);
  switch (id) {
    case "undo":
    case "redo":
      endTool(true);
      await app.apply({ kind: id });
      await app.select.update();
      return;
    case "fit": return viewer.fit();
    case "paper": return setPaper(!viewer.paper);
    case "lwt": return setLineweights(!viewer.lineweights);
    case "snap":
      app.snap.on = !app.snap.on;
      setToggle("snap", app.snap.on);
      return status(`객체 스냅 ${app.snap.on ? "켬" : "끔"}`);
    case "ortho":
      orthoOn = !orthoOn;
      setToggle("ortho", orthoOn);
      return status(`직교 ${orthoOn ? "켬" : "끔"}`);
    case "new": return newDrawing();
    case "open": return pickAndOpen();
    case "save": return save(false);
    case "saveas": return save(true);
    case "dxf":
    case "dwg":
    case "ifc": return exportCad(id);
    case "pdf": return togglePdf(true);
    case "sheetspdf":
      $("pdf-what").value = "sheets";
      $("pdf-scale").value = "1";
      return togglePdf(true);
    case "tables": return app.doc && (tables.open ? tables.hide() : tables.show());
    case "view3d": return app.doc && view3d.show();
    case "layer": return app.doc && addLayer();
    case "level": return app.doc && addLevel();
    case "copylevel": return app.doc && copyLevel();
    case "sheet": return addSheet();
    case "dimstyle": return dimStyle();
    case "sheetlist": return addSheet(true);
    case "journal": return app.doc && saveJournal();
    case "merge": return app.doc && mergeJournal();
    case "palette": return palette.show();
  }
}

const palette = new Palette($("palette"), run);

// ── 명령줄 ───────────────────────────────────────────────────────────────────
const cmd = $("cmd");
async function submit(text) {
  const t = text.trim();
  if (!t) {
    if (active) active.enter();
    else repeatLast();
    return;
  }
  // 손잡이를 옮기는 중이면 좌표로 놓기
  if (!active && app.select.grip && app.select.gripInput(t)) return;
  if (active) {
    try {
      if (await active.input(text)) return;
    } catch (e) {
      return status(String(e));
    }
  }
  const id = lookup(t);
  if (id) return run(id);
  status(active ? `알아듣지 못했습니다: 「${t}」 — 좌표(x,y·@dx,dy·@거리<각)나 숫자를 치세요` : `없는 명령: 「${t}」 — ⌘K 로 찾아보세요`);
}

cmd.addEventListener("keydown", (e) => {
  if (e.isComposing) return;
  if (e.key === "Enter") {
    e.preventDefault();
    const v = cmd.value;
    cmd.value = "";
    submit(v);
  } else if (e.code === "Space" && !cmd.value) {
    // Space: 끌면 화면 이동, 그냥 떼면 Enter (AutoCAD 처럼)
    e.preventDefault();
  } else if (e.code === "Space" && !active?.freeText && !/[가-힣]/.test(cmd.value)) {
    // 글을 받는 단계(글자·실 이름·그리드 기호)가 아니면 Space 도 Enter (AutoCAD 처럼)
    e.preventDefault();
    const v = cmd.value;
    cmd.value = "";
    submit(v);
  }
});
cmd.addEventListener("keyup", (e) => {
  if (e.code === "Space" && !cmd.value && !viewer.spacePanned) submit("");
});
// 도면을 누른 뒤에도 바로 칠 수 있게 명령줄로
$("view").addEventListener("pointerup", () => setTimeout(() => {
  if (!palette.open && $("ask").hidden && !(document.activeElement instanceof HTMLInputElement && document.activeElement !== cmd)) cmd.focus();
}, 0));

// ── 단추 ─────────────────────────────────────────────────────────────────────
for (const b of document.querySelectorAll("[data-cmd]")) {
  b.addEventListener("click", () => {
    run(b.dataset.cmd);
    cmd.focus();
  });
}
$("recent").addEventListener("change", (e) => {
  const p = e.target.value;
  e.target.value = "";
  if (p) open(p);
});
$("export").addEventListener("change", (e) => {
  const f = e.target.value;
  e.target.value = "";
  if (f) exportCad(f);
});
$("layer-add").addEventListener("click", () => run("layer"));
$("level-add").addEventListener("click", () => run("level"));
$("level-copy").addEventListener("click", () => run("copylevel"));

/** 흰 배경 켜기·끄기 (앱 설정 폴더에 기억 — 다음에 열 때도) */
function setPaper(on, save = true) {
  viewer.paper = on;
  setToggle("paper", on);
  if (save) invoke("set_setting", { key: "paper", value: on ? "1" : "0" }).catch(() => {});
  viewer.draw();
}
invoke("get_setting", { key: "paper" }).then((v) => setPaper(v === "1", false), () => {});

/** 선 굵기 보기 켜기·끄기 (앱 설정 폴더에 기억) */
function setLineweights(on, save = true) {
  viewer.lineweights = on;
  setToggle("lwt", on);
  if (save) invoke("set_setting", { key: "lwt", value: on ? "1" : "0" }).catch(() => {});
  viewer.draw();
}
invoke("get_setting", { key: "lwt" }).then((v) => setLineweights(v === "1", false), () => {});

// ── PDF ──────────────────────────────────────────────────────────────────────
function togglePdf(show = $("pdf-pop").hidden) {
  if (show && !app.doc) return;
  $("pdf-pop").hidden = !show;
}
$("pdf").addEventListener("click", () => togglePdf());

// 출력 스타일(CTB): 고른 파일을 목록에 넣고 기억한다
function setCtb(path, save = true) {
  const sel = $("pdf-ctb");
  for (const o of [...sel.options]) if (o.dataset.file) o.remove();
  if (path) {
    const o = new Option(base(path), path);
    o.dataset.file = "1";
    o.title = path;
    sel.insertBefore(o, sel.options[1]);
  }
  sel.value = path || "";
  if (save) invoke("set_setting", { key: "ctb", value: path || "" }).catch(() => {});
}
$("pdf-ctb").addEventListener("change", async (e) => {
  if (e.target.value !== "pick") {
    invoke("set_setting", { key: "ctb", value: e.target.value }).catch(() => {});
    return;
  }
  const path = await invoke("pick_ctb");
  const kept = [...e.target.options].find((o) => o.dataset.file)?.value || "";
  setCtb(path || kept);
});
invoke("get_setting", { key: "ctb" }).then((v) => setCtb(v || "", false), () => {});
$("pdf-cancel").addEventListener("click", () => togglePdf(false));
$("pdf-save").addEventListener("click", async () => {
  const scale = $("pdf-scale").value;
  const sheets = $("pdf-what").value === "sheets";
  const req = {
    paper: $("pdf-paper").value,
    landscape: $("pdf-orient").value === "land",
    scale: scale ? Number(scale) : null,
    area: !sheets && $("pdf-area").value === "view" ? viewer.viewBox() : null,
    mono: $("pdf-color").value === "mono",
    ctb: $("pdf-ctb").value || null,
  };
  togglePdf(false);
  $("pdf-what").value = "one";
  status(sheets ? "도면 세트 PDF 만드는 중…" : "PDF 만드는 중…");
  try {
    const path = await invoke(sheets ? "export_sheets_pdf" : "export_pdf", { req });
    status(path ? `PDF 저장: ${path}` : "PDF 저장을 취소했습니다");
  } catch (e) {
    status(`PDF 를 저장하지 못했습니다: ${e}`);
  }
});

// ── 단축키 ───────────────────────────────────────────────────────────────────
window.addEventListener("keydown", (e) => {
  if (e.isComposing) return;
  const mod = e.metaKey || e.ctrlKey;
  // 한글 입력기가 켜져 있어도 같은 자판 위치로 동작하게 글자(e.key)가 아니라 자판 위치(e.code)로 본다
  if (mod) {
    const map = { KeyK: "palette", KeyO: "open", KeyN: "new", KeyP: "pdf", KeyY: "redo" };
    if (e.code === "KeyZ") {
      e.preventDefault();
      return run(e.shiftKey ? "redo" : "undo");
    }
    if (e.code === "KeyS") {
      e.preventDefault();
      return save(e.shiftKey);
    }
    if (map[e.code]) {
      e.preventDefault();
      return run(map[e.code]);
    }
    return;
  }
  if (e.key === "F3") { e.preventDefault(); return run("snap"); }
  if (e.key === "F8") { e.preventDefault(); return run("ortho"); }
  if (e.key === "Escape") {
    if (palette.open) return palette.hide();
    if (view3d.open) return view3d.hide();
    if (!$("ask").hidden) return;
    togglePdf(false);
    cmd.value = "";
    if (app.select.cancelGrip()) return;
    if (active) endTool();
    else app.select.clear();
    return;
  }
  const typing = e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement;
  if ((e.key === "Delete" || e.key === "Backspace") && (!typing || (e.target === cmd && !cmd.value)) && !active && app.select.count) {
    e.preventDefault();
    return run("erase");
  }
  // 도면에 초점이 있을 때 친 글자는 명령줄로
  if (!typing && e.key.length === 1 && !e.altKey) cmd.focus();
});

// 파일을 창에 끌어 놓으면 연다
window.__TAURI__.event?.listen?.("tauri://drag-drop", (ev) => {
  const p = ev.payload?.paths?.find((x) => /\.(dwg|dxf|dsmith)$/i.test(x));
  if (p) open(p);
});

invoke("app_version").then((v) => ($("version").textContent = v));
loadRecent();
cmd.focus();
// 실행할 때 넘긴 도면이 있으면 열고, 없으면 새 도면
invoke("startup_file").then((p) => (p ? open(p) : newDrawing()));

// 명령 표를 개발 도구에서 볼 수 있게
window.draftsmith = { app, run, COMMANDS };
