// 고르기 도구 (기본 도구). 누른 곳에서 가장 가까운 객체를 Rust 코어에 묻고, 강조 표시·속성 칸을 채운다.

const PICK_PX = 6;            // 이 화면 거리 안의 객체를 고른다
const HILITE = "#4aa8ff";

export class SelectTool {
  /**
   * @param viewer  Viewer
   * @param invoke  Tauri invoke
   * @param panel   속성 칸 요소
   */
  constructor(viewer, invoke, panel) {
    this.v = viewer;
    this.invoke = invoke;
    this.panel = panel;
    this.sel = null;          // { index, props, outline }
  }

  clear() {
    this.sel = null;
    this.renderPanel();
    this.v.drawOverlay();
  }

  async click(m) {
    try {
      this.sel = await this.invoke("pick_at", { x: m[0], y: m[1], r: PICK_PX / this.v.ppu });
    } catch {
      this.sel = null;
    }
    this.renderPanel();
    this.v.drawOverlay();
  }

  renderPanel() {
    const p = this.panel;
    p.replaceChildren();
    if (!this.sel) {
      p.hidden = true;
      return;
    }
    p.hidden = false;
    const h = document.createElement("h4");
    h.textContent = "속성";
    const dl = document.createElement("dl");
    dl.className = "props";
    for (const [k, v] of this.sel.props) {
      const dt = document.createElement("dt");
      dt.textContent = k;
      const dd = document.createElement("dd");
      dd.textContent = v;
      dd.title = v;
      dl.append(dt, dd);
    }
    p.append(h, dl);
  }

  paint(ctx, toScreen) {
    if (!this.sel) return;
    ctx.strokeStyle = HILITE;
    ctx.lineWidth = 3;
    ctx.globalAlpha = 0.85;
    ctx.beginPath();
    for (const [x0, y0, x1, y1] of this.sel.outline) {
      const [ax, ay] = toScreen(x0, y0), [bx, by] = toScreen(x1, y1);
      ctx.moveTo(ax, ay);
      ctx.lineTo(bx, by);
    }
    ctx.stroke();
    ctx.globalAlpha = 1;
  }
}
