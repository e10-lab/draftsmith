// DraftSmith 화면 진입점. Rust 명령은 window.__TAURI__.core.invoke 로 부른다 (BEATS 와 같음).
import { Viewer } from "./viewer.js";
import { MeasureTool } from "./measure.js";
import { SelectTool } from "./select.js";

const invoke = window.__TAURI__.core.invoke;
const $ = (id) => document.getElementById(id);
const fmt = (n) => n.toLocaleString("ko-KR");
const ms = (v) => `${Math.round(v)} ms`;

let opened = false;
let measure = null;
let select = null;

const viewer = new Viewer($("view"), $("text"), $("overlay"), (x, y) => {
  const lat = measure && viewer.tool === measure ? measure.latency() : null;
  $("cursor").textContent =
    `X ${fmt(Math.round(x))}  Y ${fmt(Math.round(y))}` + (lat != null ? `  · 스냅 ${lat.toFixed(1)} ms` : "");
});
measure = new MeasureTool(viewer, invoke, status);
select = new SelectTool(viewer, invoke, $("props"));
viewer.tool = select; // 기본 도구: 눌러서 고르기

function status(text) {
  $("status").textContent = text;
}

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

/** 표시 데이터(선·채움, 문자)를 받아 그린다. 전달·올리기 시간을 돌려준다 (M0 ① 측정).
 *  keepView 면 보던 위치·배율을 그대로 둔다 (레이어 켜기·끄기). */
async function refresh(keepView = false) {
  const t = performance.now();
  const [buf, textBuf] = (await Promise.all([invoke("display_data"), invoke("text_data")])).map(toBuffer);
  const transfer = performance.now() - t;
  const upload = viewer.load(buf, textBuf, { keepView });
  return { transfer, upload, mb: (buf.byteLength + textBuf.byteLength) / 1e6 };
}

function renderLayers(sum) {
  const ul = $("layers");
  ul.replaceChildren();
  sum.layers.forEach((l, i) => {
    if (l.count === 0) return; // 쓰지 않는 레이어는 숨김
    const li = document.createElement("li");
    const cb = document.createElement("input");
    cb.type = "checkbox";
    cb.checked = l.visible;
    cb.addEventListener("change", async () => {
      await invoke("set_layer_visible", { index: i, visible: cb.checked });
      select.clear();
      await refresh(true);
    });
    const sw = document.createElement("span");
    sw.className = "sw";
    sw.style.background = `rgb(${l.color.join(",")})`;
    const name = document.createElement("span");
    name.className = "name";
    name.textContent = l.name;
    name.title = l.name;
    const n = document.createElement("span");
    n.className = "muted small";
    n.textContent = fmt(l.count);
    li.append(cb, sw, name, n);
    ul.append(li);
  });
  $("skipped").textContent = sum.skipped.length
    ? "표시하지 않은 객체: " + sum.skipped.map(([k, n]) => `${k} ${fmt(n)}`).join(", ")
    : "";
}

/** 모형·배치 탭 */
function renderSpaces(layouts) {
  const nav = $("spaces");
  nav.replaceChildren();
  nav.hidden = layouts.length === 0;
  const tabs = [["모형", null], ...layouts.map((n, i) => [n, i])];
  for (const [name, index] of tabs) {
    const b = document.createElement("button");
    b.textContent = name;
    b.classList.toggle("on", index === null);
    b.addEventListener("click", () => showSpace(index, b));
    nav.append(b);
  }
}

async function showSpace(index, button) {
  try {
    await invoke("set_space", { index });
    select.clear();
    for (const b of $("spaces").children) b.classList.toggle("on", b === button);
    // 배치는 종이 단위(mm)라 PDF 는 1:1 이 알맞다
    $("pdf-scale").value = index === null ? "" : "1";
    await refresh();
  } catch (e) {
    status(`배치를 보여 주지 못했습니다: ${e}`);
  }
}

async function loadRecent() {
  const list = await invoke("recent_files");
  const sel = $("recent");
  sel.replaceChildren(new Option("최근 파일", ""));
  for (const p of list) sel.append(new Option(p.split(/[\\/]/).pop(), p));
  sel.disabled = list.length === 0;
}

