// 작도·수정·건축 도구 (구현 노트 §25·§26).
//
// 도구는 모두 같은 꼴이다:
//   start()            시작 (안내 문구)
//   click(m)           도면 좌표 m 을 눌렀다 — 스냅·직교를 건 점은 this.pt(m)
//   move(m)            커서가 움직였다 (미리보기)
//   input(text) → bool 명령줄에 친 값. 받았으면 참 (안 받으면 명령 이름으로 본다)
//   enter()            빈 Enter·오른쪽 누르기 — 마치기
//   paint(ctx, ts)     미리보기 그리기
// 편집은 app.apply(req) 하나로 Rust `edit` 명령에 보낸다 (되돌리기 한 단계 = 요청 하나).

const PREVIEW = "#4aa8ff";
const fmt = (v) => (Math.round(v * 10) / 10).toLocaleString("ko-KR");

// ── 모양 JSON (Rust `Shape` serde 꼴) ─────────────────────────────────────────
const V = (p) => ({ x: p[0], y: p[1] });
export const S = {
  line: (a, b) => ({ Line: { a: V(a), b: V(b) } }),
  poly: (pts, closed = false) => ({ Polyline: { points: pts.map((p) => [V(p), 0]), closed } }),
  circle: (c, r) => ({ Circle: { center: V(c), radius: r } }),
  arc: (c, r, start, end) => ({ Arc: { center: V(c), radius: r, start, end } }),
  text: (at, value, height, rotation = 0, halign = 0, valign = "Baseline") => ({
    Text: { at: V(at), height, rotation, width_factor: 1, halign, valign, value },
  }),
};

// ── 아핀 [a, b, c, d, e, f]: x' = a·x + c·y + e, y' = b·x + d·y + f ───────────────
export const A = {
  move: (dx, dy) => [1, 0, 0, 1, dx, dy],
  rotate: (p, t) => {
    const c = Math.cos(t), s = Math.sin(t);
    return [c, s, -s, c, p[0] - c * p[0] + s * p[1], p[1] - s * p[0] - c * p[1]];
  },
  scale: (p, k) => [k, 0, 0, k, p[0] * (1 - k), p[1] * (1 - k)],
  mirror: (p, q) => {
    let ux = q[0] - p[0], uy = q[1] - p[1];
    const l = Math.hypot(ux, uy) || 1;
    ux /= l; uy /= l;
    const a = 2 * ux * ux - 1, b = 2 * ux * uy, d = 2 * uy * uy - 1;
    return [a, b, b, d, p[0] - (a * p[0] + b * p[1]), p[1] - (b * p[0] + d * p[1])];
  },
};
const apply6 = (m, x, y) => [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]];
const dist = (a, b) => Math.hypot(b[0] - a[0], b[1] - a[1]);

function strokePath(ctx, ts, pts, closed = false, dash = false) {
  if (pts.length < 2) return;
  ctx.strokeStyle = PREVIEW;
  ctx.lineWidth = 1.5;
  ctx.setLineDash(dash ? [6, 4] : []);
  ctx.beginPath();
  pts.forEach((p, i) => {
    const [x, y] = ts(p[0], p[1]);
    i ? ctx.lineTo(x, y) : ctx.moveTo(x, y);
  });
  if (closed) ctx.closePath();
  ctx.stroke();
  ctx.setLineDash([]);
}

function label(ctx, ts, at, text) {
  const [x, y] = ts(at[0], at[1]);
  ctx.font = `600 12px "Pretendard", sans-serif`;
  const w = ctx.measureText(text).width + 10;
  ctx.fillStyle = "rgba(20,22,26,0.85)";
  ctx.fillRect(x - w / 2, y - 22, w, 18);
  ctx.fillStyle = "#fff";
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText(text, x, y - 13);
}

/** 도구 바탕 */
class Tool {
  constructor(app) {
    this.app = app;
    this.pts = [];
    this.cur = null;
  }
  get snaps() { return true; }
  base() { return this.pts.at(-1) ?? null; }
  pt(m) { return this.app.snap.point(this.base(), m); }
  say(t) { this.app.prompt(t); }
  start() {}
  stop() {}
  move(m) { this.cur = this.pt(m); }
  /** 친 값을 점으로 받아 click 처럼 */
  input(text) {
    const p = this.app.snap.parse(text, this.base());
    if (!p) return false;
    this.take(p);
    return true;
  }
  click(m) { this.take(this.pt(m)); }
  take() {}
  enter() { this.app.done(); }
  paint() {}
}

// ── 작도 ────────────────────────────────────────────────────────────────────
class LineTool extends Tool {
  start() { this.say("선: 첫 점 (좌표·@상대·거리를 쳐도 됨)"); }
  async take(p) {
    const a = this.base();
    this.pts.push(p);
    if (a) await this.app.apply({ kind: "add", shapes: [S.line(a, p)] });
    this.say("선: 다음 점 (Enter 로 끝, U 로 한 칸 되돌리기, C 로 닫기)");
  }
  async input(t) {
    const k = t.trim().toUpperCase();
    if (k === "U" && this.pts.length > 1) {
      this.pts.pop();
      await this.app.apply({ kind: "undo" });
      return true;
    }
    if (k === "C" && this.pts.length > 2) {
      await this.app.apply({ kind: "add", shapes: [S.line(this.base(), this.pts[0])] });
      this.app.done();
      return true;
    }
    return super.input(t);
  }
  paint(ctx, ts) { if (this.base() && this.cur) strokePath(ctx, ts, [this.base(), this.cur], false, true); }
}

class PlineTool extends Tool {
  start() { this.say("폴리선: 첫 점"); }
  take(p) {
    this.pts.push(p);
    this.say("폴리선: 다음 점 (Enter 로 끝, C 로 닫기, U 로 한 점 빼기)");
  }
  async finish(closed) {
    if (this.pts.length >= 2) await this.app.apply({ kind: "add", shapes: [S.poly(this.pts, closed)] });
    this.app.done();
  }
  input(t) {
    const k = t.trim().toUpperCase();
    if (k === "C" && this.pts.length > 2) { this.finish(true); return true; }
    if (k === "U") { this.pts.pop(); return true; }
    return super.input(t);
  }
  enter() { this.finish(false); }
  paint(ctx, ts) { strokePath(ctx, ts, this.cur ? [...this.pts, this.cur] : this.pts); }
}

class RectTool extends Tool {
  start() { this.say("사각형: 첫 구석"); }
  async take(p) {
    if (!this.pts.length) {
      this.pts.push(p);
      this.say("사각형: 맞은편 구석 (@가로,세로)");
      return;
    }
    const [a] = this.pts;
    await this.app.apply({ kind: "add", shapes: [S.poly([a, [p[0], a[1]], p, [a[0], p[1]]], true)] });
    this.app.done();
  }
  paint(ctx, ts) {
    if (!this.pts.length || !this.cur) return;
    const [a] = this.pts, p = this.cur;
    strokePath(ctx, ts, [a, [p[0], a[1]], p, [a[0], p[1]]], true);
    label(ctx, ts, p, `${fmt(Math.abs(p[0] - a[0]))} × ${fmt(Math.abs(p[1] - a[1]))}`);
  }
}

