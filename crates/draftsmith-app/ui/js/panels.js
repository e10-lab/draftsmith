// 옆 칸: 레이어(켜기·현재·색·이름), 층(M3), 아래 서랍: 면적표·일람표 (구현 노트 §25·§22).

const fmt = (n) => n.toLocaleString("ko-KR");
const el = (tag, cls, text) => {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
};
const hex = (c) => "#" + c.map((v) => v.toString(16).padStart(2, "0")).join("");
const rgb = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));

/** 레이어 목록 */
export function renderLayers(app, doc) {
  const ul = document.getElementById("layers");
  ul.replaceChildren();
  const model = doc.sheet == null;
  doc.layers.forEach((l, i) => {
    // 레이어가 많으면 쓰지 않는 것은 숨김 (현재 레이어는 보임)
    if (l.count === 0 && i !== doc.current_layer && doc.layers.length > 24) return;
    const li = el("li");
    li.classList.toggle("current", model && i === doc.current_layer);
    const cb = el("input");
    cb.type = "checkbox";
    cb.checked = l.visible;
    cb.title = "보이기";
    cb.addEventListener("change", () => app.setLayerVisible(i, cb.checked));
    const sw = el("input", "sw");
    sw.type = "color";
    sw.value = hex(l.color);
    sw.title = "레이어 색";
    sw.disabled = !model || i >= app.baseLayers;
    sw.addEventListener("change", () => app.apply({ kind: "edit_layer", index: i, name: l.name, color: rgb(sw.value) }));
    const name = el("span", "name", l.name);
    name.title = model ? `${l.name} — 두 번 눌러 현재 레이어로` : l.name;
    name.addEventListener("dblclick", () => model && i < app.baseLayers && app.apply({ kind: "current_layer", layer: i }));
    const n = el("span", "muted small", l.count ? fmt(l.count) : "");
    li.append(cb, sw, name, n);
    ul.append(li);
  });
  document.getElementById("skipped").textContent = doc.skipped.length
    ? "표시하지 않은 객체: " + doc.skipped.map(([k, n]) => `${k} ${fmt(n)}`).join(", ")
    : "";
  const cur = doc.layers[doc.current_layer];
  document.getElementById("cur-layer").textContent = cur ? cur.name : "";
}

/** 층 목록 (건축 객체) */
export function renderLevels(app, doc) {
  const ul = document.getElementById("levels");
  ul.replaceChildren();
  const a = doc.arch;
  for (const [id, name] of a.levels) {
    const li = el("li", id === a.active_level ? "current" : "", name);
    li.title = "눌러서 지금 층으로";
    li.addEventListener("click", () => app.apply({ kind: "arch", op: { kind: "active_level", id } }));
    ul.append(li);
  }
  document.getElementById("arch-sum").textContent =
    `벽 ${a.walls} · 문·창 ${a.openings} · 실 ${a.rooms} · 그리드 ${a.grids} · 기둥 ${a.columns} · 계단 ${a.stairs} · 치수 ${a.dims}` +
    (a.has_site ? " · 대지" : "");
}

