// 3D 보기 (M6, 구현 노트 §24). Rust `model_3d` 가 준 DSM3 삼각형을 WebGL2 로 그린다.
// 끌기: 돌려 보기, Shift·가운데 끌기: 옮기기, 휠: 가까이·멀리. 유리(반투명)는 불투명 면 뒤에 따로 그린다.

const VS = `#version 300 es
in vec3 pos; in vec3 nrm; in vec4 col;
uniform mat4 mvp;
out vec3 vN; out vec4 vC;
void main() { vN = nrm; vC = col; gl_Position = mvp * vec4(pos, 1.0); }`;
const FS = `#version 300 es
precision mediump float;
in vec3 vN; in vec4 vC;
out vec4 o;
void main() {
  vec3 n = normalize(vN);
  float l = 0.45 + 0.4 * abs(dot(n, normalize(vec3(0.4, -0.6, 0.7)))) + 0.15 * abs(dot(n, normalize(vec3(-0.6, 0.3, 0.2))));
  o = vec4(vC.rgb * l, vC.a);
}`;

function sh(gl, type, src) {
  const s = gl.createShader(type);
  gl.shaderSource(s, src);
  gl.compileShader(s);
  if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s));
  return s;
}

// 4×4 행렬 (열 우선)
function mul(a, b) {
  const o = new Float32Array(16);
  for (let i = 0; i < 4; i++) for (let j = 0; j < 4; j++) {
    let s = 0;
    for (let k = 0; k < 4; k++) s += a[k * 4 + j] * b[i * 4 + k];
    o[i * 4 + j] = s;
  }
  return o;
}
function perspective(fovy, aspect, near, far) {
  const f = 1 / Math.tan(fovy / 2), nf = 1 / (near - far);
  return new Float32Array([f / aspect, 0, 0, 0, 0, f, 0, 0, 0, 0, (far + near) * nf, -1, 0, 0, 2 * far * near * nf, 0]);
}
function lookAt(e, c, up) {
  const sub = (a, b) => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
  const nz = (v) => { const l = Math.hypot(...v) || 1; return v.map((x) => x / l); };
  const cr = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
  const dt = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
  const z = nz(sub(e, c)), x = nz(cr(up, z)), y = cr(z, x);
  return new Float32Array([x[0], y[0], z[0], 0, x[1], y[1], z[1], 0, x[2], y[2], z[2], 0, -dt(x, e), -dt(y, e), -dt(z, e), 1]);
}

export class View3D {
  constructor(root, invoke, toBuffer) {
    this.root = root;
    this.invoke = invoke;
    this.toBuffer = toBuffer;
    this.cv = root.querySelector("canvas");
    this.info = root.querySelector(".v3-info");
    root.querySelector(".v3-close").addEventListener("click", () => this.hide());
    const gl = this.cv.getContext("webgl2", { antialias: true });
    this.gl = gl;
    if (!gl) return;
    const p = gl.createProgram();
    gl.attachShader(p, sh(gl, gl.VERTEX_SHADER, VS));
    gl.attachShader(p, sh(gl, gl.FRAGMENT_SHADER, FS));
    gl.linkProgram(p);
    this.prog = p;
    this.uMvp = gl.getUniformLocation(p, "mvp");
    this.vao = gl.createVertexArray();
    this.bufs = [gl.createBuffer(), gl.createBuffer(), gl.createBuffer()];
    this.opaque = 0;
    this.total = 0;
    this.yaw = -0.7;
    this.pitch = 0.55;
    this.dist = 1;
    this.pan = [0, 0, 0];
    this.radius = 1;
    this.bind();
    new ResizeObserver(() => this.draw()).observe(this.cv);
  }

  get open() { return !this.root.hidden; }
  hide() { this.root.hidden = true; }

  async show() {
    this.root.hidden = false;
    if (!this.gl) {
      this.info.textContent = "WebGL2 를 쓸 수 없습니다";
      return;
    }
    const buf = this.toBuffer(await this.invoke("model_3d"));
    this.load(buf);
    this.draw();
  }

