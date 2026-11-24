// DraftSmith 화면 진입점. Rust 명령은 window.__TAURI__.core.invoke 로 부른다 (BEATS 와 같음).
import { Viewer } from "./viewer.js";

const invoke = window.__TAURI__.core.invoke;
const $ = (id) => document.getElementById(id);
const fmt = (n) => n.toLocaleString("ko-KR");
const ms = (v) => `${Math.round(v)} ms`;

const viewer = new Viewer($("view"), (x, y) => {
  $("cursor").textContent = `X ${fmt(Math.round(x))}  Y ${fmt(Math.round(y))}`;
});

function status(text) {
  $("status").textContent = text;
}

/** 표시 데이터를 받아 그린다. 전달·올리기 시간을 돌려준다 (M0 ① 측정) */
async function refresh() {
  const t = performance.now();
  const buf = await invoke("display_data");
  const transfer = performance.now() - t;
  const upload = viewer.load(buf);
  return { transfer, upload, mb: buf.byteLength / 1e6 };
}

function renderLayers(sum) {
  const ul = $("layers");
  ul.replaceChildren();
  sum.layers.forEach((l, i) => {
    const li = document.createElement("li");
    const cb = document.createElement("input");
    cb.type = "checkbox";
    cb.checked = l.visible;
    cb.addEventListener("change", async () => {
      await invoke("set_layer_visible", { index: i, visible: cb.checked });
      await refresh();
    });
    const sw = document.createElement("span");
    sw.className = "sw";
    sw.style.background = `rgb(${l.color.join(",")})`;
    const name = document.createElement("span");
    name.className = "name";
    name.textContent = l.name;
    const n = document.createElement("span");
    n.className = "muted small";
    n.textContent = fmt(l.count);
    li.append(cb, sw, name, n);
    ul.append(li);
  });
  $("skipped").textContent = sum.skipped.length
    ? "아직 표시하지 않는 객체: " + sum.skipped.map(([k, n]) => `${k} ${fmt(n)}`).join(", ")
    : "";
}

async function open(path) {
  status("읽는 중…");
  try {
    const sum = await invoke("open_drawing", { path });
    renderLayers(sum);
    const t = await refresh();
    const name = path.split(/[\\/]/).pop();
    document.title = `${name} — DraftSmith`;
    status(
      `${name} · 객체 ${fmt(sum.entities)} 개 · 읽기 ${ms(sum.read_ms)} · ` +
        `전달 ${ms(t.transfer)} (${t.mb.toFixed(1)} MB) · 올리기 ${ms(t.upload)}`,
    );
  } catch (e) {
    status(`열지 못했습니다: ${e}`);
  }
}

async function pickAndOpen() {
  const path = await invoke("pick_drawing");
  if (path) await open(path);
}

$("open").addEventListener("click", pickAndOpen);
$("fit").addEventListener("click", () => viewer.fit());
window.addEventListener("keydown", (e) => {
  if ((e.metaKey || e.ctrlKey) && e.key === "o") {
    e.preventDefault();
    pickAndOpen();
  } else if (e.key === "f" && !e.metaKey && !e.ctrlKey && !e.isComposing) {
    viewer.fit();
  }
});

invoke("app_version").then((v) => ($("version").textContent = v));