class CircleTool extends Tool {
  start() { this.say("원: 중심"); }
  async take(p) {
    if (!this.pts.length) {
      this.pts.push(p);
      this.say("원: 반지름 (점 또는 숫자)");
      return;
    }
    const r = dist(this.pts[0], p);
    if (r > 0) await this.app.apply({ kind: "add", shapes: [S.circle(this.pts[0], r)] });
    this.app.done();
  }
  paint(ctx, ts) {
    if (!this.pts.length || !this.cur) return;
    const [c] = this.pts, r = dist(c, this.cur);
    const [x, y] = ts(c[0], c[1]);
    ctx.strokeStyle = PREVIEW;
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.arc(x, y, r * this.app.v.ppu, 0, Math.PI * 2);
    ctx.stroke();
    label(ctx, ts, this.cur, `R ${fmt(r)}`);
  }
}

/** 세 점을 지나는 원 (중심, 반지름) */
function circle3(a, b, c) {
  const d = 2 * (a[0] * (b[1] - c[1]) + b[0] * (c[1] - a[1]) + c[0] * (a[1] - b[1]));
  if (Math.abs(d) < 1e-12) return null;
  const s = (p) => p[0] * p[0] + p[1] * p[1];
  const ux = (s(a) * (b[1] - c[1]) + s(b) * (c[1] - a[1]) + s(c) * (a[1] - b[1])) / d;
  const uy = (s(a) * (c[0] - b[0]) + s(b) * (a[0] - c[0]) + s(c) * (b[0] - a[0])) / d;
  return [[ux, uy], dist([ux, uy], a)];
}

/** 시작·지나는 점·끝 → 반시계 호 (시작각, 끝각) */
function arc3(a, b, c) {
  const k = circle3(a, b, c);
  if (!k) return null;
  const [o, r] = k;
  const ang = (p) => Math.atan2(p[1] - o[1], p[0] - o[0]);
  const norm = (t) => ((t % (2 * Math.PI)) + 2 * Math.PI) % (2 * Math.PI);
  let s = ang(a), e = ang(c);
  // 지나는 점이 s→e 반시계 길에 없으면 거꾸로
  if (norm(ang(b) - s) > norm(e - s)) [s, e] = [e, s];
  return { o, r, s, e };
}

class ArcTool extends Tool {
  start() { this.say("호: 시작점"); }
  async take(p) {
    this.pts.push(p);
    if (this.pts.length === 1) return this.say("호: 지나는 점");
    if (this.pts.length === 2) return this.say("호: 끝점");
    const k = arc3(...this.pts);
    if (k) await this.app.apply({ kind: "add", shapes: [S.arc(k.o, k.r, k.s, k.e)] });
    this.app.done();
  }
  paint(ctx, ts) {
    if (this.pts.length === 2 && this.cur) {
      const k = arc3(this.pts[0], this.pts[1], this.cur);
      if (!k) return;
      const [x, y] = ts(k.o[0], k.o[1]);
      ctx.strokeStyle = PREVIEW;
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      // 화면은 y 가 아래라 각도 부호가 뒤집힌다
      ctx.arc(x, y, k.r * this.app.v.ppu, -k.s, -k.e, true);
      ctx.stroke();
    } else if (this.pts.length === 1 && this.cur) strokePath(ctx, ts, [this.pts[0], this.cur], false, true);
  }
}

class TextTool extends Tool {
  get snaps() { return !this.at; }
  get freeText() { return true; }
  start() {
    this.at = null;
    this.say(`글자: 놓을 점 (지금 높이 ${fmt(this.app.textHeight)} — 숫자를 치면 높이를 바꿈)`);
  }
  take(p) {
    this.at = p;
    this.say("글자: 내용을 치고 Enter (한글 입력 가능, 빈 Enter 로 끝)");
  }
  input(t) {
    if (!this.at) {
      const h = Number(t.trim());
      if (h > 0) {
        this.app.textHeight = h;
        this.start();
        return true;
      }
      return super.input(t);
    }
    if (!t.trim()) return false;
    const at = this.at;
    this.app.apply({ kind: "add", shapes: [S.text(at, t, this.app.textHeight)] });
    // 다음 줄로 이어 쓰기
    this.at = [at[0], at[1] - this.app.textHeight * 1.6];
    this.say("글자: 다음 줄 (빈 Enter 로 끝)");
    return true;
  }
  paint(ctx, ts) {
    const p = this.at || this.cur;
    if (!p) return;
    const [x, y] = ts(p[0], p[1]);
    const h = this.app.textHeight * this.app.v.ppu;
    ctx.strokeStyle = PREVIEW;
    ctx.strokeRect(x, y - h, Math.max(h * 3, 12), h);
  }
}