  load(buf) {
    const dv = new DataView(buf);
    const n = dv.getUint32(4, true);
    const r = dv.getFloat32(20, true);
    this.radius = r;
    this.dist = r * 2.6;
    this.pan = [0, 0, 0];
    // 불투명 먼저, 반투명 뒤에
    const order = [];
    for (let i = 0; i < n; i++) order.push(i);
    const alpha = (i) => dv.getUint8(24 + i * 52 + 51);
    order.sort((a, b) => (alpha(a) === 255 ? 0 : 1) - (alpha(b) === 255 ? 0 : 1));
    const pos = new Float32Array(n * 9), nrm = new Float32Array(n * 9), col = new Uint8Array(n * 12);
    let opaque = 0;
    order.forEach((t, k) => {
      const o = 24 + t * 52;
      for (let j = 0; j < 9; j++) pos[k * 9 + j] = dv.getFloat32(o + j * 4, true);
      for (let v = 0; v < 3; v++) for (let j = 0; j < 3; j++) nrm[k * 9 + v * 3 + j] = dv.getFloat32(o + 36 + j * 4, true);
      for (let v = 0; v < 3; v++) for (let j = 0; j < 4; j++) col[k * 12 + v * 4 + j] = dv.getUint8(o + 48 + j);
      if (dv.getUint8(o + 51) === 255) opaque++;
    });
    const gl = this.gl;
    gl.bindVertexArray(this.vao);
    const attr = (i, name, data, size, type, normalized) => {
      gl.bindBuffer(gl.ARRAY_BUFFER, this.bufs[i]);
      gl.bufferData(gl.ARRAY_BUFFER, data, gl.STATIC_DRAW);
      const loc = gl.getAttribLocation(this.prog, name);
      gl.enableVertexAttribArray(loc);
      gl.vertexAttribPointer(loc, size, type, normalized, 0, 0);
    };
    attr(0, "pos", pos, 3, gl.FLOAT, false);
    attr(1, "nrm", nrm, 3, gl.FLOAT, false);
    attr(2, "col", col, 4, gl.UNSIGNED_BYTE, true);
    gl.bindVertexArray(null);
    this.opaque = opaque * 3;
    this.total = n * 3;
    this.info.textContent = n
      ? `삼각형 ${n.toLocaleString("ko-KR")} 개 · 끌기: 돌리기 · Shift+끌기: 옮기기 · 휠: 가까이`
      : "건축 객체(벽·실)가 없습니다 — 벽(WA)을 그리면 3D 로 보입니다";
  }

  draw() {
    const gl = this.gl;
    if (!gl || this.root.hidden) return;
    const dpr = window.devicePixelRatio || 1;
    const w = Math.max(1, Math.round(this.cv.clientWidth * dpr)), h = Math.max(1, Math.round(this.cv.clientHeight * dpr));
    if (this.cv.width !== w || this.cv.height !== h) {
      this.cv.width = w;
      this.cv.height = h;
    }
    gl.viewport(0, 0, w, h);
    gl.clearColor(0.93, 0.94, 0.95, 1);
    gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
    if (!this.total) return;
    const c = this.pan;
    const eye = [
      c[0] + this.dist * Math.cos(this.pitch) * Math.cos(this.yaw),
      c[1] + this.dist * Math.cos(this.pitch) * Math.sin(this.yaw),
      c[2] + this.dist * Math.sin(this.pitch),
    ];
    const proj = perspective(0.8, w / h, this.radius * 0.01, this.radius * 20);
    const mvp = mul(proj, lookAt(eye, c, [0, 0, 1]));
    gl.useProgram(this.prog);
    gl.uniformMatrix4fv(this.uMvp, false, mvp);
    gl.bindVertexArray(this.vao);
    gl.enable(gl.DEPTH_TEST);
    gl.disable(gl.BLEND);
    gl.depthMask(true);
    gl.drawArrays(gl.TRIANGLES, 0, this.opaque);
    if (this.total > this.opaque) {
      gl.enable(gl.BLEND);
      gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
      gl.depthMask(false);
      gl.drawArrays(gl.TRIANGLES, this.opaque, this.total - this.opaque);
      gl.depthMask(true);
    }
    gl.bindVertexArray(null);
  }

  bind() {
    let down = null;
    this.cv.addEventListener("pointerdown", (e) => {
      down = { x: e.clientX, y: e.clientY, yaw: this.yaw, pitch: this.pitch, pan: [...this.pan], move: e.shiftKey || e.button === 1 };
      this.cv.setPointerCapture(e.pointerId);
    });
    this.cv.addEventListener("pointermove", (e) => {
      if (!down) return;
      const dx = e.clientX - down.x, dy = e.clientY - down.y;
      if (down.move) {
        const k = this.dist / this.cv.clientHeight;
        const rx = [-Math.sin(this.yaw), Math.cos(this.yaw)];
        this.pan = [down.pan[0] - rx[0] * dx * k, down.pan[1] - rx[1] * dx * k, down.pan[2] + dy * k];
      } else {
        this.yaw = down.yaw - dx * 0.008;
        this.pitch = Math.max(-1.5, Math.min(1.5, down.pitch + dy * 0.008));
      }
      this.draw();
    });
    this.cv.addEventListener("pointerup", () => (down = null));
    this.cv.addEventListener("wheel", (e) => {
      e.preventDefault();
      this.dist = Math.max(this.radius * 0.05, Math.min(this.radius * 10, this.dist * Math.exp(e.deltaY * 0.0015)));
      this.draw();
    }, { passive: false });
  }
}
