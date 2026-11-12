//! Snow simulation: falling flakes, drifts on window tops and the taskbar, the snow held by
//! the mouse, thrown snowballs, stacked balls and snowmen, splats, the snowplow and fireworks.

use crate::audio::Sfx;
use crate::config::{parse_timer, Config};
use crate::fx::{self, Clock, Fireworks, Plow};
use crate::geom::{lerp, RectI, V2};
use crate::scan::{Monitor, WinInfo};
use std::collections::{HashMap, VecDeque};
use std::f32::consts::{PI, TAU};
use std::ops::Range;

const MAX_PARTICLES: usize = 8000;
/// How much of a flake's area turns into drift height when it lands.
const ACCUM: f32 = 1.6;
/// Flakes on screen per 1920 px of monitor width at intensity 1.
const FLAKES_PER_1920: f32 = 320.0;
/// Landing slower than this keeps the ball whole (it sits down or stacks).
const GENTLE: f32 = 1050.0;

#[derive(Clone, Copy, Debug)]
pub enum Input {
    Move(V2),
    Up(V2),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SurfId {
    Win(isize),
    Floor(usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Flake,
    Chunk,
    /// thrown by the snowplow; fades out instead of landing
    Spray,
}

pub struct Particle {
    pub pos: V2,
    pub vel: V2,
    pub r: f32,
    pub mass: f32,
    pub kind: Kind,
    pub depth: f32,
    pub phase: f32,
    pub alpha: f32,
}

impl Particle {
    fn chunk(pos: V2, vel: V2, mass: f32, s: f32, rng: &mut fastrand::Rng) -> Particle {
        let r = ((mass / PI).sqrt() * (0.7 + rng.f32() * 0.6)).clamp(0.9 * s, 3.6 * s);
        Particle { pos, vel, r, mass, kind: Kind::Chunk, depth: 1.0, phase: 0.0, alpha: 0.95 }
    }
}

pub struct Surface {
    pub left: i32,
    pub top: i32,
    pub heights: Vec<f32>,
    /// columns whose base line is open to the sky: flakes land and the mouse grabs only there.
    /// Windows in front hide the drawn drift by clipping, not through this.
    pub visible: Vec<bool>,
    /// no snow is shown on this surface at all (a window pressed against the top of the
    /// screen, or snow on windows turned off)
    pub hidden: bool,
    pub max_h: f32,
    /// phase of the mounds a full drift settles into (see `cap`), and the display scale
    wave: f32,
    unit: f32,
    /// estimated velocity of the window, px/s (used to predict its position when drawing)
    pub vel: V2,
    /// smoothed acceleration magnitude, px/s²: high when the window is being shaken
    jerk: f32,
    /// accumulated window motion (resizes excluded) and its recent history
    track: V2,
    hist: VecDeque<(f32, V2, V2)>,
    walls: bool,
    ceiling: bool,
    spill_l: f32,
    spill_r: f32,
    rect: RectI,
    seen: bool,
    /// an end of this drift continues into a neighbouring drift at about the same height
    /// (windows side by side): snow pours across instead of spilling off, and no edge is drawn
    pub join_l: bool,
    pub join_r: bool,
    /// stable id for the renderer's geometry cache
    pub key: u64,
    /// Change tracking per `SEG`-column segment, so the renderer rebuilds only what changed.
    /// `seg_urgent` marks changes that must show up at once (scooping, the plow, occlusion);
    /// plain growth from landing flakes may be redrawn a little later.
    pub seg_rev: Vec<u64>,
    pub seg_urgent: Vec<u64>,
    counter: u64,
}

/// Width of a drift segment for change tracking and cached drawing.
pub const SEG: usize = 256;

impl Surface {
    fn new(left: i32, top: i32, width: i32, max_h: f32, walls: bool, rect: RectI) -> Surface {
        let n = width.max(1) as usize;
        Surface {
            left,
            top,
            heights: vec![0.0; n],
            visible: vec![walls; n],
            hidden: false,
            max_h,
            wave: ((left as u32).wrapping_mul(2654435761) ^ (top as u32).wrapping_mul(40503)) as f32 * 1e-6,
            unit: 1.0,
            vel: V2::ZERO,
            jerk: 0.0,
            track: V2::ZERO,
            hist: VecDeque::new(),
            walls,
            ceiling: false,
            spill_l: 0.0,
            spill_r: 0.0,
            rect,
            seen: true,
            join_l: false,
            join_r: false,
            key: 0,
            seg_rev: vec![1; n.div_ceil(SEG)],
            seg_urgent: vec![1; n.div_ceil(SEG)],
            counter: 1,
        }
    }

    /// Marks columns a..b as changed.
    fn mark(&mut self, a: usize, b: usize, urgent: bool) {
        let n = self.heights.len();
        let m = n.div_ceil(SEG);
        if self.seg_rev.len() != m {
            self.seg_rev.resize(m, 0);
            self.seg_urgent.resize(m, 0);
        }
        let b = b.min(n);
        if a >= b {
            return;
        }
        self.counter += 1;
        for k in a / SEG..=(b - 1) / SEG {
            self.seg_rev[k] = self.counter;
            if urgent {
                self.seg_urgent[k] = self.counter;
            }
        }
    }

    fn touch(&mut self, urgent: bool) {
        self.mark(0, self.heights.len(), urgent);
    }

    pub fn right(&self) -> i32 {
        self.left + self.heights.len() as i32
    }

    pub fn col(&self, x: f32) -> Option<usize> {
        let c = (x - self.left as f32).floor();
        if c < 0.0 || c >= self.heights.len() as f32 {
            None
        } else {
            Some(c as usize)
        }
    }

    pub fn surface_y(&self, c: usize) -> f32 {
        self.top as f32 - self.heights[c]
    }

    /// How high the drift can grow at column i. It is not one flat limit: it rolls gently along
    /// the window, so a full drift is a row of soft mounds instead of a shelf cut off at the top.
    fn cap(&self, i: usize) -> f32 {
        let x = i as f32 / self.unit;
        let p = self.wave;
        let n = 0.55 * (x / 61.0 + p).sin() + 0.3 * (x / 27.0 + p * 1.7).sin() + 0.15 * (x / 13.0 + p * 2.9).sin();
        self.max_h * (0.8 + 0.2 * n)
    }

    fn deposit(&mut self, x: f32, mass: f32, spread: f32) {
        let Some(c) = self.col(x) else { return };
        let n = self.heights.len() as i32;
        let w = spread.max(1.0).round() as i32;
        let mut wsum = 0.0;
        for k in -w..=w {
            let i = c as i32 + k;
            if i >= 0 && i < n {
                wsum += (w + 1 - k.abs()) as f32;
            }
        }
        for k in -w..=w {
            let i = c as i32 + k;
            if i >= 0 && i < n {
                // growth slows down well before the cap, so the top rounds off instead of flattening
                let cap = self.cap(i as usize);
                let h = self.heights[i as usize];
                let room = ((cap - h) / (0.35 * cap)).clamp(0.0, 1.0);
                self.heights[i as usize] = (h + mass * room * (w + 1 - k.abs()) as f32 / wsum).min(self.max_h);
            }
        }
        self.mark((c as i32 - w).max(0) as usize, (c as i32 + w + 1) as usize, false);
    }

    /// Scoop snow from the top of the drift in a half-disc profile. Returns the amount taken.
    fn take(&mut self, cx: f32, radius: f32, depth: f32) -> f32 {
        let mut got = 0.0;
        let a = (cx - radius - self.left as f32).floor().max(0.0) as usize;
        let b = ((cx + radius - self.left as f32).ceil().max(0.0) as usize).min(self.heights.len());
        for i in a..b {
            let dx = (self.left as f32 + i as f32 + 0.5 - cx) / radius;
            if dx.abs() >= 1.0 {
                continue;
            }
            let want = depth * (1.0 - dx * dx).sqrt();
            let t = self.heights[i].min(want);
            self.heights[i] -= t;
            got += t;
        }
        if got > 0.0 {
            self.mark(a, b, true);
        }
        got
    }

    /// Angle-of-repose settling plus a little smoothing; overhangs at window edges spill off.
    fn relax(&mut self, slope: f32, lip: f32, flip: bool, unit: f32) {
        let n = self.heights.len();
        if n < 2 {
            return;
        }
        self.unit = unit;
        // snow piled above its cap (pushed, joined across a seam, smoothed in) slowly settles
        // back down, so the mounds keep their shape instead of levelling out over time
        let low = self.max_h * 0.6;
        for i in 0..n {
            if self.heights[i] > low {
                let cap = self.cap(i);
                if self.heights[i] > cap + 0.05 {
                    self.heights[i] -= (self.heights[i] - cap) * 0.01;
                }
            }
        }
        let h = &mut self.heights;
        // biggest change per segment this pass, so settled parts stop asking to be redrawn
        let mut moved = vec![0.0f32; n.div_ceil(SEG)];
        let step = |h: &mut [f32], i: usize, moved: &mut [f32]| {
            let d = h[i] - h[i + 1];
            let t = if d > slope {
                let t = (d - slope) * 0.45;
                h[i] -= t;
                h[i + 1] += t;
                t
            } else if d < -slope {
                let t = (-d - slope) * 0.45;
                h[i + 1] -= t;
                h[i] += t;
                t
            } else {
                return;
            };
            let k = i / SEG;
            moved[k] = moved[k].max(t);
        };
        if flip {
            for i in (0..n - 1).rev() {
                step(h, i, &mut moved);
            }
        } else {
            for i in 0..n - 1 {
                step(h, i, &mut moved);
            }
        }
        let mut prev = h[0];
        for i in 1..n - 1 {
            let cur = h[i];
            let lap = (prev + h[i + 1] - 2.0 * cur) * 0.04;
            if lap.abs() > 0.002 {
                h[i] = cur + lap;
                let k = i / SEG;
                moved[k] = moved[k].max(lap.abs());
            }
            prev = cur;
        }
        if !self.walls {
            if h[0] > lip && !self.join_l {
                let e = (h[0] - lip) * 0.35;
                h[0] -= e;
                self.spill_l += e;
            }
            if h[n - 1] > lip && !self.join_r {
                let e = (h[n - 1] - lip) * 0.35;
                h[n - 1] -= e;
                self.spill_r += e;
            }
        }
        for (k, m) in moved.iter().enumerate() {
            if *m > 0.02 {
                self.mark(k * SEG, (k + 1) * SEG, false);
            }
        }
    }
}

/// Snow in the hand. The ball trails the cursor on a spring that gets softer as it grows,
/// so big balls are heavy to drag around.
pub struct Held {
    pub pos: V2,
    pub cursor: V2,
    vel: V2,
    pub mass: f32,
    pub compaction: f32,
    pub rot: f32,
    spin_ema: f32,
    last_dir: Option<V2>,
    last_p: V2,
    /// distance rolled over snow since the last crunch sound
    roll_acc: f32,
    /// (offset in unit disc, size relative to the ball radius)
    pub blobs: Vec<(V2, f32)>,
}

pub struct Ball {
    pub pos: V2,
    pub vel: V2,
    pub mass: f32,
    pub r: f32,
    pub rot: f32,
    spin: f32,
    ignore: Vec<isize>,
}

pub struct StackBall {
    pub r: f32,
    pub mass: f32,
    pub rot: f32,
    /// where the ball is drawn relative to its resting place; eases to zero so it settles
    /// instead of snapping into position
    pub off: V2,
}

/// A stack (a snowman, say) picked up by its bottom ball and carried around whole.
pub struct Carried {
    pub stack: Stack,
    /// where the bottom ball touches the ground, where the cursor wants it, and its velocity
    foot: V2,
    target: V2,
    vel: V2,
    /// from the cursor to the foot, fixed at the grab
    grab: V2,
}

/// Balls resting on a drift, one on top of another. Three of them make a snowman.
pub struct Stack {
    surf: SurfId,
    /// random choice of hat, scarf and face, picked when the first ball lands
    pub look: u32,
    /// position along the surface, relative to its left edge
    x: f32,
    pub balls: Vec<StackBall>,
    /// world-space centers, bottom first (recomputed every frame)
    pub centers: Vec<V2>,
    pub visible: bool,
    pub vel: V2,
}

impl Stack {
    /// The window this stack rests on (None for the taskbar).
    pub fn window(&self) -> Option<isize> {
        match self.surf {
            SurfId::Win(h) => Some(h),
            SurfId::Floor(_) => None,
        }
    }

    pub fn is_snowman(&self) -> bool {
        self.balls.len() >= 3
    }
}

pub struct Mark {
    pub hwnd: isize,
    rel: V2,
    pub r: f32,
    pub life: f32,
    pub max_life: f32,
    pub blobs: Vec<(V2, f32)>,
    pub pos: V2,
    pub visible: bool,
    /// the window's rectangle; the splat is clipped to it
    pub clip: RectI,
}

pub struct FireBox {
    /// bottom center
    pub pos: V2,
    pub w: f32,
    pub h: f32,
    pub label: Option<String>,
    pub floor: usize,
}

pub struct Banner {
    pub text: String,
    pub sub: Option<String>,
    pub alpha: f32,
    pub center: V2,
}

enum Target {
    Surface(SurfId),
    Window(isize, RectI),
    Edge,
    Air,
}

enum StackHit {
    Stack,
    Shatter,
    Splat,
    /// glancing contact: push the ball out along `normal` and let it roll off
    Bounce {
        normal: V2,
        contact: V2,
    },
}

pub fn ball_radius(mass: f32, s: f32) -> f32 {
    ((mass / PI).sqrt() * 1.5).max(5.0 * s)
}

/// Fixed speckle pattern drawn on balls so that rolling is visible.
pub const SPECKLES: [(f32, f32, f32); 8] = [
    (0.3, 0.55, 0.10),
    (1.4, 0.30, 0.07),
    (2.2, 0.62, 0.09),
    (3.0, 0.45, 0.06),
    (3.9, 0.70, 0.08),
    (4.7, 0.25, 0.07),
    (5.5, 0.58, 0.10),
    (6.1, 0.80, 0.06),
];

pub struct World {
    pub scale: f32,
    pub cfg: Config,
    pub monitors: Vec<Monitor>,
    pub vrect: RectI,
    pub windows: Vec<WinInfo>,
    /// screen areas of menus and flyouts: snow is not drawn there
    pub popups: Vec<RectI>,
    pub win_surf: HashMap<isize, Surface>,
    pub floors: Vec<Surface>,
    pub particles: Vec<Particle>,
    pub held: Option<Held>,
    pub carried: Option<Carried>,
    pub balls: Vec<Ball>,
    /// sounds to play this frame, with the screen x they come from
    pub sounds: Vec<(Sfx, f32)>,
    pub stacks: Vec<Stack>,
    pub marks: Vec<Mark>,
    pub plows: Vec<Plow>,
    pub fireworks: Fireworks,
    pub firebox: Option<FireBox>,
    pub banner: Option<Banner>,
    pub active: bool,
    /// how far ahead (s) to extrapolate moving windows when drawing
    pub lead: f32,
    pub time: f32,
    /// debug: start with drifts already in place (SNOWIE_PRESNOW)
    presnow: bool,
    /// debug (SNOWIE_SNOWMAN=x,y): a snowman is put on the first drift below that screen point
    debug_snowman: Option<V2>,
    frame: u64,
    next_plow: f32,
    plow_dir: f32,
    last_ny_secs: i64,
    last_minute: i64,
    greeting_until: f32,
    greeting_year: i32,
    rng: fastrand::Rng,
}

impl World {
    pub fn new(monitors: Vec<Monitor>, vrect: RectI, scale: f32, cfg: Config) -> World {
        let mut w = World {
            scale,
            next_plow: 180.0f32.min(cfg.plow_minutes * 60.0),
            cfg,
            monitors: Vec::new(),
            vrect,
            windows: Vec::new(),
            popups: Vec::new(),
            win_surf: HashMap::new(),
            floors: Vec::new(),
            particles: Vec::new(),
            held: None,
            carried: None,
            balls: Vec::new(),
            sounds: Vec::new(),
            stacks: Vec::new(),
            marks: Vec::new(),
            plows: Vec::new(),
            fireworks: Fireworks::default(),
            firebox: None,
            banner: None,
            active: true,
            lead: 0.0,
            time: 0.0,
            presnow: std::env::var_os("SNOWIE_PRESNOW").is_some(),
            debug_snowman: std::env::var("SNOWIE_SNOWMAN").ok().and_then(|v| {
                let (x, y) = v.split_once(',')?;
                Some(V2(x.trim().parse().ok()?, y.trim().parse().ok()?))
            }),
            frame: 0,
            plow_dir: 1.0,
            last_ny_secs: -1,
            last_minute: -1,
            greeting_until: 0.0,
            greeting_year: 0,
            rng: fastrand::Rng::new(),
        };
        w.set_monitors(monitors, vrect);
        w
    }

    pub fn set_monitors(&mut self, monitors: Vec<Monitor>, vrect: RectI) {
        if monitors == self.monitors && vrect == self.vrect && !self.floors.is_empty() {
            return;
        }
        let s = self.scale;
        let h = self.cfg.floor_drift * s;
        self.floors = monitors
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let mut f = Surface::new(m.work.l, m.work.b, m.work.w(), h, true, m.work);
                f.key = (1 << 62) + i as u64;
                if self.presnow {
                    f.heights.fill((30.0 * s).min(h));
                }
                f
            })
            .collect();
        self.stacks.retain(|st| !matches!(st.surf, SurfId::Floor(_)));
        self.plows.clear();
        self.monitors = monitors;
        self.vrect = vrect;
    }

