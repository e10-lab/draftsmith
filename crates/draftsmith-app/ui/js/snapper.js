// 스냅·직교·입력 좌표 — 모든 작도 도구가 함께 쓴다 (구현 노트 §25).
//
// 커서 근처 스냅 점은 Rust 코어에 묻는다 (한 번에 하나, 밀린 것은 마지막 위치만). 직교(F8)면 기준점에서
// 가로·세로로만. 입력창에 친 값은 parse() 로 점이 된다:
//   x,y      절대 좌표
//   @dx,dy   기준점에서 상대
//   @d<각    기준점에서 거리 d, 각도(도)
//   d        기준점에서 커서 쪽으로 거리 d (직접 거리 입력)

const SNAP_PX = 12;
const SNAP_NAMES = ["끝점", "중간점", "중심", "사분점", "삽입점"];
const COLOR = "#ffd24a";

export class Snapper {
  constructor(viewer, invoke) {
    this.v = viewer;
    this.invoke = invoke;
    this.on = true;        // 객체 스냅 (F3)
    this.ortho = false;    // 직교 (F8)
    this.cursor = null;
    this.snap = null;
    this.busy = false;
    this.pending = null;
    this.lat = [];
  }

  /** 커서가 움직였다: 스냅 점을 묻는다 */
  update(m) {
    this.cursor = m;
    if (!this.on) {
      this.snap = null;
      return;
    }
    this.query(m);
  }

  async query(m) {
    if (this.busy) {
      this.pending = m;
      return;
    }
    this.busy = true;
    const t = performance.now();
    try {
      this.snap = await this.invoke("snap", { x: m[0], y: m[1], r: SNAP_PX / this.v.ppu });
    } catch {
      this.snap = null;
    }
    this.lat.push(performance.now() - t);
    if (this.lat.length > 30) this.lat.shift();
    this.busy = false;
    this.v.drawOverlay();
    if (this.pending) {
      const p = this.pending;
      this.pending = null;
      this.query(p);
    }
  }

  latency() {
    if (!this.lat.length) return null;
    const s = [...this.lat].sort((a, b) => a - b);
    return s[Math.floor(s.length / 2)];
  }

  /** 지금 점: 스냅이 있으면 스냅, 직교면 기준점에서 가로·세로 */
  point(base = null, raw = this.cursor) {
    if (!raw) return null;
    if (this.snap && this.on && Math.hypot(this.snap.x - raw[0], this.snap.y - raw[1]) * this.v.ppu < SNAP_PX * 1.5) {
      return [this.snap.x, this.snap.y];
    }
    if (this.ortho && base) {
      const dx = raw[0] - base[0], dy = raw[1] - base[1];
      return Math.abs(dx) >= Math.abs(dy) ? [raw[0], base[1]] : [base[0], raw[1]];
    }
    return raw;
  }

  /** 입력창 글을 점으로. 못 알아들으면 null */
  parse(text, base) {
    const t = text.trim().replace(/\s+/g, "");
    let m;
    if ((m = t.match(/^@(-?[\d.]+)<(-?[\d.]+)$/)) && base) {
      const d = +m[1], a = (+m[2] * Math.PI) / 180;
      return [base[0] + d * Math.cos(a), base[1] + d * Math.sin(a)];
    }
    if ((m = t.match(/^@(-?[\d.]+),(-?[\d.]+)$/)) && base) return [base[0] + +m[1], base[1] + +m[2]];
    if ((m = t.match(/^(-?[\d.]+),(-?[\d.]+)$/))) return [+m[1], +m[2]];
    if ((m = t.match(/^(-?[\d.]+)$/)) && base) {
      const c = this.point(base) || [base[0] + 1, base[1]];
      let dx = c[0] - base[0], dy = c[1] - base[1];
      const l = Math.hypot(dx, dy);
      if (l < 1e-9) { dx = 1; dy = 0; } else { dx /= l; dy /= l; }
      return [base[0] + dx * +m[1], base[1] + dy * +m[1]];
    }
    return null;
  }

  /** 스냅 표시 그리기 */
  paint(ctx, toScreen) {
    if (!this.snap || !this.on) return;
    const [x, y] = toScreen(this.snap.x, this.snap.y);
    const r = 6;
    ctx.strokeStyle = COLOR;
    ctx.lineWidth = 2;
    ctx.beginPath();
    switch (this.snap.kind) {
      case 0: ctx.rect(x - r, y - r, 2 * r, 2 * r); break;
      case 1: ctx.moveTo(x, y - r); ctx.lineTo(x + r, y + r); ctx.lineTo(x - r, y + r); ctx.closePath(); break;
      case 2: ctx.arc(x, y, r, 0, Math.PI * 2); break;
      case 3: ctx.moveTo(x, y - r); ctx.lineTo(x + r, y); ctx.lineTo(x, y + r); ctx.lineTo(x - r, y); ctx.closePath(); break;
      default: ctx.moveTo(x - r, y - r); ctx.lineTo(x + r, y + r); ctx.moveTo(x + r, y - r); ctx.lineTo(x - r, y + r);
    }
    ctx.stroke();
    ctx.font = `11px "Pretendard", sans-serif`;
    ctx.fillStyle = COLOR;
    ctx.textAlign = "left";
    ctx.textBaseline = "top";
    ctx.fillText(SNAP_NAMES[this.snap.kind] ?? "", x + 9, y + 7);
  }
}
