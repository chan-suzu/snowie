/* Snowie web demo: a port of the desktop app's snow to a canvas over draggable "windows".
   Flakes, drifts with angle-of-repose settling, snow you grab and roll into a ball, throws
   that splat on windows, stacked balls that become a snowman, a snowplow and fireworks,
   with Web Audio sounds synthesized on the fly. No libraries. */
(() => {
  'use strict';
  const TAU = Math.PI * 2;
  const rand = (a = 1, b) => (b === undefined ? Math.random() * a : a + Math.random() * (b - a));
  const clamp = (v, a, b) => Math.max(a, Math.min(b, v));

  // ---------------------------------------------------------------- links to the repo
  // On GitHub Pages the address is owner.github.io/repo/, so the repo links derive from it.
  (function repoLinks() {
    const m = location.hostname.match(/^([\w-]+)\.github\.io$/);
    const repo = location.pathname.split('/').filter(Boolean)[0] || 'snowie';
    const base = m ? `https://github.com/${m[1]}/${repo}` : 'https://github.com/chan-suzu/snowie';
    document.querySelectorAll('[data-repo-link]').forEach(a => (a.href = base + (a.dataset.path || '')));
    document.querySelectorAll('[data-release-link]').forEach(a => (a.href = base + '/releases/latest'));
    // the exe of the latest release itself: the download starts right away, no GitHub page
    document.querySelectorAll('[data-download]').forEach(a => (a.href = base + '/releases/latest/download/snowie.exe'));
  })();

  // ---------------------------------------------------------------- sprites
  function makeSprite(size, paint) {
    const c = document.createElement('canvas');
    c.width = c.height = size;
    paint(c.getContext('2d'), size);
    return c;
  }
  const flakeSprite = makeSprite(32, (g, n) => {
    const r = g.createRadialGradient(n / 2, n / 2, 0, n / 2, n / 2, n / 2);
    r.addColorStop(0, 'rgba(255,255,255,1)');
    r.addColorStop(0.35, 'rgba(250,252,255,0.9)');
    r.addColorStop(0.75, 'rgba(205,220,240,0.25)');
    r.addColorStop(1, 'rgba(200,215,235,0)');
    g.fillStyle = r;
    g.fillRect(0, 0, n, n);
  });
  const ballSprite = makeSprite(128, (g, n) => {
    const c = n / 2, rad = n / 2 - 1.5;
    const r = g.createRadialGradient(c - rad * 0.4, c - rad * 0.45, rad * 0.05, c, c, rad);
    r.addColorStop(0, '#ffffff');
    r.addColorStop(0.55, '#eef4ff');
    r.addColorStop(0.85, '#c3d2ea');
    r.addColorStop(1, '#9fb2d2');
    g.fillStyle = r;
    g.beginPath();
    g.arc(c, c, rad, 0, TAU);
    g.fill();
    g.strokeStyle = 'rgba(120,140,175,0.6)';
    g.lineWidth = 1.5;
    g.stroke();
  });
  const PALETTE = ['#ff4040', '#ffc033', '#4dff66', '#4d99ff', '#d95aff', '#ff73cc', '#66ffff', '#fffff2'];
  const glowSprites = PALETTE.map(col =>
    makeSprite(32, (g, n) => {
      const r = g.createRadialGradient(n / 2, n / 2, 0, n / 2, n / 2, n / 2);
      r.addColorStop(0, '#ffffff');
      r.addColorStop(0.18, col);
      r.addColorStop(0.5, col + '55');
      r.addColorStop(1, col + '00');
      g.fillStyle = r;
      g.fillRect(0, 0, n, n);
    })
  );

  // ---------------------------------------------------------------- page sky
  (function sky() {
    const cv = document.getElementById('sky');
    if (!cv) return;
    const g = cv.getContext('2d');
    let w = 0, h = 0, flakes = [];
    const resize = () => {
      const dpr = Math.min(devicePixelRatio || 1, 2);
      w = innerWidth; h = innerHeight;
      cv.width = w * dpr; cv.height = h * dpr;
      g.setTransform(dpr, 0, 0, dpr, 0, 0);
      const n = Math.round((w * h) / 9000);
      flakes = Array.from({ length: n }, () => ({ x: rand(w), y: rand(h), d: rand(), p: rand(TAU) }));
    };
    resize();
    addEventListener('resize', resize);
    let last = performance.now();
    (function frame(now) {
      const dt = Math.min((now - last) / 1000, 0.05);
      last = now;
      g.clearRect(0, 0, w, h);
      for (const f of flakes) {
        f.p += dt * (0.7 + f.d);
        f.y += (18 + 40 * f.d) * dt;
        f.x += Math.cos(f.p) * (6 + 10 * f.d) * dt;
        if (f.y > h + 6) { f.y = -6; f.x = rand(w); }
        const r = 0.8 + f.d * f.d * 2.2;
        g.globalAlpha = 0.25 + 0.55 * f.d;
        g.drawImage(flakeSprite, f.x - r * 1.9, f.y - r * 1.9, r * 3.8, r * 3.8);
      }
      g.globalAlpha = 1;
      requestAnimationFrame(frame);
    })(last);
  })();

  // ---------------------------------------------------------------- Tems idle animation
  // A puppet warp of the illustration in WebGL: every pixel samples the picture a few pixels
  // away, by smooth per-body-part offsets. She breathes, sways, her hair and scarf move, the arm
  // with the snowball bobs, and she blinks. Coordinates are pixels of the 900x900 tems-hero.png.
  function idleTems(btn) {
    const img = btn.querySelector('img');
    if (!img || matchMedia('(prefers-reduced-motion: reduce)').matches) return;
    const cv = document.createElement('canvas');
    cv.width = cv.height = 900;
    cv.setAttribute('aria-hidden', 'true');
    const gl = cv.getContext('webgl', { premultipliedAlpha: true, alpha: true, antialias: false });
    if (!gl) return;
    const vs = `attribute vec2 p; varying vec2 uv; void main() { uv = p * 0.5 + 0.5; uv.y = 1.0 - uv.y; gl_Position = vec4(p, 0.0, 1.0); }`;
    const fs = `
      precision mediump float;
      varying vec2 uv;
      uniform sampler2D tex;
      uniform float t, blink;
      const float TAU = 6.2831853;
      float band(float v, float a, float b) { return smoothstep(a - 10.0, a + 10.0, v) * (1.0 - smoothstep(b - 10.0, b + 10.0, v)); }
      // closes one eye: skin from the top down to the lid, then a dark lid line
      vec4 lid(vec4 c, vec2 s, vec2 e, float rx) {
        float dx = (s.x - e.x) / rx, dy = (s.y - e.y) / 8.0;
        float inside = 1.0 - smoothstep(0.8, 1.0, dx * dx + dy * dy);
        if (inside <= 0.0 || blink <= 0.0) return c;
        float edge = 82.0 + 15.0 * blink + 2.0 * (1.0 - dx * dx);
        vec4 skin = vec4(vec3(0.918, 0.922, 0.91) * c.a, c.a);
        vec4 ink = vec4(vec3(0.1, 0.1, 0.12) * c.a, c.a);
        vec4 o = mix(c, skin, inside * step(s.y, edge));
        float ln = (1.0 - smoothstep(0.6, 1.6, abs(s.y - edge))) * step(0.2, blink);
        return mix(o, ink, ln * inside);
      }
      void main() {
        vec2 p = uv * 900.0;
        vec2 d = vec2(0.0);
        float breath = 0.5 - 0.5 * cos(TAU * t / 3.6);
        // the upper body rises with each breath and leans a little around the hips
        float up = 1.0 - smoothstep(300.0, 390.0, p.y);
        d.y -= 4.0 * breath * up;
        d.x += 0.011 * sin(TAU * t / 7.0) * max(0.0, 430.0 - p.y);
        // head tilt
        float head = 1.0 - smoothstep(150.0, 240.0, p.y);
        d.x += 1.6 * sin(TAU * t / 5.0 + 0.7) * head;
        // strands of hair on both sides sway, more at the tips
        float hair = (band(p.x, 360.0, 428.0) + band(p.x, 532.0, 575.0)) * band(p.y, 40.0, 270.0);
        d.x += 2.4 * sin(TAU * t / 2.7 + p.y * 0.03) * hair * smoothstep(60.0, 220.0, p.y);
        // the hanging end of the scarf swings, the fringe flutters
        float sk = clamp((p.y - 190.0) / 175.0, 0.0, 1.0);
        float scarf = band(p.x, 472.0, 560.0) * band(p.y, 190.0, 362.0);
        d.x += (3.4 * sin(TAU * t / 2.3 - sk * 2.5) + 1.1 * sin(TAU * t / 0.8 + sk * 7.0)) * sk * sk * scarf;
        // the arm with the snowball weighs the ball
        float arm = band(p.x, 228.0, 405.0) * band(p.y, 72.0, 305.0);
        float wa = clamp((300.0 - p.y) / 220.0, 0.0, 1.0);
        d.y += 3.2 * sin(TAU * t / 2.4) * wa * arm;
        d.x += 1.0 * sin(TAU * t / 2.4 + 1.2) * wa * arm;
        // the hanging sleeve swings a little
        float sl = band(p.x, 558.0, 672.0) * band(p.y, 212.0, 452.0);
        d.x += 2.8 * sin(TAU * t / 3.6 + 0.8) * clamp((p.y - 215.0) / 235.0, 0.0, 1.0) * sl;
        // the painted snowflakes drift
        vec2 f0 = vec2(320.0, 16.0), f1 = vec2(622.0, 106.0), f2 = vec2(634.0, 246.0), f3 = vec2(312.0, 365.0);
        d += vec2(1.5 * cos(t * 1.1), 3.0 * sin(t * 1.3)) * (1.0 - smoothstep(16.0, 26.0, distance(p, f0)));
        d += vec2(1.5 * cos(t * 0.9 + 1.0), 3.0 * sin(t * 1.2 + 2.0)) * (1.0 - smoothstep(16.0, 26.0, distance(p, f1)));
        d += vec2(1.5 * cos(t * 1.3 + 2.0), 3.0 * sin(t * 1.0 + 4.0)) * (1.0 - smoothstep(16.0, 26.0, distance(p, f2)));
        d += vec2(1.5 * cos(t * 1.2 + 3.0), 3.0 * sin(t * 1.4 + 1.0)) * (1.0 - smoothstep(16.0, 26.0, distance(p, f3)));
        vec2 s = p - d;
        if (s.x < 0.0 || s.y < 0.0 || s.x > 900.0 || s.y > 900.0) { gl_FragColor = vec4(0.0); return; }
        vec4 c = texture2D(tex, s / 900.0);
        c = lid(c, s, vec2(445.0, 90.0), 14.0);
        c = lid(c, s, vec2(490.0, 90.0), 15.0);
        gl_FragColor = c;
      }`;
    const sh = (type, src) => {
      const o = gl.createShader(type);
      gl.shaderSource(o, src);
      gl.compileShader(o);
      if (!gl.getShaderParameter(o, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(o));
      return o;
    };
    let prog;
    try {
      prog = gl.createProgram();
      gl.attachShader(prog, sh(gl.VERTEX_SHADER, vs));
      gl.attachShader(prog, sh(gl.FRAGMENT_SHADER, fs));
      gl.linkProgram(prog);
      if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(prog));
    } catch (e) {
      console.warn('Tems idle animation disabled:', e);
      return; // the static picture stays
    }
    gl.useProgram(prog);
    const buf = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, buf);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
    const loc = gl.getAttribLocation(prog, 'p');
    gl.enableVertexAttribArray(loc);
    gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
    const uT = gl.getUniformLocation(prog, 't'), uBlink = gl.getUniformLocation(prog, 'blink');
    const start = () => {
      const tex = gl.createTexture();
      gl.bindTexture(gl.TEXTURE_2D, tex);
      gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, true);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, img);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      btn.insertBefore(cv, img);
      btn.classList.add('gl');
      let onScreen = true, nextBlink = 2.5, blinkAt = -1;
      new IntersectionObserver(es => (onScreen = es[0].isIntersecting)).observe(btn);
      const t0 = performance.now();
      (function frame(now) {
        requestAnimationFrame(frame);
        if (!onScreen || document.hidden) return;
        // window.__temsDebug = { t, blink } pins a moment, for checking the animation by eye
        const dbg = window.__temsDebug || {};
        const t = dbg.t ?? (now - t0) / 1000;
        // a blink every 2.5-6 s, sometimes a double one
        if (t > nextBlink) { blinkAt = t; nextBlink = t + 2.5 + Math.random() * 3.5; if (Math.random() < 0.2) nextBlink = t + 0.35; }
        const bt = (t - blinkAt) / 0.16;
        const blink = dbg.blink ?? (blinkAt >= 0 && bt < 1 ? 1 - Math.abs(bt * 2 - 1) : 0);
        gl.viewport(0, 0, 900, 900);
        gl.clearColor(0, 0, 0, 0);
        gl.clear(gl.COLOR_BUFFER_BIT);
        gl.uniform1f(uT, t);
        gl.uniform1f(uBlink, blink);
        gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
      })(t0);
    };
    if (img.complete && img.naturalWidth) start(); else img.addEventListener('load', start, { once: true });
  }

  // ---------------------------------------------------------------- Tems in the hero
  // Hops now and then; a click makes her throw a snowball that flies off in an arc and splats.
  // Suzu next to Tems: hops when clicked
  (function heroSuzu() {
    const btn = document.getElementById('suzu');
    if (!btn) return;
    btn.addEventListener('click', () => {
      btn.classList.remove('hop'); void btn.offsetWidth; btn.classList.add('hop');
      setTimeout(() => btn.classList.remove('hop'), 700);
    });
  })();

  (function heroTems() {
    const btn = document.getElementById('tems');
    if (!btn) return;
    idleTems(btn);
    const replay = cls => { btn.classList.remove('hop', 'throw'); void btn.offsetWidth; btn.classList.add(cls); };
    setInterval(() => { if (!document.hidden && !btn.classList.contains('busy')) replay('hop'); }, 7000);
    btn.addEventListener('click', () => {
      btn.classList.add('used', 'busy');
      replay('throw');
      setTimeout(() => btn.classList.remove('busy'), 900);
      const r = btn.getBoundingClientRect();
      // the snowball in her raised hand
      const x0 = r.left + r.width * 0.36, y0 = r.top + r.height * 0.14;
      const vx = -(420 + Math.random() * 260), vy = -(380 + Math.random() * 200), g = 1100;
      const ball = document.createElement('div');
      ball.className = 'thrown';
      document.body.appendChild(ball);
      const floorY = Math.min(innerHeight - 20, r.bottom + 40);
      let t = 0, last = performance.now();
      (function fly(now) {
        t += Math.min((now - last) / 1000, 0.05); last = now;
        const x = x0 + vx * t, y = y0 + vy * t + 0.5 * g * t * t;
        ball.style.transform = `translate(${x}px, ${y}px) rotate(${t * -720}deg)`;
        if (y < floorY && x > -40) { requestAnimationFrame(fly); return; }
        ball.remove();
        for (let i = 0; i < 14; i++) {
          const p = document.createElement('div');
          p.className = 'puff';
          document.body.appendChild(p);
          const a = Math.PI + Math.random() * Math.PI, sp = 40 + Math.random() * 90;
          p.animate(
            [{ transform: `translate(${x}px, ${y}px)`, opacity: 1 },
             { transform: `translate(${x + Math.cos(a) * sp}px, ${y + Math.sin(a) * sp + 40}px)`, opacity: 0 }],
            { duration: 600 + Math.random() * 300, easing: 'cubic-bezier(.2,.7,.4,1)' }
          ).onfinish = () => p.remove();
        }
        Sound.play('splat', 0, 0.6);
      })(last);
      Sound.play('throw', 0, 0.6);
    });
  })();

  // ---------------------------------------------------------------- audio
  // Snow sounds, synthesized like in the app: filtered noise grains for crunch, sweeps for whooshes.
  const Sound = (() => {
    let ctx = null, master = null, on = false;
    const bufs = {};
    function svf() {
      let low = 0, band = 0;
      return {
        band(x, f, q, sr) {
          const k = 2 * Math.sin((Math.PI * f) / sr);
          const high = x - low - band / q;
          band += k * high; low += k * band;
          return band;
        },
        low(x, f, sr) { this.band(x, f, 0.7, sr); return low; },
      };
    }
    function crunch(sr, len, grains, center) {
      const out = new Float32Array(Math.ceil((len + 0.03) * sr));
      for (let g = 0; g < grains; g++) {
        const t0 = Math.floor((len * Math.pow(g / grains, 0.8) + rand(0.01)) * sr);
        const gl = Math.floor((0.003 + rand(0.009)) * sr);
        const f = center * rand(0.6, 1.5), amp = rand(0.4, 1) * (1 - (g / grains) * 0.6);
        const fl = svf();
        for (let i = 0; i < gl && t0 + i < out.length; i++) out[t0 + i] += fl.band((Math.random() * 2 - 1) * Math.exp(-i / (gl * 0.3)), f, 2.5, sr) * amp;
      }
      return out;
    }
    function thud(sr, f0, f1, len, noise) {
      const n = Math.floor(len * sr), out = new Float32Array(n), lp = svf();
      let ph = 0;
      for (let i = 0; i < n; i++) {
        const t = i / sr, k = i / n;
        ph += (TAU * (f0 + (f1 - f0) * k)) / sr;
        out[i] = Math.sin(ph) * Math.exp(-t / (len * 0.35)) * 0.8 + lp.low(Math.random() * 2 - 1, 1400, sr) * Math.exp(-t / (len * 0.25)) * noise;
      }
      return out;
    }
    function mix(a, b, gain = 1) { for (let i = 0; i < Math.min(a.length, b.length); i++) a[i] += b[i] * gain; return a; }
    function norm(a, p) { let m = 1e-6; for (const x of a) m = Math.max(m, Math.abs(x)); for (let i = 0; i < a.length; i++) a[i] *= p / m; return a; }
    function build() {
      const sr = ctx.sampleRate;
      const mk = data => { const b = ctx.createBuffer(1, data.length, sr); b.copyToChannel(data, 0); return b; };
      bufs.grab = mk(norm(crunch(sr, 0.18, 28, 2600), 0.55));
      bufs.roll = mk(norm(crunch(sr, 0.07, 9, 1800), 0.3));
      const wh = new Float32Array(Math.floor(0.32 * sr)), f = svf();
      for (let i = 0; i < wh.length; i++) { const t = i / wh.length; wh[i] = f.band(Math.random() * 2 - 1, 350 + 1400 * Math.pow(t, 0.7), 3, sr) * Math.min(t / 0.12, 1) * Math.pow(1 - t, 1.6); }
      bufs.throw = mk(norm(wh, 0.45));
      bufs.splat = mk(norm(mix(thud(sr, 95, 50, 0.28, 0.9), crunch(sr, 0.22, 34, 1700), 0.7), 0.8));
      bufs.plop = mk(norm(mix(thud(sr, 140, 80, 0.14, 0.4), crunch(sr, 0.06, 8, 2200), 0.4), 0.5));
      const ch = new Float32Array(Math.floor(1.5 * sr));
      [1046.5, 1318.5, 1568, 2093].forEach((fr, k) => {
        const at = Math.floor(k * 0.09 * sr);
        for (let i = 0; at + i < ch.length; i++) { const t = i / sr; ch[at + i] += Math.sin(TAU * fr * t) * Math.exp(-t / 0.45) * 0.35; }
      });
      bufs.chime = mk(norm(ch, 0.5));
      const bm = new Float32Array(Math.floor(1.4 * sr)), lp = svf();
      for (let i = 0; i < bm.length; i++) { const t = i / sr; bm[i] = lp.low(Math.random() * 2 - 1, 260 + 600 * Math.exp(-t / 0.05), sr) * Math.exp(-t / 0.35); }
      norm(bm, 0.9);
      for (let k = 0; k < 70; k++) { const at = Math.floor(rand(0.25, 1.3) * sr); for (let i = 0; i < 60 && at + i < bm.length; i++) bm[at + i] += (Math.random() * 2 - 1) * Math.exp(-i / 12) * rand(0.1, 0.25); }
      bufs.boom = mk(bm);
      const wl = new Float32Array(Math.floor(1.0 * sr));
      let ph = 0;
      for (let i = 0; i < wl.length; i++) { const t = i / wl.length; ph += (TAU * (900 + 1700 * t)) / sr; wl[i] = Math.sin(ph) * 0.22 * Math.min(t / 0.05, 1) * Math.pow(1 - t, 0.8); }
      bufs.launch = mk(wl);
    }
    function ensure() {
      if (ctx) return;
      const AC = window.AudioContext || window.webkitAudioContext;
      if (!AC) return;
      ctx = new AC();
      master = ctx.createGain();
      master.gain.value = 0.7;
      master.connect(ctx.destination);
      build();
    }
    return {
      get on() { return on; },
      toggle() { ensure(); on = !on; if (ctx && ctx.state === 'suspended') ctx.resume(); return on; },
      unlock() { if (on) { ensure(); if (ctx.state === 'suspended') ctx.resume(); } },
      play(name, pan = 0, gain = 1, rate = 1) {
        if (!on || !ctx || !bufs[name]) return;
        const s = ctx.createBufferSource();
        s.buffer = bufs[name];
        s.playbackRate.value = rate;
        const g = ctx.createGain();
        g.gain.value = gain;
        let node = s.connect(g);
        if (ctx.createStereoPanner) { const p = ctx.createStereoPanner(); p.pan.value = clamp(pan, -1, 1); node = g.connect(p); }
        node.connect(master);
        s.start();
      },
    };
  })();

  // ---------------------------------------------------------------- the desk
  const desk = document.getElementById('desk');
  const cv = document.getElementById('snow');
  if (!desk || !cv) return;
  const g = cv.getContext('2d');
  const COL = 2; // drift resolution, px per column
  let W = 0, H = 0, FLOOR = 0, dpr = 1;

  // windows keep their relative place (from the inline left: N%) until the user drags them,
  // and shrink on narrow screens
  const wins = [...desk.querySelectorAll('.win')].map((el, i) => ({
    el, z: i + 1, fx: parseFloat(el.style.left) / 100 || 0, fy: el.offsetTop, ow: parseFloat(el.style.width) || el.offsetWidth,
    x: 0, y: el.offsetTop, w: el.offsetWidth, h: el.offsetHeight, moved: false,
    surf: null, hist: [], vel: { x: 0, y: 0 }, jerk: 0,
  }));
  const zSorted = () => [...wins].sort((a, b) => b.z - a.z); // topmost first

  function newSurface(left, top, width, maxH, walls) {
    const n = Math.max(1, Math.ceil(width / COL));
    return { left, top, n, h: new Float32Array(n), vis: new Uint8Array(n).fill(walls ? 1 : 0), maxH, walls, spillL: 0, spillR: 0, wave: Math.random() * 100 };
  }
  let floor = newSurface(0, 0, 10, 50, true);

  function layout() {
    const r = desk.getBoundingClientRect();
    dpr = Math.min(devicePixelRatio || 1, 2);
    const oldW = W;
    W = r.width; H = r.height;
    cv.width = Math.round(W * dpr); cv.height = Math.round(H * dpr);
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    FLOOR = H - desk.querySelector('.taskbar').offsetHeight;
    if (oldW !== W || floor.n !== Math.ceil(W / COL)) {
      // keep the snow when the page is resized: stretch the old drift onto the new width
      const old = floor;
      floor = newSurface(0, FLOOR, W, 50, true);
      if (old.n > 1) for (let c = 0; c < floor.n; c++) floor.h[c] = old.h[Math.min(old.n - 1, Math.floor((c * old.n) / floor.n))];
    }
    floor.top = FLOOR;
    const narrow = W < 700;
    wins.forEach((w, i) => {
      const nw = Math.min(w.ow, W - 24);
      if (nw !== w.w) {
        w.el.style.width = nw + 'px';
        w.w = nw;
        if (w.surf) {
          const old = w.surf;
          w.surf = newSurface(w.x, w.y, w.w, 24, false);
          for (let c = 0; c < w.surf.n; c++) w.surf.h[c] = old.h[Math.min(old.n - 1, Math.floor((c * old.n) / w.surf.n))];
        }
      }
      w.h = w.el.offsetHeight;
      if (!w.moved) {
        w.x = clamp(narrow ? 12 + i * 18 : w.fx * W, 8, Math.max(8, W - w.w - 8));
        w.y = narrow ? 175 + i * 105 : w.fy; // room above the first window for Tems and the buttons
      } else w.x = clamp(w.x, -w.w + 60, W - 60);
      w.el.style.left = w.x + 'px';
      w.el.style.top = w.y + 'px';
    });
  }
  layout();
  addEventListener('resize', layout);
  for (const w of wins) w.surf = newSurface(w.x, w.y, w.w, 24, false);

  // ---------------------------------------------------------------- state
  const parts = []; // {x,y,vx,vy,r,m,kind:0 flake 1 chunk 2 spray,d,p,a}
  const balls = [];
  const stacks = []; // {surf: win|null(floor), x (rel), balls:[{r,m,off:{x,y}}], centers:[], seed}
  const marks = []; // {win, rx, ry, r, life, blobs}
  let held = null;
  const fw = { rockets: [], sparks: [], left: 0, rate: 0, acc: 0 };
  let plow = null, nextPlow = 25;
  let time = 0;
  const MAXP = 2600;

  // ---------------------------------------------------------------- drift helpers
  const colOf = (s, x) => { const c = Math.floor((x - s.left) / COL); return c >= 0 && c < s.n ? c : -1; };
  const surfY = (s, c) => s.top - s.h[c];
  // how high the drift can grow at column i: not one flat limit but a gentle roll along the
  // window, so a full drift is a row of soft mounds instead of a shelf cut off at the top
  function cap(s, i) {
    const x = i * COL, p = s.wave;
    const n = 0.55 * Math.sin(x / 61 + p) + 0.3 * Math.sin(x / 27 + p * 1.7) + 0.15 * Math.sin(x / 13 + p * 2.9);
    return s.maxH * (0.8 + 0.2 * n);
  }
  function deposit(s, x, m, spread) {
    const c = colOf(s, x);
    if (c < 0) return;
    const w = Math.max(1, Math.round(spread / COL));
    let sum = 0;
    for (let k = -w; k <= w; k++) if (c + k >= 0 && c + k < s.n) sum += w + 1 - Math.abs(k);
    for (let k = -w; k <= w; k++) {
      const i = c + k;
      if (i < 0 || i >= s.n) continue;
      // growth slows down well before the cap, so the top rounds off instead of flattening
      const lim = cap(s, i), room = clamp((lim - s.h[i]) / (0.35 * lim), 0, 1);
      s.h[i] = Math.min(s.maxH, s.h[i] + room * (m / COL) * (w + 1 - Math.abs(k)) / sum);
    }
  }
  function take(s, cx, radius, depth) {
    let got = 0;
    const a = Math.max(0, Math.floor((cx - radius - s.left) / COL)), b = Math.min(s.n, Math.ceil((cx + radius - s.left) / COL));
    for (let i = a; i < b; i++) {
      const dx = (s.left + i * COL + COL / 2 - cx) / radius;
      if (Math.abs(dx) >= 1) continue;
      const t = Math.min(s.h[i], depth * Math.sqrt(1 - dx * dx));
      s.h[i] -= t; got += t * COL;
    }
    return got;
  }
  function relax(s) {
    const h = s.h, n = s.n, slope = 0.85 * COL;
    if (n < 2) return;
    // snow piled above its cap slowly settles back, so the mounds keep their shape
    const low = s.maxH * 0.6;
    for (let i = 0; i < n; i++) if (h[i] > low) { const c = cap(s, i); if (h[i] > c + 0.05) h[i] -= (h[i] - c) * 0.01; }
    const flip = Math.random() < 0.5;
    const step = i => {
      const d = h[i] - h[i + 1];
      if (d > slope) { const t = (d - slope) * 0.45; h[i] -= t; h[i + 1] += t; }
      else if (d < -slope) { const t = (-d - slope) * 0.45; h[i + 1] -= t; h[i] += t; }
    };
    if (flip) for (let i = n - 2; i >= 0; i--) step(i); else for (let i = 0; i < n - 1; i++) step(i);
    let prev = h[0];
    for (let i = 1; i < n - 1; i++) { const cur = h[i]; h[i] = cur + (prev + h[i + 1] - 2 * cur) * 0.04; prev = cur; }
    if (!s.walls) {
      const lip = 4;
      if (h[0] > lip) { const e = (h[0] - lip) * 0.35; h[0] -= e; s.spillL += e * COL; }
      if (h[n - 1] > lip) { const e = (h[n - 1] - lip) * 0.35; h[n - 1] -= e; s.spillR += e * COL; }
    }
  }
  function chunk(x, y, vx, vy, m) {
    if (parts.length >= MAXP) return;
    parts.push({ x, y, vx, vy, m, r: clamp(Math.sqrt(m / Math.PI) * rand(0.7, 1.3), 1, 3.6), kind: 1, a: 1 });
  }
  function shed(s, frac, vx, vy) {
    for (let i = 0; i < s.n; i += 2) {
      let m = 0, hmax = 0;
      for (let k = i; k < Math.min(i + 2, s.n); k++) { const t = s.h[k] * frac; s.h[k] -= t; m += t * COL; hmax = Math.max(hmax, s.h[k]); }
      if (m > 0.5) chunk(s.left + i * COL + COL, s.top - hmax - 1 - rand(2), vx + rand(-30, 30), vy - rand(30));
    }
  }

  function surfaces() {
    // back to front: windows bottom-up, then the floor
    return [...wins].sort((a, b) => a.z - b.z).map(w => w.surf).concat([floor]);
  }
  function visibility() {
    const order = zSorted();
    order.forEach((w, i) => {
      const s = w.surf, y = w.y - 1;
      for (let c = 0; c < s.n; c++) {
        const x = s.left + c * COL + 1;
        let vis = y > 4 && x >= 0 && x < W;
        for (let j = 0; j < i && vis; j++) { const o = order[j]; if (x >= o.x && x < o.x + o.w && y >= o.y && y < o.y + o.h) vis = false; }
        s.vis[c] = vis ? 1 : 0;
      }
    });
  }
  function findLanding(x, y0, y1, strict) {
    let best = null, by = Infinity;
    for (const s of surfaces()) {
      if (y1 < s.top - s.maxH - 2 || y0 > s.top + 1) continue;
      const c = colOf(s, x);
      if (c < 0 || !s.vis[c]) continue;
      const sy = surfY(s, c);
      const hit = strict ? y0 <= sy + 1 && y1 >= sy : y0 <= s.top && y1 >= sy;
      if (hit && sy < by) { best = s; by = sy; }
    }
    return best;
  }
  function hitSurface(x, y, slack, minH) {
    const test = s => { const c = colOf(s, x); return c >= 0 && s.vis[c] && s.h[c] >= minH && y >= surfY(s, c) - slack && y < s.top + 1; };
    if (test(floor)) return floor;
    for (const w of zSorted()) if (test(w.surf)) return w.surf;
    return null;
  }
  const ballR = m => Math.max(5, Math.sqrt(m / Math.PI) * 1.5);
  const panOf = x => (x / W) * 2 - 1;
  const winOf = s => wins.find(w => w.surf === s);

  // ---------------------------------------------------------------- windows: drag, shake
  let drag = null;
  desk.addEventListener('pointerdown', e => {
    Sound.unlock();
    const p = local(e);
    // snow first: a press on snow grabs it instead of the window under it
    if (tryGrab(p)) {
      try { desk.setPointerCapture(e.pointerId); } catch (_) { /* not an active pointer */ }
      e.preventDefault();
      e.stopPropagation();
      return;
    }
    const winEl = e.target.closest('.win');
    if (!winEl) return;
    const w = wins.find(o => o.el === winEl);
    const top = Math.max(...wins.map(o => o.z));
    if (w.z !== top) { w.z = top + 1; winEl.style.zIndex = w.z; }
    if (e.target.closest('.bar') && !e.target.closest('.ctl')) {
      drag = { w, dx: p.x - w.x, dy: p.y - w.y, id: e.pointerId };
      w.moved = true;
      winEl.classList.add('drag');
      try { desk.setPointerCapture(e.pointerId); } catch (_) { /* not an active pointer */ }
      e.preventDefault();
    }
  }, true);
  desk.addEventListener('pointermove', e => {
    const p = local(e);
    if (held) moveHeld(p);
    else if (carry) { carry.tx = p.x + carry.gx; carry.ty = p.y + carry.gy; }
    else if (drag && e.pointerId === drag.id) {
      const w = drag.w;
      w.x = clamp(p.x - drag.dx, -w.w + 60, W - 60);
      w.y = clamp(p.y - drag.dy, 0, FLOOR - 34);
      w.el.style.left = w.x + 'px';
      w.el.style.top = w.y + 'px';
    }
  });
  const endPointer = e => {
    if (held) release(local(e));
    if (carry) putDown(local(e));
    if (drag) { drag.w.el.classList.remove('drag'); drag = null; }
  };
  desk.addEventListener('pointerup', endPointer);
  desk.addEventListener('pointercancel', endPointer);
  function local(e) { const r = desk.getBoundingClientRect(); return { x: e.clientX - r.left, y: e.clientY - r.top }; }

  // A moving window works like a snowplow blade on every other drift: snow that ends up inside
  // the window (or inside the snow on its top) is shoved out ahead of it, in the direction of
  // motion; pushed past the end of a window, it falls off.
  function pushSnow() {
    for (const a of wins) {
      if (a.px === undefined) { a.px = a.x; a.py = a.y; continue; }
      const dx = a.x - a.px, dy = a.y - a.py;
      a.px = a.x; a.py = a.y;
      if (!dx && !dy) continue;
      const own = a.surf;
      const edge = Math.max(own.h[0] || 0, own.h[own.n - 1] || 0, ...Array.from(own.h.subarray(0, own.n), v => v * 0.6));
      // only a drift pushes a drift: the top edge with its snow, moving sideways, against snow at
      // about the same level; passing in front of other snow (up, down, other height) just hides it
      // direction from the smoothed velocity: a hand drag jitters, and a single sideways step
      // while the window rises past another drift must not count as a push
      const v = a.vel;
      const sideways = Math.abs(v.x) >= 150 && Math.abs(v.x) >= Math.abs(v.y) * 2;
      // a jump (the window snapped somewhere) is not a drag through the snow
      if (!sideways || !dx || Math.abs(dx) > 150 || Math.sign(dx) !== Math.sign(v.x)) continue;
      const dir = Math.sign(dx);
      const bandTop = a.y - edge - 1, bandBot = a.y + 6;
      // the blade is the strip the leading edge swept this frame: snow already under the window
      // (it came from below or above) stays where it is, hidden behind it
      const sweep = Math.ceil(Math.abs(dx)) + 2 + COL;
      const blade = dir > 0 ? { x: a.x + a.w - sweep, w: sweep } : { x: a.x, w: sweep };
      for (const o of wins) {
        if (o === a || o.z > a.z) continue; // a window in front of the moving one is not touched
        plowSurface(o.surf, blade, bandTop, bandBot, dir);
        pushStacks(o, blade, bandTop, bandBot, dir);
      }
    }
  }
  function plowSurface(s, a, bandTop, bandBot, dir) {
    if (s.top <= bandTop || s.top - s.maxH * 1.6 >= bandBot) return;
    const c0 = Math.max(0, Math.floor((a.x - s.left) / COL)), c1 = Math.min(s.n, Math.ceil((a.x + a.w - s.left) / COL));
    if (c0 >= c1) return;
    const mid = (c0 + c1) / 2;
    let left = 0, right = 0;
    for (let c = c0; c < c1; c++) {
      if (s.h[c] <= 0.05 || s.top - s.h[c] >= bandBot) continue;
      if (dir > 0 || (dir === 0 && c >= mid)) right += s.h[c]; else left += s.h[c];
      s.h[c] = 0;
    }
    dump(s, c1, right, 1);
    dump(s, c0 - 1, left, -1);
  }
  // piles pushed snow up just past the blade: a mound leaning on the blade and sloping away from
  // it, taller and wider the more snow is pushed; what runs off the end of the drift falls
  function dump(s, c, m, side) {
    if (m <= 0) return;
    const slope = 0.5 * COL, top = s.maxH * 1.4;
    let end = c;
    for (let pass = 0; pass < 4 && m > 0.01 && c >= 0 && c < s.n; pass++) {
      const base = s.h[c], peak = Math.min(top, base + Math.sqrt(m * slope)); // a triangle holds H² / slope
      if (peak <= base + 0.01) break;
      for (let k = 0; ; k++) {
        const i = c + side * k;
        if (i < 0 || i >= s.n) { end = i; break; }
        const target = peak - slope * k;
        if (target <= s.h[i]) { if (target < base - 2) break; continue; }
        const add = Math.min(m, target - s.h[i]);
        s.h[i] += add; m -= add;
        if (m <= 0.01) break;
      }
    }
    if (m > 0.01) {
      const at = end < 0 || end >= s.n ? end : c;
      const x = s.left + clamp(at, 0, s.n) * COL, n = Math.min(12, Math.ceil(m / 3));
      for (let k = 0; k < n; k++) chunk(x + side * rand(1, 6), s.top - rand(4, 14), side * rand(30, 120), -rand(20, 80), (m * COL) / n);
    }
  }
  function pushStacks(o, a, bandTop, bandBot, dir) {
    const s = o ? o.surf : floor;
    for (const st of stacks) {
      if (st.carried || (st.surf || null) !== o || !st.centers.length) continue;
      const x = s.left + st.x, r = st.balls[0].r;
      if (s.top <= bandTop || s.top - r * 2 >= bandBot) continue;
      if (x + r < a.x || x - r > a.x + a.w) continue;
      const toRight = dir > 0 || (dir === 0 && x >= a.x + a.w / 2);
      st.x = (toRight ? a.x + a.w + r : a.x - r) - s.left;
    }
  }

  function syncWindows(dt) {
    for (const w of wins) {
      const s = w.surf;
      // the drift rides with its window; speed and jerk over the last ~60 ms
      if (s.left !== w.x || s.top !== w.y) { s.left = w.x; s.top = w.y; }
      w.hist.push({ t: time, x: w.x, y: w.y, vx: w.vel.x, vy: w.vel.y });
      while (w.hist.length > 2 && time - w.hist[1].t >= 0.035) w.hist.shift();
      const h0 = w.hist[0], span = time - h0.t;
      if (span > 1e-3) {
        const vx = (w.x - h0.x) / span, vy = (w.y - h0.y) / span;
        w.jerk = Math.hypot(vx - h0.vx, vy - h0.vy) / span;
        w.vel = { x: vx, y: vy };
      }
      // browsers report pointer moves less often than the OS, so a shake reads softer here
      if (w.jerk > 12000) shed(s, Math.min(0.7, ((w.jerk - 12000) / 30000) * dt * 10), w.vel.x * 0.35, w.vel.y * 0.35);
      if (s.spillL > 5) { chunk(s.left - 1.5, s.top - s.h[0] - 1, -rand(10, 40), 0, s.spillL); s.spillL = 0; }
      if (s.spillR > 5) { chunk(s.left + s.n * COL + 0.5, s.top - s.h[s.n - 1] - 1, rand(10, 40), 0, s.spillR); s.spillR = 0; }
    }
  }

  // ---------------------------------------------------------------- grabbing and rolling
  function tryGrab(p) {
    if (held || carry) return false;
    if (box && p.x > box.x - box.w / 2 - 4 && p.x < box.x + box.w / 2 + 4 && p.y > box.y - box.h - 4 && p.y < box.y + 4) {
      salute(4, 2.5);
      return true;
    }
    for (let i = 0; i < stacks.length; i++) {
      const st = stacks[i];
      const j = st.centers.findIndex((c, k) => Math.hypot(p.x - c.x, p.y - c.y) <= st.balls[k].r);
      if (j < 0) continue;
      // grabbed by the bottom ball, a stack or a snowman moves whole; anywhere else the top ball comes off
      if (j === 0 && st.balls.length >= 2) {
        const fx = st.centers[0].x, fy = st.centers[0].y + st.balls[0].r * 0.85;
        st.carried = true;
        carry = { st, fx, fy, tx: fx, ty: fy, vx: 0, gx: fx - p.x, gy: fy - p.y };
        Sound.play('grab', panOf(p.x));
        return true;
      }
      {
        const c = st.centers[st.centers.length - 1], b = st.balls.pop();
        if (!st.balls.length) stacks.splice(i, 1);
        held = mkHeld(c, p, b.m, 1, []);
        Sound.play('grab', panOf(p.x));
        return true;
      }
    }
    const s = hitSurface(p.x, p.y, 5, 3);
    if (!s) return false;
    const m = take(s, p.x, 24, 14);
    if (m < 15) return false;
    const blobs = Array.from({ length: 9 }, (_, i) => {
      const a = i * 2.4 + rand(), d = i === 0 ? 0 : rand(0.45, 0.95);
      return { x: Math.cos(a) * d, y: Math.sin(a) * d, s: rand(0.35, 0.65) };
    });
    held = mkHeld(p, p, m, 0, blobs);
    Sound.play('grab', panOf(p.x), 1, rand(0.9, 1.15));
    return true;
  }
  function mkHeld(pos, cursor, m, comp, blobs) {
    return { x: pos.x, y: pos.y, cx: cursor.x, cy: cursor.y, vx: 0, vy: 0, m, comp, rot: 0, spin: 0, lastDir: null, lx: cursor.x, ly: cursor.y, blobs, roll: 0 };
  }
  let carry = null;
  // the carried stack trails the cursor, slower the heavier it is; the upper balls sway behind
  function updateCarry(dt) {
    const c = carry;
    if (!c) return;
    const st = c.st, r0 = st.balls[0].r, m = st.balls.reduce((a, b) => a + b.m, 0);
    const rate = 22 / (1 + (ballR(m) / 45) ** 2), k = 1 - Math.exp(-rate * dt), px = c.fx;
    c.fx += (c.tx - c.fx) * k; c.fy += (c.ty - c.fy) * k;
    c.vx = c.vx * 0.6 + ((c.fx - px) / Math.max(dt, 1e-4)) * 0.4;
    st.centers = [];
    let y = c.fy + r0 * 0.15;
    const ease = Math.exp(-10 * dt);
    st.balls.forEach((b, j) => {
      y -= b.r * (j === 0 ? 1 : 0.8);
      const lean = clamp(-c.vx * 0.006 * j, -b.r * 0.35, b.r * 0.35);
      b.off.x = lean + (b.off.x - lean) * ease; b.off.y *= ease;
      st.centers.push({ x: c.fx + b.off.x, y: y + b.off.y });
      y -= b.r * 0.8;
    });
  }
  // sets the carried stack down on the first drift below it; over nothing it falls apart
  function putDown(p) {
    const c = carry, st = c.st;
    carry = null;
    st.carried = false;
    c.tx = p.x + c.gx; c.ty = p.y + c.gy;
    const r0 = st.balls[0].r, s = findLanding(c.fx, c.fy - r0 * 0.5, 1e5, false), col = s ? colOf(s, c.fx) : -1;
    if (s && col >= 0) {
      st.surf = s === floor ? null : winOf(s);
      st.x = c.fx - s.left;
      // the balls ease from where they are down onto the drift instead of jumping
      let y = surfY(s, col) + r0 * 0.15;
      st.balls.forEach((b, j) => {
        y -= b.r * (j === 0 ? 1 : 0.8);
        const o = st.centers[j] || { x: c.fx, y };
        b.off = { x: o.x - c.fx, y: o.y - y };
        y -= b.r * 0.8;
      });
      Sound.play('plop', panOf(c.fx));
      return;
    }
    stacks.splice(stacks.indexOf(st), 1);
    st.balls.forEach((b, j) => balls.push({ x: st.centers[j]?.x ?? c.fx, y: st.centers[j]?.y ?? c.fy, vx: c.vx * 0.5, vy: 0, m: b.m, r: b.r, rot: 0, spin: 0, ignore: [] }));
  }
  function moveHeld(p) {
    const h = held;
    h.cx = p.x; h.cy = p.y;
    const dx = p.x - h.lx, dy = p.y - h.ly, dist = Math.hypot(dx, dy);
    if (dist < 2) return;
    const dir = { x: dx / dist, y: dy / dist };
    if (h.lastDir) {
      const ang = Math.atan2(h.lastDir.x * dir.y - h.lastDir.y * dir.x, h.lastDir.x * dir.x + h.lastDir.y * dir.y);
      h.spin = h.spin * 0.8 + ang * 0.2;
      if (ang * h.spin > 0 && Math.abs(h.spin) > 0.05) h.comp = Math.min(1, h.comp + Math.abs(ang) / (1.6 * TAU));
    }
    h.lastDir = dir; h.lx = p.x; h.ly = p.y;
  }
  function updateHeld(dt) {
    const h = held;
    if (!h) return;
    const r = ballR(h.m);
    const rate = 28 / (1 + (r / 40) ** 2); // heavier balls trail the cursor more
    const px = h.x, py = h.y, k = 1 - Math.exp(-rate * dt);
    h.x += (h.cx - h.x) * k; h.y += (h.cy - h.y) * k;
    const dx = h.x - px, dy = h.y - py, dist = Math.hypot(dx, dy);
    h.vx = h.vx * 0.5 + (dx / Math.max(dt, 1e-4)) * 0.5;
    h.vy = h.vy * 0.5 + (dy / Math.max(dt, 1e-4)) * 0.5;
    if (dist > 0.01) {
      h.rot += (dist / r) * (h.spin < 0 ? -1 : 1);
      if (h.comp > 0.25) {
        const s = hitSurface(h.x, h.y + r * 0.7, r * 0.4, 1);
        if (s) {
          const got = take(s, h.x, r, dist * 0.05);
          h.m += got;
          if (got > 0 && (h.roll += dist) > 26) { h.roll = 0; Sound.play('roll', panOf(h.x), 0.8, clamp(1.3 - r / 120, 0.6, 1.3)); }
        }
      }
    }
  }
  function release() {
    const h = held;
    held = null;
    const r = ballR(h.m);
    let vx = h.vx, vy = h.vy;
    const sp = Math.hypot(vx, vy);
    if (sp > 5000) { vx *= 5000 / sp; vy *= 5000 / sp; }
    if (h.comp >= 0.5) {
      const ignore = wins.filter(w => h.x > w.x - r && h.x < w.x + w.w + r && h.y > w.y - r && h.y < w.y + w.h + r);
      balls.push({ x: h.x, y: h.y, vx, vy, m: h.m, r, rot: h.rot, spin: vx / r, ignore });
      if (sp > 250) Sound.play('throw', panOf(h.x), 0.3 + Math.min(1, sp / 2500) * 0.8, 0.85 + Math.min(1, sp / 2500) * 0.35);
    } else {
      Sound.play('splat', panOf(h.x), 0.4, 1.3);
      const n = clamp(h.m / 4, 6, 140) | 0;
      for (let i = 0; i < n; i++) { const a = rand(TAU), d = rand(r); chunk(h.x + Math.cos(a) * d, h.y + Math.sin(a) * d, vx * 0.6 + rand(-40, 40), vy * 0.6 + rand(-56, 24), h.m / n); }
    }
  }

  // ---------------------------------------------------------------- snowballs, stacks, splats
  function splat(x, y, vx, vy, m, r, target) {
    const speed = Math.hypot(vx, vy), power = clamp(speed / 900, 0.3, 1.6);
    Sound.play('splat', panOf(x), clamp(0.35 + Math.sqrt(m / 300) * power * 0.45, 0.2, 1.3), clamp(1.15 - Math.sqrt(m / 300) * 0.15, 0.75, 1.2));
    let rest = m, a0 = 0, a1 = TAU, bx = vx * 0.2, by = vy * 0.2;
    if (target && target.surf) {
      const dep = m * (0.6 - 0.3 * Math.min(power, 1));
      deposit(target.surf, x, dep, r * 1.3);
      rest -= dep; a0 = -Math.PI; a1 = 0; bx = vx * 0.25; by = -Math.abs(vy) * 0.08;
    } else if (target && target.win) {
      const w = target.win, n = Math.hypot(vx, vy) || 1;
      const cx = clamp(x + (vx / n) * r * 0.8, w.x + r * 0.4, w.x + w.w - r * 0.4), cy = clamp(y + (vy / n) * r * 0.8, w.y + r * 0.4, w.y + w.h - r * 0.4);
      const blobs = [{ x: 0, y: 0, s: 0.85 }];
      for (let i = 0; i < 9; i++) { const a = rand(TAU), d = 0.55 + rand(0.75) * power; blobs.push({ x: Math.cos(a) * d, y: Math.sin(a) * d, s: rand(0.18, 0.5) }); }
      for (let i = 1; i <= 4; i++) { const d = 0.6 + i * 0.35 * power, side = rand(-0.25, 0.25); blobs.push({ x: (vx / n) * d - (vy / n) * side, y: (vy / n) * d + (vx / n) * side, s: 0.32 - i * 0.05 }); }
      marks.push({ win: w, rx: cx - w.x, ry: cy - w.y, r: r * 1.05, life: 12, blobs });
      rest -= m * 0.3; bx = -vx * 0.08; by = -vy * 0.08;
    } else if (target && target.edge) {
      const back = vx > 0 ? Math.PI / 2 : -Math.PI / 2; a0 = back; a1 = back + Math.PI; bx = -vx * 0.15; by = vy * 0.1;
    }
    const count = clamp(rest / 3, 12, 220) | 0, sizeK = 0.6 + Math.min(r / 30, 2.5);
    for (let i = 0; i < count; i++) {
      const a = a0 + rand() * (a1 - a0), sp = rand(60, 440) * sizeK * Math.max(power, 0.5), d = rand(r * 0.6), b = rand(TAU);
      chunk(x + Math.cos(b) * d, y + Math.sin(b) * d, bx + Math.cos(a) * sp, by + Math.sin(a) * sp, rest / count);
    }
  }
  function stackSurf(st) { return st.surf ? st.surf.surf : floor; }
  function updateStacks(dt) {
    for (let i = stacks.length - 1; i >= 0; i--) {
      const st = stacks[i];
      if (st.carried) continue; // positioned by updateCarry
      const s = stackSurf(st), x = s.left + st.x, c = colOf(s, x);
      const shaken = st.surf && st.surf.jerk > 30000;
      if (c < 0 || shaken) {
        stacks.splice(i, 1);
        st.balls.forEach((b, j) => balls.push({ x: st.centers[j]?.x ?? x, y: st.centers[j]?.y ?? s.top, vx: (st.surf?.vel.x || 0) * 0.5, vy: 0, m: b.m, r: b.r, rot: 0, spin: 0, ignore: [] }));
        continue;
      }
      st.visible = !!s.vis[c];
      st.centers = [];
      let y = surfY(s, c) + st.balls[0].r * 0.15;
      const ease = Math.exp(-10 * dt);
      st.balls.forEach((b, k) => {
        y -= b.r * (k === 0 ? 1 : 0.8);
        b.off.x *= ease; b.off.y *= ease;
        st.centers.push({ x: x + b.off.x, y: y + b.off.y });
        y -= b.r * 0.8;
      });
    }
  }
  function stackHit(b) {
    for (let i = 0; i < stacks.length; i++) {
      const st = stacks[i];
      if (!st.visible || st.carried) continue;
      for (let j = 0; j < st.centers.length; j++) {
        const c = st.centers[j], rj = st.balls[j].r, dx = b.x - c.x, dy = b.y - c.y, d = Math.hypot(dx, dy), contact = (b.r + rj) * 0.92;
        if (d >= contact) continue;
        const speed = Math.hypot(b.vx, b.vy);
        if (speed > 900) return { i, kind: 'shatter' };
        const top = j === st.balls.length - 1;
        const onTop = top && dy < -rj * 0.5 && Math.abs(dx) < rj * 0.45 && b.vy >= -50;
        if (onTop && speed < 1050 && st.balls.length < 3) return { i, kind: 'stack' };
        if (speed > 600 && !onTop) return { i, kind: 'splat' };
        const nx = dx / (d || 1), ny = dy / (d || 1);
        return { i, kind: 'bounce', nx, ny, cx: c.x + nx * contact, cy: c.y + ny * contact };
      }
    }
    return null;
  }
  function updateBalls(dt) {
    for (let bi = balls.length - 1; bi >= 0; bi--) {
      const b = balls[bi];
      const steps = clamp(Math.ceil((Math.hypot(b.vx, b.vy) * dt) / Math.max(b.r * 0.4, 1)), 1, 60), h = dt / steps;
      let gone = false;
      for (let k = 0; k < steps && !gone; k++) {
        const py = b.y;
        b.vy += 1700 * h; b.vx *= 1 - 0.12 * h; b.vy *= 1 - 0.12 * h;
        b.x += b.vx * h; b.y += b.vy * h; b.rot += b.spin * h;
        b.ignore = b.ignore.filter(w => b.x > w.x - b.r && b.x < w.x + w.w + b.r && b.y > w.y - b.r && b.y < w.y + w.h + b.r);
        const hit = stackHit(b);
        if (hit) {
          const st = stacks[hit.i];
          if (hit.kind === 'stack') {
            const top = st.centers[st.centers.length - 1], rt = st.balls[st.balls.length - 1].r;
            st.balls.push({ r: b.r, m: b.m, off: { x: b.x - top.x, y: b.y - (top.y - rt * 0.8 - b.r * 0.8) } });
            st.centers.push({ x: b.x, y: b.y }); // until the next stack update places it
            Sound.play('plop', panOf(b.x), 0.9, clamp(1.4 - b.r / 80, 0.6, 1.4));
            if (st.balls.length === 3) Sound.play('chime', panOf(b.x), 0.9);
            gone = true;
          } else if (hit.kind === 'shatter') {
            stacks.splice(hit.i, 1);
            st.balls.forEach((sb, j) => splat(st.centers[j].x, st.centers[j].y, b.vx * 0.4, b.vy * 0.4, sb.m, sb.r, null));
            splat(b.x, b.y, b.vx, b.vy, b.m, b.r, null);
            gone = true;
          } else if (hit.kind === 'splat') {
            splat(b.x, b.y, b.vx, b.vy, b.m, b.r, null);
            gone = true;
          } else {
            b.x = hit.cx; b.y = hit.cy;
            const vn = b.vx * hit.nx + b.vy * hit.ny;
            if (vn < 0) { b.vx -= hit.nx * vn * 1.35; b.vy -= hit.ny * vn * 1.35; }
            b.vx += (Math.abs(hit.nx) < 0.05 ? (Math.random() < 0.5 ? 1 : -1) : Math.sign(hit.nx)) * 40;
          }
          continue;
        }
        const s = findLanding(b.x, py + b.r * 0.8, b.y + b.r * 0.8, true);
        if (s) {
          const w = winOf(s), sv = w ? Math.hypot(b.vx - w.vel.x, b.vy - w.vel.y) : Math.hypot(b.vx, b.vy);
          if (sv < 1050) {
            stacks.push({ surf: w || null, x: b.x - s.left, balls: [{ r: b.r, m: b.m, off: { x: 0, y: 0 } }], centers: [{ x: b.x, y: b.y }], visible: true, seed: (Math.random() * 4) | 0, look: (Math.random() * 1e9) | 0 });
            Sound.play('plop', panOf(b.x), 0.9, clamp(1.4 - b.r / 80, 0.6, 1.4));
          } else splat(b.x, b.y + b.r * 0.8, b.vx, b.vy, b.m, b.r, { surf: s });
          gone = true;
          continue;
        }
        const topWin = zSorted().find(w => b.x >= w.x && b.x < w.x + w.w && b.y >= w.y && b.y < w.y + w.h);
        if (topWin && !b.ignore.includes(topWin)) { splat(b.x, b.y, b.vx, b.vy, b.m, b.r, { win: topWin }); gone = true; continue; }
        if (b.x < b.r * 0.5 || b.x > W - b.r * 0.5) { splat(b.x, b.y, b.vx, b.vy, b.m, b.r, { edge: true }); gone = true; continue; }
        if (b.y > H + 200) gone = true;
      }
      if (gone) balls.splice(bi, 1);
    }
  }

  // ---------------------------------------------------------------- particles
  function spawnFlakes(dt, initial) {
    const target = Math.round((W / 1920) * 420 * intensity);
    let cur = 0;
    for (const p of parts) if (p.kind === 0) cur++;
    const n = initial ? target - cur : Math.min(target - cur, Math.ceil(target * dt * 0.5) + 1);
    for (let i = 0; i < n && parts.length < MAXP; i++) {
      const d = rand(), r = 0.8 + d * d * 2.4;
      parts.push({ x: rand(W), y: initial ? rand(H) : -rand(4, 40), vx: 0, vy: 30, r, m: 1.6 * Math.PI * r * r, kind: 0, d, p: rand(TAU), a: 0.45 + 0.55 * d });
    }
  }
  let intensity = 1;
  function updateParticles(dt) {
    const wind = Math.sin(time * 0.07) * 22 + Math.sin(time * 0.23 + 1.3) * 12;
    for (let i = parts.length - 1; i >= 0; i--) {
      const p = parts[i], oy = p.y;
      if (p.kind === 0) {
        p.p += dt * (0.8 + p.d * 0.9);
        const tx = wind * (0.4 + 0.8 * p.d) + Math.cos(p.p) * (10 + 16 * p.d);
        p.vx += (tx - p.vx) * Math.min(1, dt * 3);
        p.vy += (26 + 55 * p.d - p.vy) * Math.min(1, dt * 2);
      } else {
        p.vy = Math.min(p.vy + 1500 * dt, 1400);
        p.vx *= 1 - 0.5 * dt; p.vy *= 1 - 0.5 * dt;
      }
      p.x += p.vx * dt; p.y += p.vy * dt;
      if (p.kind === 2) {
        p.a -= dt * 0.9;
        if (p.a <= 0 || p.y > H) parts.splice(i, 1);
        continue;
      }
      const s = findLanding(p.x, oy, p.y, false);
      if (s) { deposit(s, p.x, p.m, Math.max(p.r * 1.6, Math.sqrt(p.m) * 0.8)); parts.splice(i, 1); continue; }
      if (p.y > H + 40 || p.x < -200 || p.x > W + 200) parts.splice(i, 1);
    }
  }

  // ---------------------------------------------------------------- fireworks and the box
  let box = null;
  function salute(secs, rate) {
    if (fw.left <= 0) { fw.rate = rate; fw.acc = 1; } else fw.rate = Math.max(fw.rate, rate);
    fw.left = Math.max(fw.left, secs);
  }
  function launch() {
    const from = box ? { x: box.x + rand(-0.3, 0.3) * box.w, y: box.y - box.h } : { x: rand(W), y: FLOOR };
    const apex = (from.y - 10) * rand(0.45, 0.8), gr = 420;
    fw.rockets.push({ x: from.x, y: from.y, vx: rand(-0.17, 0.17) * Math.sqrt(2 * gr * apex), vy: -Math.sqrt(2 * gr * apex), c: (Math.random() * 8) | 0, k: (Math.random() * 5) | 0, pw: rand(0.6, 1.0) });
    Sound.play('launch', panOf(from.x), 0.6, rand(0.9, 1.15));
  }
  function burst(r) {
    Sound.play('boom', panOf(r.x), 0.4 + r.pw * 0.5, rand(0.85, 1.15));
    const sp = 200 * r.pw, c2 = (Math.random() * 8) | 0;
    const add = (vx, vy, c, life, size, tw, drag, grav) => {
      if (fw.sparks.length > 4000) return;
      const l = life * rand(0.8, 1.2);
      fw.sparks.push({ x: r.x, y: r.y, vx, vy, c, life: l, max: l, size, tw, drag, grav });
    };
    if (r.k === 0) for (let i = 0; i < 110; i++) { const a = rand(TAU), v = sp * Math.pow(rand(), 0.3); add(Math.cos(a) * v, Math.sin(a) * v, i % 3 ? r.c : c2, 1.7, 2.6, false, 1.6, 90); }
    else if (r.k === 1) { const tilt = rand(0.4, 1); for (let i = 0; i < 70; i++) { const a = (i / 70) * TAU; add(Math.cos(a) * sp * 1.1, Math.sin(a) * tilt * sp * 1.1, r.c, 1.5, 2.8, false, 1.5, 80); } }
    else if (r.k === 2) for (let i = 0; i < 100; i++) { const a = rand(TAU), v = sp * 0.8 * Math.pow(rand(), 0.35); add(Math.cos(a) * v, Math.sin(a) * v, 1, 3.2, 2.2, false, 2.1, 70); }
    else if (r.k === 3) for (let i = 0; i < 120; i++) { const a = rand(TAU), v = sp * 0.9 * Math.pow(rand(), 0.4); add(Math.cos(a) * v, Math.sin(a) * v, 7, 1.4, 2.4, true, 1.8, 100); }
    else for (let i = 0; i < 80; i++) {
      const t = (i / 80) * TAU, x = 16 * Math.sin(t) ** 3, y = -(13 * Math.cos(t) - 5 * Math.cos(2 * t) - 2 * Math.cos(3 * t) - Math.cos(4 * t));
      add((x / 16) * sp, (y / 16) * sp, i % 2 ? 5 : 0, 1.6, 2.8, false, 1.4, 60);
    }
  }
  function updateFireworks(dt) {
    if (fw.left > 0) { fw.left -= dt; fw.acc += fw.rate * dt; while (fw.acc >= 1) { fw.acc -= 1; launch(); } }
    for (let i = fw.rockets.length - 1; i >= 0; i--) {
      const r = fw.rockets[i];
      r.vy += 420 * dt; r.x += r.vx * dt; r.y += r.vy * dt;
      if (fw.sparks.length < 4000) fw.sparks.push({ x: r.x, y: r.y, vx: rand(-15, 15), vy: rand(40), c: 1, life: 0.4, max: 0.4, size: 2, tw: false, drag: 2, grav: 30 });
      if (r.vy > -40) { burst(r); fw.rockets.splice(i, 1); }
    }
    for (let i = fw.sparks.length - 1; i >= 0; i--) {
      const s = fw.sparks[i];
      s.vy += s.grav * dt; const k = Math.exp(-s.drag * dt); s.vx *= k; s.vy *= k;
      s.x += s.vx * dt; s.y += s.vy * dt; s.life -= dt;
      if (s.life <= 0) fw.sparks.splice(i, 1);
    }
  }
  function updateBox() {
    const x = W * 0.86, c = colOf(floor, x);
    box = { x, y: c >= 0 ? surfY(floor, c) + 3 : FLOOR, w: 58, h: 48 };
  }
  function countdown() {
    const now = new Date(), ny = new Date(now.getFullYear() + 1, 0, 1);
    let s = Math.floor((ny - now) / 1000);
    const d = Math.floor(s / 86400); s %= 86400;
    const hh = String(Math.floor(s / 3600)).padStart(2, '0'), mm = String(Math.floor((s % 3600) / 60)).padStart(2, '0'), ss = String(s % 60).padStart(2, '0');
    return d > 0 ? `До Нового года ${d} д ${hh}:${mm}:${ss}` : `До Нового года ${hh}:${mm}:${ss}`;
  }

  // ---------------------------------------------------------------- snowplow
  function startPlow() {
    if (plow) return;
    const dir = Math.random() < 0.5 ? 1 : -1;
    plow = { x: dir > 0 ? -140 : W + 140, dir, load: 0, wheel: 0, t: 0 };
  }
  function updatePlow(dt) {
    if ((time > nextPlow) && !plow) { startPlow(); nextPlow = time + 60; }
    if (!plow) return;
    const p = plow, oldF = p.x + p.dir * 86;
    p.x += p.dir * 150 * dt; p.t += dt; p.wheel += (150 * dt) / 11;
    const newF = p.x + p.dir * 86;
    const prot = stacks.filter(st => !st.surf && st.centers.length).map(st => [st.centers[0].x - st.balls[0].r, st.centers[0].x + st.balls[0].r]);
    const a = Math.max(0, Math.floor(Math.min(oldF, newF) / COL)), b = Math.min(floor.n, Math.ceil(Math.max(oldF, newF) / COL));
    for (let c = a; c < b; c++) {
      const x = c * COL + 1;
      if (prot.some(([l, r]) => x >= l && x <= r)) continue;
      if (floor.h[c] > 0.8) { p.load += (floor.h[c] - 0.8) * COL; floor.h[c] = 0.8; }
    }
    const emit = Math.min(p.load, p.load * dt * 3);
    p.load -= emit;
    const n = Math.min(4, Math.ceil(emit / 25));
    for (let i = 0; i < n && parts.length < MAXP; i++) parts.push({ x: p.x - p.dir * 40 + rand(-3, 3), y: FLOOR - 70, vx: -p.dir * rand(80, 300), vy: -rand(300, 550), r: rand(1.5, 3.5), m: 0, kind: 2, a: rand(0.8, 1.1) });
    if (p.x < -200 || p.x > W + 200) plow = null;
  }

  // ---------------------------------------------------------------- drawing
  // Rectangles of the windows in front of window `w` (null: the taskbar, in front of everything).
  const frontOf = w => (w ? wins.filter(o => o.z > w.z) : []);
  // `area` minus the occluders, as a few disjoint rectangles
  function subtract(area, occ) {
    let parts = [area];
    for (const o of occ) {
      const next = [];
      for (const p of parts) {
        if (o.x + o.w <= p.x || o.x >= p.x + p.w || o.y + o.h <= p.y || o.y >= p.y + p.h) { next.push(p); continue; }
        if (o.y > p.y) next.push({ x: p.x, y: p.y, w: p.w, h: o.y - p.y });
        if (o.y + o.h < p.y + p.h) next.push({ x: p.x, y: o.y + o.h, w: p.w, h: p.y + p.h - (o.y + o.h) });
        const t = Math.max(p.y, o.y), b = Math.min(p.y + p.h, o.y + o.h);
        if (o.x > p.x) next.push({ x: p.x, y: t, w: o.x - p.x, h: b - t });
        if (o.x + o.w < p.x + p.w) next.push({ x: o.x + o.w, y: t, w: p.x + p.w - (o.x + o.w), h: b - t });
      }
      parts = next;
    }
    return parts;
  }
  // paints only inside `area` and never over the windows in front of `w`
  function occluded(w, area, draw) {
    const front = frontOf(w);
    if (!front.length) { draw(); return; }
    for (const p of subtract(area, front)) {
      g.save(); g.beginPath(); g.rect(p.x, p.y, p.w, p.h); g.clip();
      draw();
      g.restore();
    }
  }
  function drawDrift(s, w) {
    // the whole drift is drawn, even where a window in front covers its base line: the windows
    // in front clip it below, so it is hidden exactly up to their edge instead of vanishing
    if (w && w.y <= 5) return; // pressed against the top: no room for snow
    const fill = new Path2D(), line = new Path2D();
    let any = false;
    for (let i = 0; i < s.n;) {
      if (s.h[i] < 0.3) { i++; continue; }
      const a = i;
      while (i < s.n && s.h[i] >= 0.3) i++;
      const x0 = s.left + a * COL, x1 = s.left + i * COL;
      fill.moveTo(x0, s.top);
      line.moveTo(x0, s.top);
      for (let k = a; k < i; k++) { const x = s.left + k * COL + COL / 2, y = s.top - s.h[k]; fill.lineTo(x, y); line.lineTo(x, y); }
      fill.lineTo(x1, s.top);
      line.lineTo(x1, s.top);
      fill.closePath();
      any = true;
    }
    if (!any) return;
    const area = { x: s.left - 2, y: s.top - s.maxH * 1.7 - 4, w: s.n * COL + 4, h: s.maxH * 1.7 + 6 };
    occluded(w, area, () => {
      g.globalAlpha = 1;
      g.fillStyle = '#f8fbff';
      g.fill(fill);
      g.strokeStyle = 'rgba(140,163,199,0.75)';
      g.lineWidth = 1;
      g.stroke(line);
    });
  }
  function sprite(img, x, y, r, a = 1) { g.globalAlpha = a; g.drawImage(img, x - r, y - r, r * 2, r * 2); }
  function speckles(x, y, r, rot, a) {
    if (r < 6 || a <= 0.01) return;
    g.fillStyle = `rgba(140,158,188,${a})`;
    for (const [an, d, sz] of [[0.3, 0.55, 0.1], [1.4, 0.3, 0.07], [2.2, 0.62, 0.09], [3.9, 0.7, 0.08], [5.5, 0.58, 0.1]]) {
      g.beginPath(); g.arc(x + Math.cos(an + rot) * d * r * 0.85, y + Math.sin(an + rot) * d * r * 0.85, sz * r, 0, TAU); g.fill();
    }
  }
  // sprites, the same ones the app uses
  const SPR = {};
  for (const n of ['plow', 'box', 'arm', 'nose', 'hat', 'hat_santa', 'hat_beanie', 'hat_bucket', 'scarf', 'scarf_blue', 'scarf_green', 'face', 'face_wink', 'face_grin']) {
    const im = new Image();
    im.src = `assets/sprites/${n}.png`;
    SPR[n] = im;
  }
  const ready = im => im.complete && im.naturalWidth > 0;
  // draws a sprite into a rectangle, optionally mirrored
  function img(im, l, t, w, h, flip) {
    if (!ready(im)) return;
    g.globalAlpha = 1;
    if (flip) { g.save(); g.translate(l + w, t); g.scale(-1, 1); g.drawImage(im, 0, 0, w, h); g.restore(); }
    else g.drawImage(im, l, t, w, h);
  }
  const aspect = im => (ready(im) ? im.naturalHeight / im.naturalWidth : 1);

  function drawSnowmanArms(st) {
    const B = st.centers[1], rb = st.balls[1].r, a = rb * 1.45, ah = a * aspect(SPR.arm);
    img(SPR.arm, B.x + rb * 0.62, B.y + rb * 0.2 - ah, a, ah, false);
    img(SPR.arm, B.x - rb * 0.62 - a, B.y + rb * 0.2 - ah, a, ah, true);
  }
  function drawSnowman(st) {
    const B = st.centers[1], rb = st.balls[1].r, Hc = st.centers[2], rh = st.balls[2].r;
    g.globalAlpha = 1;
    for (const k of [-0.42, 0.02, 0.46]) {
      g.fillStyle = '#1f1f26'; g.beginPath(); g.arc(B.x, B.y + rb * k, Math.max(1.5, rb * 0.085), 0, TAU); g.fill();
      g.fillStyle = '#73737f'; g.beginPath(); g.arc(B.x - rb * 0.025, B.y + rb * k - rb * 0.025, Math.max(0.6, rb * 0.03), 0, TAU); g.fill();
    }
    // every snowman gets a random combination of hat, scarf and face
    const look = st.look || 0;
    const scarf = SPR[['scarf', 'scarf_blue', 'scarf_green'][Math.floor(look / 4) % 3]];
    const face = SPR[['face', 'face_wink', 'face_grin'][Math.floor(look / 12) % 3]];
    const sw = rh * 2.3;
    img(scarf, Hc.x - sw / 2, Hc.y + rh * 0.62, sw, sw * aspect(scarf), false);
    const fw = rh * 1.1;
    img(face, Hc.x - fw / 2, Hc.y - rh * 0.45, fw, fw * aspect(face), false);
    const nw = rh * 0.8, nh = nw * aspect(SPR.nose);
    img(SPR.nose, Hc.x - rh * 0.05, Hc.y - nh * 0.35, nw, nh, false);
    // [sprite, width in head radii, how far down onto the head it sits]
    const [hn, wk, sink] = [['hat', 2.05, 0.55], ['hat_santa', 2.0, 0.35], ['hat_beanie', 1.9, 0.38], ['hat_bucket', 1.95, 0.5]][look % 4];
    const hw = rh * wk, hh = hw * aspect(SPR[hn]);
    img(SPR[hn], Hc.x - hw / 2, Hc.y - rh * sink - hh, hw, hh, false);
  }
  function drawBox() {
    const { x, y, w, h } = box;
    const iw = w * 1.08, ih = iw * aspect(SPR.box);
    img(SPR.box, x - iw / 2, y - ih, iw, ih, false);
    if (fw.left > 0 || fw.rockets.length) {
      g.globalCompositeOperation = 'lighter';
      sprite(glowSprites[1], x + w * 0.28, y - h - 2, 6 + 3 * Math.sin(time * 25), 1);
      g.globalCompositeOperation = 'source-over';
    }
    g.globalAlpha = 1;
    g.font = '600 12px Manrope, sans-serif'; g.textAlign = 'center';
    g.fillStyle = 'rgba(5,10,25,0.7)'; g.fillText(countdown(), x + 1, y - ih - 9);
    g.fillStyle = '#fff4d6'; g.fillText(countdown(), x, y - ih - 10);
  }
  function drawPlow() {
    const p = plow, pw = 2 * 88, ph = pw * aspect(SPR.plow);
    const bounce = Math.abs(Math.sin(p.t * 13)) * 1.2;
    const t = FLOOR - ph - bounce + 2;
    const load = Math.min(16, Math.sqrt(p.load) * 0.7);
    if (load > 1) sprite(ballSprite, p.x + p.dir * (pw / 2 + load * 0.4), FLOOR - load * 0.6, load);
    img(SPR.plow, p.x - pw / 2, t, pw, ph, p.dir < 0);
    if (Math.sin(p.t * 7) > 0) {
      g.globalCompositeOperation = 'lighter';
      sprite(glowSprites[1], p.x + p.dir * 0.03 * pw, t + ph * 0.06, 20, 0.9);
      g.globalCompositeOperation = 'source-over';
    }
    g.globalAlpha = 1;
  }

  function draw() {
    g.clearRect(0, 0, W, H);
    // splats stuck on windows, clipped to the window
    for (const m of marks) {
      const w = m.win, a = Math.min(1, m.life / 3) * 0.95;
      occluded(w, { x: w.x, y: w.y, w: w.w, h: w.h }, () => {
        for (const b of m.blobs) sprite(ballSprite, w.x + m.rx + b.x * m.r, w.y + m.ry + b.y * m.r, b.s * m.r, a);
      });
    }
    g.globalAlpha = 1;
    for (const w of [...wins].sort((a, b) => a.z - b.z)) drawDrift(w.surf, w);
    drawDrift(floor, null);
    for (const st of stacks) {
      if (!st.centers.length) continue; // a covered base is fine: windows in front clip it
      let l = Infinity, t = Infinity, r = -Infinity, b = -Infinity;
      st.centers.forEach((c, j) => { const R = st.balls[j].r; l = Math.min(l, c.x - R * 2.2); r = Math.max(r, c.x + R * 2.2); t = Math.min(t, c.y - R * 2); b = Math.max(b, c.y + R * 1.9); });
      const body = () => {
        if (st.balls.length >= 3 && st.centers.length >= 3) drawSnowmanArms(st); // behind the body
        st.centers.forEach((c, j) => { sprite(ballSprite, c.x, c.y, st.balls[j].r); speckles(c.x, c.y, st.balls[j].r, 0, 0.25); });
        if (st.balls.length >= 3 && st.centers.length >= 3) drawSnowman(st);
      };
      // a carried snowman is in the hand, in front of every window
      if (st.carried) body(); else occluded(st.surf || null, { x: l, y: t, w: r - l, h: b - t }, body);
    }
    if (box) drawBox();
    if (plow) drawPlow();
    for (const p of parts) {
      if (p.kind === 1) sprite(ballSprite, p.x, p.y, p.r, 1);
      else sprite(flakeSprite, p.x, p.y, p.r * (p.kind === 2 ? 2 : 1.9), p.kind === 2 ? Math.min(1, p.a) : p.a);
    }
    for (const b of balls) { sprite(ballSprite, b.x, b.y, b.r); speckles(b.x, b.y, b.r, b.rot, 0.35); }
    if (held) {
      const h = held, r = ballR(h.m), spread = Math.pow(1 - h.comp, 1.5);
      if (spread > 0.01 && h.blobs.length) {
        const c = Math.cos(h.rot * 0.15), sn = Math.sin(h.rot * 0.15);
        for (const b of h.blobs) sprite(ballSprite, h.x + (b.x * c - b.y * sn) * r * 0.9 * spread, h.y + (b.x * sn + b.y * c) * r * 0.9 * spread, r * (b.s + (1 - b.s) * h.comp));
      } else sprite(ballSprite, h.x, h.y, r);
      speckles(h.x, h.y, r, h.rot, clamp((h.comp - 0.4) * 1.5, 0, 0.35));
    }
    g.globalCompositeOperation = 'lighter';
    fw.sparks.forEach((s, i) => {
      let a = Math.pow(clamp(s.life / s.max, 0, 1), 0.7);
      if (s.tw && Math.sin(time * 30 + i * 1.7) < 0) a *= 0.15;
      sprite(glowSprites[s.c], s.x, s.y, s.size * 2.2, a);
    });
    for (const r of fw.rockets) { sprite(glowSprites[7], r.x, r.y, 4.5, 1); sprite(glowSprites[r.c], r.x, r.y, 9, 0.5); }
    g.globalCompositeOperation = 'source-over';
    g.globalAlpha = 1;
  }

  // ---------------------------------------------------------------- Tems and Suzu on windows
  // Each sitter names its window (data-win), the end of its top edge (data-side, and
  // data-side-narrow on a phone, where the windows stand one under another) and how far down
  // the picture its "ledge" is, where the hands and seat rest (data-seat).
  const sitters = [...document.querySelectorAll('.sitter')]
    .map(el => ({ el, win: wins.find(w => w.el.dataset.title === el.dataset.win), seat: parseFloat(el.dataset.seat) || 0.54 }))
    .filter(s => s.win);
  function placeSitter() {
    for (const s of sitters) {
      const el = s.el, w = s.win;
      if (!el.naturalWidth) continue;
      const sw = el.offsetWidth, sh = sw * (el.naturalHeight / el.naturalWidth);
      // sit near one end of the window's top edge
      const side = (W < 600 && el.dataset.sideNarrow) || el.dataset.side;
      const cx = side === 'right' ? w.x + w.w - sw * 0.5 - 18 : w.x + sw * 0.5 + 18;
      // fixed to the window edge itself: only the window moves the sitter
      const seat = w.y - 4;
      const left = Math.round(cx - sw / 2), top = Math.round(seat - sh * s.seat);
      el.style.transform = `translate(${left}px, ${top}px)`;
      // windows in front of the seat window cover the sitter exactly like they cover the snow
      const area = { x: left - 20, y: top - 20, w: sw + 40, h: sh + 40 };
      const parts = subtract(area, frontOf(w));
      const key = parts.map(p => `${p.x | 0},${p.y | 0},${p.w | 0},${p.h | 0}`).join(';');
      if (key !== el._clipKey) {
        el._clipKey = key;
        if (parts.length === 1 && parts[0] === area) el.style.clipPath = '';
        else if (!parts.length) el.style.clipPath = 'inset(50%)';
        else el.style.clipPath = `path('${parts.map(p => `M${p.x - left} ${p.y - top}h${p.w}v${p.h}h${-p.w}Z`).join(' ')}')`;
      }
    }
  }

  // ---------------------------------------------------------------- loop and controls
  function fill() {
    for (const w of wins) for (let c = 0; c < w.surf.n; c++) { const e = Math.min(c, w.surf.n - c) * COL; w.surf.h[c] = Math.min(16, 4 + e * 0.6); }
    for (let c = 0; c < floor.n; c++) floor.h[c] = 24 + Math.sin(c * 0.05) * 3;
  }
  desk.querySelector('.desk-tools').addEventListener('click', e => {
    const b = e.target.closest('button');
    if (!b) return;
    const act = b.dataset.act;
    if (act === 'fill') fill();
    if (act === 'salute') { Sound.unlock(); salute(5, 2.5); }
    if (act === 'plow') startPlow();
    if (act === 'clear') { for (const s of surfaces()) s.h.fill(0); marks.length = 0; stacks.length = 0; balls.length = 0; }
    if (act === 'sound') b.textContent = Sound.toggle() ? '🔊 Звук' : '🔇 Звук';
  });
  // pointer events on buttons must not start a grab
  desk.querySelector('.desk-tools').addEventListener('pointerdown', e => e.stopPropagation(), true);

  const clockEl = document.getElementById('clock');
  let visible = true;
  new IntersectionObserver(es => (visible = es[0].isIntersecting)).observe(desk);
  fill();
  spawnFlakes(0, true);
  let last = performance.now();
  function frame(now) {
    const dt = Math.min((now - last) / 1000, 0.05);
    last = now;
    if (visible) {
      try {
        step(dt);
      } catch (err) {
        // never let one bad frame stop the snow
        console.error('snowie demo:', err);
      }
    }
    requestAnimationFrame(frame);
  }
  function step(dt) {
    {
      time += dt;
      syncWindows(dt);
      pushSnow();
      visibility();
      updateHeld(dt);
      updateCarry(dt);
      spawnFlakes(dt, false);
      updateParticles(dt);
      for (const s of surfaces()) relax(s);
      updateStacks(dt);
      updateBalls(dt);
      for (let i = marks.length - 1; i >= 0; i--) {
        const m = marks[i];
        m.life -= dt;
        if (m.life < 10) m.ry += 4 * dt;
        if (m.life <= 0 || m.ry > m.win.h) marks.splice(i, 1);
      }
      updatePlow(dt);
      updateBox();
      updateFireworks(dt);
      draw();
      placeSitter();
      if (clockEl) { const d = new Date(); clockEl.textContent = d.toTimeString().slice(0, 5); }
    }
  }
  requestAnimationFrame(frame);

  // read-only peek for automated checks
  window.__snowie = {
    // debug: give every snowman a particular look (hat + 4 * scarf + 12 * face)
    setLook(v) { for (const st of stacks) st.look = v; },
    // debug: put a stack of n balls (3 = a snowman) on window i, or on the floor when i < 0
    addStack(i, n = 3) {
      const w = i >= 0 ? wins[i] : null, s = w ? w.surf : floor;
      const balls = Array.from({ length: n }, (_, k) => { const r = [22, 16, 11][k] || 9; return { r, m: Math.PI * (r / 1.5) ** 2, off: { x: 0, y: 0 } }; });
      stacks.push({ surf: w, x: (s.n * COL) / 2, balls, centers: [], visible: true, seed: 0, look: (Math.random() * 1e9) | 0 });
    },
    get state() {
      return {
        balls: balls.length, stacks: stacks.map(st => st.balls.length), held: !!held, parts: parts.length,
        marks: marks.length, sparks: fw.sparks.length, plow: !!plow,
        ballPos: balls.map(b => [Math.round(b.x), Math.round(b.y), Math.round(b.r), Math.round(b.vx), Math.round(b.vy)]),
        jerk: wins.map(w => Math.round(w.jerk)),
        snow: wins.map(w => Math.round(w.surf.h.reduce((a, b) => a + b, 0))),
        stackPos: stacks.map(st => st.centers.map((c, j) => [Math.round(c.x), Math.round(c.y), Math.round(st.balls[j].r)])),
      };
    },
  };
})();
