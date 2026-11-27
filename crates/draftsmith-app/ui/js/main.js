// DraftSmith 화면 진입점. Rust 명령은 window.__TAURI__.core.invoke 로 부른다 (BEATS 와 같음).
//
// 도면은 Rust 가 들고 있고 (편집 엔진), 화면은 상태(DocState)와 표시 데이터만 받는다. 편집은 app.apply(req)
// 하나로 `edit` 명령에 보내고, 돌아온 상태로 옆 칸·탭·되돌리기 단추를 다시 그린다 (구현 노트 §18·§25).
import { tok } from "./tokens.js";
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
  ask: (title, fields) => ask(title, fields),
  status: (t) => status(t),
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
  beamW: 300,
  beamD: 600,
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
let binaryProbe = null;
/** 바이너리 응답이 ArrayBuffer 로 오는가 (앱을 띄운 뒤 한 번 시험, §70) */
function binaryOk() {
  binaryProbe ??= invoke("probe_binary").then((r) => r instanceof ArrayBuffer || ArrayBuffer.isView(r), () => false);
  return binaryProbe;
}
function fromB64(s) {
  const bin = atob(s);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out.buffer;
}
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
let vpDrag = null;      // 끄는 중인 뷰포트 (미리보기)

/** 시트 뷰포트 맞추기: 모서리(크기) > 테두리(옮기기) > Alt 면 안쪽(보는 곳). 위 뷰포트(뒤 번호)부터 */
function vpHit(m, alt) {
  const tol = 8 / viewer.ppu;
  const vps = [...(app.doc?.viewports ?? [])].reverse();
  for (const v of vps) {
    const hw = v.size[0] / 2, hh = v.size[1] / 2;
    const dx = m[0] - v.center[0], dy = m[1] - v.center[1];
    for (const sx of [-1, 1]) for (const sy of [-1, 1]) {
      if (Math.hypot(dx - sx * hw, dy - sy * hh) < tol * 1.5) return { v, kind: "size", corner: [sx, sy] };
    }
  }
  for (const v of vps) {
    const hw = v.size[0] / 2, hh = v.size[1] / 2;
    const ax = Math.abs(m[0] - v.center[0]), ay = Math.abs(m[1] - v.center[1]);
    const inside = ax <= hw + tol && ay <= hh + tol;
    if (inside && (Math.abs(ax - hw) < tol || Math.abs(ay - hh) < tol)) return { v, kind: "move" };
    if (alt && ax < hw && ay < hh) return { v, kind: "view" };
  }
  return null;
}
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
  /** 시트에서 도구 없이 뷰포트를 누르면: 모서리 → 크기, 테두리 → 옮기기, Alt+안쪽 → 보는 곳 옮기기 (구현 노트 §51) */
  grab(m, e) {
    if (active || app.doc?.sheet == null) return null;
    const hit = vpHit(m, e?.altKey);
    if (!hit) return null;
    const { v, kind, corner } = hit;
    const r1 = (x) => Math.round(x);
    const at = { center: [...v.center], size: [...v.size], view_center: [...v.view_center] };
    vpDrag = { index: v.index, ...at };
    return {
      move(p) {
        const dx = p[0] - m[0], dy = p[1] - m[1];
        if (kind === "move") {
          vpDrag.center = [r1(at.center[0] + dx), r1(at.center[1] + dy)];
        } else if (kind === "view") {
          vpDrag.view_center = [at.view_center[0] - dx * v.scale, at.view_center[1] - dy * v.scale];
        } else {
          // 맞은편 모서리는 그대로, 모형 내용도 종이 위에 그대로 (보는 가운데를 함께 옮김)
          const [sx, sy] = corner;
          const fx = at.center[0] - sx * at.size[0] / 2, fy = at.center[1] - sy * at.size[1] / 2;
          const gx = r1(at.center[0] + sx * at.size[0] / 2 + dx), gy = r1(at.center[1] + sy * at.size[1] / 2 + dy);
          const w = Math.max(10, Math.abs(gx - fx)), h = Math.max(10, Math.abs(gy - fy));
          const cx = fx + Math.sign(gx - fx || sx) * w / 2, cy = fy + Math.sign(gy - fy || sy) * h / 2;
          vpDrag.size = [w, h];
          vpDrag.center = [cx, cy];
          vpDrag.view_center = [at.view_center[0] + (cx - at.center[0]) * v.scale, at.view_center[1] + (cy - at.center[1]) * v.scale];
        }
      },
      async end() {
        const d = vpDrag;
        vpDrag = null;
        const edit = kind === "move" ? { center: d.center } : kind === "view" ? { view_center: d.view_center } : { size: d.size, center: d.center, view_center: d.view_center };
        await app.apply({ kind: "viewport", index: v.index, ...edit });
      },
      cancel() { vpDrag = null; },
    };
  },
  move(m, e) {
    // 시트: 뷰포트 테두리 위면 손 모양으로 알려 준다
    if (!active && app.doc?.sheet != null) {
      const hit = vpHit(m, e?.altKey);
      viewer.cv.style.cursor = !hit ? "" : hit.kind === "move" ? "move" : hit.kind === "view" ? "grab" : hit.corner[0] * hit.corner[1] > 0 ? "nesw-resize" : "nwse-resize";
    }
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
  /** 두 번 누르기 (§62): 글자면 내용 고치기, 실이면 이름 고치기. 아니면 화면 맞춤 */
  dblclick(m) {
    if (active || !app.doc || app.doc.sheet != null) return false;
    editAt(m);
    return true;
  },
  paint(ctx, ts) {
    // 시트에서는 뷰포트 테두리를 점선으로
    for (const v0 of app.doc?.sheet != null ? app.doc.viewports ?? [] : []) {
      const v = vpDrag?.index === v0.index ? vpDrag : v0;
      const [ax, ay] = ts(v.center[0] - v.size[0] / 2, v.center[1] - v.size[1] / 2);
      const [bx, by] = ts(v.center[0] + v.size[0] / 2, v.center[1] + v.size[1] / 2);
      ctx.strokeStyle = v === vpDrag ? tok("--ov-viewport-on") : tok("--ov-viewport");
      ctx.lineWidth = v === vpDrag ? 2 : 1;
      ctx.setLineDash([5, 4]);
      ctx.strokeRect(Math.min(ax, bx), Math.min(ay, by), Math.abs(bx - ax), Math.abs(by - ay));
      ctx.setLineDash([]);
      // 모서리 손잡이
      ctx.fillStyle = tok("--ov-viewport-grip");
      for (const [x, y] of [[ax, ay], [bx, ay], [bx, by], [ax, by]]) ctx.fillRect(x - 3, y - 3, 6, 6);
      if (v === vpDrag) {
        ctx.fillStyle = tok("--ov-viewport-on");
        ctx.font = "12px sans-serif";
        ctx.fillText(`${Math.round(v.size[0])} × ${Math.round(v.size[1])} mm`, Math.min(ax, bx) + 4, Math.min(ay, by) - 6);
      }
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
  // 편집 뒤(보던 자리 그대로)에는 바뀐 객체만 부분 갱신될 수 있다 (§68)
  const update = keepView && viewer.hasData;
  let buf, textBuf;
  if (await binaryOk()) {
    // 차례로: 표시 데이터가 캐시를 맞춘 뒤 문자 데이터가 그 원점을 쓴다 (도면 범위를 다시 재지 않게, §74)
    buf = toBuffer(await invoke(update ? "display_update" : "display_data"));
    textBuf = toBuffer(await invoke("text_data"));
  } else {
    // 바이너리가 숫자 배열로 오는 환경: base64 글로 받아 푼다 (숫자 배열보다 수십 배 빠름, §70)
    const [a, b] = await invoke("display_b64", { update });
    [buf, textBuf] = [fromB64(a), fromB64(b)];
    slowPath = true;
  }
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
        `전달 ${ms(t.transfer)} (${t.mb.toFixed(1)} MB${slowPath ? ", base64" : ""}) · 올리기 ${ms(t.upload)}`,
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
      if (choices === "textarea") {
        i = document.createElement("textarea");
        i.value = value;
        i.rows = Math.min(8, value.split("\n").length + 1);
      } else if (choices) {
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

// 같은 파일에서 갈라진 다른 사본(.dsmith)의 편집을 변경 이력으로 합친다 (§86)
async function mergeCopy() {
  try {
    const r = await invoke("merge_copy");
    if (!r) return status("취소했습니다");
    await setDoc(r.state, true);
    status(`사본 합침: 객체 ${fmt(r.before)} → ${fmt(r.after)} 개 (되돌리기 가능)` +
      (r.no_history ? " · 고른 사본에 변경 기록이 없어 각자 지운 객체가 되살아났을 수 있습니다" : ""));
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
    case "mergecopy": return app.doc && mergeCopy();
    case "palette": return palette.show();
    case "shx": return shxFolder();
    case "find": return findReplace();
  }
}

/** 두 번 누른 곳의 글자·실 이름 고치기 (§62) */
async function editAt(m) {
  const r = await invoke("pick_at", { x: m[0], y: m[1], r: 6 / viewer.ppu }).catch(() => null);
  if (!r) return viewer.fit();
  if (r.arch && r.arch[0] === "room") {
    const name = (r.props.find(([k]) => k === "실 이름:고침") || [])[1] ?? "";
    const v = await ask("실 이름", [["name", "이름", name]]);
    if (v && v.name.trim() && v.name !== name) await app.apply({ kind: "arch", op: { kind: "set", what: "room", id: r.arch[1], fields: [["실 이름", v.name]] } });
    return;
  }
  if (!r.editable) return;
  const text = await invoke("text_of", { id: r.index });
  if (text == null) return app.select.click?.(m);
  const v = await ask("글자 고치기", [["value", "내용", text, text.includes("\n") ? "textarea" : null]]);
  if (v && v.value !== text) await app.apply({ kind: "set_text", id: r.index, value: v.value });
}

/** 찾아 바꾸기 (§62): 글자·블록 속성값·실 이름·치수 글자 */
async function findReplace() {
  if (!app.doc) return;
  const v = await ask("찾아 바꾸기 (글자·속성값·실 이름·치수 글자)", [["find", "찾을 글자", ""], ["repl", "바꿀 글자", ""]]);
  if (!v || !v.find) return;
  const n = await invoke("count_text", { find: v.find });
  if (!n) return status(`「${v.find}」 을 찾지 못했습니다`);
  await app.apply({ kind: "replace_text", find: v.find, repl: v.repl });
  status(`${n} 곳을 바꿨습니다 — ⌘Z 로 한 번에 되돌림`);
}

/** SHX 글꼴 폴더: 있으면 끌지·바꿀지 묻고, 없으면 고른다 (§59) */
async function shxFolder() {
  const now = await invoke("get_setting", { key: "shx_folder" });
  let pick = true;
  if (now) {
    const v = await ask(`SHX 글꼴 폴더: ${now}`, [["what", "어떻게", "다른 폴더 고르기", ["다른 폴더 고르기", "SHX 끄기 (내장 글꼴로)"]]]);
    if (!v) return;
    pick = v.what !== "SHX 끄기 (내장 글꼴로)";
  }
  try {
    const r = await invoke("set_shx_folder", { pick });
    if (app.doc) await redraw(true);
    status(r ? `SHX 글꼴 폴더: ${r[0]} (SHX ${r[1]} 개) — 가져온 글자를 SHX 로 그립니다 · 「표 → 글꼴」 에서 확인` : "SHX 끔 — 모든 글자를 내장 글꼴로 그립니다");
  } catch (err) {
    status(String(err));
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
  const frames = $("pdf-what").value === "frames";
  $("pdf-what").value = "one";
  if (frames) {
    // 모형의 도면 틀마다 한 쪽 — 용지·축척은 틀 크기로 (§63)
    status("도면 틀마다 PDF 만드는 중…");
    try {
      const r = await invoke("export_frames_pdf", { req: { ...req, area: null } });
      status(r ? `PDF 저장 (도면 틀 ${r[1]} 장): ${r[0]}` : "PDF 저장을 취소했습니다");
    } catch (e) {
      status(`PDF 를 저장하지 못했습니다: ${e}`);
    }
    return;
  }
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
  const p = ev.payload?.paths?.find((x) => /\.(dwg|dxf|dsmith|ifc|shp)$/i.test(x));
  if (p) open(p);
});

invoke("app_version").then((v) => ($("version").textContent = v));
loadRecent();
cmd.focus();
// 실행할 때 넘긴 도면이 있으면 열고, 없으면 새 도면
invoke("startup_file").then((p) => (p ? open(p) : newDrawing()));

// 명령 표를 개발 도구에서 볼 수 있게
window.draftsmith = { app, run, COMMANDS };
