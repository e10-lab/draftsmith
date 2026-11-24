// 도면 보기. 선·채움은 WebGL2, 문자는 그 위 Canvas 2D 층에 글꼴로 그린다.
// 표시 데이터 형식은 draftsmith_core::display 참고. 화면이 바뀔 때만 다시 그린다 (계속 그리지 않음).

const VS = `#version 300 es
in vec2 pos;
in vec4 col;
uniform vec2 center;   // 보는 중심 (origin 기준 도면 좌표)
uniform vec2 scale;    // 도면 단위 → 클립 좌표
out vec4 vcol;
void main() {
  gl_Position = vec4((pos - center) * scale, 0.0, 1.0);
  vcol = col;
}`;

const FS = `#version 300 es
precision mediump float;
in vec4 vcol;
uniform float paper;   // 1 = 흰 배경: 흰색(ACI 7) 선을 검게 (AutoCAD 와 같음)
out vec4 color;
void main() {
  vec3 c = vcol.rgb;
  if (paper > 0.5) {
    if (min(c.r, min(c.g, c.b)) > 0.92) c = vec3(0.0);
    // 노랑·하늘색처럼 밝은 색은 흰 바탕에서 안 보여 조금 어둡게
    float lum = dot(c, vec3(0.299, 0.587, 0.114));
    if (lum > 0.5) c *= 0.45 / lum;
  }
  color = vec4(c, vcol.a);
}`;

// 도면 글자 높이(대문자·한글 높이)에 맞추려면 글꼴 크기를 조금 키워야 한다
const EM_PER_HEIGHT = 1.35;
// MTEXT 줄 간격 (AutoCAD 기본 「최소」 간격 = 글자 높이의 5/3)
const LINE_GAP = 5 / 3;
// 이보다 작게 보이는 글자는 그리지 않는다 (화면 점)
const MIN_TEXT_PX = 3;
const FONT = `"Pretendard", "Apple SD Gothic Neo", "Malgun Gothic", "Noto Sans KR", sans-serif`;

function shader(gl, type, src) {
  const s = gl.createShader(type);
  gl.shaderSource(s, src);
  gl.compileShader(s);
  if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s));
  return s;
}

/** 선분·삼각형 하나당 색 하나를 꼭짓점마다로 늘린다 */
function perVertex(colors, count, verts) {
  const out = new Uint8Array(count * verts * 4);
  for (let i = 0; i < count; i++) {
    for (let v = 0; v < verts; v++) {
      const d = (i * verts + v) * 4, s = i * 4;
      out[d] = colors[s]; out[d + 1] = colors[s + 1]; out[d + 2] = colors[s + 2]; out[d + 3] = colors[s + 3];
    }
  }
  return out;
}

/** DST1 문자 표시 데이터 → 배열 */
function parseTexts(buf) {
  if (!buf || buf.byteLength < 24) return [];
  const dv = new DataView(buf);
  const magic = String.fromCharCode(...new Uint8Array(buf, 0, 4));
  if (magic !== "DST1") throw new Error("문자 데이터 형식이 다릅니다");
  const k = dv.getUint32(4, true);
  const dec = new TextDecoder();
  const out = new Array(k);
  let p = 24;
  for (let i = 0; i < k; i++) {
    const f = (j) => dv.getFloat32(p + j * 4, true);
    const t = { x: f(0), y: f(1), h: f(2), rot: f(3), wf: f(4), ha: f(5) };
    p += 24;
    t.va = dv.getUint8(p);
    const [cr, cg, cb] = [dv.getUint8(p + 1), dv.getUint8(p + 2), dv.getUint8(p + 3)];
    t.color = `rgb(${cr},${cg},${cb})`;
    t.white = Math.min(cr, cg, cb) > 235;
    // 흰 배경용 색: 흰색은 검정, 밝은 색은 조금 어둡게 (셰이더와 같은 규칙)
    const lum = (0.299 * cr + 0.587 * cg + 0.114 * cb) / 255;
    const k = lum > 0.5 ? 0.45 / lum : 1;
    t.paperColor = t.white ? "#000" : `rgb(${Math.round(cr * k)},${Math.round(cg * k)},${Math.round(cb * k)})`;
    const len = dv.getUint32(p + 4, true);
    p += 8;
    t.lines = dec.decode(new Uint8Array(buf, p, len)).split("\n");
    p += len;
    // 화면 밖 걸러내기용 대략 반지름 (도면 단위)
    const longest = Math.max(...t.lines.map((l) => l.length));
    t.r = Math.hypot(longest * t.h * t.wf, t.lines.length * t.h * LINE_GAP) + t.h;
    out[i] = t;
  }
  return out;
}

