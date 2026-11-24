// 도면 보기 (WebGL2). 코어가 만든 표시 데이터(바이너리)를 그대로 GPU 버퍼에 올려 선으로 그린다.
// 형식은 draftsmith_core::display 참고. 화면이 바뀔 때만 다시 그린다 (계속 그리지 않음).

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
out vec4 color;
void main() { color = vcol; }`;

function shader(gl, type, src) {
  const s = gl.createShader(type);
  gl.shaderSource(s, src);
  gl.compileShader(s);
  if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s));
  return s;
}

export class Viewer {
  constructor(canvas, onCursor) {
    this.cv = canvas;
    this.onCursor = onCursor;
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
    this.vao = gl.createVertexArray();
    this.posBuf = gl.createBuffer();
    this.colBuf = gl.createBuffer();
    this.count = 0;             // 꼭짓점 수
    this.origin = [0, 0];
    this.bbox = [0, 0, 0, 0];
    this.center = [0, 0];
    this.ppu = 1;               // 도면 단위당 화면 점 수
    this.pending = false;
    this.bindInput();
    new ResizeObserver(() => this.draw()).observe(canvas);
  }

  /** 표시 데이터(ArrayBuffer)를 올린다. 걸린 시간(ms)을 돌려준다 */
  load(buf) {
    const t = performance.now();
    const dv = new DataView(buf);
    const magic = String.fromCharCode(...new Uint8Array(buf, 0, 4));
    if (magic !== "DSD1") throw new Error("표시 데이터 형식이 다릅니다");
    const n = dv.getUint32(4, true);
    this.origin = [dv.getFloat64(8, true), dv.getFloat64(16, true)];
    this.bbox = [0, 1, 2, 3].map((i) => dv.getFloat32(24 + i * 4, true));
    const coords = new Float32Array(buf, 40, n * 4);
    const colors = new Uint8Array(buf, 40 + n * 16, n * 4);
    // 선분 색은 두 끝점에 같게 — 꼭짓점마다 색 하나로 늘린다
    const vcol = new Uint8Array(n * 8);
    for (let i = 0; i < n; i++) {
      const s = i * 4, d = i * 8;
      vcol[d] = vcol[d + 4] = colors[s];
      vcol[d + 1] = vcol[d + 5] = colors[s + 1];
      vcol[d + 2] = vcol[d + 6] = colors[s + 2];
      vcol[d + 3] = vcol[d + 7] = colors[s + 3];
    }
    const gl = this.gl;
    gl.bindVertexArray(this.vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.posBuf);
    gl.bufferData(gl.ARRAY_BUFFER, coords, gl.STATIC_DRAW);
    const aPos = gl.getAttribLocation(this.prog, "pos");
    gl.enableVertexAttribArray(aPos);
    gl.vertexAttribPointer(aPos, 2, gl.FLOAT, false, 0, 0);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.colBuf);
    gl.bufferData(gl.ARRAY_BUFFER, vcol, gl.STATIC_DRAW);
    const aCol = gl.getAttribLocation(this.prog, "col");
    gl.enableVertexAttribArray(aCol);
    gl.vertexAttribPointer(aCol, 4, gl.UNSIGNED_BYTE, true, 0, 0);
    gl.bindVertexArray(null);
    this.count = n * 2;
    this.fit();
    gl.finish();
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
    if (cv.width !== w || cv.height !== h) { cv.width = w; cv.height = h; }
    gl.viewport(0, 0, w, h);
    const bg = getComputedStyle(cv.parentElement).backgroundColor.match(/\d+/g) || [27, 29, 33];
    gl.clearColor(bg[0] / 255, bg[1] / 255, bg[2] / 255, 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
    if (!this.count) return;
    gl.useProgram(this.prog);
    gl.uniform2f(this.uCenter, this.center[0], this.center[1]);
    gl.uniform2f(this.uScale, (2 * this.ppu) / r.width, (2 * this.ppu) / r.height);
    gl.bindVertexArray(this.vao);
    gl.drawArrays(gl.LINES, 0, this.count);
    gl.bindVertexArray(null);
  }

  bindInput() {
    const cv = this.cv;
    let drag = null;
    cv.addEventListener("pointerdown", (e) => {
      drag = { x: e.clientX, y: e.clientY, c: [...this.center] };
      cv.setPointerCapture(e.pointerId);
    });
    cv.addEventListener("pointermove", (e) => {
      const r = cv.getBoundingClientRect();
      const [mx, my] = this.toModel(e.clientX - r.left, e.clientY - r.top);
      this.onCursor?.(mx + this.origin[0], my + this.origin[1]);
      if (!drag) return;
      this.center = [
        drag.c[0] - (e.clientX - drag.x) / this.ppu,
        drag.c[1] + (e.clientY - drag.y) / this.ppu,
      ];
      this.draw();
    });
    cv.addEventListener("pointerup", () => (drag = null));
    // 휠·트랙패드: 커서 위치를 중심으로 확대·축소 (트랙패드 두 손가락 이동은 ctrl 없이 오는 작은 값)
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