/** 정렬 치수: 두 점, 치수선 위치 → 치수 객체 (값은 두 점에서 늘 새로 셈 — 구현 노트 §28). 미리보기만 여기서 그린다 */
class DimTool extends Tool {
  start() {
    this.chain = null;
    this.say("치수: 첫 점 (벽·그리드·기둥 위 점은 그 객체에 묶여 따라감)");
  }
  base() { return this.chain && !this.pts.length ? this.chain.from : super.base(); }
  input(t) {
    const k = t.trim().toUpperCase();
    if (k === "N" && this.chain) {
      this.start();
      return true;
    }
    // B: 누적(기준선) 치수 ↔ C: 연속 치수 (§36)
    if ((k === "B" || k === "C") && this.chain) {
      this.chain.mode = k === "B" ? "b" : "c";
      this.chain.k = 0;
      this.sayChain();
      return true;
    }
    return super.input(t);
  }
  sayChain() {
    this.say(this.chain.mode === "b"
      ? "치수: 누적 치수 — 다음 점 (첫 점에서 잼, C 연속, N 새 치수, Enter 끝)"
      : "치수: 연속 치수 — 다음 점 (B 누적, N 새 치수, Enter 끝)");
  }
  op(a, b, at) {
    return { kind: "arch", op: { kind: "dim", a, b, at, text_height: this.app.textHeight, arrow: this.app.dimArrow, plain: this.app.dimPlain, precision: this.app.dimPrec ?? 0 } };
  }
  /** 연속·누적 치수의 다음 점: [시작점, 치수선 점] */
  next(p) {
    const c = this.chain;
    if (c.mode === "b") {
      const off = c.off + Math.sign(c.off || 1) * this.app.textHeight * 3 * (c.k + 1);
      return [c.base, [c.base[0] + c.n[0] * off, c.base[1] + c.n[1] * off]];
    }
    return [c.from, [c.from[0] + c.n[0] * c.off, c.from[1] + c.n[1] * c.off]];
  }
  async chainTo(p) {
    const [a, at] = this.next(p);
    await this.app.apply(this.op(a, p, at));
    this.chain.from = p;
    this.chain.k += 1;
    this.pts = [];
  }
  shapes(a, b, at) {
    const L = dist(a, b);
    if (L < 1e-9) return [];
    const u = [(b[0] - a[0]) / L, (b[1] - a[1]) / L], n = [-u[1], u[0]];
    const off = (at[0] - a[0]) * n[0] + (at[1] - a[1]) * n[1];
    const sg = off >= 0 ? 1 : -1;
    const h = this.app.textHeight, gap = h * 0.4, ext = h * 0.6, tk = h * 0.5;
    const P = (p, k) => [p[0] + n[0] * k, p[1] + n[1] * k];
    const da = P(a, off), db = P(b, off);
    const tick = (p) => {
      const d = [(u[0] + n[0]) * tk * 0.5, (u[1] + n[1]) * tk * 0.5];
      return S.line([p[0] - d[0], p[1] - d[1]], [p[0] + d[0], p[1] + d[1]]);
    };
    let rot = Math.atan2(u[1], u[0]);
    if (rot > Math.PI / 2 + 1e-9 || rot <= -Math.PI / 2 + 1e-9) rot += Math.PI; // 읽을 수 있게
    const up = [-Math.sin(rot), Math.cos(rot)];
    const mid = [(da[0] + db[0]) / 2 + up[0] * h * 0.3, (da[1] + db[1]) / 2 + up[1] * h * 0.3];
    return [
      S.line(P(a, sg * gap), P(a, off + sg * ext)),
      S.line(P(b, sg * gap), P(b, off + sg * ext)),
      S.line(P(da, 0), P(db, 0)),
      tick(da),
      tick(db),
      S.text(mid, Math.round(L).toLocaleString("en-US"), h, rot, 0.5, "Bottom"),
    ];
  }
  async take(p) {
    if (this.chain && !this.pts.length) return this.chainTo(p);
    this.pts.push(p);
    if (this.pts.length === 1) return this.say("치수: 둘째 점");
    if (this.pts.length === 2) return this.say("치수: 치수선 위치");
    const [a, b, at] = this.pts;
    await this.app.apply(this.op(a, b, at));
    // 연속 치수: 앞 치수 끝에서 같은 치수선으로 이어 간다 (B 면 첫 점에서 누적)
    const L = Math.hypot(b[0] - a[0], b[1] - a[1]) || 1;
    const n = [-(b[1] - a[1]) / L, (b[0] - a[0]) / L];
    this.chain = { from: b, base: a, n, off: (at[0] - a[0]) * n[0] + (at[1] - a[1]) * n[1], mode: "c", k: 0 };
    this.pts = [];
    this.sayChain();
  }
  paint(ctx, ts) {
    if (!this.cur) return;
    if (this.pts.length === 1) strokePath(ctx, ts, [this.pts[0], this.cur], false, true);
    const c = this.chain;
    const trio = this.pts.length === 2 ? [this.pts[0], this.pts[1], this.cur]
      : c && !this.pts.length ? (() => { const [a, at] = this.next(this.cur); return [a, this.cur, at]; })() : null;
    if (trio) {
      for (const s of this.shapes(...trio)) {
        if (s.Line) strokePath(ctx, ts, [[s.Line.a.x, s.Line.a.y], [s.Line.b.x, s.Line.b.y]]);
        else label(ctx, ts, [s.Text.at.x, s.Text.at.y], s.Text.value);
      }
    }
  }
}

class MeasureTool extends Tool {
  start() { this.say("거리 재기: 첫 점 (끝점·중간점·중심에 붙음)"); }
  take(p) {
    if (this.pts.length !== 1) {
      this.pts = [p];
      this.done = null;
      return this.say("거리 재기: 둘째 점");
    }
    const a = this.pts[0], dx = p[0] - a[0], dy = p[1] - a[1];
    const ang = ((Math.atan2(dy, dx) * 180) / Math.PI + 360) % 360;
    this.done = p;
    this.pts = [a, p];
    this.app.report(`거리 ${fmt(Math.hypot(dx, dy))} · ΔX ${fmt(dx)} · ΔY ${fmt(dy)} · 각도 ${fmt(ang)}°`);
    this.say("거리 재기: 다시 첫 점 (Esc 로 끝)");
  }
  base() { return this.pts.length === 1 ? this.pts[0] : null; }
  paint(ctx, ts) {
    const a = this.pts[0], b = this.pts[1] || this.cur;
    if (!a || !b) return;
    strokePath(ctx, ts, [a, b], false, !this.pts[1]);
    label(ctx, ts, [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2], fmt(dist(a, b)));
  }
}

// ── 수정 (고른 객체에) ────────────────────────────────────────────────────────
/** 고른 것이 없으면 먼저 고르게 한다: 누르기·상자로 고르고 Enter */
class ModifyTool extends Tool {
  get picking() { return this.phase === "pick"; }
  get snaps() { return !this.picking; }
  get drag() { return this.picking ? (a, b, c) => this.app.select.drag(a, b, c) : undefined; }
  dragEnd(a, b, c) { if (this.picking) this.app.select.dragEnd(a, b, c, true); }
  start() {
    if (this.app.select.ids.length || (this.archOk && this.app.select.arch)) this.begin();
    else {
      this.phase = "pick";
      this.say(`${this.title}: 객체를 고르고 Enter (누르기·상자)`);
    }
  }
  click(m) {
    if (this.picking) this.app.select.click(m, { shiftKey: true });
    else super.click(m);
  }
  enter() {
    if (this.picking && (this.app.select.ids.length || (this.archOk && this.app.select.arch))) this.begin();
    else this.app.done();
  }
  begin() {
    this.phase = "go";
    this.pts = [];
    this.go();
  }
  /** 고른 것 미리보기 (변환 m 을 걸어) */
  ghost(ctx, ts, m) {
    const o = this.app.select.info?.outline;
    if (!o) return;
    ctx.strokeStyle = PREVIEW;
    ctx.globalAlpha = 0.7;
    ctx.lineWidth = 1;
    ctx.beginPath();
    for (const [x0, y0, x1, y1] of o.length > 20000 ? o.slice(0, 20000) : o) {
      const [ax, ay] = ts(...apply6(m, x0, y0)), [bx, by] = ts(...apply6(m, x1, y1));
      ctx.moveTo(ax, ay);
      ctx.lineTo(bx, by);
    }
    ctx.stroke();
    ctx.globalAlpha = 1;
  }
  async transform(m, copy = false) {
    const ids = this.app.select.ids;
    const st = await this.app.apply({ kind: copy ? "copy" : "transform", ids, m });
    if (st) await this.app.select.set(copy ? st.added : ids);
  }
}

class MoveTool extends ModifyTool {
  get title() { return this.copy ? "복사" : "이동"; }
  get archOk() { return !this.copy; }
  go() { this.say(`${this.title}: 기준점`); }
  async take(p) {
    if (this.picking) return;
    if (!this.pts.length) {
      this.pts.push(p);
      return this.say(`${this.title}: 옮길 곳 (@dx,dy 또는 거리)`);
    }
    const [a] = this.pts, dx = p[0] - a[0], dy = p[1] - a[1];
    const arch = this.app.select.arch;
    if (arch && !this.app.select.ids.length) {
      await this.app.apply({ kind: "arch", op: { kind: "move", what: arch.what, id: arch.id, dx, dy } });
      this.app.select.clear();
      return this.app.done();
    }
    await this.transform(A.move(dx, dy), this.copy);
    if (this.copy) this.say("복사: 다음 곳 (Enter 로 끝)");
    else this.app.done();
  }
  paint(ctx, ts) {
    if (this.pts.length && this.cur) {
      strokePath(ctx, ts, [this.pts[0], this.cur], false, true);
      this.ghost(ctx, ts, A.move(this.cur[0] - this.pts[0][0], this.cur[1] - this.pts[0][1]));
    }
  }
}
class CopyTool extends MoveTool {
  constructor(app) { super(app); this.copy = true; }
}

