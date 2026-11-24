// 거리 재기 도구. 커서 근처의 스냅 점(끝점·중간점·중심·사분점·삽입점)을 Rust 코어에 묻는다.
// 두 점을 누르면 거리·ΔX·ΔY·각도를 알린다. 스냅 지연(ms)도 재어 알린다 (M0 ⑤).

const SNAP_PX = 12;           // 이 화면 거리 안이면 스냅
const SNAP_NAMES = ["끝점", "중간점", "중심", "사분점", "삽입점"];
const COLOR = "#ffd24a";
const LINE = "#4aa8ff";

const fmt = (v) => v.toLocaleString("ko-KR", { minimumFractionDigits: 1, maximumFractionDigits: 1 });

export class MeasureTool {
  /**
   * @param viewer   Viewer
   * @param invoke   Tauri invoke
   * @param report   (text) 결과·안내 알림
   */
  constructor(viewer, invoke, report) {
    this.v = viewer;
    this.invoke = invoke;
    this.report = report;
    this.p1 = null;
    this.p2 = null;
    this.cursor = null;      // [x, y]
    this.snap = null;        // { x, y, kind }
    this.busy = false;
    this.pending = null;
    this.lat = [];           // 최근 스냅 지연 (ms)
  }

  start() {
    this.report("측정: 첫 점을 누르세요 (끝점·중간점·중심에 붙습니다, Esc 로 끝)");
  }

  stop() {
    this.p1 = this.p2 = this.cursor = this.snap = null;
  }

  /** 스냅 묻기 — 한 번에 하나만, 밀린 것은 마지막 위치만 */
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

  /** 스냅 지연 중앙값 (ms) */
  latency() {
    if (!this.lat.length) return null;
    const s = [...this.lat].sort((a, b) => a - b);
    return s[Math.floor(s.length / 2)];
  }

  point() {
    return this.snap ? [this.snap.x, this.snap.y] : this.cursor;
  }

  move(m) {
    this.cursor = m;
    this.v.drawOverlay();
    this.query(m);
  }

  click(m) {
    this.cursor = m;
    const p = this.point();
    if (!this.p1 || this.p2) {
      this.p1 = p;
      this.p2 = null;
      this.report("측정: 둘째 점을 누르세요");
    } else {
      this.p2 = p;
      const [dx, dy] = [this.p2[0] - this.p1[0], this.p2[1] - this.p1[1]];
      const ang = (Math.atan2(dy, dx) * 180) / Math.PI;
      this.report(`거리 ${fmt(Math.hypot(dx, dy))} · ΔX ${fmt(dx)} · ΔY ${fmt(dy)} · 각도 ${fmt((ang + 360) % 360)}°`);
    }
    this.v.drawOverlay();
  }

  paint(ctx, toScreen) {
    // 측정선
    const end = this.p2 || (this.p1 && this.point());
    if (this.p1 && end) {
      const [ax, ay] = toScreen(...this.p1), [bx, by] = toScreen(...end);
      ctx.strokeStyle = LINE;
      ctx.lineWidth = 1.5;
      ctx.setLineDash(this.p2 ? [] : [6, 4]);
      ctx.beginPath();
      ctx.moveTo(ax, ay);
      ctx.lineTo(bx, by);
      ctx.stroke();
      ctx.setLineDash([]);
      const d = Math.hypot(end[0] - this.p1[0], end[1] - this.p1[1]);
      const label = fmt(d);
      ctx.font = `600 12px "Pretendard", sans-serif`;
      const w = ctx.measureText(label).width + 10;
      const [mx, my] = [(ax + bx) / 2, (ay + by) / 2];
      ctx.fillStyle = "rgba(20,22,26,0.85)";
      ctx.fillRect(mx - w / 2, my - 20, w, 18);
      ctx.fillStyle = "#fff";
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.fillText(label, mx, my - 11);
    }
    // 스냅 표시
    if (this.snap) {
      const [x, y] = toScreen(this.snap.x, this.snap.y);
      const r = 6;
      ctx.strokeStyle = COLOR;
      ctx.lineWidth = 2;
      ctx.beginPath();
      switch (this.snap.kind) {
        case 0: ctx.rect(x - r, y - r, 2 * r, 2 * r); break;                                   // 끝점 □
        case 1: ctx.moveTo(x, y - r); ctx.lineTo(x + r, y + r); ctx.lineTo(x - r, y + r); ctx.closePath(); break; // 중간점 △
        case 2: ctx.arc(x, y, r, 0, Math.PI * 2); break;                                       // 중심 ○
        case 3: ctx.moveTo(x, y - r); ctx.lineTo(x + r, y); ctx.lineTo(x, y + r); ctx.lineTo(x - r, y); ctx.closePath(); break; // 사분점 ◇
        default: ctx.moveTo(x - r, y - r); ctx.lineTo(x + r, y + r); ctx.moveTo(x + r, y - r); ctx.lineTo(x - r, y + r); // 삽입점 ×
      }
      ctx.stroke();
      ctx.font = `11px "Pretendard", sans-serif`;
      ctx.fillStyle = COLOR;
      ctx.textAlign = "left";
      ctx.textBaseline = "top";
      ctx.fillText(SNAP_NAMES[this.snap.kind] ?? "", x + 9, y + 7);
    }
  }
}
