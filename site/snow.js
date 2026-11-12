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
    const base = m ? `https://github.com/${m[1]}/${repo}` : 'https://github.com/search?q=snowie+desktop+snow&type=repositories';
    document.querySelectorAll('[data-repo-link]').forEach(a => (a.href = base + (a.dataset.path || '')));
    document.querySelectorAll('[data-release-link]').forEach(a => (a.href = m ? base + '/releases/latest' : base));
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
    return { left, top, n, h: new Float32Array(n), vis: new Uint8Array(n).fill(walls ? 1 : 0), maxH, walls, spillL: 0, spillR: 0 };
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
        w.y = narrow ? 70 + i * 120 : w.fy;
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
  function deposit(s, x, m, spread) {
    const c = colOf(s, x);
    if (c < 0) return;
    const w = Math.max(1, Math.round(spread / COL));
    let sum = 0;
    for (let k = -w; k <= w; k++) if (c + k >= 0 && c + k < s.n) sum += w + 1 - Math.abs(k);
    for (let k = -w; k <= w; k++) {
      const i = c + k;
      if (i >= 0 && i < s.n) s.h[i] = Math.min(s.maxH, s.h[i] + (m / COL) * (w + 1 - Math.abs(k)) / sum);
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
    if (drag) { drag.w.el.classList.remove('drag'); drag = null; }
  };
  desk.addEventListener('pointerup', endPointer);
  desk.addEventListener('pointercancel', endPointer);
  function local(e) { const r = desk.getBoundingClientRect(); return { x: e.clientX - r.left, y: e.clientY - r.top }; }

  function syncWindows(dt) {
    for (const w of wins) {
      const s = w.surf;
      // the drift rides with its window; speed and jerk over the last ~60 ms
      if (s.left !== w.x || s.top !== w.y) { s.left = w.x; s.top = w.y; }
      w.hist.push({ t: time, x: w.x, y: w.y, vx: w.vel.x, vy: w.vel.y });
      while (w.hist.length > 2 && time - w.hist[1].t >= 0.06) w.hist.shift();
      const h0 = w.hist[0], span = time - h0.t;
      if (span > 1e-3) {
        const vx = (w.x - h0.x) / span, vy = (w.y - h0.y) / span;
        w.jerk = Math.hypot(vx - h0.vx, vy - h0.vy) / span;
        w.vel = { x: vx, y: vy };
      }
      if (w.jerk > 40000) shed(s, Math.min(0.7, ((w.jerk - 40000) / 80000) * dt * 8), w.vel.x * 0.35, w.vel.y * 0.35);
      if (s.spillL > 5) { chunk(s.left - 1.5, s.top - s.h[0] - 1, -rand(10, 40), 0, s.spillL); s.spillL = 0; }
      if (s.spillR > 5) { chunk(s.left + s.n * COL + 0.5, s.top - s.h[s.n - 1] - 1, rand(10, 40), 0, s.spillR); s.spillR = 0; }
    }
  }

  // ---------------------------------------------------------------- grabbing and rolling
  function tryGrab(p) {
    if (held) return false;
    if (box && p.x > box.x - box.w / 2 - 4 && p.x < box.x + box.w / 2 + 4 && p.y > box.y - box.h - 4 && p.y < box.y + 4) {
      salute(4, 2.5);
      return true;
    }
    for (let i = 0; i < stacks.length; i++) {
      const st = stacks[i];
      if (st.centers.some((c, j) => Math.hypot(p.x - c.x, p.y - c.y) <= st.balls[j].r)) {
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
      const st = stacks[i], s = stackSurf(st), x = s.left + st.x, c = colOf(s, x);
      const shaken = st.surf && st.surf.jerk > 60000;
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
      if (!st.visible) continue;
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
            stacks.push({ surf: w || null, x: b.x - s.left, balls: [{ r: b.r, m: b.m, off: { x: 0, y: 0 } }], centers: [{ x: b.x, y: b.y }], visible: true, seed: (Math.random() * 4) | 0 });
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
    box = { x, y: c >= 0 ? surfY(floor, c) + 3 : FLOOR, w: 50, h: 32 };
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
    for (let i = 0; i < n && parts.length < MAXP; i++) parts.push({ x: p.x + p.dir * 50 + rand(-3, 3), y: FLOOR - 72, vx: -p.dir * rand(80, 300), vy: -rand(300, 550), r: rand(1.5, 3.5), m: 0, kind: 2, a: rand(0.8, 1.1) });
    if (p.x < -200 || p.x > W + 200) plow = null;
  }

  // ---------------------------------------------------------------- drawing
  function drawDrift(s) {
    // one filled path per visible run, outline only where the drift really ends
    g.beginPath();
    let any = false;
    for (let i = 0; i < s.n;) {
      if (!s.vis[i] || s.h[i] < 0.3) { i++; continue; }
      const a = i;
      while (i < s.n && s.vis[i] && s.h[i] >= 0.3) i++;
      const x0 = s.left + a * COL, x1 = s.left + i * COL;
      g.moveTo(x0, s.top);
      for (let k = a; k < i; k++) g.lineTo(s.left + k * COL + COL / 2, s.top - s.h[k]);
      g.lineTo(x1, s.top);
      g.closePath();
      any = true;
    }
    if (!any) return;
    g.fillStyle = '#f8fbff';
    g.fill();
    g.strokeStyle = 'rgba(140,163,199,0.75)';
    g.lineWidth = 1;
    g.stroke();
  }
  function sprite(img, x, y, r, a = 1) { g.globalAlpha = a; g.drawImage(img, x - r, y - r, r * 2, r * 2); }
  function speckles(x, y, r, rot, a) {
    if (r < 6 || a <= 0.01) return;
    g.fillStyle = `rgba(140,158,188,${a})`;
    for (const [an, d, sz] of [[0.3, 0.55, 0.1], [1.4, 0.3, 0.07], [2.2, 0.62, 0.09], [3.9, 0.7, 0.08], [5.5, 0.58, 0.1]]) {
      g.beginPath(); g.arc(x + Math.cos(an + rot) * d * r * 0.85, y + Math.sin(an + rot) * d * r * 0.85, sz * r, 0, TAU); g.fill();
    }
  }
  function drawSnowman(st) {
    const B = st.centers[1], rb = st.balls[1].r, Hc = st.centers[2], rh = st.balls[2].r;
    const scarf = ['#d91f26', '#2673d9', '#26994d', '#9933bf'][st.seed % 4];
    g.globalAlpha = 1;
    g.strokeStyle = '#6b4524'; g.lineCap = 'round';
    for (const side of [-1, 1]) {
      const ax = B.x + side * rb * 0.8, ay = B.y - rb * 0.15, ex = B.x + side * rb * 1.9, ey = B.y - rb * 0.85;
      g.lineWidth = Math.max(2, rb * 0.08);
      g.beginPath(); g.moveTo(ax, ay); g.lineTo(ex, ey); g.stroke();
      g.lineWidth *= 0.7;
      g.beginPath(); g.moveTo(ex - side * rb * 0.15, ey + rb * 0.1); g.lineTo(ex + side * rb * 0.05, ey - rb * 0.3); g.stroke();
    }
    g.fillStyle = '#1f1f29';
    for (const k of [-0.45, 0, 0.45]) { g.beginPath(); g.arc(B.x, B.y + rb * k, Math.max(1.5, rb * 0.08), 0, TAU); g.fill(); }
    g.fillStyle = scarf;
    g.beginPath(); g.roundRect(Hc.x - rh * 0.95, Hc.y + rh * 0.62, rh * 1.9, rh * 0.38, rh * 0.15); g.fill();
    g.beginPath(); g.roundRect(Hc.x + rh * 0.3, Hc.y + rh * 0.8, rh * 0.38, rh * 0.95, rh * 0.1); g.fill();
    g.fillStyle = '#1f1f29';
    for (const side of [-1, 1]) { g.beginPath(); g.arc(Hc.x + side * rh * 0.33, Hc.y - rh * 0.18, Math.max(1.5, rh * 0.1), 0, TAU); g.fill(); }
    g.fillStyle = '#ff851a';
    g.beginPath(); g.moveTo(Hc.x, Hc.y - rh * 0.06); g.lineTo(Hc.x + rh * 0.8, Hc.y + rh * 0.1); g.lineTo(Hc.x, Hc.y + rh * 0.16); g.fill();
    g.fillStyle = '#1f1f29';
    for (let i = 0; i < 5; i++) { const a = Math.PI * (0.25 + (i / 4) * 0.5); g.beginPath(); g.arc(Hc.x + Math.cos(a) * rh * 0.48, Hc.y + Math.sin(a) * rh * 0.38 + rh * 0.12, Math.max(1, rh * 0.055), 0, TAU); g.fill(); }
    g.fillStyle = '#18181f';
    g.fillRect(Hc.x - rh * 0.95, Hc.y - rh * 0.86, rh * 1.9, rh * 0.18);
    g.fillRect(Hc.x - rh * 0.6, Hc.y - rh * 1.8, rh * 1.2, rh * 1.05);
    g.fillStyle = scarf;
    g.fillRect(Hc.x - rh * 0.6, Hc.y - rh * 1.0, rh * 1.2, rh * 0.18);
  }
  function drawBox() {
    const { x, y, w, h } = box;
    g.globalAlpha = 1;
    ['#cc3333', '#33cc55', '#9944dd', '#33bbcc'].forEach((c, k) => {
      const tx = x - w * 0.33 + k * w * 0.22, th = h * (0.3 + 0.1 * ((k * 7) % 3));
      g.fillStyle = c; g.fillRect(tx - w * 0.08, y - h - th, w * 0.16, th + 2);
      g.fillStyle = '#261a1a'; g.fillRect(tx - w * 0.06, y - h - th, w * 0.12, 3);
    });
    g.fillStyle = '#c71f29'; g.beginPath(); g.roundRect(x - w / 2, y - h, w, h, 4); g.fill();
    g.fillStyle = '#ffd140'; g.fillRect(x - w * 0.07, y - h, w * 0.14, h); g.fillRect(x - w / 2, y - h * 0.55, w, h * 0.13);
    g.font = '600 12px Manrope, sans-serif'; g.textAlign = 'center';
    g.fillStyle = 'rgba(5,10,25,0.7)'; g.fillText(countdown(), x + 1, y - h - 22);
    g.fillStyle = '#fff4d6'; g.fillText(countdown(), x, y - h - 23);
  }
  function drawPlow() {
    const p = plow, d = p.dir, X = lx => p.x + d * lx, Y = ly => FLOOR - ly;
    const rect = (l, t, r, b, c, rad = 0) => { g.fillStyle = c; g.beginPath(); g.roundRect(Math.min(X(l), X(r)), Y(t), Math.abs(X(r) - X(l)), Y(b) - Y(t), rad); g.fill(); };
    g.globalAlpha = 1;
    rect(-72, 26, 62, 14, '#2e2e33', 2);
    rect(-72, 60, 4, 24, '#ff8c14', 4);
    for (let k = 0; k < 4; k++) rect(-64 + k * 18, 54, -54 + k * 18, 30, '#e6730d', 2);
    rect(6, 66, 58, 24, '#ff8c14', 6);
    rect(22, 60, 52, 42, '#8cccf2', 3);
    rect(26, 72, 38, 66, '#2e2e33', 2);
    const blink = Math.sin(p.t * 7) > 0;
    rect(27, 78, 37, 71, blink ? '#ffbf1a' : '#99590d', 3);
    if (blink) { g.globalCompositeOperation = 'lighter'; sprite(glowSprites[1], X(32), Y(75), 22, 0.9); g.globalCompositeOperation = 'source-over'; }
    g.globalAlpha = 1;
    const load = Math.min(16, Math.sqrt(p.load) * 0.7);
    if (load > 1) sprite(ballSprite, X(90) + d * load * 0.4, FLOOR - load * 0.6, load);
    g.globalAlpha = 1;
    g.fillStyle = '#f2cc1a';
    g.beginPath(); g.moveTo(X(74), Y(34)); g.lineTo(X(84), Y(30)); g.lineTo(X(88), Y(2)); g.lineTo(X(76), Y(1)); g.fill();
    for (const lx of [-48, 36]) {
      g.fillStyle = '#141417'; g.beginPath(); g.arc(X(lx), Y(12), 12, 0, TAU); g.fill();
      g.fillStyle = '#9a9a9e'; g.beginPath(); g.arc(X(lx), Y(12), 6, 0, TAU); g.fill();
      g.strokeStyle = '#4d4d52'; g.lineWidth = 1.5;
      for (let k = 0; k < 3; k++) { const a = p.wheel * d + k * 2.094; g.beginPath(); g.moveTo(X(lx), Y(12)); g.lineTo(X(lx) + Math.cos(a) * 5, Y(12) + Math.sin(a) * 5); g.stroke(); }
    }
  }

  function draw() {
    g.clearRect(0, 0, W, H);
    // splats stuck on windows, clipped to the window
    for (const m of marks) {
      const w = m.win, a = Math.min(1, m.life / 3) * 0.95;
      const topmost = zSorted().find(o => w.x + m.rx >= o.x && w.x + m.rx < o.x + o.w && w.y + m.ry >= o.y && w.y + m.ry < o.y + o.h);
      if (topmost !== w) continue;
      g.save(); g.beginPath(); g.rect(w.x, w.y, w.w, w.h); g.clip();
      for (const b of m.blobs) sprite(ballSprite, w.x + m.rx + b.x * m.r, w.y + m.ry + b.y * m.r, b.s * m.r, a);
      g.restore();
    }
    g.globalAlpha = 1;
    for (const s of surfaces()) drawDrift(s);
    for (const st of stacks) {
      if (!st.visible) continue;
      st.centers.forEach((c, j) => { sprite(ballSprite, c.x, c.y, st.balls[j].r); speckles(c.x, c.y, st.balls[j].r, 0, 0.25); });
      if (st.balls.length >= 3 && st.centers.length >= 3) drawSnowman(st);
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
      visibility();
      updateHeld(dt);
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
      if (clockEl) { const d = new Date(); clockEl.textContent = d.toTimeString().slice(0, 5); }
    }
  }
  requestAnimationFrame(frame);

  // read-only peek for automated checks
  window.__snowie = {
    get state() {
      return {
        balls: balls.length, stacks: stacks.map(st => st.balls.length), held: !!held, parts: parts.length,
        marks: marks.length, sparks: fw.sparks.length, plow: !!plow,
        ballPos: balls.map(b => [Math.round(b.x), Math.round(b.y), Math.round(b.r), Math.round(b.vx), Math.round(b.vy)]),
        stackPos: stacks.map(st => st.centers.map((c, j) => [Math.round(c.x), Math.round(c.y), Math.round(st.balls[j].r)])),
      };
    },
  };
})();