class RotateTool extends ModifyTool {
  get title() { return "회전"; }
  go() { this.say("회전: 기준점"); }
  angleTo(p) { return Math.atan2(p[1] - this.pts[0][1], p[0] - this.pts[0][0]); }
  async take(p) {
    if (this.picking) return;
    if (!this.pts.length) {
      this.pts.push(p);
      return this.say("회전: 각도 (점 또는 숫자 °)");
    }
    await this.transform(A.rotate(this.pts[0], this.angleTo(p)));
    this.app.done();
  }
  input(t) {
    if (this.pts.length === 1 && /^-?[\d.]+$/.test(t.trim())) {
      this.transform(A.rotate(this.pts[0], (Number(t) * Math.PI) / 180)).then(() => this.app.done());
      return true;
    }
    return super.input(t);
  }
  paint(ctx, ts) {
    if (this.pts.length && this.cur) {
      strokePath(ctx, ts, [this.pts[0], this.cur], false, true);
      this.ghost(ctx, ts, A.rotate(this.pts[0], this.angleTo(this.cur)));
      label(ctx, ts, this.cur, `${fmt((((this.angleTo(this.cur) * 180) / Math.PI) + 360) % 360)}°`);
    }
  }
}

class MirrorTool extends ModifyTool {
  get title() { return "대칭"; }
  go() { this.say("대칭: 대칭선 첫 점 (원본은 남김)"); }
  async take(p) {
    if (this.picking) return;
    if (!this.pts.length) {
      this.pts.push(p);
      return this.say("대칭: 대칭선 둘째 점");
    }
    await this.transform(A.mirror(this.pts[0], p), true);
    this.app.done();
  }
  paint(ctx, ts) {
    if (this.pts.length && this.cur) {
      strokePath(ctx, ts, [this.pts[0], this.cur], false, true);
      if (dist(this.pts[0], this.cur) > 0) this.ghost(ctx, ts, A.mirror(this.pts[0], this.cur));
    }
  }
}

class ScaleTool extends ModifyTool {
  get title() { return "크기"; }
  go() { this.say("크기: 기준점"); }
  take(p) {
    if (this.picking) return;
    if (!this.pts.length) {
      this.pts.push(p);
      this.say("크기: 배율을 치세요 (예: 2, 0.5)");
    }
  }
  input(t) {
    const k = Number(t.trim());
    if (this.pts.length === 1 && k > 0) {
      this.transform(A.scale(this.pts[0], k)).then(() => this.app.done());
      return true;
    }
    return super.input(t);
  }
}

class EraseTool extends ModifyTool {
  get title() { return "지우기"; }
  get archOk() { return true; }
  async go() {
    const s = this.app.select;
    if (s.arch && !s.ids.length) await this.app.apply({ kind: "arch", op: { kind: "delete", what: s.arch.what, id: s.arch.id } });
    else await this.app.apply({ kind: "delete", ids: s.ids });
    s.clear();
    this.app.done();
  }
}

class OffsetTool extends Tool {
  get snaps() { return false; }
  start() {
    this.target = null;
    this.say(`간격 띄우기: 거리 (지금 ${fmt(this.app.offsetDist)}, Enter 로 그대로) 또는 객체를 누르세요`);
  }
  input(t) {
    const d = Number(t.trim());
    if (d > 0) {
      this.app.offsetDist = d;
      this.say(`간격 띄우기 ${fmt(d)}: 객체를 누르세요`);
      return true;
    }
    return false;
  }
  enter() {
    if (!this.target) this.app.done();
  }
  async click(m) {
    if (!this.target) {
      const r = await this.app.invoke("pick_at", { x: m[0], y: m[1], r: 6 / this.app.v.ppu });
      if (!r || !r.editable) return this.app.report("간격 띄울 선·폴리선·원·호를 누르세요");
      this.target = r;
      this.app.select.showPick(r);
      return this.say("간격 띄우기: 띄울 쪽을 누르세요");
    }
    const st = await this.app.apply({ kind: "offset", id: this.target.index, dist: this.app.offsetDist, side: m });
    this.target = null;
    this.app.select.clear();
    if (st && !st.added.length) this.app.report("이 객체는 간격 띄우기를 할 수 없습니다");
    this.say(`간격 띄우기 ${fmt(this.app.offsetDist)}: 다음 객체 (Enter 로 끝)`);
  }
}

// ── 건축 객체 (M3) ───────────────────────────────────────────────────────────
const WALL_KINDS = { 외벽: "Exterior", 내벽: "Interior", 내력벽: "Structural" };

class WallTool extends Tool {
  start() {
    const k = this.app.wallKind;
    this.say(`벽 (${k} ${fmt(this.app.wallThickness)}): 첫 점 — 숫자를 치면 두께, 외벽·내벽·내력벽을 치면 종류`);
  }
  input(t) {
    const s = t.trim();
    if (WALL_KINDS[s]) {
      this.app.wallKind = s;
      this.pts.length ? this.say(`벽 (${s}): 다음 점`) : this.start();
      return true;
    }
    if (!this.pts.length && /^[\d.]+$/.test(s) && Number(s) > 0) {
      this.app.wallThickness = Number(s);
      this.start();
      return true;
    }
    const k = s.toUpperCase();
    if (k === "C" && this.pts.length > 2) { this.finish(true); return true; }
    if (k === "U") { this.pts.pop(); return true; }
    return super.input(t);
  }
  take(p) {
    this.pts.push(p);
    this.say("벽: 다음 점 (Enter 로 끝, C 로 닫기, U 로 한 점 빼기)");
  }
  async finish(closed) {
    if (this.pts.length >= 2) {
      await this.app.apply({
        kind: "arch",
        op: { kind: "walls", points: this.pts, closed, thickness: this.app.wallThickness, wall: WALL_KINDS[this.app.wallKind] },
      });
    }
    this.app.done();
  }
  enter() { this.finish(false); }
  paint(ctx, ts) {
    const pts = this.cur ? [...this.pts, this.cur] : this.pts;
    const h = this.app.wallThickness / 2;
    for (let i = 0; i + 1 < pts.length; i++) {
      const a = pts[i], b = pts[i + 1], L = dist(a, b);
      if (L < 1e-9) continue;
      const n = [-(b[1] - a[1]) / L * h, (b[0] - a[0]) / L * h];
      strokePath(ctx, ts, [[a[0] + n[0], a[1] + n[1]], [b[0] + n[0], b[1] + n[1]], [b[0] - n[0], b[1] - n[1]], [a[0] - n[0], a[1] - n[1]]], true);
    }
    if (this.pts.length && this.cur) label(ctx, ts, this.cur, fmt(dist(this.pts.at(-1), this.cur)));
  }
}