    pub fn set_config(&mut self, cfg: Config) {
        let s = self.scale;
        for surf in self.win_surf.values_mut() {
            surf.max_h = cfg.window_drift * s;
            surf.heights.iter_mut().for_each(|h| *h = h.min(surf.max_h));
            surf.touch(true);
        }
        for f in &mut self.floors {
            f.max_h = cfg.floor_drift * s;
            f.heights.iter_mut().for_each(|h| *h = h.min(f.max_h));
            f.touch(true);
        }
        let interval = cfg.plow_minutes.max(1.0) * 60.0;
        if self.next_plow > self.time + interval {
            self.next_plow = self.time + interval;
        }
        self.cfg = cfg;
    }

    /// Popups that cover most of a monitor are invisible helper windows, not menus.
    pub fn set_popups(&mut self, mut popups: Vec<RectI>) {
        popups.retain(|p| {
            !self.monitors.iter().any(|m| {
                let l = p.l.max(m.rect.l);
                let r = p.r.min(m.rect.r);
                let t = p.t.max(m.rect.t);
                let b = p.b.min(m.rect.b);
                r > l && b > t && ((r - l) as i64 * (b - t) as i64) * 10 >= m.rect.w() as i64 * m.rect.h() as i64 * 9
            })
        });
        self.popups = popups;
    }