async function open(path) {
  status("읽는 중…");
  try {
    const sum = await invoke("open_drawing", { path });
    select.clear();
    renderLayers(sum);
    renderSpaces(sum.layouts);
    $("pdf-scale").value = "";
    const t = await refresh();
    const name = path.split(/[\\/]/).pop();
    document.title = `${name} — DraftSmith`;
    opened = true;
    status(
      `${name} · 객체 ${fmt(sum.entities)} 개` +
        (sum.inserts ? ` (블록 ${fmt(sum.inserts)} 개 펼침)` : "") +
        ` · 읽기 ${ms(sum.read_ms)} · ` +
        `전달 ${ms(t.transfer)} (${t.mb.toFixed(1)} MB${slowPath ? ", 느린 경로" : ""}) · 올리기 ${ms(t.upload)}`,
    );
    loadRecent();
  } catch (e) {
    status(`열지 못했습니다: ${e}`);
  }
}

async function pickAndOpen() {
  const path = await invoke("pick_drawing");
  if (path) await open(path);
}

function setMeasure(on) {
  if (on && !opened) return;
  select.clear();
  viewer.tool = on ? measure : select;
  $("measure").classList.toggle("on", on);
  if (on) measure.start();
  else measure.stop();
  viewer.drawOverlay();
}

$("open").addEventListener("click", pickAndOpen);
$("recent").addEventListener("change", (e) => {
  const p = e.target.value;
  e.target.value = "";
  if (p) open(p);
});
$("fit").addEventListener("click", () => viewer.fit());
$("measure").addEventListener("click", () => setMeasure(viewer.tool !== measure));

/** 흰 배경 켜기·끄기 (앱 설정 폴더에 기억 — 다음에 열 때도) */
function setPaper(on, save = true) {
  viewer.paper = on;
  $("paper").classList.toggle("on", on);
  if (save) invoke("set_setting", { key: "paper", value: on ? "1" : "0" }).catch(() => {});
  viewer.draw();
}
$("paper").addEventListener("click", () => setPaper(!viewer.paper));
invoke("get_setting", { key: "paper" }).then((v) => setPaper(v === "1", false), () => {});

// PDF 저장
function togglePdf(show = $("pdf-pop").hidden) {
  if (show && !opened) return;
  $("pdf-pop").hidden = !show;
}
$("pdf").addEventListener("click", () => togglePdf());
$("pdf-cancel").addEventListener("click", () => togglePdf(false));
$("pdf-save").addEventListener("click", async () => {
  const scale = $("pdf-scale").value;
  const req = {
    paper: $("pdf-paper").value,
    landscape: $("pdf-orient").value === "land",
    scale: scale ? Number(scale) : null,
    area: $("pdf-area").value === "view" ? viewer.viewBox() : null,
    mono: $("pdf-color").value === "mono",
  };
  togglePdf(false);
  status("PDF 만드는 중…");
  try {
    const path = await invoke("export_pdf", { req });
    status(path ? `PDF 저장: ${path}` : "PDF 저장을 취소했습니다");
  } catch (e) {
    status(`PDF 를 저장하지 못했습니다: ${e}`);
  }
});

window.addEventListener("keydown", (e) => {
  if (e.isComposing || e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement) return; // 한글 조합 중·입력칸에서는 단축키 없음
  // 한글 입력기가 켜져 있어도 같은 자판 위치로 동작하게 글자(e.key)가 아니라 자판 위치(e.code)로 본다
  const mod = e.metaKey || e.ctrlKey;
  if (mod && e.code === "KeyP") {
    e.preventDefault();
    togglePdf(true);
  } else if (mod && e.code === "KeyO") {
    e.preventDefault();
    pickAndOpen();
  } else if (!mod && e.code === "KeyF") {
    viewer.fit();
  } else if (!mod && e.code === "KeyB") {
    setPaper(!viewer.paper);
  } else if (!mod && e.code === "KeyD") {
    setMeasure(viewer.tool !== measure);
  } else if (e.key === "Escape") {
    togglePdf(false);
    if (viewer.tool === measure) setMeasure(false);
    else select.clear();
  }
});

// 파일을 창에 끌어 놓으면 연다
window.__TAURI__.event?.listen?.("tauri://drag-drop", (ev) => {
  const p = ev.payload?.paths?.find((x) => /\.(dwg|dxf)$/i.test(x));
  if (p) open(p);
});

invoke("app_version").then((v) => ($("version").textContent = v));
loadRecent();
// 실행할 때 넘긴 도면이 있으면 바로 연다
invoke("startup_file").then((p) => p && open(p));