/** 곡선 벽: 시작점 → 지나는 점 → 끝점 (§54). 두께·종류는 벽 도구와 같다 */
class ArcWallTool extends Tool {
  start() { this.say(`곡선 벽 (${this.app.wallKind} ${fmt(this.app.wallThickness)}): 시작점 — 숫자를 치면 두께`); }
  input(t) {
    const s = t.trim();
    if (WALL_KINDS[s]) { this.app.wallKind = s; this.start(); return true; }
    if (!this.pts.length && /^[\d.]+$/.test(s) && Number(s) > 0) {
      this.app.wallThickness = Number(s);
      this.start();
      return true;
    }
    return super.input(t);
  }
  async take(p) {
    this.pts.push(p);
    if (this.pts.length === 1) return this.say("곡선 벽: 지나는 점");
    if (this.pts.length === 2) return this.say("곡선 벽: 끝점");
    const [a, m, b] = this.pts;
    this.pts = [];
    await this.app.apply({
      kind: "arch",
      op: { kind: "arc_wall", a, m, b, thickness: this.app.wallThickness, wall: WALL_KINDS[this.app.wallKind] },
    });
    this.say("곡선 벽: 다음 시작점 (Enter 로 끝)");
  }
  enter() { this.app.done(); }
  paint(ctx, ts) {
    if (this.pts.length === 2 && this.cur) {
      const k = arc3(this.pts[0], this.pts[1], this.cur);
      if (!k) return;
      const [x, y] = ts(k.o[0], k.o[1]);
      ctx.strokeStyle = PREVIEW;
      ctx.lineWidth = 1.5;
      const h = this.app.wallThickness / 2;
      for (const r of [k.r - h, k.r + h]) {
        if (r <= 0) continue;
        ctx.beginPath();
        ctx.arc(x, y, r * this.app.v.ppu, -k.s, -k.e, true);
        ctx.stroke();
      }
    } else if (this.pts.length === 1 && this.cur) strokePath(ctx, ts, [this.pts[0], this.cur], false, true);
  }
}

class OpeningTool extends Tool {
  get door() { return this.kind === "Door"; }
  get width() { return this.door ? this.app.doorWidth : this.app.windowWidth; }
  start() {
    this.say(`${this.door ? "문" : "창"} (폭 ${fmt(this.width)}): 벽 위를 누르세요 — 숫자를 치면 폭`);
  }
  get snaps() { return false; }
  input(t) {
    const w = Number(t.trim());
    if (w > 0) {
      if (this.door) this.app.doorWidth = w;
      else this.app.windowWidth = w;
      this.start();
      return true;
    }
    return super.input(t);
  }
  async take(p) {
    const op = this.door
      ? { kind: "opening", at: p, opening: "Door", width: this.width, height: 2100, sill: 0 }
      : { kind: "opening", at: p, opening: "Window", width: this.width, height: 1200, sill: 900 };
    await this.app.apply({ kind: "arch", op });
  }
  paint(ctx, ts) {
    if (!this.cur) return;
    const [x, y] = ts(...this.cur), r = (this.width / 2) * this.app.v.ppu;
    ctx.strokeStyle = PREVIEW;
    ctx.setLineDash([4, 3]);
    ctx.beginPath();
    ctx.arc(x, y, r, 0, Math.PI * 2);
    ctx.stroke();
    ctx.setLineDash([]);
  }
}
class DoorTool extends OpeningTool { constructor(a) { super(a); this.kind = "Door"; } }
class WindowTool extends OpeningTool { constructor(a) { super(a); this.kind = "Window"; } }

class RoomTool extends Tool {
  get snaps() { return false; }
  get freeText() { return !!this.at; }
  start() {
    this.at = null;
    this.say("실: 벽으로 둘러싸인 안쪽을 누르세요 (경계를 찾아 면적을 셈)");
  }
  take(p) {
    this.at = p;
    this.say("실: 이름을 치고 Enter (빈 Enter 면 「실」)");
  }
  async put(name) {
    const at = this.at;
    this.at = null;
    try {
      await this.app.apply({ kind: "arch", op: { kind: "room_at", at, name: name || "실" } }, true);
      this.say("실: 다음 실 안쪽 (Enter 로 끝)");
    } catch (e) {
      this.app.report(`실을 만들지 못했습니다: ${e}`);
      this.start();
    }
  }
  input(t) {
    if (!this.at) return super.input(t);
    this.put(t.trim());
    return true;
  }
  enter() {
    if (this.at) this.put("");
    else this.app.done();
  }
}

class GridTool extends Tool {
  get freeText() { return this.pts.length === 2; }
  start() { this.say("그리드: 첫 점"); }
  take(p) {
    if (this.pts.length >= 2) return;
    this.pts.push(p);
    if (this.pts.length === 1) return this.say("그리드: 끝점");
    this.say(`그리드: 기호를 치고 Enter (빈 Enter 면 ${this.auto()})`);
  }
  auto() { return String((this.app.doc?.arch?.grids ?? 0) + 1); }
  async put(label) {
    await this.app.apply({ kind: "arch", op: { kind: "grid", a: this.pts[0], b: this.pts[1], label: label || this.auto() } });
    this.pts = [];
    this.say("그리드: 다음 첫 점 (Enter 로 끝)");
  }
  input(t) {
    if (this.pts.length === 2) { this.put(t.trim()); return true; }
    return super.input(t);
  }
  enter() {
    if (this.pts.length === 2) this.put("");
    else this.app.done();
  }
  paint(ctx, ts) { if (this.pts.length === 1 && this.cur) strokePath(ctx, ts, [this.pts[0], this.cur], false, true); }
}

/** 그리드 여럿: 「6000,6000,6000 / 7200,7200」 처럼 X 간격 / Y 간격, 그다음 원점 */
class GridSetTool extends Tool {
  start() {
    this.spec = null;
    this.say("그리드 세트: X 간격 / Y 간격 (예: 6000,6000,6000 / 7200,7200)");
  }
  get snaps() { return !!this.spec; }
  get freeText() { return !this.spec; }
  input(t) {
    if (!this.spec) {
      const [xs, ys = ""] = t.split("/");
      const nums = (s) => s.split(/[,\s]+/).filter(Boolean).map(Number).filter((v) => v > 0);
      this.spec = { xs: nums(xs), ys: nums(ys) };
      if (!this.spec.xs.length && !this.spec.ys.length) {
        this.spec = null;
        return true;
      }
      this.say("그리드 세트: 원점(X1·Y1 교점)을 누르세요");
      return true;
    }
    return super.input(t);
  }
  async take(p) {
    if (!this.spec) return;
    await this.app.apply({ kind: "arch", op: { kind: "grid_set", origin: p, xs: this.spec.xs, ys: this.spec.ys, extend: 1500 } });
    this.app.done();
  }
  paint(ctx, ts) {
    if (!this.spec || !this.cur) return;
    const o = this.cur, sx = [0], sy = [0];
    for (const v of this.spec.xs) sx.push(sx.at(-1) + v);
    for (const v of this.spec.ys) sy.push(sy.at(-1) + v);
    const W = sx.at(-1), H = sy.at(-1);
    for (const x of sx) strokePath(ctx, ts, [[o[0] + x, o[1] - 1500], [o[0] + x, o[1] + H + 1500]], false, true);
    for (const y of sy) strokePath(ctx, ts, [[o[0] - 1500, o[1] + y], [o[0] + W + 1500, o[1] + y]], false, true);
  }
}