    /// Fixed random seed, for reproducible benchmarks.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn seed(&mut self, seed: u64) {
        self.rng = fastrand::Rng::with_seed(seed);
    }

    pub fn clear(&mut self) {
        for s in self.win_surf.values_mut().chain(self.floors.iter_mut()) {
            s.heights.fill(0.0);
            s.touch(true);
        }
        self.marks.clear();
        self.balls.clear();
        self.stacks.clear();
        self.particles.retain(|p| p.kind == Kind::Flake);
    }

    pub fn call_plow(&mut self) {
        self.next_plow = self.time;
        self.start_plows();
    }

    pub fn fire_salute(&mut self) {
        self.fireworks.show(4.0, 2.5);
    }

    /// True while something moves fast enough that we should draw at the full refresh rate.
    pub fn in_motion(&self) -> bool {
        self.held.is_some()
            || self.carried.is_some()
            || !self.balls.is_empty()
            || self.win_surf.iter().any(|(hwnd, s)| {
                // only windows that carry something of ours need smooth tracking
                s.vel.len() > 20.0
                    && (s.heights.iter().zip(&s.visible).any(|(h, v)| *v && *h >= 0.3)
                        || self.marks.iter().any(|m| m.hwnd == *hwnd)
                        || self.stacks.iter().any(|st| st.surf == SurfId::Win(*hwnd)))
            })
    }

    fn surf_mut(&mut self, id: SurfId) -> &mut Surface {
        match id {
            SurfId::Win(h) => self.win_surf.get_mut(&h).unwrap(),
            SurfId::Floor(i) => &mut self.floors[i],
        }
    }

    fn surf_opt(&self, id: SurfId) -> Option<&Surface> {
        match id {
            SurfId::Win(h) => self.win_surf.get(&h),
            SurfId::Floor(i) => self.floors.get(i),
        }
    }

    /// Drift surfaces from back to front: windows bottom-up in z-order, then the taskbar
    /// (which is always on top).
    pub fn surfaces(&self) -> impl Iterator<Item = &Surface> {
        self.windows.iter().rev().filter_map(|w| self.win_surf.get(&w.hwnd)).chain(self.floors.iter())
    }

    /// Rectangles of the windows in front of `hwnd` (topmost first). Snow that belongs to
    /// `hwnd` must not be drawn over them.
    pub fn front_of(&self, hwnd: isize) -> Vec<RectI> {
        match self.windows.iter().position(|w| w.hwnd == hwnd) {
            Some(i) => self.windows[..i].iter().map(|w| w.rect).collect(),
            None => Vec::new(),
        }
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn window_vel(&self, hwnd: isize) -> V2 {
        self.win_surf.get(&hwnd).map_or(V2::ZERO, |s| s.vel)
    }

    fn primary_floor(&self) -> usize {
        self.monitors.iter().position(|m| m.rect.l == 0 && m.rect.t == 0).unwrap_or(0)
    }

    pub fn update(&mut self, dt: f32, windows: Vec<WinInfo>, inputs: Vec<Input>, clock: Clock) {
        self.time += dt;
        self.frame += 1;
        let moves = self.sync_windows(windows, dt);
        self.compute_visibility();
        self.push_snow(&moves);
        for i in inputs {
            self.handle_input(i);
        }
        self.update_held(dt);
        self.update_carried(dt);
        self.spawn_flakes(dt);
        self.update_particles(dt);
        self.relax_surfaces();
        self.update_stacks(dt);
        self.update_balls(dt);
        self.update_marks(dt);
        self.update_plows(dt);
        self.update_firebox(clock);
        self.update_fireworks(dt, clock);
    }

    // ---------------------------------------------------------------- windows

    fn sync_windows(&mut self, windows: Vec<WinInfo>, dt: f32) -> Vec<(isize, V2)> {
        let mut moves = Vec::new();
        let s = self.scale;
        let max_h = self.cfg.window_drift * s;
        for surf in self.win_surf.values_mut() {
            surf.seen = false;
        }
        for w in &windows {
            let r = w.rect;
            let cx = (r.l + r.r) / 2;
            let ceiling = match self
                .monitors
                .iter()
                .find(|m| m.rect.l <= cx && cx < m.rect.r && m.rect.t <= r.t && r.t < m.rect.b)
            {
                Some(m) => (r.t - m.rect.t) as f32 <= 12.0 * s,
                None => true,
            };
            let presnow = self.presnow;
            let surf = self.win_surf.entry(w.hwnd).or_insert_with(|| {
                let mut surf = Surface::new(r.l, r.t, r.w(), max_h, false, r);
                surf.key = w.hwnd as u64;
                if presnow {
                    let n = surf.heights.len() as f32;
                    for (i, h) in surf.heights.iter_mut().enumerate() {
                        let e = (i as f32).min(n - i as f32);
                        *h = (16.0 * s).min(4.0 * s + e * 0.6).min(max_h);
                    }
                }
                surf
            });
            surf.seen = true;
            surf.ceiling = ceiling;
            let old = surf.rect;
            if old != r {
                moves.push((w.hwnd, V2((r.l - old.l) as f32, (r.t - old.t) as f32)));
                let new_w = r.w().max(1) as usize;
                let old_w = surf.heights.len();
                let right_anchor = old.r == r.r && old.l != r.l;
                if new_w < old_w {
                    let cut = old_w - new_w;
                    let range = if right_anchor { 0..cut } else { new_w..old_w };
                    shed_range(surf, range, 1.0, V2::ZERO, &mut self.particles, &mut self.rng, s);
                    if right_anchor {
                        surf.heights.drain(0..cut);
                    } else {
                        surf.heights.truncate(new_w);
                    }
                } else if new_w > old_w {
                    if right_anchor {
                        surf.heights.splice(0..0, std::iter::repeat_n(0.0, new_w - old_w));
                    } else {
                        surf.heights.resize(new_w, 0.0);
                    }
                }
                surf.visible.resize(new_w, false);
                surf.touch(true);
                let dpos = V2((r.l - old.l) as f32, (r.t - old.t) as f32);
                let moved = if right_anchor { V2::ZERO } else { dpos };
                surf.track += moved;
                surf.left = r.l;
                surf.top = r.t;
                surf.rect = r;
            }
            // velocity and acceleration over the last ~35 ms: the window may move less often than we draw,
            // and a short window makes the prediction catch up quickly when a drag starts
            let now = self.time;
            surf.hist.push_back((now, surf.track, surf.vel));
            while surf.hist.len() > 2 && now - surf.hist[1].0 >= 0.035 {
                surf.hist.pop_front();
            }
            let (t0, p0, v0) = surf.hist[0];
            let span = now - t0;
            if span > 1e-3 {
                surf.vel = (surf.track - p0) / span;
                surf.jerk = (surf.vel - v0).len() / span;
            }
            // shaking a window (sharp jerks back and forth, not a smooth drag) knocks the snow off
            let thr = 40000.0 * s;
            if surf.jerk > thr {
                let frac = (((surf.jerk - thr) / (80000.0 * s)).min(1.0) * dt * 8.0).min(0.7);
                let n = surf.heights.len();
                let v = surf.vel * 0.35;
                shed_range(surf, 0..n, frac, v, &mut self.particles, &mut self.rng, s);
            }
            if ceiling {
                let n = surf.heights.len();
                shed_range(surf, 0..n, 1.0, V2::ZERO, &mut self.particles, &mut self.rng, s);
            }
        }
        let gone: Vec<isize> = self.win_surf.iter().filter(|(_, v)| !v.seen).map(|(k, _)| *k).collect();
        for k in gone {
            if let Some(mut surf) = self.win_surf.remove(&k) {
                let n = surf.heights.len();
                shed_range(&mut surf, 0..n, 1.0, V2::ZERO, &mut self.particles, &mut self.rng, s);
            }
        }
        self.windows = windows;
        moves
    }

    /// A moving window works like a snowplow blade on every other drift: snow that ends up inside
    /// the window (or inside the snow on its top) is shoved out ahead of it, in the direction of
    /// motion (to the nearer side when it moves up or down). Pushed past the end of a window, the
    /// snow falls off. Snowmen in the way get shoved along too.
    fn push_snow(&mut self, moves: &[(isize, V2)]) {
        let s = self.scale;
        for &(hwnd, d) in moves {
            let Some(a) = self.win_surf.get(&hwnd) else { continue };
            let r = a.rect;
            let ends = a.heights.first().copied().unwrap_or(0.0).max(a.heights.last().copied().unwrap_or(0.0));
            let peak = a.heights.iter().fold(0.0f32, |m, h| m.max(*h));
            // Only a drift pushes a drift: the window's top edge with its own snow, moving sideways,
            // against snow lying at about the same level. A window passing in front of other
            // snow (up, down or at another height) just hides it.
            // The direction comes from the smoothed velocity: a hand drag jitters, and a single
            // sideways step while the window rises past another drift must not count as a push.
            let v = a.vel;
            let sideways = v.x.abs() >= 150.0 * s && v.x.abs() >= v.y.abs() * 2.0;
            // a jump (snapping to a screen edge, maximize, restore) is not a drag through the snow
            let jump = d.x.abs() > 150.0 * s;
            if !sideways || jump || d.x == 0.0 || d.x.signum() != v.x.signum() {
                continue;
            }
            let dir = d.x.signum();
            let band = (r.t as f32 - ends.max(peak * 0.6) - 1.0, r.t as f32 + 6.0 * s);
            // the blade is the strip the leading edge swept this frame: snow already under the
            // window (it arrived from below or above) stays where it is, hidden behind it
            let sweep = d.x.abs().ceil() as i32 + (2.0 * s).ceil() as i32;
            let r = if dir > 0.0 { RectI { l: r.r - sweep, ..r } } else { RectI { r: r.l + sweep, ..r } };
            let mut spills: Vec<(V2, f32, f32)> = Vec::new();
            // only windows behind the moving one: a window in front of it does not touch it
            let behind: Vec<isize> = match self.windows.iter().position(|w| w.hwnd == hwnd) {
                Some(i) => self.windows[i + 1..].iter().map(|w| w.hwnd).collect(),
                None => Vec::new(),
            };
            for (k, surf) in self.win_surf.iter_mut() {
                if behind.contains(k) {
                    plow(surf, r, band, dir, &mut spills);
                }
            }
            for (pos, m, side) in spills {
                let n = ((m / 3.0).ceil() as usize).clamp(1, 12);
                for _ in 0..n {
                    let p = pos + V2(side * (1.0 + self.rng.f32() * 5.0) * s, -(4.0 + self.rng.f32() * 10.0) * s);
                    let v = V2(side * (30.0 + self.rng.f32() * 90.0) * s, -(20.0 + self.rng.f32() * 60.0) * s);
                    let c = Particle::chunk(p, v, m / n as f32, s, &mut self.rng);
                    if self.particles.len() < MAX_PARTICLES {
                        self.particles.push(c);
                    }
                }
            }
            // snowmen and resting balls in the way are shoved along
            let places: Vec<Option<(i32, i32)>> =
                self.stacks
                    .iter()
                    .map(|st| {
                        if st.surf == SurfId::Win(hwnd) {
                            None
                        } else {
                            self.surf_opt(st.surf).map(|f| (f.left, f.top))
                        }
                    })
                    .collect();
            for (st, place) in self.stacks.iter_mut().zip(places) {
                let Some((left, top)) = place else { continue };
                if st.window().is_some_and(|h| !behind.contains(&h)) {
                    continue;
                }
                let rb = st.balls.first().map_or(0.0, |b| b.r);
                let x = left as f32 + st.x;
                let top = top as f32;
                if top <= band.0 || top - rb * 2.0 >= band.1 || x + rb < r.l as f32 || x - rb > r.r as f32 {
                    continue;
                }
                let to_right = dir > 0.0 || (dir == 0.0 && x >= (r.l + r.r) as f32 / 2.0);
                st.x = if to_right { r.r as f32 + rb } else { r.l as f32 - rb } - left as f32;
            }
        }
    }

    fn compute_visibility(&mut self) {
        fn set(v: &mut [bool], left: i32, a: i32, b: i32, val: bool) {
            let a = (a - left).max(0) as usize;
            let b = ((b - left).max(0) as usize).min(v.len());
            if a < b {
                v[a..b].fill(val);
            }
        }
        for (i, w) in self.windows.iter().enumerate() {
            let Some(surf) = self.win_surf.get_mut(&w.hwnd) else { continue };
            let mut vis = vec![false; surf.visible.len()];
            if !surf.ceiling && self.cfg.snow_on_windows {
                let y = w.rect.t - 1;
                let l = surf.left;
                for m in &self.monitors {
                    if m.rect.t <= y && y < m.rect.b {
                        set(&mut vis, l, m.rect.l, m.rect.r, true);
                    }
                }
                for o in &self.windows[..i] {
                    if o.rect.t <= y && y < o.rect.b {
                        set(&mut vis, l, o.rect.l, o.rect.r, false);
                    }
                }
            }
            surf.visible = vis;
            let hidden = surf.ceiling || !self.cfg.snow_on_windows;
            if hidden != surf.hidden {
                surf.hidden = hidden;
                let n = surf.heights.len();
                surf.mark(0, n, true);
            }
        }
    }

    /// Finds the first drift surface something moving down from y0 to y1 at column x lands on.
    /// `strict` requires actually crossing the snow surface from above (used for snowballs).
    fn find_landing(&self, x: f32, y0: f32, y1: f32, strict: bool) -> Option<SurfId> {
        let mut best: Option<(SurfId, f32)> = None;
        let mut check = |id: SurfId, s: &Surface| {
            let top = s.top as f32;
            if y1 < top - s.max_h - 2.0 || y0 > top + 1.0 {
                return;
            }
            let Some(c) = s.col(x) else { return };
            if !s.visible[c] {
                return;
            }
            let sy = s.surface_y(c);
            let hit = if strict { y0 <= sy + 1.0 && y1 >= sy } else { y0 <= top && y1 >= sy };
            if hit && best.is_none_or(|(_, b)| sy < b) {
                best = Some((id, sy));
            }
        };
        for (k, s) in &self.win_surf {
            check(SurfId::Win(*k), s);
        }
        for (i, s) in self.floors.iter().enumerate() {
            check(SurfId::Floor(i), s);
        }
        best.map(|b| b.0)
    }

    /// Is there grabbable snow at p?
    fn hit_surface(&self, p: V2, slack: f32, min_h: f32) -> Option<SurfId> {
        let test = |s: &Surface| -> bool {
            let Some(c) = s.col(p.x) else { return false };
            s.visible[c] && s.heights[c] >= min_h && p.y >= s.surface_y(c) - slack && p.y < s.top as f32 + 1.0
        };
        // same order as drawn: the taskbar drift is in front, then windows from the top down
        if let Some(i) = self.floors.iter().position(test) {
            return Some(SurfId::Floor(i));
        }
        self.windows.iter().find(|w| self.win_surf.get(&w.hwnd).is_some_and(test)).map(|w| SurfId::Win(w.hwnd))
    }

    // ---------------------------------------------------------------- mouse

    /// Would a press at `p` take something (snow, a ball, the fireworks box)? Lets an overlay
    /// that cannot swallow clicks (macOS) catch the mouse only where there is something to take.
    #[cfg_attr(windows, allow(dead_code))]
    pub fn hit(&self, p: V2) -> bool {
        if !self.active {
            return false;
        }
        let s = self.scale;
        if let Some(b) = &self.firebox {
            let r = RectI {
                l: (b.pos.x - b.w / 2.0) as i32,
                t: (b.pos.y - b.h) as i32,
                r: (b.pos.x + b.w / 2.0) as i32,
                b: b.pos.y as i32,
            };
            if r.contains_inflated(p, 4.0 * s) {
                return true;
            }
        }
        let on_stack = self
            .stacks
            .iter()
            .any(|st| st.visible && st.centers.iter().zip(&st.balls).any(|(c, b)| (p - *c).len() <= b.r));
        on_stack || self.hit_surface(p, 5.0 * s, 3.0 * s).is_some()
    }

    /// Called from the mouse hook on left button down. Returns true if the click hit something
    /// of ours (the click is then swallowed).
    pub fn try_grab(&mut self, p: V2) -> bool {
        if !self.active || self.held.is_some() || self.carried.is_some() {
            return false;
        }
        let s = self.scale;
        if let Some(b) = &self.firebox {
            let r = RectI {
                l: (b.pos.x - b.w / 2.0) as i32,
                t: (b.pos.y - b.h) as i32,
                r: (b.pos.x + b.w / 2.0) as i32,
                b: b.pos.y as i32,
            };
            if r.contains_inflated(p, 4.0 * s) {
                self.fire_salute();
                return true;
            }
        }
        // a stack or a snowman: grabbed by the bottom ball it moves whole, anywhere else the top
        // ball comes off
        for i in 0..self.stacks.len() {
            let st = &self.stacks[i];
            if !st.visible {
                continue;
            }
            let Some(j) = st.centers.iter().zip(&st.balls).position(|(c, b)| (p - *c).len() <= b.r) else {
                continue;
            };
            if j == 0 && st.balls.len() >= 2 {
                let st = self.stacks.remove(i);
                let foot = st.centers[0] + V2(0.0, st.balls[0].r * 0.85);
                self.carried = Some(Carried { foot, target: foot, vel: V2::ZERO, grab: foot - p, stack: st });
                self.sounds.push((Sfx::Grab, p.x));
                return true;
            }
            {
                let st = &mut self.stacks[i];
                let c = *st.centers.last().unwrap();
                let b = st.balls.pop().unwrap();
                st.centers.pop();
                if st.balls.is_empty() {
                    self.stacks.remove(i);
                }
                self.held = Some(Held::new(c, p, b.mass, 1.0, b.rot, Vec::new()));
                self.sounds.push((Sfx::Grab, p.x));
                return true;
            }
        }
        let Some(id) = self.hit_surface(p, 5.0 * s, 3.0 * s) else { return false };
        let mass = self.surf_mut(id).take(p.x, 24.0 * s, 14.0 * s);
        if mass < 15.0 * s {
            return false;
        }
        let blobs = (0..9)
            .map(|i| {
                let a = i as f32 * 2.4 + self.rng.f32();
                let d = if i == 0 { 0.0 } else { 0.45 + self.rng.f32() * 0.5 };
                (V2::from_angle(a) * d, 0.35 + self.rng.f32() * 0.3)
            })
            .collect();
        self.held = Some(Held::new(p, p, mass, 0.0, 0.0, blobs));
        self.sounds.push((Sfx::Grab, p.x));
        true
    }

    fn handle_input(&mut self, input: Input) {
        if self.carried.is_some() {
            match input {
                Input::Move(p) => {
                    let c = self.carried.as_mut().unwrap();
                    c.target = p + c.grab;
                }
                Input::Up(p) => {
                    let mut c = self.carried.take().unwrap();
                    c.target = p + c.grab;
                    self.put_down(c);
                }
            }
            return;
        }
        let Some(mut h) = self.held.take() else { return };
        let s = self.scale;
        match input {
            Input::Move(p) => {
                if p == h.cursor {
                    self.held = Some(h);
                    return;
                }
                h.cursor = p;
                // circling the mouse rolls the snow into a ball
                let d = p - h.last_p;
                let dist = d.len();
                if dist >= 2.0 * s {
                    let dir = d / dist;
                    if let Some(ld) = h.last_dir {
                        let ang = ld.cross(dir).atan2(ld.dot(dir));
                        h.spin_ema = h.spin_ema * 0.8 + ang * 0.2;
                        if ang * h.spin_ema > 0.0 && h.spin_ema.abs() > 0.05 {
                            h.compaction = (h.compaction + ang.abs() / (1.6 * TAU)).min(1.0);
                        }
                    }
                    h.last_dir = Some(dir);
                    h.last_p = p;
                }
                self.held = Some(h);
            }
            Input::Up(p) => {
                h.cursor = p;
                let r = ball_radius(h.mass, s);
                let max = 5000.0 * s;
                let vel = if h.vel.len() > max { h.vel.norm() * max } else { h.vel };
                let at = h.pos;
                if h.compaction >= 0.5 {
                    let ignore =
                        self.windows.iter().filter(|w| w.rect.contains_inflated(at, r)).map(|w| w.hwnd).collect();
                    self.balls.push(Ball { pos: at, vel, mass: h.mass, r, rot: h.rot, spin: vel.x / r, ignore });
                    if vel.len() > 250.0 * s {
                        self.sounds.push((Sfx::Throw(vel.len() / (2500.0 * s)), at.x));
                    }
                } else {
                    // a loose clump just crumbles
                    self.sounds.push((Sfx::Crumble, at.x));
                    let count = (h.mass / 4.0).clamp(6.0, 160.0) as usize;
                    let pm = h.mass / count as f32;
                    for _ in 0..count {
                        let off = V2::from_angle(self.rng.f32() * TAU) * (self.rng.f32() * r);
                        let v = vel * 0.6 + V2((self.rng.f32() - 0.5) * 80.0 * s, (self.rng.f32() - 0.7) * 80.0 * s);
                        let c = Particle::chunk(at + off, v, pm, s, &mut self.rng);
                        self.particles.push(c);
                    }
                }
            }
        }
    }

    /// The carried stack trails the cursor, slower the heavier it is; the upper balls sway behind.
    fn update_carried(&mut self, dt: f32) {
        let Some(c) = self.carried.as_mut() else { return };
        let s = self.scale;
        let r0 = c.stack.balls[0].r;
        let mass: f32 = c.stack.balls.iter().map(|b| b.mass).sum();
        let rate = 22.0 / (1.0 + (ball_radius(mass, s) / (45.0 * s)).powi(2));
        let prev = c.foot;
        c.foot += (c.target - c.foot) * (1.0 - (-rate * dt).exp());
        c.vel = c.vel * 0.6 + (c.foot - prev) / dt.max(1e-4) * 0.4;
        let st = &mut c.stack;
        st.vel = c.vel;
        st.centers.clear();
        let mut y = c.foot.y + r0 * 0.15;
        let ease = (-10.0 * dt).exp();
        for (k, b) in st.balls.iter_mut().enumerate() {
            let overlap = if k == 0 { 1.0 } else { 0.8 };
            y -= b.r * overlap;
            // upper balls lean against the motion a little
            let lean = V2((-c.vel.x * 0.006 * k as f32 * s).clamp(-b.r * 0.35, b.r * 0.35), 0.0);
            b.off = lean + (b.off - lean) * ease;
            st.centers.push(V2(c.foot.x, y) + b.off);
            y -= b.r * 0.8;
        }
    }

    /// Sets a carried stack down on the first drift below it; over nothing it falls apart.
    fn put_down(&mut self, c: Carried) {
        let s = self.scale;
        let mut st = c.stack;
        let r0 = st.balls[0].r;
        let foot = c.foot;
        let landing = self.find_landing(foot.x, foot.y - r0 * 0.5, self.vrect.b as f32 + 1e4, false);
        let place =
            landing.and_then(|id| self.surf_opt(id).map(|f| (id, f.left, f.col(foot.x).map(|k| f.surface_y(k)))));
        if let Some((id, left, Some(sy))) = place {
            st.surf = id;
            st.x = foot.x - left as f32;
            // the balls ease from where they are down onto the drift instead of jumping
            let mut y = sy + r0 * 0.15;
            for (k, (b, o)) in st.balls.iter_mut().zip(&st.centers).enumerate() {
                let overlap = if k == 0 { 1.0 } else { 0.8 };
                y -= b.r * overlap;
                b.off = *o - V2(foot.x, y);
                y -= b.r * 0.8;
            }
            self.sounds.push((Sfx::Plop(r0 / s), foot.x));
            self.stacks.push(st);
            return;
        }
        for (b, ctr) in st.balls.iter().zip(&st.centers) {
            self.balls.push(Ball {
                pos: *ctr,
                vel: c.vel * 0.5,
                mass: b.mass,
                r: b.r,
                rot: b.rot,
                spin: 0.0,
                ignore: Vec::new(),
            });
        }
    }

    fn update_held(&mut self, dt: f32) {
        let Some(mut h) = self.held.take() else { return };
        let s = self.scale;
        let r = ball_radius(h.mass, s);
        // the bigger the ball, the softer the spring: heavy balls lag behind the cursor
        let rate = 28.0 / (1.0 + (r / (40.0 * s)).powi(2));
        let prev = h.pos;
        h.pos += (h.cursor - h.pos) * (1.0 - (-rate * dt).exp());
        let d = h.pos - prev;
        let dist = d.len();
        h.vel = h.vel * 0.5 + d / dt.max(1e-4) * 0.5;
        if dist > 0.01 {
            h.rot += dist / r * if h.spin_ema < 0.0 { -1.0 } else { 1.0 };
            // rolling over a drift picks up more snow, with no upper limit
            if h.compaction > 0.25 {
                let bottom = V2(h.pos.x, h.pos.y + r * 0.7);
                if let Some(id) = self.hit_surface(bottom, r * 0.4, 1.0 * s) {
                    let got = self.surf_mut(id).take(h.pos.x, r, dist * 0.05);
                    h.mass += got;
                    if got > 0.0 {
                        h.roll_acc += dist;
                        if h.roll_acc > 26.0 * s {
                            h.roll_acc = 0.0;
                            self.sounds.push((Sfx::Roll(r / s), h.pos.x));
                        }
                    }
                }
            }
        }
        self.held = Some(h);
    }

    // ---------------------------------------------------------------- particles

    fn spawn_flakes(&mut self, dt: f32) {
        let s = self.scale;
        let total_w: i32 = self.monitors.iter().map(|m| m.rect.w()).sum();
        if total_w <= 0 {
            return;
        }
        let target = (FLAKES_PER_1920 * self.cfg.intensity * total_w as f32 / 1920.0) as usize;
        let cur = self.particles.iter().filter(|p| p.kind == Kind::Flake).count();
        if cur >= target || self.particles.len() >= MAX_PARTICLES {
            return;
        }
        let initial = self.frame <= 1;
        let n =
            if initial { target - cur } else { ((target - cur) as f32).min(target as f32 * dt * 0.5 + 1.0) as usize };
        for _ in 0..n {
            let mut pick = self.rng.i32(0..total_w);
            let m = *self
                .monitors
                .iter()
                .find(|m| {
                    if pick < m.rect.w() {
                        true
                    } else {
                        pick -= m.rect.w();
                        false
                    }
                })
                .unwrap_or(&self.monitors[0]);
            let x = m.rect.l as f32 + pick as f32 + self.rng.f32();
            let y = if initial {
                m.rect.t as f32 + self.rng.f32() * m.rect.h() as f32
            } else {
                m.rect.t as f32 - 4.0 * s - self.rng.f32() * 40.0 * s
            };
            let depth = self.rng.f32();
            let r = (0.8 + depth * depth * 2.4) * s * self.cfg.flake_size;
            self.particles.push(Particle {
                pos: V2(x, y),
                vel: V2(0.0, 30.0 * s),
                r,
                mass: ACCUM * PI * r * r,
                kind: Kind::Flake,
                depth,
                phase: self.rng.f32() * TAU,
                alpha: 0.45 + 0.55 * depth,
            });
        }
    }

    fn update_particles(&mut self, dt: f32) {
        let s = self.scale;
        let t = self.time;
        let wind = ((t * 0.07).sin() * 22.0 + (t * 0.23 + 1.3).sin() * 12.0) * s * self.cfg.wind;
        let acc = self.cfg.accumulation;
        let g = 1500.0 * s;
        let mut parts = std::mem::take(&mut self.particles);
        let vr = self.vrect;
        parts.retain_mut(|p| {
            let old = p.pos;
            match p.kind {
                Kind::Flake => {
                    let fall = (26.0 + 55.0 * p.depth) * s;
                    p.phase += dt * (0.8 + p.depth * 0.9);
                    let sway = (10.0 + 16.0 * p.depth) * s;
                    let tx = wind * (0.4 + 0.8 * p.depth) + p.phase.cos() * sway;
                    p.vel.x += (tx - p.vel.x) * (dt * 3.0).min(1.0);
                    p.vel.y += (fall - p.vel.y) * (dt * 2.0).min(1.0);
                }
                Kind::Chunk | Kind::Spray => {
                    p.vel.y += g * dt;
                    p.vel = p.vel * (1.0 - 0.5 * dt);
                    p.vel.y = p.vel.y.min(1400.0 * s);
                }
            }
            p.pos += p.vel * dt;
            if p.kind == Kind::Spray {
                p.alpha -= dt * 0.9;
                return p.alpha > 0.0 && p.pos.y < vr.b as f32 + 40.0;
            }
            if let Some(id) = self.find_landing(p.pos.x, old.y, p.pos.y, false) {
                // heavy chunks spread wider, so a falling drift does not land as spikes
                let spread = (p.r * 1.6).max(p.mass.sqrt() * 0.8).max(1.0);
                let m = if p.kind == Kind::Flake { p.mass * acc } else { p.mass };
                self.surf_mut(id).deposit(p.pos.x, m, spread);
                return false;
            }
            !(p.pos.y > vr.b as f32 + 40.0 || p.pos.x < vr.l as f32 - 300.0 || p.pos.x > vr.r as f32 + 300.0)
        });
        parts.append(&mut self.particles);
        self.particles = parts;
    }

    /// Finds window drifts whose end touches another visible drift at nearly the same height.
    /// Returns (window, right end?, neighbour, neighbour column).
    fn find_seams(&self) -> Vec<(isize, bool, SurfId, usize)> {
        let tol = 8.0 * self.scale;
        let mut seams = Vec::new();
        for (k, a) in &self.win_surf {
            if a.ceiling || a.heights.is_empty() {
                continue;
            }
            for right in [false, true] {
                // the drift's own end column must be visible, and the neighbour's first column next to it
                let end = if right { a.heights.len() - 1 } else { 0 };
                if !a.visible[end] {
                    continue;
                }
                let x = if right { a.right() as f32 + 0.5 } else { a.left as f32 - 0.5 };
                let near = |b: &Surface| b.col(x).filter(|&c| b.visible[c] && (b.top - a.top).abs() as f32 <= tol);
                let hit = self
                    .win_surf
                    .iter()
                    .filter(|(kb, _)| *kb != k)
                    .find_map(|(kb, b)| near(b).map(|c| (SurfId::Win(*kb), c)))
                    .or_else(|| {
                        self.floors.iter().enumerate().find_map(|(i, b)| near(b).map(|c| (SurfId::Floor(i), c)))
                    });
                if let Some((id, c)) = hit {
                    seams.push((*k, right, id, c));
                }
            }
        }
        seams
    }

    fn relax_surfaces(&mut self) {
        let s = self.scale;
        let flip = self.frame.is_multiple_of(2);
        let (slope, lip) = (0.85, 4.0 * s);
        let seams = self.find_seams();
        for (k, surf) in self.win_surf.iter_mut() {
            let l = seams.iter().any(|&(w, right, _, _)| w == *k && !right);
            let r = seams.iter().any(|&(w, right, _, _)| w == *k && right);
            // the end's outline changes, so its piece of the texture must be redrawn
            if l != surf.join_l {
                surf.mark(0, 1, true);
            }
            if r != surf.join_r {
                let n = surf.heights.len();
                surf.mark(n - 1, n, true);
            }
            surf.join_l = l;
            surf.join_r = r;
        }
        // snow levels out across a seam like inside one drift (compare absolute snow surfaces)
        for &(k, right, other, c) in &seams {
            let Some(a) = self.win_surf.get(&k) else { continue };
            let ca = if right { a.heights.len() - 1 } else { 0 };
            let ya = a.surface_y(ca);
            let Some(b) = self.surf_opt(other) else { continue };
            let yb = b.surface_y(c);
            let d = yb - ya; // > 0: a's snow stands higher than b's
            if d.abs() <= slope {
                continue;
            }
            let t = (d.abs() - slope) * 0.45;
            let (from_a, room) =
                if d > 0.0 { (true, b.max_h - b.heights[c]) } else { (false, a.max_h - a.heights[ca]) };
            let t = t.min(if from_a { a.heights[ca] } else { b.heights[c] }).min(room.max(0.0));
            if t <= 0.0 {
                continue;
            }
            let sign = if from_a { -1.0 } else { 1.0 };
            let a = self.win_surf.get_mut(&k).unwrap();
            a.heights[ca] += sign * t;
            a.mark(ca, ca + 1, false);
            let b = self.surf_mut(other);
            b.heights[c] -= sign * t;
            b.mark(c, c + 1, false);
        }
        let spill_q = 5.0 * s;
        for surf in self.win_surf.values_mut() {
            surf.relax(slope, lip, flip, s);
            if surf.spill_l >= spill_q {
                if self.particles.len() < MAX_PARTICLES && !surf.ceiling {
                    let pos = V2(surf.left as f32 - 1.5, surf.top as f32 - surf.heights[0] - 1.0);
                    let v = V2(-(10.0 + self.rng.f32() * 30.0) * s, 0.0);
                    self.particles.push(Particle::chunk(pos, v, surf.spill_l, s, &mut self.rng));
                }
                surf.spill_l = 0.0;
            }
            if surf.spill_r >= spill_q {
                if self.particles.len() < MAX_PARTICLES && !surf.ceiling {
                    let h = *surf.heights.last().unwrap();
                    let pos = V2(surf.right() as f32 + 0.5, surf.top as f32 - h - 1.0);
                    let v = V2((10.0 + self.rng.f32() * 30.0) * s, 0.0);
                    self.particles.push(Particle::chunk(pos, v, surf.spill_r, s, &mut self.rng));
                }
                surf.spill_r = 0.0;
            }
        }
        for f in &mut self.floors {
            f.relax(slope * 0.8, lip, flip, s);
        }
    }

    // ---------------------------------------------------------------- stacks & snowmen

    fn update_stacks(&mut self, dt: f32) {
        if let Some(p) = self.debug_snowman {
            if self.time > 2.0 {
                self.debug_snowman = None;
                let s = self.scale;
                if let Some(id) = self.find_landing(p.x, p.y, self.vrect.b as f32 + 1e4, false) {
                    let left = self.surf_opt(id).map_or(0, |f| f.left);
                    let balls = [26.0, 19.0, 13.0]
                        .iter()
                        .map(|r| StackBall { r: r * s, mass: PI * (r * s / 1.5).powi(2), rot: 0.0, off: V2::ZERO })
                        .collect();
                    let look = self.rng.u32(..);
                    let (x, vel) = (p.x - left as f32, V2::ZERO);
                    self.stacks.push(Stack { surf: id, look, x, balls, centers: Vec::new(), visible: true, vel });
                }
            }
        }
        let s = self.scale;
        let mut dropped: Vec<usize> = Vec::new();
        for (i, st) in self.stacks.iter_mut().enumerate() {
            let surf = match st.surf {
                SurfId::Win(h) => self.win_surf.get(&h),
                SurfId::Floor(f) => self.floors.get(f),
            };
            let Some(surf) = surf else {
                dropped.push(i);
                continue;
            };
            let x = surf.left as f32 + st.x;
            let col = surf.col(x);
            let window = matches!(st.surf, SurfId::Win(_));
            if col.is_none() || surf.ceiling || surf.jerk > 60000.0 * s || (window && !self.cfg.snow_on_windows) {
                dropped.push(i);
                continue;
            }
            let c = col.unwrap();
            st.visible = surf.visible[c];
            st.vel = surf.vel;
            st.centers.clear();
            let mut y = surf.surface_y(c) + st.balls[0].r * 0.15;
            let ease = (-10.0 * dt).exp();
            for (k, b) in st.balls.iter_mut().enumerate() {
                let overlap = if k == 0 { 1.0 } else { 0.8 };
                y -= b.r * overlap;
                b.off = b.off * ease;
                st.centers.push(V2(x, y) + b.off);
                y -= b.r * 0.8;
            }
        }
        // stacks whose window vanished or was shaken: the balls fall down
        for i in dropped.into_iter().rev() {
            let st = self.stacks.remove(i);
            for (b, c) in st.balls.iter().zip(&st.centers) {
                self.balls.push(Ball {
                    pos: *c,
                    vel: st.vel * 0.5,
                    mass: b.mass,
                    r: b.r,
                    rot: b.rot,
                    spin: 0.0,
                    ignore: Vec::new(),
                });
            }
        }
    }

    fn stack_hit(&self, b: &Ball) -> Option<(usize, StackHit)> {
        let s = self.scale;
        for (i, st) in self.stacks.iter().enumerate() {
            if !st.visible {
                continue;
            }
            for (j, c) in st.centers.iter().enumerate() {
                let rj = st.balls[j].r;
                let d = b.pos - *c;
                let contact = (b.r + rj) * 0.92;
                if d.len() >= contact {
                    continue;
                }
                let speed = (b.vel - st.vel).len();
                if speed > 900.0 * s {
                    return Some((i, StackHit::Shatter));
                }
                let top = j + 1 == st.balls.len();
                // it stays only if it really came down on top: nearly centred and from above
                let on_top = top && d.y < -rj * 0.5 && d.x.abs() < rj * 0.45 && b.vel.y >= -50.0 * s;
                if on_top && speed < GENTLE * s && st.balls.len() < 3 {
                    return Some((i, StackHit::Stack));
                }
                if speed > 600.0 * s && !on_top {
                    return Some((i, StackHit::Splat));
                }
                return Some((i, StackHit::Bounce { normal: d.norm(), contact: *c + d.norm() * contact }));
            }
        }
        None
    }

    // ---------------------------------------------------------------- snowballs

    fn update_balls(&mut self, dt: f32) {
        let s = self.scale;
        let g = 1700.0 * s;
        let vr = self.vrect;
        let mut balls = std::mem::take(&mut self.balls);
        balls.retain_mut(|b| {
            let steps = ((b.vel.len() * dt) / (b.r * 0.4).max(1.0)).ceil().clamp(1.0, 80.0) as usize;
            let h = dt / steps as f32;
            for _ in 0..steps {
                let prev = b.pos;
                b.vel.y += g * h;
                b.vel = b.vel * (1.0 - 0.12 * h);
                b.pos += b.vel * h;
                b.rot += b.spin * h;
                let windows = &self.windows;
                b.ignore.retain(|hw| {
                    windows.iter().find(|w| w.hwnd == *hw).is_some_and(|w| w.rect.contains_inflated(b.pos, b.r))
                });
                if let Some((i, hit)) = self.stack_hit(b) {
                    match hit {
                        StackHit::Stack => {
                            // remember where it touched, so it eases onto the top instead of snapping
                            let st = &mut self.stacks[i];
                            let top = *st.centers.last().unwrap();
                            let rt = st.balls.last().unwrap().r;
                            let rest = V2(top.x, top.y - rt * 0.8 - b.r * 0.8);
                            st.balls.push(StackBall { r: b.r, mass: b.mass, rot: b.rot, off: b.pos - rest });
                            self.sounds.push((Sfx::Plop(b.r), b.pos.x));
                            if st.balls.len() == 3 {
                                self.sounds.push((Sfx::Snowman, b.pos.x));
                            }
                        }
                        StackHit::Bounce { normal, contact } => {
                            // slide off the other ball: no sticking, no magnet
                            b.pos = contact;
                            let vn = b.vel.dot(normal);
                            if vn < 0.0 {
                                b.vel = b.vel - normal * (vn * 1.35);
                            }
                            // a nudge sideways so it never balances on the very top
                            let side = if normal.x.abs() < 0.05 {
                                if self.rng.bool() {
                                    1.0
                                } else {
                                    -1.0
                                }
                            } else {
                                normal.x.signum()
                            };
                            b.vel.x += side * 40.0 * self.scale;
                            continue;
                        }
                        StackHit::Shatter => {
                            let st = self.stacks.remove(i);
                            for (sb, c) in st.balls.iter().zip(&st.centers) {
                                self.splat(*c, b.vel * 0.4, sb.mass, sb.r, Target::Air);
                            }
                            self.splat(b.pos, b.vel, b.mass, b.r, Target::Air);
                        }
                        StackHit::Splat => self.splat(b.pos, b.vel, b.mass, b.r, Target::Air),
                    }
                    return false;
                }
                let off = V2(0.0, b.r * 0.8);
                if let Some(id) = self.find_landing(b.pos.x, (prev + off).y, (b.pos + off).y, true) {
                    let surf_vel = self.surf_opt(id).map_or(V2::ZERO, |s| s.vel);
                    if (b.vel - surf_vel).len() < GENTLE * s {
                        // a gentle landing: the ball sits down on the drift
                        self.sounds.push((Sfx::Plop(b.r / self.scale), b.pos.x));
                        let left = self.surf_opt(id).map_or(0, |s| s.left);
                        let look = self.rng.u32(..);
                        self.stacks.push(Stack {
                            surf: id,
                            look,
                            x: b.pos.x - left as f32,
                            balls: vec![StackBall { r: b.r, mass: b.mass, rot: b.rot, off: V2::ZERO }],
                            centers: vec![b.pos],
                            visible: true,
                            vel: surf_vel,
                        });
                    } else {
                        self.splat(b.pos + off, b.vel, b.mass, b.r, Target::Surface(id));
                    }
                    return false;
                }
                if let Some(w) = self.windows.iter().find(|w| w.rect.contains(b.pos)).copied() {
                    if !b.ignore.contains(&w.hwnd) {
                        self.splat(b.pos, b.vel, b.mass, b.r, Target::Window(w.hwnd, w.rect));
                        return false;
                    }
                }
                if b.pos.x < vr.l as f32 + b.r * 0.5 || b.pos.x > vr.r as f32 - b.r * 0.5 {
                    self.splat(b.pos, b.vel, b.mass, b.r, Target::Edge);
                    return false;
                }
                if b.pos.y > vr.b as f32 + 200.0 + b.r {
                    return false;
                }
            }
            true
        });
        balls.append(&mut self.balls);
        self.balls = balls;
    }

    fn splat(&mut self, pos: V2, vel: V2, mass: f32, r: f32, target: Target) {
        let s = self.scale;
        let mut rest = mass;
        let speed = vel.len();
        let power = (speed / (900.0 * s)).clamp(0.3, 1.6);
        self.sounds.push((Sfx::Splat(((mass / (300.0 * s * s)).sqrt() * power).clamp(0.2, 2.0)), pos.x));
        let (a0, a1, base) = match target {
            Target::Surface(id) => {
                let dep = mass * (0.6 - 0.3 * power.min(1.0));
                self.surf_mut(id).deposit(pos.x, dep, r * 1.3);
                rest -= dep;
                (-PI, 0.0, V2(vel.x * 0.25, -vel.y.abs() * 0.08))
            }
            Target::Window(hwnd, rect) => {
                let mut c = pos + vel.norm() * r * 0.8;
                c.x = c.x.clamp(rect.l as f32 + r * 0.4, rect.r as f32 - r * 0.4);
                c.y = c.y.clamp(rect.t as f32 + r * 0.4, rect.b as f32 - r * 0.4);
                let dir = vel.norm();
                let mut blobs = vec![(V2::ZERO, 0.85)];
                for _ in 0..9 {
                    let a = self.rng.f32() * TAU;
                    let d = 0.55 + self.rng.f32() * 0.75 * power;
                    blobs.push((V2::from_angle(a) * d, 0.18 + self.rng.f32() * 0.32));
                }
                for i in 1..=4 {
                    let d = 0.6 + i as f32 * 0.35 * power;
                    let side = V2(-dir.y, dir.x) * ((self.rng.f32() - 0.5) * 0.5);
                    blobs.push((dir * d + side, 0.32 - i as f32 * 0.05));
                }
                self.marks.push(Mark {
                    hwnd,
                    rel: c - rect.tl(),
                    r: r * 1.05,
                    life: 14.0,
                    max_life: 14.0,
                    blobs,
                    pos: c,
                    visible: true,
                    clip: rect,
                });
                rest -= mass * 0.3;
                (0.0, TAU, vel * -0.08)
            }
            Target::Edge => {
                let back = if vel.x > 0.0 { PI * 0.5 } else { -PI * 0.5 };
                (back, back + PI, V2(-vel.x * 0.15, vel.y * 0.1))
            }
            Target::Air => (0.0, TAU, vel * 0.2),
        };
        let count = (rest / 3.0).clamp(12.0, 320.0) as usize;
        let pm = rest / count as f32;
        let size_k = 0.6 + (r / (30.0 * s)).min(2.5);
        for _ in 0..count {
            if self.particles.len() >= MAX_PARTICLES {
                break;
            }
            let a = lerp(a0, a1, self.rng.f32());
            let sp = (60.0 + self.rng.f32() * 380.0) * s * size_k * power.max(0.5);
            let v = base + V2::from_angle(a) * sp;
            let off = V2::from_angle(self.rng.f32() * TAU) * (self.rng.f32() * r * 0.6);
            let c = Particle::chunk(pos + off, v, pm, s, &mut self.rng);
            self.particles.push(c);
        }
    }

    fn update_marks(&mut self, dt: f32) {
        let s = self.scale;
        let windows = &self.windows;
        self.marks.retain_mut(|m| {
            let Some(w) = windows.iter().find(|w| w.hwnd == m.hwnd) else { return false };
            m.life -= dt;
            if m.max_life - m.life > 2.0 {
                m.rel.y += 4.0 * s * dt;
                m.r *= 1.0 - 0.01 * dt;
            }
            m.pos = w.rect.tl() + m.rel;
            m.clip = w.rect;
            m.visible = windows.iter().find(|o| o.rect.contains(m.pos)).is_some_and(|o| o.hwnd == m.hwnd);
            m.life > 0.0 && m.rel.y < w.rect.h() as f32
        });
    }

    // ---------------------------------------------------------------- snowplow

    fn start_plows(&mut self) {
        if !self.plows.is_empty() {
            return;
        }
        let s = self.scale;
        let dir = self.plow_dir;
        self.plow_dir = -dir;
        for (i, f) in self.floors.iter().enumerate() {
            let x = if dir > 0.0 {
                f.left as f32 - fx::PLOW_HALF_LEN * s * 1.5
            } else {
                f.right() as f32 + fx::PLOW_HALF_LEN * s * 1.5
            };
            self.plows.push(Plow { floor: i, x, dir, load: 0.0, wheel: 0.0, t: 0.0 });
        }
        self.next_plow = self.time + self.cfg.plow_minutes.max(1.0) * 60.0;
    }

    fn update_plows(&mut self, dt: f32) {
        let s = self.scale;
        if self.cfg.plow && self.time >= self.next_plow {
            self.start_plows();
        }
        let mut plows = std::mem::take(&mut self.plows);
        plows.retain_mut(|p| {
            let Some(f) = self.floors.get(p.floor) else { return false };
            // keep snowmen and resting balls: the plow lifts its blade over them
            let protected: Vec<(f32, f32)> = self
                .stacks
                .iter()
                .filter(|st| st.surf == SurfId::Floor(p.floor))
                .filter_map(|st| st.centers.first().map(|c| (c.x - st.balls[0].r, c.x + st.balls[0].r)))
                .collect();
            let (left, right) = (f.left as f32, f.right() as f32);
            let old_front = p.x + p.dir * fx::PLOW_BLADE * s;
            p.x += p.dir * fx::PLOW_SPEED * s * dt;
            p.t += dt;
            p.wheel += fx::PLOW_SPEED * dt / 11.0;
            let new_front = p.x + p.dir * fx::PLOW_BLADE * s;
            let (a, b) = if old_front < new_front { (old_front, new_front) } else { (new_front, old_front) };
            let keep = 0.8 * s;
            let f = &mut self.floors[p.floor];
            let ca = (a - left).floor().max(0.0) as usize;
            let cb = ((b - left).ceil().max(0.0) as usize).min(f.heights.len());
            for c in ca..cb {
                let x = left + c as f32 + 0.5;
                if protected.iter().any(|&(l, r)| x >= l && x <= r) {
                    continue;
                }
                let h = &mut f.heights[c];
                if *h > keep {
                    p.load += *h - keep;
                    *h = keep;
                    f.mark(c, c + 1, true);
                }
            }
            // the blower chute on the cab roof throws the snow back over the truck; it melts in the air
            let emit = (p.load * dt * 3.0).min(p.load);
            p.load -= emit;
            let n = ((emit / 25.0).ceil() as usize).min(5);
            // thrown up over the back of the truck, from the dump bed
            let chute = V2(p.x - p.dir * 40.0 * s, f.top as f32 - 70.0 * s);
            for _ in 0..n {
                if self.particles.len() >= MAX_PARTICLES {
                    break;
                }
                let vel = V2(-p.dir * (80.0 + self.rng.f32() * 220.0) * s, -(300.0 + self.rng.f32() * 250.0) * s);
                let r = (1.5 + self.rng.f32() * 2.0) * s;
                self.particles.push(Particle {
                    pos: chute + V2((self.rng.f32() - 0.5) * 6.0 * s, 0.0),
                    vel,
                    r,
                    mass: 0.0,
                    kind: Kind::Spray,
                    depth: 1.0,
                    phase: 0.0,
                    alpha: 0.8 + self.rng.f32() * 0.3,
                });
            }
            let margin = fx::PLOW_HALF_LEN * s * 2.0;
            p.x > left - margin && p.x < right + margin
        });
        self.plows = plows;
    }

    // ---------------------------------------------------------------- fireworks

    fn update_firebox(&mut self, clock: Clock) {
        if !self.cfg.fireworks_box || self.floors.is_empty() {
            self.firebox = None;
            return;
        }
        let s = self.scale;
        let fi = self.primary_floor().min(self.floors.len() - 1);
        let f = &self.floors[fi];
        let x = f.left as f32 + self.cfg.box_pos.clamp(0.03, 0.97) * f.heights.len() as f32;
        let y = f.col(x).map_or(f.top as f32, |c| f.surface_y(c) + 3.0 * s);
        let label = (self.cfg.countdown && clock.secs_to_ny > 0).then(|| fx::countdown_text(clock.secs_to_ny));
        self.firebox = Some(FireBox { pos: V2(x, y), w: 60.0 * s, h: 50.0 * s, label, floor: fi });
    }

    fn update_fireworks(&mut self, dt: f32, clock: Clock) {
        let s = self.scale;
        // New Year: the countdown just wrapped around, or we started within the first minutes of 1 January
        if self.cfg.new_year_show {
            let wrapped = self.last_ny_secs >= 0 && self.last_ny_secs <= 5 && clock.secs_to_ny > 300 * 86400;
            let fresh = self.frame <= 2 && clock.month == 1 && clock.day == 1 && clock.hour == 0 && clock.minute < 10;
            if wrapped || fresh {
                self.fireworks.show(15.0, 6.0);
                self.fireworks.show(180.0, 2.5);
                self.greeting_until = self.time + 60.0;
                self.greeting_year = if wrapped { clock.year + 1 } else { clock.year };
                if wrapped && clock.month == 1 {
                    self.greeting_year = clock.year;
                }
            }
        }
        self.last_ny_secs = clock.secs_to_ny;
        // daily timers
        let minute_id = (clock.hour * 60 + clock.minute) as i64;
        if minute_id != self.last_minute {
            if self.last_minute >= 0 {
                let due = self
                    .cfg
                    .timers
                    .iter()
                    .filter_map(|t| parse_timer(t))
                    .any(|(h, m)| h == clock.hour && m == clock.minute);
                if due {
                    self.fireworks.show(40.0, 1.6);
                }
            }
            self.last_minute = minute_id;
        }
        // big countdown in the last ten seconds, then the greeting
        let pm = self.monitors.get(self.primary_floor()).map_or(self.vrect, |m| m.rect);
        let center = V2((pm.l + pm.r) as f32 / 2.0, pm.t as f32 + pm.h() as f32 * 0.38);
        self.banner = if self.cfg.new_year_show && clock.secs_to_ny > 0 && clock.secs_to_ny <= 10 {
            Some(Banner { text: clock.secs_to_ny.to_string(), sub: None, alpha: 1.0, center })
        } else if self.time < self.greeting_until {
            let left = self.greeting_until - self.time;
            Some(Banner {
                text: "С Новым годом!".into(),
                sub: Some(self.greeting_year.to_string()),
                alpha: (left / 4.0).min(1.0).min((60.0 - left) * 2.0),
                center,
            })
        } else {
            None
        };
        // launch
        for _ in 0..self.fireworks.due(dt) {
            let (from, mon) = match &self.firebox {
                Some(b) => (
                    V2(b.pos.x + (self.rng.f32() - 0.5) * b.w * 0.6, b.pos.y - b.h),
                    self.monitors.get(b.floor).map_or(pm, |m| m.rect),
                ),
                None => {
                    let f = &self.floors[self.primary_floor().min(self.floors.len() - 1)];
                    let x = f.left as f32 + self.rng.f32() * f.heights.len() as f32;
                    (V2(x, f.top as f32), pm)
                }
            };
            let apex = (from.y - mon.t as f32) * (0.45 + self.rng.f32() * 0.35);
            self.fireworks.launch(from, apex, s, &mut self.rng);
        }
        self.fireworks.update(dt, s, &mut self.rng);
        for (boom, at, power) in self.fireworks.events.drain(..) {
            self.sounds.push((if boom { Sfx::Boom(power) } else { Sfx::Launch }, at.x));
        }
    }
}

