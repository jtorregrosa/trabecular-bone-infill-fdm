// A small WebGL 2 renderer (orbit camera, section plane, flat or smooth
// shading, overhang colouring) and the wiring of plain-HTML view controls.
//
// Written as a classic script with no imports or exports, because it is used
// in two ways: the Rust crate inlines it into the standalone viewer page
// (`bone-infill view`), and the studio kit loads it as text (see viewer.tsx).

/**
 * Creates the renderer on `canvas`. Returns null if WebGL 2 is unavailable.
 * `opts.leftInset()` – pixels at the left of the canvas hidden by a panel.
 * `opts.onChange()`  – called when the view state changes from the keyboard.
 */
function createViewer(canvas, opts) {
  opts = opts || {};
  const gl = canvas.getContext("webgl2", { antialias: true });
  if (!gl) return null;
  // Every listener goes through `listen`, so that `dispose` can remove them.
  const abort = new AbortController();
  const listen = (target, type, fn, options) => target.addEventListener(type, fn, Object.assign({ signal: abort.signal }, options));
  let alive = true;
  // Behind the part: the page's --canvas token, so the theme stays in one place.
  const background = (() => {
    const hex = getComputedStyle(document.documentElement).getPropertyValue("--canvas").trim();
    const m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex);
    return m ? m.slice(1).map((h) => parseInt(h, 16) / 255) : [0.106, 0.106, 0.122];
  })();

  const VS = `#version 300 es
    layout(location = 0) in vec3 aPos;          // quantised to 0..1 inside the bounding box
    layout(location = 1) in vec3 aNormal;       // averaged over the triangles around the vertex
    uniform mat4 uViewProj; uniform vec3 uLo; uniform vec3 uSize;
    out vec3 vWorld; out vec3 vNormal;
    void main() { vWorld = uLo + aPos * uSize; vNormal = aNormal; gl_Position = uViewProj * vec4(vWorld, 1.0); }`;
  const FS = `#version 300 es
    precision highp float;
    in vec3 vWorld; in vec3 vNormal;
    uniform vec3 uEye; uniform vec3 uRight; uniform vec3 uUp;
    uniform vec4 uPlane;        // xyz normal, w offset: fragments with dot(n, p) > w are cut away
    uniform int uClip; uniform int uMode; uniform int uSmooth; uniform float uBedZ;
    out vec4 color;
    // Colours follow the design tokens in theme.css (--oh-*, --bone).
    vec3 ramp(float deg) {
      if (deg < 30.0) return vec3(0.498, 0.706, 0.549);
      if (deg < 45.0) return vec3(0.851, 0.784, 0.376);
      if (deg < 60.0) return vec3(0.878, 0.541, 0.235);
      return vec3(0.839, 0.282, 0.231);
    }
    void main() {
      if (uClip == 1 && dot(uPlane.xyz, vWorld) > uPlane.w) discard;
      vec3 toEye = normalize(uEye - vWorld);
      // Flat normal from screen-space derivatives, turned towards the viewer.
      vec3 n = normalize(cross(dFdx(vWorld), dFdy(vWorld)));
      if (dot(n, toEye) < 0.0) n = -n;
      if (uSmooth == 1 && gl_FrontFacing) {
        // Interpolated vertex normal, except across a crease: where it
        // strays more than about 37 degrees from the face, the edge is a
        // real one (a corner of the model) and stays sharp.
        vec3 s = normalize(vNormal);
        if (dot(s, n) > 0.8) n = s;
      }
      vec3 light = normalize(toEye + 0.45 * uUp - 0.35 * uRight);
      float diffuse = max(dot(n, light), 0.0);
      if (!gl_FrontFacing) {
        // The back of a surface seen through a section is solid material.
        // Same bone as the rest, with softer light: a cut reads as one material.
        color = vec4(vec3(0.929, 0.902, 0.839) * (0.62 + 0.30 * diffuse), 1.0);
        return;
      }
      vec3 base = vec3(0.929, 0.902, 0.839);
      if (uMode == 1) {
        base = vec3(0.58, 0.58, 0.60);
        float down = -n.z;      // front faces: n is the outward normal
        if (down > 0.02 && vWorld.z > uBedZ) base = ramp(degrees(asin(min(down, 1.0))));
      }
      color = vec4(base * (0.24 + 0.76 * diffuse), 1.0);
    }`;
  const PICK_FS = `#version 300 es
    precision highp float; in vec3 vWorld; uniform vec4 uPlane; uniform int uClip; out vec4 color;
    // Alpha tells front faces (1) from the backs seen through a section (0.5).
    void main() { if (uClip == 1 && dot(uPlane.xyz, vWorld) > uPlane.w) discard; color = vec4(vWorld, gl_FrontFacing ? 1.0 : 0.5); }`;

  // Markers drawn over the part: translucent spheres (reinforced regions)
  // and a double-headed arrow (load direction). Seen through the part, with
  // the rim brighter than the middle so that a sphere reads as a volume.
  const OVERLAY_VS = `#version 300 es
    layout(location = 0) in vec3 aPos; layout(location = 1) in vec3 aNormal;
    uniform mat4 uViewProj; uniform mat4 uModel;
    out vec3 vWorld; out vec3 vNormal;
    void main() { vec4 w = uModel * vec4(aPos, 1.0); vWorld = w.xyz; vNormal = mat3(uModel) * aNormal; gl_Position = uViewProj * w; }`;
  const OVERLAY_FS = `#version 300 es
    precision highp float;
    in vec3 vWorld; in vec3 vNormal;
    uniform vec3 uEye; uniform vec4 uColor; uniform float uRim;
    out vec4 color;
    void main() {
      float facing = abs(dot(normalize(vNormal), normalize(uEye - vWorld)));
      float rim = mix(1.0, pow(1.0 - facing, 2.0) * 0.85 + 0.15, uRim);
      float light = mix(0.55 + 0.45 * facing, 1.0, uRim);
      color = vec4(uColor.rgb * light, uColor.a * rim);
    }`;

  function program(vs, fs, names) {
    const p = gl.createProgram();
    for (const [type, src] of [[gl.VERTEX_SHADER, vs], [gl.FRAGMENT_SHADER, fs]]) {
      const s = gl.createShader(type); gl.shaderSource(s, src); gl.compileShader(s);
      if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s));
      gl.attachShader(p, s);
    }
    gl.linkProgram(p);
    if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p));
    const u = {};
    for (const n of names) u[n] = gl.getUniformLocation(p, n);
    return { p, u };
  }
  const main = program(VS, FS, ["uViewProj", "uLo", "uSize", "uEye", "uRight", "uUp", "uPlane", "uClip", "uMode", "uSmooth", "uBedZ"]);
  const picker = program(VS, PICK_FS, ["uViewProj", "uLo", "uSize", "uPlane", "uClip"]);
  const marker = program(OVERLAY_VS, OVERLAY_FS, ["uViewProj", "uModel", "uEye", "uColor", "uRim"]);

  // Marker geometry, built once: a unit sphere, and an arrow along Z from -1
  // to +1 with a head at each end (a load direction has no sign).
  function shape(positions, normals, indices) {
    const vao = gl.createVertexArray();
    gl.bindVertexArray(vao);
    for (const [loc, data] of [[0, positions], [1, normals]]) {
      const b = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, b); gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(data), gl.STATIC_DRAW);
      gl.enableVertexAttribArray(loc); gl.vertexAttribPointer(loc, 3, gl.FLOAT, false, 0, 0);
    }
    const ib = gl.createBuffer();
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, ib); gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, new Uint16Array(indices), gl.STATIC_DRAW);
    gl.bindVertexArray(null);
    return { vao, count: indices.length };
  }
  const sphereShape = (() => {
    const P = [], I = [], RINGS = 16, SEGS = 28;
    for (let r = 0; r <= RINGS; r++) {
      const t = Math.PI * r / RINGS;
      for (let s = 0; s <= SEGS; s++) {
        const f = 2 * Math.PI * s / SEGS;
        P.push(Math.sin(t) * Math.cos(f), Math.sin(t) * Math.sin(f), Math.cos(t));
      }
    }
    for (let r = 0; r < RINGS; r++) for (let s = 0; s < SEGS; s++) {
      const a = r * (SEGS + 1) + s, b = a + SEGS + 1;
      I.push(a, b, a + 1, a + 1, b, b + 1);
    }
    return shape(P, P, I);
  })();
  const arrowShape = (() => {
    const P = [], N = [], I = [], SEGS = 14, SHAFT = 0.012, HEAD = 0.045, TIP = 0.16;
    // Rings: [z, radius, normal z]; a cylinder between two cones.
    const rings = [[-1, 0, -1], [-1 + TIP, HEAD, -0.3], [-1 + TIP, SHAFT, 0], [1 - TIP, SHAFT, 0], [1 - TIP, HEAD, 0.3], [1, 0, 1]];
    for (const [z, rad, nz] of rings) for (let s = 0; s <= SEGS; s++) {
      const f = 2 * Math.PI * s / SEGS, c = Math.cos(f), sn = Math.sin(f);
      P.push(rad * c, rad * sn, z); N.push(c, sn, nz);
    }
    for (let r = 0; r < rings.length - 1; r++) for (let s = 0; s < SEGS; s++) {
      const a = r * (SEGS + 1) + s, b = a + SEGS + 1;
      I.push(a, b, a + 1, a + 1, b, b + 1);
    }
    return shape(P, N, I);
  })();
  /** What the markers show: `loads` [{center, radius}], `selected` index, `direction` [x, y, z] or null. */
  let overlays = { loads: [], selected: -1, direction: null };

  const vao = gl.createVertexArray();
  const vbo = gl.createBuffer(), nbo = gl.createBuffer(), ibo = gl.createBuffer();
  gl.bindVertexArray(vao);
  gl.bindBuffer(gl.ARRAY_BUFFER, vbo);
  gl.enableVertexAttribArray(0); gl.vertexAttribPointer(0, 3, gl.UNSIGNED_SHORT, true, 0, 0);
  gl.bindBuffer(gl.ARRAY_BUFFER, nbo);
  gl.enableVertexAttribArray(1); gl.vertexAttribPointer(1, 3, gl.SHORT, true, 0, 0);
  gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, ibo);

  // Vertex normals for smooth shading: the area-weighted mean of the normals
  // of the triangles around each vertex, packed as signed 16-bit.
  function vertexNormals(positions, indices, size) {
    const sum = new Float32Array(positions.length);
    const sx = size[0] / 65535, sy = size[1] / 65535, sz = size[2] / 65535;
    for (let t = 0; t < indices.length; t += 3) {
      const a = indices[t] * 3, b = indices[t + 1] * 3, c = indices[t + 2] * 3;
      const ux = (positions[b] - positions[a]) * sx, uy = (positions[b + 1] - positions[a + 1]) * sy, uz = (positions[b + 2] - positions[a + 2]) * sz;
      const vx = (positions[c] - positions[a]) * sx, vy = (positions[c + 1] - positions[a + 1]) * sy, vz = (positions[c + 2] - positions[a + 2]) * sz;
      const nx = uy * vz - uz * vy, ny = uz * vx - ux * vz, nz = ux * vy - uy * vx;   // length = 2 x area
      sum[a] += nx; sum[a + 1] += ny; sum[a + 2] += nz;
      sum[b] += nx; sum[b + 1] += ny; sum[b + 2] += nz;
      sum[c] += nx; sum[c + 1] += ny; sum[c + 2] += nz;
    }
    const out = new Int16Array(positions.length);
    for (let i = 0; i < sum.length; i += 3) {
      const k = 32767 / (Math.hypot(sum[i], sum[i + 1], sum[i + 2]) || 1);
      out[i] = sum[i] * k; out[i + 1] = sum[i + 1] * k; out[i + 2] = sum[i + 2] * k;
    }
    return out;
  }

  let mesh = null;   // { count, lo, size, centre, radius }
  const cam = { yaw: -1.05, pitch: 0.5, dist: 1, target: [0, 0, 0] };
  const state = { axis: -1, cut: 0.5, flip: false, mode: 0, smooth: true };
  let dirty = true;
  const redraw = () => { dirty = true; };
  const changed = () => { redraw(); if (opts.onChange) opts.onChange(); };

  /** Shows a mesh. With `keepCamera` the view is left where it is. */
  function setMesh(positions, indices, lo, hi, keepCamera) {
    gl.bindVertexArray(vao);
    const size = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
    gl.bindBuffer(gl.ARRAY_BUFFER, vbo); gl.bufferData(gl.ARRAY_BUFFER, positions, gl.STATIC_DRAW);
    gl.bindBuffer(gl.ARRAY_BUFFER, nbo); gl.bufferData(gl.ARRAY_BUFFER, vertexNormals(positions, indices, size), gl.STATIC_DRAW);
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, ibo); gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, indices, gl.STATIC_DRAW);
    const first = mesh === null;
    mesh = { count: indices.length, lo: Array.from(lo), size, centre: [0, 1, 2].map((a) => 0.5 * (lo[a] + hi[a])), radius: 0.5 * Math.hypot(size[0], size[1], size[2]) || 1 };
    if (first || !keepCamera) { cam.target = mesh.centre.slice(); cam.dist = mesh.radius * 3.2; }
    changed();
  }

  /** Reads the app's binary mesh format (see `viewer::mesh_binary`). */
  function setMeshFromBinary(buffer, keepCamera) {
    const view = new DataView(buffer);
    const magic = String.fromCharCode(...new Uint8Array(buffer, 0, 4));
    if (magic !== "BIM1") throw new Error("unexpected mesh data");
    const nv = view.getUint32(4, true), nt = view.getUint32(8, true);
    const box = [0, 1, 2, 3, 4, 5].map((i) => view.getFloat32(12 + 4 * i, true));
    const positions = new Uint16Array(buffer, 36, nv * 3);
    const indices = new Uint32Array(buffer, 36 + Math.ceil(nv * 6 / 4) * 4, nt * 3);
    setMesh(positions, indices, box.slice(0, 3), box.slice(3), keepCamera);
    return { vertices: nv, triangles: nt, lo: box.slice(0, 3), hi: box.slice(3) };
  }

  const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
  const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
  function frame() {
    const cp = Math.cos(cam.pitch), sp = Math.sin(cam.pitch);
    const back = [cp * Math.cos(cam.yaw), cp * Math.sin(cam.yaw), sp];   // from target towards the eye
    const eye = [0, 1, 2].map((a) => cam.target[a] + back[a] * cam.dist);
    let right = cross([0, 0, 1], back);
    let len = Math.hypot(right[0], right[1], right[2]);
    if (len < 1e-6) { right = [-Math.sin(cam.yaw), Math.cos(cam.yaw), 0]; len = 1; }
    right = right.map((v) => v / len);
    return { eye, back, right, up: cross(back, right) };
  }
  const FOV = 35 * Math.PI / 180;
  function viewProj(f) {
    const aspect = canvas.width / Math.max(canvas.height, 1), t = 1 / Math.tan(FOV / 2);
    const near = Math.max(cam.dist - mesh.radius * 2.5, mesh.radius * 0.002), far = cam.dist + mesh.radius * 2.5;
    // The third entry of the first row shifts the picture sideways, so the
    // model is centred in the area a panel leaves free.
    const inset = opts.leftInset ? opts.leftInset() : 0;
    const shift = Math.min(0.45, inset / Math.max(canvas.clientWidth, 1));
    const e = f.eye, r = f.right, u = f.up, b = f.back;
    const V = [r[0], r[1], r[2], -dot(r, e), u[0], u[1], u[2], -dot(u, e), b[0], b[1], b[2], -dot(b, e), 0, 0, 0, 1];
    const P = [t / aspect, 0, -shift, 0, 0, t, 0, 0, 0, 0, (far + near) / (near - far), 2 * far * near / (near - far), 0, 0, -1, 0];
    const M = new Float32Array(16);   // column-major P * V
    for (let i = 0; i < 4; i++) for (let j = 0; j < 4; j++) {
      let s = 0; for (let k = 0; k < 4; k++) s += P[i * 4 + k] * V[k * 4 + j];
      M[j * 4 + i] = s;
    }
    return M;
  }
  const cutPosition = () => mesh.lo[state.axis] + state.cut * mesh.size[state.axis];
  function setCommon(u, f) {
    gl.uniformMatrix4fv(u.uViewProj, false, viewProj(f));
    gl.uniform3fv(u.uLo, mesh.lo); gl.uniform3fv(u.uSize, mesh.size);
    if (state.axis >= 0) {
      const n = [0, 0, 0], s = state.flip ? -1 : 1; n[state.axis] = s;
      gl.uniform4f(u.uPlane, n[0], n[1], n[2], s * cutPosition()); gl.uniform1i(u.uClip, 1);
    } else gl.uniform1i(u.uClip, 0);
  }
  function resize() {
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const w = Math.round(canvas.clientWidth * dpr), h = Math.round(canvas.clientHeight * dpr);
    if (canvas.width !== w || canvas.height !== h) { canvas.width = w; canvas.height = h; dirty = true; }
  }
  function draw() {
    if (!alive) return;
    resize();
    if (dirty) {
      dirty = false;
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      gl.viewport(0, 0, canvas.width, canvas.height);
      gl.clearColor(background[0], background[1], background[2], 1); gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
      if (mesh) {
        const f = frame();
        gl.enable(gl.DEPTH_TEST);
        // Closed surfaces never show their back unless they are sectioned.
        if (state.axis < 0) { gl.enable(gl.CULL_FACE); gl.cullFace(gl.BACK); } else gl.disable(gl.CULL_FACE);
        gl.useProgram(main.p);
        setCommon(main.u, f);
        gl.uniform3fv(main.u.uEye, f.eye); gl.uniform3fv(main.u.uRight, f.right); gl.uniform3fv(main.u.uUp, f.up);
        gl.uniform1i(main.u.uMode, state.mode); gl.uniform1i(main.u.uSmooth, state.smooth ? 1 : 0);
        gl.uniform1f(main.u.uBedZ, mesh.lo[2] + 0.05);
        gl.bindVertexArray(vao);
        gl.drawElements(gl.TRIANGLES, mesh.count, gl.UNSIGNED_INT, 0);
        drawMarkers(f);
      }
    }
    requestAnimationFrame(draw);
  }

  // Markers are drawn without depth testing, so they show through the part:
  // they mark regions inside it. Only front faces, so each sphere is
  // blended once.
  function drawMarkers(f) {
    const { loads, selected, direction } = overlays;
    if (!loads.length && !direction) return;
    gl.useProgram(marker.p);
    gl.uniformMatrix4fv(marker.u.uViewProj, false, viewProj(f));
    gl.uniform3fv(marker.u.uEye, f.eye);
    gl.disable(gl.DEPTH_TEST);
    gl.enable(gl.CULL_FACE); gl.cullFace(gl.BACK);
    gl.enable(gl.BLEND); gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
    gl.bindVertexArray(sphereShape.vao);
    gl.uniform1f(marker.u.uRim, 1);
    loads.forEach((l, i) => {
      const r = l.radius, c = l.center;
      gl.uniformMatrix4fv(marker.u.uModel, false, new Float32Array([r, 0, 0, 0, 0, r, 0, 0, 0, 0, r, 0, c[0], c[1], c[2], 1]));
      // Jade, brighter for the sphere being edited.
      gl.uniform4f(marker.u.uColor, 0.24, 0.75, 0.55, i === selected ? 0.95 : 0.6);
      gl.drawElements(gl.TRIANGLES, sphereShape.count, gl.UNSIGNED_SHORT, 0);
    });
    if (direction) {
      const len = Math.hypot(direction[0], direction[1], direction[2]);
      if (len > 0) {
        const z = direction.map((v) => v / len);
        let x = cross(Math.abs(z[2]) < 0.9 ? [0, 0, 1] : [1, 0, 0], z);
        const lx = Math.hypot(x[0], x[1], x[2]); x = x.map((v) => v / lx);
        const y = cross(z, x), L = mesh.radius * 1.15, c = mesh.centre;
        gl.bindVertexArray(arrowShape.vao);
        gl.disable(gl.CULL_FACE);
        gl.uniform1f(marker.u.uRim, 0);
        gl.uniformMatrix4fv(marker.u.uModel, false, new Float32Array([
          x[0] * L, x[1] * L, x[2] * L, 0, y[0] * L, y[1] * L, y[2] * L, 0, z[0] * L, z[1] * L, z[2] * L, 0, c[0], c[1], c[2], 1]));
        gl.uniform4f(marker.u.uColor, 0.34, 0.8, 0.62, 0.9);
        gl.drawElements(gl.TRIANGLES, arrowShape.count, gl.UNSIGNED_SHORT, 0);
      }
    }
    gl.disable(gl.BLEND);
    gl.enable(gl.DEPTH_TEST);
  }

  // World position under a pixel: draw once into an offscreen float target
  // that stores positions instead of colours, and read that pixel back.
  function pick(px, py) {
    if (!mesh || !gl.getExtension("EXT_color_buffer_float")) return null;
    const fb = gl.createFramebuffer(), tex = gl.createTexture(), rb = gl.createRenderbuffer();
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA32F, canvas.width, canvas.height, 0, gl.RGBA, gl.FLOAT, null);
    gl.bindRenderbuffer(gl.RENDERBUFFER, rb);
    gl.renderbufferStorage(gl.RENDERBUFFER, gl.DEPTH_COMPONENT24, canvas.width, canvas.height);
    gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
    gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.RENDERBUFFER, rb);
    let result = null;
    if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) === gl.FRAMEBUFFER_COMPLETE) {
      gl.viewport(0, 0, canvas.width, canvas.height);
      gl.clearColor(0, 0, 0, 0); gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
      gl.enable(gl.DEPTH_TEST); gl.disable(gl.CULL_FACE);
      gl.useProgram(picker.p);
      setCommon(picker.u, frame());
      gl.bindVertexArray(vao); gl.drawElements(gl.TRIANGLES, mesh.count, gl.UNSIGNED_INT, 0);
      const out = new Float32Array(4); gl.readPixels(px, py, 1, 1, gl.RGBA, gl.FLOAT, out);
      if (out[3] > 0.25) {
        result = [out[0], out[1], out[2]];
        // The back of the surface seen through a section stands for the cut
        // face: the point meant is where the view ray crosses the plane.
        if (out[3] < 0.75 && state.axis >= 0) {
          const e = frame().eye, d = [0, 1, 2].map((a) => result[a] - e[a]);
          const s = state.flip ? -1 : 1, w = s * cutPosition(), nd = s * d[state.axis];
          if (Math.abs(nd) > 1e-9) {
            const t = (w - s * e[state.axis]) / nd;
            if (t > 0 && t <= 1) result = [0, 1, 2].map((a) => e[a] + t * d[a]);
          }
        }
      }
    }
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.deleteFramebuffer(fb); gl.deleteTexture(tex); gl.deleteRenderbuffer(rb);
    dirty = true;
    return result;
  }

  // ---- interaction ---------------------------------------------------------
  let drag = null;
  listen(canvas, "contextmenu", (e) => e.preventDefault());
  listen(canvas, "pointerdown", (e) => {
    canvas.setPointerCapture(e.pointerId); canvas.classList.add("dragging");
    drag = { x: e.clientX, y: e.clientY, pan: e.button === 2 || e.button === 1 || e.shiftKey };
  });
  listen(canvas, "pointerup", (e) => { drag = null; canvas.classList.remove("dragging"); canvas.releasePointerCapture(e.pointerId); });
  listen(canvas, "pointermove", (e) => {
    if (!drag || !mesh) return;
    const dx = e.clientX - drag.x, dy = e.clientY - drag.y; drag.x = e.clientX; drag.y = e.clientY;
    if (drag.pan) {
      const f = frame(), k = cam.dist * 2 * Math.tan(FOV / 2) / Math.max(canvas.clientHeight, 1);
      for (let a = 0; a < 3; a++) cam.target[a] += (-dx * f.right[a] + dy * f.up[a]) * k;
    } else {
      cam.yaw -= dx * 0.008;
      cam.pitch = Math.max(-1.55, Math.min(1.55, cam.pitch + dy * 0.008));
    }
    redraw();
  });
  listen(canvas, "wheel", (e) => {
    e.preventDefault();
    if (!mesh) return;
    cam.dist = Math.max(mesh.radius * 0.02, Math.min(mesh.radius * 20, cam.dist * Math.exp(e.deltaY * 0.0012)));
    redraw();
  }, { passive: false });
  listen(canvas, "dblclick", (e) => {
    resize();
    const box = canvas.getBoundingClientRect();
    const px = Math.round((e.clientX - box.left) * canvas.width / box.width);
    const py = Math.round((box.bottom - e.clientY) * canvas.height / box.height);
    const point = pick(px, py);
    if (point) { cam.target = point; redraw(); }
  });
  listen(window, "resize", redraw);

  const VIEWS = { iso: [-1.05, 0.5], top: [-Math.PI / 2, 1.55], front: [-Math.PI / 2, 0], side: [0, 0] };
  function setView(name) {
    if (!mesh) return;
    [cam.yaw, cam.pitch] = VIEWS[name]; cam.target = mesh.centre.slice(); cam.dist = mesh.radius * 3.2; redraw();
  }
  function setAxis(axis) {
    state.axis = axis;
    // Keep the half away from the viewer, so the cut faces the camera.
    if (axis >= 0) state.flip = frame().back[axis] < 0;
    changed();
  }
  /** Sections the model at a world coordinate along `axis`. */
  function cutAt(axis, position) {
    if (!mesh) return;
    if (state.axis !== axis) setAxis(axis);
    state.cut = Math.max(0, Math.min(1, (position - mesh.lo[axis]) / (mesh.size[axis] || 1)));
    changed();
  }
  /** Point of the part under a page position, or null; on a cut face, the point of the cut. */
  function pickClient(clientX, clientY) {
    resize();
    const box = canvas.getBoundingClientRect();
    const px = Math.round((clientX - box.left) * canvas.width / box.width);
    const py = Math.round((box.bottom - clientY) * canvas.height / box.height);
    return pick(px, py);
  }
  /** Updates the markers (see `overlays`) and redraws. */
  function setOverlays(patch) { overlays = Object.assign({}, overlays, patch); redraw(); }
  /** Changes some of the view state (`mode`, `smooth`, `flip`, `cut`) and redraws. */
  function update(patch) { Object.assign(state, patch); changed(); }
  /** Stops drawing and removes the listeners; the object is unusable afterwards. */
  function dispose() { alive = false; abort.abort(); }
  requestAnimationFrame(draw);
  return { state, setMesh, setMeshFromBinary, setView, setAxis, cutAt, update, pickClient, setOverlays, redraw, changed, cutPosition, bounds: () => mesh, dispose };
}