class SiteTool extends PlineTool {
  start() { this.say("대지 경계: 첫 점"); }
  take(p) {
    this.pts.push(p);
    this.say("대지 경계: 다음 점 (Enter 로 닫고 끝)");
  }
  async finish() {
    if (this.pts.length >= 3) {
      await this.app.apply({
        kind: "arch",
        op: { kind: "site", boundary: this.pts, zone: "제2종일반주거지역", coverage_limit: 60, far_limit: 200 },
      });
    }
    this.app.done();
  }
  paint(ctx, ts) { strokePath(ctx, ts, this.cur ? [...this.pts, this.cur] : this.pts, true, true); }
}

// ── 자르기·연장·모깎기·해치 (§27) ─────────────────────────────────────────────
/** 누른 객체 하나에 편집 (자르기·연장): 되풀이 */
class PickEditTool extends Tool {
  get snaps() { return false; }
  start() { this.say(`${this.title}: ${this.hint} (Enter 로 끝)`); }
  /** 좌표를 쳐도 그 점을 누른 것으로 */
  input(t) {
    const p = this.app.snap.parse(t, null);
    if (!p) return false;
    this.click(p);
    return true;
  }
  async click(m) {
    const r = await this.app.invoke("pick_at", { x: m[0], y: m[1], r: 6 / this.app.v.ppu });
    if (!r) return;
    if (!r.editable) return this.app.report("선·호·원·폴리선을 누르세요 (건축 객체 선은 속성으로 고칩니다)");
    await this.app.apply({ kind: this.kind, id: r.index, pick: m });
  }
}
class TrimTool extends PickEditTool {
  get title() { return "자르기"; }
  get kind() { return "trim"; }
  get hint() { return "없앨 부분을 누르세요 — 다른 선과 만나는 곳에서 끊음"; }
}
class ExtendTool extends PickEditTool {
  get title() { return "연장"; }
  get kind() { return "extend"; }
  get hint() { return "늘릴 끝 가까이를 누르세요 — 처음 만나는 선까지"; }
}

class FilletTool extends Tool {
  get snaps() { return false; }
  start() {
    this.first = null;
    this.say(`모깎기 (반지름 ${fmt(this.app.filletRadius)} — 숫자로 바꿈, 0 이면 모서리): 첫 선 (폴리선은 모서리 양쪽 변)`);
  }
  input(t) {
    const r = Number(t.trim());
    if (t.trim() !== "" && r >= 0) {
      this.app.filletRadius = r;
      this.start();
      return true;
    }
    const p = this.app.snap.parse(t, null);
    if (!p) return false;
    this.click(p);
    return true;
  }
  async click(m) {
    const r = await this.app.invoke("pick_at", { x: m[0], y: m[1], r: 6 / this.app.v.ppu });
    if (!r?.editable) return this.app.report("선을 누르세요");
    if (!this.first) {
      this.first = { id: r.index, pick: m };
      this.app.select.showPick(r);
      return this.say("모깎기: 둘째 선");
    }
    const f = this.first;
    this.first = null;
    this.app.select.clear();
    await this.app.apply({ kind: "fillet", a: f.id, pa: f.pick, b: r.index, pb: m, r: this.app.filletRadius });
    this.say(`모깎기 (반지름 ${fmt(this.app.filletRadius)}): 다음 첫 선 (Enter 로 끝)`);
  }
}

/** 모따기: 두 선 또는 폴리선 모서리 양쪽 변. 「거리」 또는 「거리1/거리2」 */
class ChamferTool extends FilletTool {
  start() {
    this.first = null;
    const [a, b] = this.app.chamfer;
    this.say(`모따기 (${fmt(a)} / ${fmt(b)} — 「500」 또는 「500/300」 으로 바꿈): 첫 선 (폴리선은 모서리 양쪽 변)`);
  }
  input(t) {
    const m = t.trim().match(/^([\d.]+)(?:\/([\d.]+))?$/);
    if (m) {
      this.app.chamfer = [Number(m[1]), Number(m[2] ?? m[1])];
      this.start();
      return true;
    }
    const p = this.app.snap.parse(t, null);
    if (!p) return false;
    this.click(p);
    return true;
  }
  async click(m) {
    const r = await this.app.invoke("pick_at", { x: m[0], y: m[1], r: 6 / this.app.v.ppu });
    if (!r?.editable) return this.app.report("선을 누르세요");
    if (!this.first) {
      this.first = { id: r.index, pick: m };
      this.app.select.showPick(r);
      return this.say("모따기: 둘째 선");
    }
    const f = this.first;
    this.first = null;
    this.app.select.clear();
    const [d1, d2] = this.app.chamfer;
    await this.app.apply({ kind: "chamfer", a: f.id, pa: f.pick, b: r.index, pb: m, d1, d2 });
    this.start();
  }
}

const PATTERNS = { SOLID: "SOLID", S: "SOLID", 칠: "SOLID", 사선: "사선", ANSI31: "사선", 격자: "격자", NET: "격자", 콘크리트: "콘크리트", AR_CONC: "콘크리트" };
class HatchTool extends Tool {
  get snaps() { return false; }
  start() {
    this.say(`해치 (${this.app.hatchPattern}, 간격 ${fmt(this.app.hatchSpacing)}): 닫힌 영역 안을 누르세요 — SOLID·사선·격자·콘크리트, 숫자는 간격`);
  }
  input(t) {
    const k = t.trim();
    const p = PATTERNS[k] || PATTERNS[k.toUpperCase()];
    if (p) {
      this.app.hatchPattern = p;
      this.start();
      return true;
    }
    if (Number(k) > 0) {
      this.app.hatchSpacing = Number(k);
      this.start();
      return true;
    }
    const pt = this.app.snap.parse(t, null);
    if (!pt) return false;
    this.click(pt);
    return true;
  }
  async click(m) {
    await this.app.apply({ kind: "hatch", at: m, pattern: this.app.hatchPattern, spacing: this.app.hatchSpacing });
  }
}

// ── 기둥·계단 (M3) ────────────────────────────────────────────────────────────
class ColumnTool extends Tool {
  start() { this.say(`기둥 (${fmt(this.app.colW)}×${fmt(this.app.colD)}): 가운데를 누르세요 — 「600x400」 처럼 치면 크기`); }
  input(t) {
    const m = t.trim().match(/^([\d.]+)\s*[x×*]\s*([\d.]+)$/i) || t.trim().match(/^([\d.]+)$/);
    if (m) {
      this.app.colW = Number(m[1]);
      this.app.colD = Number(m[2] ?? m[1]);
      this.start();
      return true;
    }
    return super.input(t);
  }
  async take(p) {
    await this.app.apply({ kind: "arch", op: { kind: "column", at: p, width: this.app.colW, depth: this.app.colD } });
  }
  paint(ctx, ts) {
    if (!this.cur) return;
    const [w, d] = [this.app.colW / 2, this.app.colD / 2], c = this.cur;
    strokePath(ctx, ts, [[c[0] - w, c[1] - d], [c[0] + w, c[1] - d], [c[0] + w, c[1] + d], [c[0] - w, c[1] + d]], true);
  }
}