impl Held {
    fn new(pos: V2, cursor: V2, mass: f32, compaction: f32, rot: f32, blobs: Vec<(V2, f32)>) -> Held {
        Held {
            pos,
            cursor,
            vel: V2::ZERO,
            mass,
            compaction,
            rot,
            spin_ema: 0.0,
            last_dir: None,
            last_p: cursor,
            roll_acc: 0.0,
            blobs,
        }
    }
}

fn shed_range(
    surf: &mut Surface,
    range: Range<usize>,
    frac: f32,
    vel: V2,
    parts: &mut Vec<Particle>,
    rng: &mut fastrand::Rng,
    s: f32,
) {
    let mut i = range.start;
    while i < range.end {
        let j = (i + 3).min(range.end);
        let mut m = 0.0;
        let mut hmax = 0.0f32;
        for h in &mut surf.heights[i..j] {
            let t = *h * frac;
            *h -= t;
            hmax = hmax.max(*h);
            m += t;
        }
        if m > 0.0 {
            surf.mark(i, j, true);
        }
        if m > 0.5 && parts.len() < MAX_PARTICLES {
            let x = surf.left as f32 + (i + j) as f32 * 0.5;
            let y = surf.top as f32 - hmax - 1.0 - rng.f32() * 2.0;
            let v = vel + V2((rng.f32() - 0.5) * 60.0 * s, -rng.f32() * 30.0 * s);
            parts.push(Particle::chunk(V2(x, y), v, m, s, rng));
        }
        i = j;
    }
}