export class Viewer {
  /**
   * @param canvas      WebGL2 (선·채움)
   * @param textCanvas  Canvas 2D (문자)
   * @param overlay     Canvas 2D (도구: 스냅 표시·측정선 — 마우스가 움직일 때마다 이것만 다시 그림)
   * @param onCursor    (x, y) 도면 좌표 커서 위치 알림
   */
  constructor(canvas, textCanvas, overlay, onCursor) {
    this.cv = canvas;
    this.tc = textCanvas;
    this.ov = overlay;
    this.ctx = textCanvas.getContext("2d");
    this.octx = overlay.getContext("2d");
    this.onCursor = onCursor;
    /** 지금 도구: { move(m), click(m), paint(ctx, toScreen) } — m 은 도면 좌표 [x, y] */
    this.tool = null;
    const gl = canvas.getContext("webgl2", { antialias: true, alpha: false });
    if (!gl) throw new Error("WebGL2 를 쓸 수 없습니다");
    this.gl = gl;
    const p = gl.createProgram();
    gl.attachShader(p, shader(gl, gl.VERTEX_SHADER, VS));
    gl.attachShader(p, shader(gl, gl.FRAGMENT_SHADER, FS));
    gl.linkProgram(p);
    if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p));
    this.prog = p;
    this.uCenter = gl.getUniformLocation(p, "center");
    this.uScale = gl.getUniformLocation(p, "scale");
    this.uPaper = gl.getUniformLocation(p, "paper");
    /** 흰 배경 모드 */
    this.paper = false;
    this.lines = this.mesh();
    this.tris = this.mesh();
    this.texts = [];
    this.origin = [0, 0];
    this.bbox = [0, 0, 0, 0];
    this.center = [0, 0];
    this.ppu = 1;               // 도면 단위당 화면 점 수
    this.pending = false;
    this.bindInput();
    new ResizeObserver(() => this.draw()).observe(canvas);
    // 내장 글꼴을 읽은 뒤 문자를 다시 그린다 (처음엔 대체 글꼴로 그려질 수 있음)
    document.fonts?.load(`16px "Pretendard"`, "가A").then(() => this.draw(), () => {});
  }

  /** 도면 좌표 → 화면 점 (CSS px) */
  toScreen(x, y) {
    const r = this.cv.getBoundingClientRect();
    return [
      r.width / 2 + (x - this.origin[0] - this.center[0]) * this.ppu,
      r.height / 2 - (y - this.origin[1] - this.center[1]) * this.ppu,
    ];
  }

  /** 지금 보이는 범위 [x0, y0, x1, y1] (도면 좌표) */
  viewBox() {
    const r = this.cv.getBoundingClientRect();
    const [ax, ay] = this.toModel(0, r.height), [bx, by] = this.toModel(r.width, 0);
    const [ox, oy] = this.origin;
    return [ax + ox, ay + oy, bx + ox, by + oy];
  }

  /** 도구 층만 다시 그린다 */
  drawOverlay() {
    const dpr = window.devicePixelRatio || 1;
    const ctx = this.octx;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, this.ov.width, this.ov.height);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    this.tool?.paint?.(ctx, (x, y) => this.toScreen(x, y));
  }

  mesh() {
    const gl = this.gl;
    return { vao: gl.createVertexArray(), pos: gl.createBuffer(), col: gl.createBuffer(), count: 0 };
  }

  upload(m, coords, vcol) {
    const gl = this.gl;
    gl.bindVertexArray(m.vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, m.pos);
    gl.bufferData(gl.ARRAY_BUFFER, coords, gl.STATIC_DRAW);
    const aPos = gl.getAttribLocation(this.prog, "pos");
    gl.enableVertexAttribArray(aPos);
    gl.vertexAttribPointer(aPos, 2, gl.FLOAT, false, 0, 0);
    gl.bindBuffer(gl.ARRAY_BUFFER, m.col);
    gl.bufferData(gl.ARRAY_BUFFER, vcol, gl.STATIC_DRAW);
    const aCol = gl.getAttribLocation(this.prog, "col");
    gl.enableVertexAttribArray(aCol);
    gl.vertexAttribPointer(aCol, 4, gl.UNSIGNED_BYTE, true, 0, 0);
    gl.bindVertexArray(null);
    m.count = coords.length / 2;
  }

  /** 표시 데이터(선·채움 ArrayBuffer, 문자 ArrayBuffer)를 올린다. 걸린 시간(ms)을 돌려준다 */
  load(buf, textBuf, { keepView = false } = {}) {
    const t = performance.now();
    const dv = new DataView(buf);
    const magic = String.fromCharCode(...new Uint8Array(buf, 0, 4));
    if (magic !== "DSD2") throw new Error("표시 데이터 형식이 다릅니다");
    const n = dv.getUint32(4, true), m = dv.getUint32(8, true);
    this.origin = [dv.getFloat64(12, true), dv.getFloat64(20, true)];
    this.bbox = [0, 1, 2, 3].map((i) => dv.getFloat32(28 + i * 4, true));
    let p = 44;
    const segs = new Float32Array(buf, p, n * 4); p += n * 16;
    const segCol = new Uint8Array(buf, p, n * 4); p += n * 4;
    const tris = new Float32Array(buf, p, m * 6); p += m * 24;
    const triCol = new Uint8Array(buf, p, m * 4);
    this.upload(this.lines, segs, perVertex(segCol, n, 2));
    this.upload(this.tris, tris, perVertex(triCol, m, 3));
    this.texts = parseTexts(textBuf);
    if (!keepView) this.fit();
    else this.draw();
    this.gl.finish();
    return performance.now() - t;
  }

  fit() {
    const [x0, y0, x1, y1] = this.bbox;
    const w = Math.max(x1 - x0, 1e-9), h = Math.max(y1 - y0, 1e-9);
    const r = this.cv.getBoundingClientRect();
    this.center = [(x0 + x1) / 2, (y0 + y1) / 2];
    this.ppu = 0.92 * Math.min(r.width / w, r.height / h) || 1;
    this.draw();
  }

  /** 화면 점(CSS px) → origin 기준 도면 좌표 */
  toModel(px, py) {
    const r = this.cv.getBoundingClientRect();
    return [
      this.center[0] + (px - r.width / 2) / this.ppu,
      this.center[1] - (py - r.height / 2) / this.ppu,
    ];
  }

  draw() {
    if (this.pending) return;
    this.pending = true;
    requestAnimationFrame(() => {
      this.pending = false;
      this.render();
    });
  }

  render() {
    const gl = this.gl, cv = this.cv;
    const dpr = window.devicePixelRatio || 1;
    const r = cv.getBoundingClientRect();
    const w = Math.max(1, Math.round(r.width * dpr)), h = Math.max(1, Math.round(r.height * dpr));
    for (const c of [cv, this.tc, this.ov]) {
      if (c.width !== w || c.height !== h) { c.width = w; c.height = h; }
    }
    gl.viewport(0, 0, w, h);
    const bg = this.paper ? [255, 255, 255] : getComputedStyle(cv.parentElement).backgroundColor.match(/\d+/g) || [27, 29, 33];
    gl.clearColor(bg[0] / 255, bg[1] / 255, bg[2] / 255, 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.useProgram(this.prog);
    gl.uniform2f(this.uCenter, this.center[0], this.center[1]);
    gl.uniform2f(this.uScale, (2 * this.ppu) / r.width, (2 * this.ppu) / r.height);
    gl.uniform1f(this.uPaper, this.paper ? 1 : 0);
    // 채움 먼저, 그 위에 선
    for (const [m, mode] of [[this.tris, gl.TRIANGLES], [this.lines, gl.LINES]]) {
      if (!m.count) continue;
      gl.bindVertexArray(m.vao);
      gl.drawArrays(mode, 0, m.count);
    }
    gl.bindVertexArray(null);
    this.renderTexts(r.width, r.height, dpr);
    this.drawOverlay();
  }

  renderTexts(W, H, dpr) {
    const ctx = this.ctx;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, this.tc.width, this.tc.height);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    const [cx, cy] = this.center, k = this.ppu;
    // 보이는 영역 (도면 단위)
    const hw = W / 2 / k, hh = H / 2 / k;
    let drawn = 0;
    for (const t of this.texts) {
      const px = t.h * k;
      if (px < MIN_TEXT_PX) continue;
      if (Math.abs(t.x - cx) > hw + t.r || Math.abs(t.y - cy) > hh + t.r) continue;
      const sx = W / 2 + (t.x - cx) * k, sy = H / 2 - (t.y - cy) * k;
      const n = t.lines.length, gap = px * LINE_GAP;
      // 세로 맞춤: 첫 줄 바탕선이 0 일 때 글자 덩어리는 [−px, (n−1)·gap]
      let dy = 0;
      if (t.va === 1) dy = -(n - 1) * gap;                 // 아래
      else if (t.va === 2) dy = -((n - 1) * gap - px) / 2; // 가운데
      else if (t.va === 3) dy = px;                        // 위
      ctx.save();
      ctx.translate(sx, sy);
      ctx.rotate(-t.rot);
      ctx.scale(t.wf || 1, 1);
      ctx.font = `${px * EM_PER_HEIGHT}px ${FONT}`;
      ctx.fillStyle = this.paper ? t.paperColor : t.color;
      ctx.textAlign = t.ha >= 0.75 ? "right" : t.ha >= 0.25 ? "center" : "left";
      ctx.textBaseline = "alphabetic";
      for (let i = 0; i < n; i++) ctx.fillText(t.lines[i], 0, dy + i * gap);
      ctx.restore();
      drawn++;
    }
    this.textsDrawn = drawn;
  }

  bindInput() {
    const cv = this.cv;
    // 끌기 = 화면 이동 (왼쪽·가운데 버튼), 제자리 누르기 = 도구 클릭
    let down = null;
    const model = (e) => {
      const r = cv.getBoundingClientRect();
      const [mx, my] = this.toModel(e.clientX - r.left, e.clientY - r.top);
      return [mx + this.origin[0], my + this.origin[1]];
    };
    cv.addEventListener("pointerdown", (e) => {
      down = { x: e.clientX, y: e.clientY, c: [...this.center], moved: false, button: e.button };
      cv.setPointerCapture(e.pointerId);
    });
    cv.addEventListener("pointermove", (e) => {
      const m = model(e);
      this.onCursor?.(m[0], m[1]);
      if (down) {
        const dx = e.clientX - down.x, dy = e.clientY - down.y;
        if (!down.moved && Math.hypot(dx, dy) < 4) return;
        down.moved = true;
        this.center = [down.c[0] - dx / this.ppu, down.c[1] + dy / this.ppu];
        this.draw();
        return;
      }
      this.tool?.move?.(m);
    });
    cv.addEventListener("pointerup", (e) => {
      const click = down && !down.moved && down.button === 0;
      down = null;
      if (click) this.tool?.click?.(model(e));
    });
    // 휠·트랙패드: 커서 위치를 중심으로 확대·축소 (트랙패드 두 손가락 오므리기는 ctrlKey 로 온다)
    cv.addEventListener("wheel", (e) => {
      e.preventDefault();
      const r = cv.getBoundingClientRect();
      const px = e.clientX - r.left, py = e.clientY - r.top;
      const before = this.toModel(px, py);
      const k = Math.exp(-e.deltaY * (e.ctrlKey ? 0.01 : 0.0015));
      this.ppu *= k;
      const after = this.toModel(px, py);
      this.center = [this.center[0] + before[0] - after[0], this.center[1] + before[1] - after[1]];
      this.draw();
    }, { passive: false });
    cv.addEventListener("dblclick", () => this.fit());
  }
}
