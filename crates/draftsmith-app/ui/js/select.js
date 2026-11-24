// 고르기 (기본 도구, 구현 노트 §25).
//
// 누르기: 가장 가까운 객체 하나 (Shift 면 더하고 빼기). 끌기: 왼→오른쪽은 창 고르기(상자 안에 다 든 것),
// 오른→왼쪽은 걸침 고르기(상자에 걸친 것도). 고른 모형 객체 번호는 ids, 건축 객체(벽·문·창·실·그리드)는 arch.
// 속성 칸: 모형 객체는 레이어·색을 바꿀 수 있고, 건축 객체는 「:고침」 붙은 값을 바로 고친다.

const PICK_PX = 6;
const HILITE = "#4aa8ff";
const ACI = [
  ["", "레이어 색 (ByLayer)"], ["1", "빨강 (1)"], ["2", "노랑 (2)"], ["3", "초록 (3)"], ["4", "하늘 (4)"],
  ["5", "파랑 (5)"], ["6", "자홍 (6)"], ["7", "흰색·검정 (7)"], ["8", "회색 (8)"], ["9", "밝은 회색 (9)"],
];
const CHOICES = {
  종류: ["외벽", "내벽", "내력벽"],
  "여는 쪽 뒤집기": ["아니오", "예"],
  "경첩 반대": ["아니오", "예"],
};

export class SelectTool {
  /**
   * @param app    { v, invoke, apply, doc }
   * @param panel  속성 칸 요소
   */
  constructor(app, panel) {
    this.app = app;
    this.panel = panel;
    this.ids = [];
    this.arch = null;    // { what, id, at }
    this.info = null;    // { props, outline }
    this.box = null;
  }

  get snaps() { return false; }
  get count() { return this.ids.length + (this.arch ? 1 : 0); }

  clear() {
    this.ids = [];
    this.arch = null;
    this.info = null;
    this.render();
  }

  render() {
    this.renderPanel();
    this.app.v.drawOverlay();
    this.app.onSelect?.();
  }

  /** 고르기 결과(pick_at) 하나를 그대로 보여 준다 (간격 띄우기 대상 표시 등) */
  showPick(r) {
    this.ids = [];
    this.arch = null;
    this.info = { props: r.props, outline: r.outline };
    this.render();
  }

  async click(m, e) {
    let r = null;
    try {
      r = await this.app.invoke("pick_at", { x: m[0], y: m[1], r: PICK_PX / this.app.v.ppu });
    } catch {
      r = null;
    }
    const add = e?.shiftKey;
    if (!r) {
      if (!add) this.clear();
      return;
    }
    if (r.arch) {
      this.ids = [];
      this.arch = { what: r.arch[0], id: r.arch[1], at: m };
      this.info = { props: r.props, outline: r.outline };
      return this.render();
    }
    if (!r.editable) return this.showPick(r);
    this.arch = null;
    if (add) {
      const i = this.ids.indexOf(r.index);
      if (i >= 0) this.ids.splice(i, 1);
      else this.ids.push(r.index);
    } else this.ids = [r.index];
    await this.update();
  }

  drag(a, b, crossing) {
    this.box = { a, b, crossing };
    this.app.v.drawOverlay();
  }

  async dragEnd(a, b, crossing, add) {
    this.box = null;
    const rect = [Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.max(a[0], b[0]), Math.max(a[1], b[1])];
    let found = [];
    try {
      found = await this.app.invoke("pick_box", { rect, crossing });
    } catch {
      found = [];
    }
    this.arch = null;
    this.ids = add ? [...new Set([...this.ids, ...found])] : found;
    await this.update();
  }

  async set(ids) {
    this.arch = null;
    this.ids = [...ids];
    await this.update();
  }

  /** 고른 것의 강조선·속성을 다시 받는다 (편집 뒤에도) */
  async update() {
    if (this.arch) {
      // 건축 객체는 같은 자리를 다시 골라 속성을 새로 받는다
      const m = this.arch.at;
      const r = await this.app.invoke("pick_at", { x: m[0], y: m[1], r: PICK_PX / this.app.v.ppu }).catch(() => null);
      if (r?.arch && r.arch[0] === this.arch.what && r.arch[1] === this.arch.id) this.info = { props: r.props, outline: r.outline };
      else this.clear();
      return this.render();
    }
    this.ids = this.ids.filter((i) => i < (this.app.doc?.entities ?? 0));
    if (!this.ids.length) {
      this.info = null;
      return this.render();
    }
    try {
      this.info = await this.app.invoke("selection_info", { ids: this.ids, tol: 1 / this.app.v.ppu });
    } catch {
      this.info = null;
    }
    this.render();
  }

