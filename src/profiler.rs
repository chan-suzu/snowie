//! Built-in profiler: per-stage CPU frame times, GPU frame time from D3D11 timestamp queries,
//! and process resources as the OS reports them. Shows an on-screen HUD, can log CSV, and
//! drives `--bench`, a reproducible run that writes a report.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::io::Write as _;
use std::time::Instant;
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

/// CPU milliseconds spent in each stage of one frame.
#[derive(Clone, Copy, Default)]
pub struct Frame {
    /// enumerate windows, monitors, input
    pub scan: f32,
    /// simulation step
    pub update: f32,
    /// recording Direct2D commands
    pub record: f32,
    /// EndDraw: D2D flushes the batch to the GPU driver
    pub submit: f32,
    /// Present
    pub present: f32,
    /// time since the previous frame started
    pub interval: f32,
}

impl Frame {
    pub fn cpu(&self) -> f32 {
        self.scan + self.update + self.record + self.submit + self.present
    }
}

/// Process-wide numbers, sampled once a second.
#[derive(Clone, Copy, Default)]
pub struct ProcStats {
    /// CPU time over wall time, % of one logical core
    pub cpu_core: f32,
    pub working_set_mb: f32,
    pub private_mb: f32,
    pub peak_working_set_mb: f32,
    pub vram_mb: f32,
    pub fps: f32,
    pub drift_rebuilds: f32,
    /// mouse events through the global hook per second, and that thread's CPU, % of a core
    pub hook_events: f32,
    pub hook_cpu: f32,
}

/// Counts that describe the scene being drawn.
#[derive(Clone, Copy, Default)]
pub struct Scene {
    pub particles: usize,
    pub sparks: usize,
    pub windows: usize,
    pub sync: u32,
}

pub struct Hud {
    pub lines: Vec<String>,
    /// recent CPU and GPU frame times, ms, oldest first
    pub cpu: Vec<f32>,
    pub gpu: Vec<f32>,
    /// frame budget line for the graph, ms
    pub budget: f32,
}

const HISTORY: usize = 600;