/** 보: 점을 이어 누르면 앞 점에서 이어진 보들 (Enter·Esc 로 끝). 「300x600」 처럼 치면 폭×춤 (§52) */
class BeamTool extends Tool {
  start() { this.say(`보 (폭×춤 ${fmt(this.app.beamW)}×${fmt(this.app.beamD)}): 첫 점 — 「300x600」 처럼 치면 크기`); }
  input(t) {
    const m = t.trim().match(/^([\d.]+)\s*[x×*]\s*([\d.]+)$/i);
    if (m) {
      this.app.beamW = Number(m[1]);
      this.app.beamD = Number(m[2]);
      if (this.pts.length) this.say(`보 (${fmt(this.app.beamW)}×${fmt(this.app.beamD)}): 다음 점`);
      else this.start();
      return true;
    }
    return super.input(t);
  }
  async take(p) {
    const a = this.pts[this.pts.length - 1];
    this.pts = [p];
    if (!a) return this.say("보: 다음 점 (Enter 로 끝)");
    await this.app.apply({ kind: "arch", op: { kind: "beam", a, b: p, width: this.app.beamW, depth: this.app.beamD } });
    this.say("보: 다음 점 (Enter 로 끝)");
  }
  enter() { this.app.done(); }
  paint(ctx, ts) {
    const a = this.pts[0], b = this.cur;
    if (!a || !b) return;
    const l = Math.hypot(b[0] - a[0], b[1] - a[1]);
    if (l < 1e-9) return;
    const w = this.app.beamW / 2, nx = -(b[1] - a[1]) / l * w, ny = (b[0] - a[0]) / l * w;
    ctx.setLineDash([6, 4]);
    strokePath(ctx, ts, [[a[0] + nx, a[1] + ny], [b[0] + nx, b[1] + ny], [b[0] - nx, b[1] - ny], [a[0] - nx, a[1] - ny]], true);
    ctx.setLineDash([]);
  }
}

class StairTool extends Tool {
  start() {
    this.spiral = false;
    this.say(`계단 (폭 ${fmt(this.app.stairWidth)}, ${this.app.stairRisers} 단): 오르기 시작점 — 숫자는 폭, R18 은 단 수, S 는 돌음 계단`);
  }
  input(t) {
    const k = t.trim();
    // 돌음 계단 (§43): S → 중심 → 중심선 위 시작점 → 회전각
    if (/^S$/i.test(k) && !this.pts.length) {
      this.spiral = true;
      this.say("돌음 계단: 중심");
      return true;
    }
    if (this.spiral && this.pts.length === 2 && /^-?[\d.]+$/.test(k)) {
      const [a, b] = this.pts;
      this.app.apply({ kind: "arch", op: { kind: "stair", a, b, mid: null, spiral: Number(k), width: this.app.stairWidth, risers: this.app.stairRisers } })
        .then(() => this.app.done());
      return true;
    }
    const r = k.match(/^R(\d+)$/i);
    if (r) {
      this.app.stairRisers = Number(r[1]);
      this.pts.length ? this.say(`계단 (${this.app.stairRisers} 단): 끝점`) : this.start();
      return true;
    }
    if (!this.pts.length && /^[\d.]+$/.test(k) && Number(k) > 0) {
      this.app.stairWidth = Number(k);
      this.start();
      return true;
    }
    return super.input(t);
  }
  async take(p) {
    if (this.spiral && this.pts.length >= 2) return;
    this.pts.push(p);
    if (this.spiral) {
      return this.say(this.pts.length === 1 ? "돌음 계단: 중심선 위 시작점 (중심에서 거리 = 중심선 반지름)" : "돌음 계단: 회전각을 치세요 (도, 예: 270 · 음수면 시계 방향)");
    }
    if (this.pts.length === 1) return this.say("계단: 다음 점 (거리를 쳐도 됨)");
    if (this.pts.length === 2) return this.say("계단: Enter 면 곧은 계단 — 꺾으려면 다음 점 (둘째 점에 참, 옆으로 돌아오면 U자)");
    const [a, mid, b] = this.pts;
    await this.put(a, b, mid);
  }
  async put(a, b, mid = null) {
    await this.app.apply({ kind: "arch", op: { kind: "stair", a, b, mid, width: this.app.stairWidth, risers: this.app.stairRisers } });
    this.app.done();
  }
  enter() {
    if (this.pts.length === 2 && !this.spiral) this.put(this.pts[0], this.pts[1]);
    else this.app.done();
  }
  paint(ctx, ts) {
    if (!this.pts.length || !this.cur) return;
    if (this.pts.length === 2) {
      strokePath(ctx, ts, [this.pts[0], this.pts[1], this.cur], false, true);
      return;
    }
    const a = this.pts[0], b = this.cur, L = dist(a, b);
    if (L < 1e-9) return;
    const h = this.app.stairWidth / 2, n = [-(b[1] - a[1]) / L * h, (b[0] - a[0]) / L * h];
    strokePath(ctx, ts, [[a[0] + n[0], a[1] + n[1]], [b[0] + n[0], b[1] + n[1]], [b[0] - n[0], b[1] - n[1]], [a[0] - n[0], a[1] - n[1]]], true);
    label(ctx, ts, b, `${fmt(L)} · 디딤판 ${fmt(L / this.app.stairRisers)}`);
  }
}

// ── 블록 (§30) ───────────────────────────────────────────────────────────────
class BlockTool extends ModifyTool {
  get title() { return "블록 만들기"; }
  get freeText() { return this.pts.length === 1; }
  go() { this.say("블록 만들기: 기준점 (넣을 때 이 점이 커서에 붙음)"); }
  take(p) {
    if (this.picking || this.pts.length) return;
    this.pts.push(p);
    this.say(`블록 만들기: 이름을 치고 Enter${this.app.doc?.blocks?.length ? ` (있는 것: ${this.app.doc.blocks.join(", ")})` : ""}`);
  }
  input(t) {
    if (this.pts.length === 1) {
      const name = t.trim();
      if (!name) return true;
      this.app.apply({ kind: "make_block", name, ids: this.app.select.ids, base: this.pts[0] }).then((st) => {
        if (st) {
          this.app.select.set(st.added);
          this.app.report(`블록 「${name}」 을 만들었습니다 — I 로 넣기, X 로 펼치기`);
          this.app.done();
        }
      });
      return true;
    }
    return super.input(t);
  }
}