  // ── 속성 칸 ───────────────────────────────────────────────────────────────
  renderPanel() {
    const p = this.panel;
    p.replaceChildren();
    if (!this.info) {
      p.hidden = true;
      return;
    }
    p.hidden = false;
    const h = document.createElement("h4");
    h.textContent = this.arch ? "건축 객체" : this.ids.length > 1 ? `고른 객체 ${this.ids.length} 개` : "속성";
    const dl = document.createElement("dl");
    dl.className = "props";
    for (const [k, v] of this.info.props) {
      if (k.startsWith("_")) continue;
      const editable = this.arch && k.endsWith(":고침");
      const key = editable ? k.slice(0, -3) : k;
      const dt = document.createElement("dt");
      dt.textContent = key;
      const dd = document.createElement("dd");
      if (editable) dd.append(this.field(key, v));
      else {
        dd.textContent = v;
        dd.title = v;
      }
      dl.append(dt, dd);
    }
    p.append(h, dl);
    if (this.ids.length) p.append(this.modelControls());
    if (this.arch) {
      const del = document.createElement("button");
      del.textContent = "지우기";
      del.className = "danger";
      del.addEventListener("click", async () => {
        await this.app.apply({ kind: "arch", op: { kind: "delete", what: this.arch.what, id: this.arch.id } });
        this.clear();
      });
      p.append(del);
    }
  }

  field(key, value) {
    const send = async (v) => {
      if (v === value) return;
      await this.app.apply({ kind: "arch", op: { kind: "set", what: this.arch.what, id: this.arch.id, fields: [[key, v]] } });
      await this.update();
    };
    const ch = CHOICES[key];
    if (ch) {
      const s = document.createElement("select");
      for (const c of ch) s.append(new Option(c, c, false, c === value));
      s.addEventListener("change", () => send(s.value));
      return s;
    }
    const i = document.createElement("input");
    i.value = value;
    i.addEventListener("keydown", (e) => {
      if (e.key === "Enter") i.blur();
      if (e.key === "Escape") {
        i.value = value;
        i.blur();
      }
      e.stopPropagation();
    });
    i.addEventListener("change", () => send(i.value));
    return i;
  }

  /** 모형 객체: 레이어·색 바꾸기 */
  modelControls() {
    const box = document.createElement("div");
    box.className = "props-edit";
    const layers = this.app.doc?.layers ?? [];
    const ls = document.createElement("select");
    ls.append(new Option("레이어 바꾸기…", ""));
    layers.forEach((l, i) => ls.append(new Option(l.name, String(i))));
    ls.addEventListener("change", async () => {
      if (ls.value === "") return;
      await this.app.apply({ kind: "set_layer", ids: this.ids, layer: Number(ls.value) });
      await this.update();
    });
    const cs = document.createElement("select");
    cs.append(new Option("색 바꾸기…", "-"));
    for (const [v, t] of ACI) cs.append(new Option(t, v));
    cs.addEventListener("change", async () => {
      if (cs.value === "-") return;
      await this.app.apply({ kind: "set_color", ids: this.ids, aci: cs.value === "" ? null : Number(cs.value) });
      await this.update();
    });
    box.append(ls, cs);
    return box;
  }

  paint(ctx, toScreen) {
    if (this.info) {
      ctx.strokeStyle = HILITE;
      ctx.lineWidth = 3;
      ctx.globalAlpha = 0.85;
      ctx.beginPath();
      for (const [x0, y0, x1, y1] of this.info.outline) {
        const [ax, ay] = toScreen(x0, y0), [bx, by] = toScreen(x1, y1);
        ctx.moveTo(ax, ay);
        ctx.lineTo(bx, by);
      }
      ctx.stroke();
      ctx.globalAlpha = 1;
    }
    if (this.box) {
      const [ax, ay] = toScreen(...this.box.a), [bx, by] = toScreen(...this.box.b);
      const cross = this.box.crossing;
      ctx.fillStyle = cross ? "rgba(80,200,120,0.12)" : "rgba(74,168,255,0.12)";
      ctx.strokeStyle = cross ? "#50c878" : HILITE;
      ctx.lineWidth = 1;
      ctx.setLineDash(cross ? [5, 4] : []);
      ctx.fillRect(Math.min(ax, bx), Math.min(ay, by), Math.abs(bx - ax), Math.abs(by - ay));
      ctx.strokeRect(Math.min(ax, bx), Math.min(ay, by), Math.abs(bx - ax), Math.abs(by - ay));
      ctx.setLineDash([]);
    }
  }
}
