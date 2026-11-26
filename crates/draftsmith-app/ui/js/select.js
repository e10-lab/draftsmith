// 고르기 (기본 도구, 구현 노트 §25).
//
// 누르기: 가장 가까운 객체 하나 (Shift 면 더하고 빼기). 끌기: 왼→오른쪽은 창 고르기(상자 안에 다 든 것),
// 오른→왼쪽은 걸침 고르기(상자에 걸친 것도). 고른 모형 객체 번호는 ids, 건축 객체(벽·문·창·실·그리드)는 arch.
// 속성 칸: 모형 객체는 레이어·색을 바꿀 수 있고, 건축 객체는 「:고침」 붙은 값을 바로 고친다.
import { tok } from "./tokens.js";

const PICK_PX = 6;
const ACI = [
  ["", "레이어 색 (ByLayer)"], ["1", "빨강 (1)"], ["2", "노랑 (2)"], ["3", "초록 (3)"], ["4", "하늘 (4)"],
  ["5", "파랑 (5)"], ["6", "자홍 (6)"], ["7", "흰색·검정 (7)"], ["8", "회색 (8)"], ["9", "밝은 회색 (9)"],
];
const CHOICES = {
  "끝 모양": ["사선", "화살표"],
  쉼표: ["예", "아니오"],
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
    this.uids = null;    // 고른 객체의 고유 번호 (§84)
    this.arch = null;    // { what, id, at }
    this.info = null;    // { props, outline }
    this.box = null;
    this.grips = [];     // [{ x, y, id, arch, k }] — 고른 것의 손잡이 (§33)
    this.grip = null;    // 지금 옮기는 손잡이
    this.cur = null;
    this.gripSeq = 0;
  }

  get snaps() { return !!this.grip; }
  base() { return this.grip ? [this.grip.x, this.grip.y] : null; }
  get count() { return this.ids.length + (this.arch ? 1 : 0); }

  clear() {
    this.grip = null;
    this.grips = [];
    this.ids = [];
    this.uids = null;
    this.arch = null;
    this.info = null;
    this.render();
  }

  render() {
    this.renderPanel();
    this.app.v.drawOverlay();
    this.app.onSelect?.();
    this.loadGrips();
  }

  // ── 그립 (§33) ─────────────────────────────────────────────────────────────
  async loadGrips() {
    const seq = ++this.gripSeq;
    const arch = this.arch ? [this.arch.what, this.arch.id] : null;
    if ((!this.ids.length && !arch) || this.ids.length > 50) {
      this.grips = [];
      return;
    }
    let g = [];
    try {
      g = await this.app.invoke("grips", { ids: this.ids, arch });
    } catch {
      g = [];
    }
    if (seq !== this.gripSeq) return;
    this.grips = g;
    this.app.v.drawOverlay();
  }

  /** 화면 점 근처 손잡이 */
  gripAt(m) {
    const r = 7 / this.app.v.ppu;
    let best = null, bd = r;
    for (const g of this.grips) {
      const d = Math.hypot(g.x - m[0], g.y - m[1]);
      if (d <= bd) { bd = d; best = g; }
    }
    return best;
  }

  move(m) {
    if (this.grip) this.cur = this.app.snap.point(this.base(), m);
  }

  cancelGrip() {
    if (!this.grip) return false;
    this.grip = null;
    this.cur = null;
    this.app.prompt("명령:");
    this.app.v.drawOverlay();
    return true;
  }

  /** 명령줄에 친 좌표로 손잡이 놓기 (@ 상대는 손잡이 기준) */
  gripInput(text) {
    const p = this.app.snap.parse(text, this.base());
    if (!p) return false;
    this.placeGrip(p);
    return true;
  }

  async placeGrip(p) {
    const g = this.grip;
    this.grip = null;
    this.cur = null;
    this.app.prompt("명령:");
    const req = g.id != null
      ? { kind: "grip", id: g.id, k: g.k, to: p }
      : { kind: "arch", op: { kind: "grip", what: g.arch[0], id: g.arch[1], k: g.k, to: p } };
    const st = await this.app.apply(req);
    if (st && this.arch) this.arch.at = p;
    await this.update();
  }

  /** 고르기 결과(pick_at) 하나를 그대로 보여 준다 (간격 띄우기 대상 표시 등) */
  showPick(r) {
    this.ids = [];
    this.uids = null;
    this.arch = null;
    this.info = { props: r.props, outline: r.outline };
    this.render();
  }

  async click(m, e) {
    // 손잡이를 옮기는 중이면 놓고, 손잡이를 누르면 잡는다
    if (this.grip) return this.placeGrip(this.app.snap.point(this.base(), m));
    const g = !e?.shiftKey && this.gripAt(m);
    if (g) {
      this.grip = g;
      this.cur = [g.x, g.y];
      this.app.prompt("손잡이: 옮길 곳을 누르세요 (@dx,dy 도 됨, Esc 취소)");
      this.app.v.drawOverlay();
      return;
    }
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
      this.uids = null;
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
    this.uids = null;
    await this.update();
  }

  drag(a, b, crossing) {
    // 손잡이에서 끌기 시작하면 손잡이 옮기기 (§35)
    if (!this.box && !this.grip) {
      const g = this.gripAt(a);
      if (g) {
        this.grip = g;
        this.dragGrip = true;
      }
    }
    if (this.dragGrip) {
      this.app.snap.update(b);
      this.cur = this.app.snap.point(this.base(), b);
      this.app.v.drawOverlay();
      return;
    }
    this.box = { a, b, crossing };
    this.app.v.drawOverlay();
  }

  async dragEnd(a, b, crossing, add) {
    if (this.dragGrip) {
      this.dragGrip = false;
      return this.placeGrip(this.app.snap.point(this.base(), b));
    }
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
    this.uids = null;
    await this.update();
  }

  async set(ids) {
    this.arch = null;
    this.ids = [...ids];
    this.uids = null;
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
    // 편집 뒤: 고유 번호로 같은 객체를 다시 찾는다 — 앞 객체가 빠져 목록 번호가 밀려도 (§84)
    if (this.uids?.some((u) => u !== 0)) {
      const found = await this.app.invoke("indices_of_uids", { uids: this.uids }).catch(() => null);
      if (found) this.ids = found;
    }
    this.ids = this.ids.filter((i) => i < (this.app.doc?.entities ?? 0));
    if (!this.ids.length) {
      this.info = null;
      return this.render();
    }
    try {
      this.info = await this.app.invoke("selection_info", { ids: this.ids, tol: 1 / this.app.v.ppu });
      this.uids = this.info.uids;
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
      // 블록 참조의 속성값도 바로 고친다 (§46)
      const attr = !this.arch && this.ids.length === 1 && k.startsWith("속성 ") && k.endsWith(":고침");
      const editable = (this.arch || attr) && k.endsWith(":고침");
      const key = editable ? k.slice(0, -3) : k;
      const dt = document.createElement("dt");
      dt.textContent = key;
      const dd = document.createElement("dd");
      if (attr) dd.append(this.attrField(key.slice(3), v));
      else if (editable) dd.append(this.field(key, v));
      else {
        dd.textContent = v;
        dd.title = v;
      }
      dl.append(dt, dd);
    }
    p.append(h, dl);
    if (this.ids.length) p.append(this.modelControls());
    if (this.ids.length === 1 && this.info.props.some(([k]) => k === "블록")) {
      const add = document.createElement("button");
      add.textContent = "속성 더하기…";
      add.title = "블록 참조에 태그·값 글자를 붙입니다 (참조와 함께 움직임, DWG 에서는 ATTRIB)";
      add.addEventListener("click", async () => {
        const v = await this.app.ask("속성 더하기", [["tag", "태그 (예: NO)", ""], ["value", "값", ""], ["height", "글자 높이", String(this.app.textHeight)]]);
        if (!v) return;
        await this.app.apply({ kind: "add_attr", id: this.ids[0], tag: v.tag, value: v.value, height: Number(v.height) || this.app.textHeight });
        await this.update();
      });
      p.append(add);
      if (this.info.props.some(([k]) => k.startsWith("속성 "))) {
        const def = document.createElement("button");
        def.textContent = "블록 기본 속성으로";
        def.title = "이 참조의 속성(태그·위치·값)을 블록 정의에 저장합니다 — 이 블록을 새로 넣으면 기본값으로 붙고, DWG 에는 ATTDEF 로 나갑니다";
        def.addEventListener("click", async () => {
          await this.app.apply({ kind: "define_attrs", id: this.ids[0] });
          this.app.status?.("블록 속성 정의를 저장했습니다");
        });
        p.append(def);
      }
    }
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

  /** 블록 참조 속성값 칸 */
  attrField(tag, value) {
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
    i.addEventListener("change", async () => {
      if (i.value === value) return;
      await this.app.apply({ kind: "set_attr", id: this.ids[0], tag, value: i.value });
      await this.update();
    });
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
    if (this.grip && this.cur) {
      const [ax, ay] = toScreen(this.grip.x, this.grip.y), [bx, by] = toScreen(...this.cur);
      ctx.strokeStyle = tok("--ov-hilite");
      ctx.lineWidth = 1.5;
      ctx.setLineDash([6, 4]);
      ctx.beginPath();
      ctx.moveTo(ax, ay);
      ctx.lineTo(bx, by);
      ctx.stroke();
      ctx.setLineDash([]);
    }
    for (const g of this.grips) {
      const [x, y] = toScreen(g.x, g.y);
      const on = this.grip && g.k === this.grip.k && g.id === this.grip.id && String(g.arch) === String(this.grip.arch);
      ctx.fillStyle = on ? tok("--ov-hot") : tok("--ov-hilite");
      ctx.strokeStyle = tok("--ov-grip-line");
      ctx.lineWidth = 1;
      ctx.fillRect(x - 4, y - 4, 8, 8);
      ctx.strokeRect(x - 4, y - 4, 8, 8);
    }
    if (this.info) {
      ctx.strokeStyle = tok("--ov-hilite");
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
      ctx.fillStyle = cross ? tok("--ov-cross-fill") : tok("--ov-hilite-fill");
      ctx.strokeStyle = cross ? tok("--ov-cross") : tok("--ov-hilite");
      ctx.lineWidth = 1;
      ctx.setLineDash(cross ? [5, 4] : []);
      ctx.fillRect(Math.min(ax, bx), Math.min(ay, by), Math.abs(bx - ax), Math.abs(by - ay));
      ctx.strokeRect(Math.min(ax, bx), Math.min(ay, by), Math.abs(bx - ax), Math.abs(by - ay));
      ctx.setLineDash([]);
    }
  }
}