/** 면적표·창호일람표·실별 마감표·법규 검토 서랍 */
export class TablesDrawer {
  constructor(root, app) {
    this.root = root;
    this.app = app;
    this.tabs = root.querySelector(".tb-tabs");
    this.body = root.querySelector(".tb-body");
    this.cur = null;
    root.querySelector(".tb-close").addEventListener("click", () => this.hide());
    root.querySelector(".tb-csv").addEventListener("click", async () => {
      if (!this.cur) return;
      try {
        const p = await app.invoke("export_table_csv", { name: this.cur });
        app.report(p ? `CSV 저장: ${p}` : "CSV 저장을 취소했습니다");
      } catch (e) {
        app.report(`CSV 를 저장하지 못했습니다: ${e}`);
      }
    });
  }
  get open() { return !this.root.hidden; }
  hide() { this.root.hidden = true; }
  async show() {
    this.root.hidden = false;
    await this.refresh();
  }
  async refresh() {
    if (this.root.hidden) return;
    let t;
    try {
      t = await this.app.invoke("arch_tables");
    } catch (e) {
      this.body.textContent = String(e);
      return;
    }
    const list = t.tables;
    if (!list.some((x) => x.name === this.cur)) this.cur = list[0]?.name ?? null;
    this.tabs.replaceChildren(
      ...list.map((x) => {
        const b = el("button", x.name === this.cur ? "on" : "", x.title);
        b.addEventListener("click", () => {
          this.cur = x.name;
          this.refresh();
        });
        return b;
      }),
    );
    const tb = list.find((x) => x.name === this.cur);
    this.body.replaceChildren();
    if (!tb) return;
    if (!tb.rows.length) {
      this.body.append(el("p", "muted", "아직 내용이 없습니다 — 벽·문·창·실을 그리면 채워집니다"));
      return;
    }
    const table = el("table");
    const tr = el("tr");
    for (const h of tb.head) tr.append(el("th", "", h));
    table.append(tr);
    for (const r of tb.rows) {
      const row = el("tr");
      for (const c of r) row.append(el("td", /^[\d,.\-\s㎡평%]+$/.test(c) ? "num" : "", c));
      table.append(row);
    }
    this.body.append(table);
  }
}

/** 시트의 뷰포트 고치기 (§35): 축척·크기·가운데·보는 가운데, 모형에 맞춤 */
const SCALES = [1, 2, 5, 10, 20, 25, 30, 50, 100, 150, 200, 250, 300, 400, 500, 600, 1000, 1200, 1500, 2000, 3000, 5000];
export function renderViewports(app, doc) {
  const box = document.getElementById("vports");
  box.replaceChildren();
  const vps = doc.viewports ?? [];
  box.hidden = doc.sheet == null || !vps.length;
  if (box.hidden) return;
  const r1 = (v) => String(Math.round(v * 10) / 10);
  const send = (index, edit) => app.apply({ kind: "viewport", index, ...edit });
  box.append(el("h4", "", "뷰포트"));
  for (const v of vps) {
    const dl = el("dl", "props");
    const field = (label, value, on) => {
      const i = el("input");
      i.value = value;
      i.addEventListener("keydown", (e) => {
        if (e.key === "Enter") i.blur();
        e.stopPropagation();
      });
      i.addEventListener("change", () => on(i.value));
      dl.append(el("dt", "", label));
      const dd = el("dd");
      dd.append(i);
      dl.append(dd);
      return i;
    };
    const sc = field("축척 1:", r1(v.scale), (x) => Number(x) > 0 && send(v.index, { scale: Number(x) }));
    const list = el("datalist");
    list.id = `vp-scales-${v.index}`;
    for (const k of SCALES) list.append(new Option(String(k)));
    sc.setAttribute("list", list.id);
    dl.append(list);
    const num = (s) => s.split(/[,x×\s]+/).map(Number);
    field("크기 (mm)", `${r1(v.size[0])} × ${r1(v.size[1])}`, (x) => {
      const [w, h] = num(x);
      if (w > 0 && h > 0) send(v.index, { size: [w, h] });
    });
    field("종이 위치", `${r1(v.center[0])}, ${r1(v.center[1])}`, (x) => {
      const [a, b] = num(x);
      if (Number.isFinite(a) && Number.isFinite(b)) send(v.index, { center: [a, b] });
    });
    field("보는 가운데", `${Math.round(v.view_center[0])}, ${Math.round(v.view_center[1])}`, (x) => {
      const [a, b] = num(x);
      if (Number.isFinite(a) && Number.isFinite(b)) send(v.index, { view_center: [a, b] });
    });
    const fit = el("button", "", "모형에 맞춤");
    fit.title = "그 층 모형 전체가 들어오게 가운데와 표준 축척을 맞춥니다";
    fit.addEventListener("click", () => send(v.index, { fit: true }));
    box.append(el("p", "muted small", `뷰포트 ${v.index + 1}`), dl, fit);
  }
}