/// Clears the snow of `surf` that lies inside window rectangle `r` (extended up by its own snow,
/// `band` = top..bottom) and piles it up next to the window on the side it is moving to.
fn plow(surf: &mut Surface, r: RectI, band: (f32, f32), dir: f32, spills: &mut Vec<(V2, f32, f32)>) {
    let top = surf.top as f32;
    if top <= band.0 || top - surf.max_h * 1.6 >= band.1 {
        return;
    }
    let n = surf.heights.len();
    let c0 = (r.l - surf.left).max(0) as usize;
    let c1 = ((r.r - surf.left).max(0) as usize).min(n);
    if c0 >= c1 {
        return;
    }
    let mid = (c0 + c1) as f32 / 2.0;
    let (mut left, mut right) = (0.0f32, 0.0f32);
    for (c, hc) in surf.heights[c0..c1].iter_mut().enumerate() {
        let h = *hc;
        if h <= 0.05 || top - h >= band.1 {
            continue;
        }
        if dir > 0.0 || (dir == 0.0 && (c0 + c) as f32 >= mid) {
            right += h;
        } else {
            left += h;
        }
        *hc = 0.0;
    }
    if left + right <= 0.0 {
        return;
    }
    surf.mark(c0, c1, true);
    dump(surf, c1 as isize, right, 1, spills);
    dump(surf, c0 as isize - 1, left, -1, spills);
}