class InsertTool extends Tool {
  get freeText() { return !this.name; }
  get snaps() { return !!this.name; }
  start() {
    const list = this.app.doc?.blocks ?? [];
    this.name = list.length === 1 ? list[0] : null;
    this.scale = 1;
    this.rot = 0;
    if (!list.length) {
      this.app.report("블록이 없습니다 — 객체를 고르고 B 로 만드세요");
      return this.app.done();
    }
    this.name ? this.ask() : this.say(`블록 넣기: 이름을 치세요 (${list.join(", ")})`);
  }
  ask() { this.say(`블록 「${this.name}」 넣기: 삽입점 (S2 배율, R90 회전 — 지금 ${this.scale} · ${this.rot}°)`); }
  input(t) {
    const k = t.trim();
    if (!this.name) {
      if (!(this.app.doc?.blocks ?? []).includes(k)) {
        this.app.report(`「${k}」 블록이 없습니다`);
        return true;
      }
      this.name = k;
      this.ask();
      return true;
    }
    let m;
    if ((m = k.match(/^S(-?[\d.]+)$/i))) { this.scale = Number(m[1]) || 1; this.ask(); return true; }
    if ((m = k.match(/^R(-?[\d.]+)$/i))) { this.rot = Number(m[1]) || 0; this.ask(); return true; }
    return super.input(t);
  }
  async take(p) {
    if (!this.name) return;
    await this.app.apply({ kind: "insert_block", name: this.name, at: p, scale: this.scale, rotation: this.rot });
    this.say(`블록 「${this.name}」: 다음 삽입점 (Enter 로 끝)`);
  }
  paint(ctx, ts) {
    if (!this.name || !this.cur) return;
    const [x, y] = ts(...this.cur);
    ctx.strokeStyle = PREVIEW;
    ctx.strokeRect(x - 6, y - 6, 12, 12);
  }
}

class ExplodeTool extends ModifyTool {
  get title() { return "펼치기"; }
  async go() {
    await this.app.apply({ kind: "explode", ids: this.app.select.ids });
    this.app.select.clear();
    this.app.done();
  }
}

// ── 반지름·지름·각도 치수 (§36) ───────────────────────────────────────────────
class DimRadialTool extends Tool {
  get snaps() { return false; }
  get dia() { return false; }
  start() { this.say(`${this.dia ? "지름" : "반지름"} 치수: 원·호를 누르세요 (누른 쪽에 치수선)`); }
  take(p) { return this.click(p); }
  async click(m) {
    const s = await this.app.invoke("shape_at", { x: m[0], y: m[1], r: 6 / this.app.v.ppu });
    const c = s?.Circle || s?.Arc;
    if (!c) return this.app.report("원이나 호를 누르세요");
    const o = [c.center.x, c.center.y];
    let d = [m[0] - o[0], m[1] - o[1]];
    const l = Math.hypot(...d) || 1;
    d = [d[0] / l, d[1] / l];
    const point = [o[0] + d[0] * c.radius, o[1] + d[1] * c.radius];
    await this.app.apply({
      kind: "arch",
      op: { kind: "dim_radial", center: o, point, diameter: this.dia, text_height: this.app.textHeight, arrow: true, plain: this.app.dimPlain },
    });
  }
}
class DimDiameterTool extends DimRadialTool {
  get dia() { return true; }
}

class DimAngleTool extends Tool {
  get snaps() { return !!this.vertex; }
  start() {
    this.lines = [];
    this.vertex = null;
    this.say("각도 치수: 첫 선");
  }
  take(p) { return this.click(p); }
  async click(m) {
    if (this.vertex) {
      const [a, b] = this.rays;
      await this.app.apply({ kind: "arch", op: { kind: "dim_angle", vertex: this.vertex, a, b, at: this.pt(m), text_height: this.app.textHeight, arrow: true } });
      return this.start();
    }
    const s = await this.app.invoke("shape_at", { x: m[0], y: m[1], r: 6 / this.app.v.ppu });
    if (!s?.Line) return this.app.report("선을 누르세요");
    this.lines.push({ a: [s.Line.a.x, s.Line.a.y], b: [s.Line.b.x, s.Line.b.y], pick: m });
    if (this.lines.length === 1) return this.say("각도 치수: 둘째 선");
    // 두 선의 교점과, 누른 쪽으로 가는 반직선 위 점
    const [p, q] = this.lines;
    const r = [p.b[0] - p.a[0], p.b[1] - p.a[1]], t = [q.b[0] - q.a[0], q.b[1] - q.a[1]];
    const den = r[0] * t[1] - r[1] * t[0];
    if (Math.abs(den) < 1e-12) {
      this.app.report("평행한 선입니다");
      return this.start();
    }
    const k = ((q.a[0] - p.a[0]) * t[1] - (q.a[1] - p.a[1]) * t[0]) / den;
    const v = [p.a[0] + r[0] * k, p.a[1] + r[1] * k];
    const ray = (ln, d) => {
      const s = (ln.pick[0] - v[0]) * d[0] + (ln.pick[1] - v[1]) * d[1] >= 0 ? 1 : -1;
      const l = Math.hypot(...d) || 1;
      return [v[0] + (d[0] / l) * s * l, v[1] + (d[1] / l) * s * l];
    };
    this.vertex = v;
    this.rays = [ray(p, r), ray(q, t)];
    this.say("각도 치수: 호가 지날 곳을 누르세요");
  }
  paint(ctx, ts) {
    if (!this.vertex || !this.cur) return;
    strokePath(ctx, ts, [this.vertex, this.cur], false, true);
  }
}

// ── 단면 (§38) ───────────────────────────────────────────────────────────────
class SectionTool extends Tool {
  start() { this.say("단면: 자르는 선 첫 점 (보는 쪽은 선의 왼쪽)"); }
  async take(p) {
    this.pts.push(p);
    if (this.pts.length === 1) return this.say("단면: 자르는 선 끝점");
    if (this.pts.length === 2) return this.say("단면: 단면도를 놓을 곳 (단면도 왼쪽 아래 = 자르는 선 첫 점, 높이 0)");
    const [a, b, at] = this.pts;
    await this.app.apply({ kind: "arch", op: { kind: "section", a, b, at, name: "" } });
    this.app.report("단면도를 만들었습니다 — 벽·창·바닥을 고치면 단면도도 따라 바뀝니다");
    this.app.done();
  }
  paint(ctx, ts) {
    if (!this.cur) return;
    if (this.pts.length === 1) strokePath(ctx, ts, [this.pts[0], this.cur], false, true);
    if (this.pts.length === 2) {
      strokePath(ctx, ts, this.pts, false, true);
      const L = dist(this.pts[0], this.pts[1]), c = this.cur;
      strokePath(ctx, ts, [c, [c[0] + L, c[1]], [c[0] + L, c[1] + 3000], [c[0], c[1] + 3000]], true, true);
    }
  }
}

export const TOOLS = {
  line: LineTool,
  pline: PlineTool,
  rect: RectTool,
  circle: CircleTool,
  arc: ArcTool,
  text: TextTool,
  dim: DimTool,
  dimrad: DimRadialTool,
  dimdia: DimDiameterTool,
  dimang: DimAngleTool,
  measure: MeasureTool,
  move: MoveTool,
  copy: CopyTool,
  rotate: RotateTool,
  mirror: MirrorTool,
  scale: ScaleTool,
  erase: EraseTool,
  offset: OffsetTool,
  wall: WallTool,
  arcwall: ArcWallTool,
  door: DoorTool,
  window: WindowTool,
  room: RoomTool,
  grid: GridTool,
  gridset: GridSetTool,
  site: SiteTool,
  trim: TrimTool,
  extend: ExtendTool,
  fillet: FilletTool,
  chamfer: ChamferTool,
  hatch: HatchTool,
  column: ColumnTool,
  beam: BeamTool,
  stair: StairTool,
  block: BlockTool,
  insert: InsertTool,
  explode: ExplodeTool,
  section: SectionTool,
};