/**
 * Wires the view controls found under `root`: `[data-axis]`, `[data-mode]`,
 * `[data-shade]` and `[data-view]` buttons, and the elements with class
 * `cut`, `flip`, `cut-label` and `legend-block`. Returns a function that
 * refreshes them.
 */
function bindViewControls(viewer, root, layerHeight) {
  const q = (sel) => root.querySelector(sel);
  const axisButtons = [...root.querySelectorAll("[data-axis]")];
  const modeButtons = [...root.querySelectorAll("[data-mode]")];
  const shadeButtons = [...root.querySelectorAll("[data-shade]")];
  const state = viewer.state;
  function sync() {
    axisButtons.forEach((b) => b.classList.toggle("on", +b.dataset.axis === state.axis));
    modeButtons.forEach((b) => b.classList.toggle("on", +b.dataset.mode === state.mode));
    shadeButtons.forEach((b) => b.classList.toggle("on", (b.dataset.shade === "smooth") === state.smooth));
    q(".legend-block").style.display = state.mode === 1 ? "block" : "none";
    const mesh = viewer.bounds();
    q(".cut").disabled = state.axis < 0 || !mesh;
    q(".cut").value = Math.round(state.cut * 1000);
    // Flip is an action, not a mode: which half is kept is first chosen
    // from the camera, so an "on" look would carry no meaning.
    q(".flip").disabled = state.axis < 0 || !mesh;
    if (state.axis < 0 || !mesh) q(".cut-label").textContent = "no section";
    else {
      const pos = viewer.cutPosition();
      const h = layerHeight();
      const layer = state.axis === 2 && h ? ` · layer ${Math.max(1, Math.ceil((pos - mesh.lo[2]) / h))} at ${h} mm` : "";
      q(".cut-label").textContent = `${"XYZ"[state.axis]} = ${pos.toFixed(2)} mm${layer}`;
    }
  }
  axisButtons.forEach((b) => b.addEventListener("click", () => viewer.setAxis(+b.dataset.axis)));
  modeButtons.forEach((b) => b.addEventListener("click", () => { state.mode = +b.dataset.mode; viewer.changed(); }));
  shadeButtons.forEach((b) => b.addEventListener("click", () => { state.smooth = b.dataset.shade === "smooth"; viewer.changed(); }));
  root.querySelectorAll("[data-view]").forEach((b) => b.addEventListener("click", () => viewer.setView(b.dataset.view)));
  q(".cut").addEventListener("input", (e) => { state.cut = e.target.value / 1000; viewer.changed(); });
  q(".flip").addEventListener("click", () => { state.flip = !state.flip; viewer.changed(); });
  window.addEventListener("keydown", (e) => {
    // Leave typing in form fields alone.
    if (e.ctrlKey || e.metaKey || e.altKey || /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName)) return;
    const k = e.key.toLowerCase();
    if (k === "x" || k === "y" || k === "z") { const a = "xyz".indexOf(k); viewer.setAxis(state.axis === a ? -1 : a); return; }
    if (k === "f") state.flip = !state.flip;
    else if (k === "o") state.mode = 1 - state.mode;
    else if (k === "s") state.smooth = !state.smooth;
    else if (k === "r") { viewer.setView("iso"); state.axis = -1; state.cut = 0.5; state.flip = false; }
    else if (k === "arrowleft" || k === "arrowright") {
      const mesh = viewer.bounds();
      if (state.axis < 0 || !mesh) return;
      const h = layerHeight();
      const step = (e.shiftKey ? 5 : 1) * (state.axis === 2 && h ? h / mesh.size[2] : 0.002);
      state.cut = Math.max(0, Math.min(1, state.cut + (k === "arrowright" ? step : -step)));
    } else return;
    viewer.changed();
  });
  return sync;
}