/// (user, kernel) CPU seconds of this process.
fn process_cpu_split() -> (f64, f64) {
    let (mut c, mut e, mut k, mut u) =
        (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    unsafe {
        let _ = GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u);
    }
    let t = |f: FILETIME| ((f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64) as f64 / 1e7;
    (t(u), t(k))
}

fn process_cpu_seconds() -> f64 {
    let (u, k) = process_cpu_split();
    u + k
}

fn process_memory() -> (f32, f32, f32) {
    let mut pm = PROCESS_MEMORY_COUNTERS_EX {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    unsafe {
        let _ = GetProcessMemoryInfo(GetCurrentProcess(), &mut pm as *mut _ as *mut PROCESS_MEMORY_COUNTERS, pm.cb);
    }
    let mb = |b: usize| b as f32 / (1024.0 * 1024.0);
    (mb(pm.WorkingSetSize), mb(pm.PrivateUsage), mb(pm.PeakWorkingSetSize))
}

pub struct Stat {
    pub avg: f32,
    pub p50: f32,
    pub p95: f32,
    pub p99: f32,
    pub max: f32,
}

pub fn stat(values: impl Iterator<Item = f32>) -> Stat {
    let mut v: Vec<f32> = values.collect();
    if v.is_empty() {
        return Stat { avg: 0.0, p50: 0.0, p95: 0.0, p99: 0.0, max: 0.0 };
    }
    v.sort_by(f32::total_cmp);
    let q = |p: f32| v[((v.len() - 1) as f32 * p).round() as usize];
    Stat {
        avg: v.iter().sum::<f32>() / v.len() as f32,
        p50: q(0.5),
        p95: q(0.95),
        p99: q(0.99),
        max: *v.last().unwrap(),
    }
}

pub struct Profiler {
    pub hud_on: bool,
    frames: VecDeque<Frame>,
    gpu: VecDeque<f32>,
    pub proc: ProcStats,
    pub scene: Scene,
    sample_at: Instant,
    sample_cpu: f64,
    sample_hook: (u64, f64),
    frames_in_sample: u32,
    csv: Option<std::fs::File>,
    started: Instant,
    bench: Option<Bench>,
}

/// Everything recorded after the warm-up of a `--bench` run.
struct Bench {
    warmup: f32,
    frames: Vec<Frame>,
    gpu: Vec<f32>,
    proc: Vec<ProcStats>,
    cpu_at_start: Option<(Instant, (f64, f64))>,
    scenes: Vec<Scene>,
}

impl Profiler {
    pub fn new(csv_path: Option<std::path::PathBuf>) -> Profiler {
        let csv = csv_path.and_then(|p| std::fs::File::create(p).ok()).map(|mut f| {
            let _ = writeln!(
                f,
                "t_s,fps,cpu_frame_avg_ms,cpu_frame_p95_ms,cpu_frame_p99_ms,gpu_frame_avg_ms,gpu_frame_p95_ms,\
                 process_cpu_pct_core,working_set_mb,private_mb,vram_mb,particles,sparks,drift_rebuilds_per_s"
            );
            f
        });
        Profiler {
            hud_on: false,
            frames: VecDeque::with_capacity(HISTORY),
            gpu: VecDeque::with_capacity(HISTORY),
            proc: ProcStats::default(),
            scene: Scene::default(),
            sample_at: Instant::now(),
            sample_cpu: process_cpu_seconds(),
            sample_hook: (0, 0.0),
            frames_in_sample: 0,
            csv,
            started: Instant::now(),
            bench: None,
        }
    }

    pub fn start_bench(&mut self, warmup: f32) {
        self.bench = Some(Bench {
            warmup,
            frames: Vec::new(),
            gpu: Vec::new(),
            proc: Vec::new(),
            cpu_at_start: None,
            scenes: Vec::new(),
        });
    }

    fn benching(&self) -> bool {
        self.bench.as_ref().is_some_and(|b| self.started.elapsed().as_secs_f32() >= b.warmup)
    }

    pub fn push_frame(&mut self, f: Frame) {
        if self.frames.len() == HISTORY {
            self.frames.pop_front();
        }
        self.frames.push_back(f);
        self.frames_in_sample += 1;
        if self.benching() {
            let b = self.bench.as_mut().unwrap();
            if b.cpu_at_start.is_none() {
                b.cpu_at_start = Some((Instant::now(), process_cpu_split()));
            }
            b.frames.push(f);
        }
    }

    pub fn push_gpu(&mut self, ms: f32) {
        if self.gpu.len() == HISTORY {
            self.gpu.pop_front();
        }
        self.gpu.push_back(ms);
        if self.benching() {
            self.bench.as_mut().unwrap().gpu.push(ms);
        }
    }

    /// Once a second: process CPU and memory, video memory, CSV line.
    pub fn sample(&mut self, vram_mb: f32, drift_builds: u32) {
        let el = self.sample_at.elapsed().as_secs_f32();
        if el < 1.0 {
            return;
        }
        let cpu = process_cpu_seconds();
        let (ws, private, peak) = process_memory();
        let hook_now =
            (crate::hook::EVENTS.load(std::sync::atomic::Ordering::Relaxed), crate::hook::thread_cpu_seconds());
        self.proc = ProcStats {
            cpu_core: ((cpu - self.sample_cpu) / el as f64 * 100.0) as f32,
            working_set_mb: ws,
            private_mb: private,
            peak_working_set_mb: peak,
            vram_mb,
            fps: self.frames_in_sample as f32 / el,
            drift_rebuilds: drift_builds as f32 / el,
            hook_events: (hook_now.0 - self.sample_hook.0) as f32 / el,
            hook_cpu: ((hook_now.1 - self.sample_hook.1) / el as f64 * 100.0) as f32,
        };
        self.sample_hook = hook_now;
        self.sample_at = Instant::now();
        self.sample_cpu = cpu;
        self.frames_in_sample = 0;
        if self.benching() {
            let (p, sc) = (self.proc, self.scene);
            let b = self.bench.as_mut().unwrap();
            b.proc.push(p);
            b.scenes.push(sc);
        }
        if let Some(f) = &mut self.csv {
            let recent = self.frames.iter().rev().take(self.proc.fps.round().max(1.0) as usize);
            let c = stat(recent.map(|f| f.cpu()));
            let g = stat(self.gpu.iter().rev().take(self.proc.fps.round().max(1.0) as usize).copied());
            let p = self.proc;
            let _ = writeln!(
                f,
                "{:.1},{:.1},{:.3},{:.3},{:.3},{:.3},{:.3},{:.2},{:.1},{:.1},{:.1},{},{},{:.0}",
                self.started.elapsed().as_secs_f32(),
                p.fps,
                c.avg,
                c.p95,
                c.p99,
                g.avg,
                g.p95,
                p.cpu_core,
                p.working_set_mb,
                p.private_mb,
                p.vram_mb,
                self.scene.particles,
                self.scene.sparks,
                p.drift_rebuilds
            );
        }
    }

    pub fn hud(&self, cores: u32, refresh_hz: f32) -> Option<Hud> {
        if !self.hud_on {
            return None;
        }
        let n = (self.proc.fps.round() as usize).clamp(30, HISTORY);
        let recent: Vec<Frame> = self.frames.iter().rev().take(n).copied().collect();
        let c = stat(recent.iter().map(|f| f.cpu()));
        let g = stat(self.gpu.iter().rev().take(n).copied());
        let iv = stat(recent.iter().map(|f| f.interval));
        let avg = |k: fn(&Frame) -> f32| recent.iter().map(k).sum::<f32>() / recent.len().max(1) as f32;
        let p = self.proc;
        let s = self.scene;
        let lines = vec![
            format!(
                "Snowie profiler   {:.0} fps (vsync/{}), {:.0} Hz   interval p99 {:.1} ms",
                p.fps, s.sync, refresh_hz, iv.p99
            ),
            format!("CPU frame  avg {:.2}  p95 {:.2}  p99 {:.2}  max {:.2} ms", c.avg, c.p95, c.p99, c.max),
            format!(
                "  scan {:.2}  update {:.2}  record {:.2}  submit {:.2}  present {:.2}",
                avg(|f| f.scan),
                avg(|f| f.update),
                avg(|f| f.record),
                avg(|f| f.submit),
                avg(|f| f.present)
            ),
            format!("GPU frame  avg {:.2}  p95 {:.2}  p99 {:.2}  max {:.2} ms", g.avg, g.p95, g.p99, g.max),
            format!(
                "Process CPU {:.1}% of a core ({:.2}% of {} threads)",
                p.cpu_core,
                p.cpu_core / cores.max(1) as f32,
                cores
            ),
            format!(
                "RAM {:.0} MB (private {:.0}, peak {:.0})   VRAM {:.0} MB",
                p.working_set_mb, p.private_mb, p.peak_working_set_mb, p.vram_mb
            ),
            format!(
                "particles {}  sparks {}  windows {}  drift rebuilds {:.0}/s",
                s.particles, s.sparks, s.windows, p.drift_rebuilds
            ),
            format!("mouse hook {:.0} events/s, {:.2}% of a core", p.hook_events, p.hook_cpu),
        ];
        let take = 240usize;
        let cpu: Vec<f32> = self.frames.iter().rev().take(take).rev().map(|f| f.cpu()).collect();
        let gpu: Vec<f32> = self.gpu.iter().rev().take(take).rev().copied().collect();
        Some(Hud { lines, cpu, gpu, budget: 1000.0 / 60.0 })
    }

    /// Writes the `--bench` report (Markdown).
    pub fn bench_report(&self, env: &str) -> String {
        let Some(b) = &self.bench else { return String::new() };
        let mut r = String::new();
        let _ = writeln!(r, "# Snowie benchmark\n\n{env}\n");
        let secs = b.cpu_at_start.map_or(0.0, |(t, _)| t.elapsed().as_secs_f32());
        let (u1, k1) = process_cpu_split();
        let (user_pct, kernel_pct) = b.cpu_at_start.map_or((0.0, 0.0), |(_, (u0, k0))| {
            let d = secs.max(1e-3) as f64;
            (((u1 - u0) / d * 100.0) as f32, ((k1 - k0) / d * 100.0) as f32)
        });
        let _ = writeln!(
            r,
            "Measured {:.1} s after {:.0} s warm-up, {} frames ({:.1} fps).\n",
            secs,
            b.warmup,
            b.frames.len(),
            b.frames.len() as f32 / secs.max(1e-3)
        );
        let _ =
            writeln!(r, "## Frame time, ms\n\n| stage | avg | p50 | p95 | p99 | max |\n|---|---:|---:|---:|---:|---:|");
        type Row = (&'static str, fn(&Frame) -> f32);
        let rows: [Row; 7] = [
            ("CPU total", |f| f.cpu()),
            ("  scan", |f| f.scan),
            ("  update", |f| f.update),
            ("  record", |f| f.record),
            ("  submit", |f| f.submit),
            ("  present", |f| f.present),
            ("frame interval", |f| f.interval),
        ];
        for (name, k) in rows {
            let s = stat(b.frames.iter().map(k));
            let _ =
                writeln!(r, "| {name} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} |", s.avg, s.p50, s.p95, s.p99, s.max);
        }
        let g = stat(b.gpu.iter().copied());
        let _ = writeln!(
            r,
            "| GPU (timestamp queries) | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} |",
            g.avg, g.p50, g.p95, g.p99, g.max
        );
        let _ = writeln!(
            r,
            "
GPU time is the elapsed time between two timestamps on the GPU timeline. Work from other              processes that the GPU runs in between is included, so p50 is the fair figure for Snowie              itself and the tail shows contention."
        );
        let budget = 1000.0 / (b.frames.len() as f32 / secs.max(1e-3));
        let late = b.frames.iter().filter(|f| f.interval > budget * 1.5).count();
        let _ = writeln!(r, "\nFrames later than 1.5× the average interval: {late}.\n");
        let last = b.proc.last().copied().unwrap_or_default();
        let maxv = |k: fn(&ProcStats) -> f32| b.proc.iter().map(k).fold(0.0f32, f32::max);
        let _ = writeln!(r, "## Process\n\n| metric | value |\n|---|---:|");
        let _ = writeln!(r, "| CPU, % of one logical core | {:.2} |", user_pct + kernel_pct);
        let _ = writeln!(r, "| of which user / kernel | {:.2} / {:.2} |", user_pct, kernel_pct);
        let _ = writeln!(
            r,
            "| Working set at end / peak, MB | {:.1} / {:.1} |",
            last.working_set_mb, last.peak_working_set_mb
        );
        let _ =
            writeln!(r, "| Private bytes at end / max, MB | {:.1} / {:.1} |", last.private_mb, maxv(|p| p.private_mb));
        let _ = writeln!(r, "| Video memory (process), max MB | {:.1} |", maxv(|p| p.vram_mb));
        let avg_scene =
            |k: fn(&Scene) -> usize| b.scenes.iter().map(k).sum::<usize>() as f32 / b.scenes.len().max(1) as f32;
        let _ = writeln!(r, "| Particles, average | {:.0} |", avg_scene(|s| s.particles));
        let _ = writeln!(r, "| Firework sparks, average | {:.0} |", avg_scene(|s| s.sparks));
        let avg_proc = |k: fn(&ProcStats) -> f32| b.proc.iter().map(k).sum::<f32>() / b.proc.len().max(1) as f32;
        let _ = writeln!(
            r,
            "| Mouse hook: events per s / CPU, % of a core | {:.0} / {:.3} |",
            avg_proc(|p| p.hook_events),
            avg_proc(|p| p.hook_cpu)
        );
        let _ = writeln!(
            r,
            "| Drift segment rebuilds, per s | {:.0} |",
            b.proc.iter().map(|p| p.drift_rebuilds).sum::<f32>() / b.proc.len().max(1) as f32
        );
        r
    }
}