/// Piles pushed snow up just past the blade: a mound that leans on the blade and slopes away
/// from it, taller and wider the more snow is pushed. What runs off the end of the drift falls.
fn dump(surf: &mut Surface, c: isize, mut m: f32, side: isize, spills: &mut Vec<(V2, f32, f32)>) {
    if m <= 0.0 {
        return;
    }
    let n = surf.heights.len() as isize;
    // gentler than the angle of repose, so the mound stays a mound while the blade moves on
    let slope = 0.5;
    let top = surf.max_h * 1.4;
    let (mut lo, mut hi) = (isize::MAX, isize::MIN);
    let mut end = c;
    // a few passes: each one builds the mound for the snow still left over the snow already there
    for _ in 0..4 {
        if m <= 0.01 || c < 0 || c >= n {
            break;
        }
        // a triangle of height H and slope k holds H² / k
        let base = surf.heights[c as usize];
        let peak = (base + (m * slope).sqrt()).min(top);
        if peak <= base + 0.01 {
            break;
        }
        for k in 0..n {
            let i = c + side * k;
            if i < 0 || i >= n {
                end = i;
                break;
            }
            let target = peak - slope * k as f32;
            let h = &mut surf.heights[i as usize];
            if target <= *h {
                if target < base - 2.0 {
                    break;
                }
                continue;
            }
            let add = m.min(target - *h);
            *h += add;
            m -= add;
            lo = lo.min(i);
            hi = hi.max(i);
            if m <= 0.01 {
                break;
            }
        }
    }
    if lo <= hi {
        surf.mark(lo as usize, hi as usize + 1, true);
    }
    if m > 0.01 {
        // ran off the end of the drift (or the mound is as tall as it gets): falls down
        let at = if end < 0 || end >= n { end } else { c };
        let x = (surf.left + at.clamp(0, n) as i32) as f32;
        spills.push((V2(x, surf.top as f32), m, side as f32));
    }
}
