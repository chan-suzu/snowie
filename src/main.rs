#![windows_subsystem = "windows"]

mod audio;
mod config;
mod fx;
mod geom;
mod hook;
mod music;
mod profiler;
mod render;
mod scan;
mod settings;
mod tray;
mod world;

use config::Config;
use geom::V2;
use render::Renderer;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DwmGetCompositionTimingInfo, DWM_TIMING_INFO};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{CreateMutexW, Sleep};
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use world::World;

static CONFIG: Mutex<Option<Config>> = Mutex::new(None);
static RELOAD: AtomicBool = AtomicBool::new(false);
static CLEAR: AtomicBool = AtomicBool::new(false);
static PLOW: AtomicBool = AtomicBool::new(false);
static FIREWORKS: AtomicBool = AtomicBool::new(false);
static DISPLAY_CHANGED: AtomicBool = AtomicBool::new(false);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(u32::MAX);
/// Set by the WinEvent hook when any top-level window moves or resizes: wakes the idle wait,
/// so a drag gets a frame at once instead of at the next idle tick.
static WINDOW_MOVED: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn win_event(
    _hook: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    _time: u32,
) {
    if id_object == OBJID_WINDOW.0
        && id_child == 0
        && !hwnd.is_invalid()
        && unsafe { GetAncestor(hwnd, GA_ROOT) } == hwnd
    {
        WINDOW_MOVED.store(true, Ordering::Relaxed);
    }
}
static MUSIC_PCM: OnceLock<std::sync::Arc<Vec<i16>>> = OnceLock::new();

fn edit_config(f: impl FnOnce(&mut Config)) {
    if let Ok(mut g) = CONFIG.lock() {
        let cfg = g.get_or_insert_with(Config::load);
        f(cfg);
        let _ = cfg.save();
    }
    RELOAD.store(true, Ordering::Relaxed);
}

fn current_config() -> Config {
    CONFIG.lock().ok().and_then(|g| g.clone()).unwrap_or_default()
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        tray::WM_TRAY => {
            let ev = (lp.0 as u32) & 0xFFFF;
            if ev == WM_LBUTTONUP {
                tray::open_settings();
            } else if ev == WM_RBUTTONUP {
                match tray::show_menu(hwnd, &current_config()) {
                    tray::ID_SETTINGS => tray::open_settings(),
                    tray::ID_PAUSE => edit_config(|c| c.paused = !c.paused),
                    tray::ID_MUSIC => edit_config(|c| c.music = !c.music),
                    tray::ID_PROFILER => edit_config(|c| c.profiler = !c.profiler),
                    tray::ID_CLEAR => CLEAR.store(true, Ordering::Relaxed),
                    tray::ID_PLOW => PLOW.store(true, Ordering::Relaxed),
                    tray::ID_FIREWORKS => FIREWORKS.store(true, Ordering::Relaxed),
                    c if (tray::ID_INTENSITY..tray::ID_INTENSITY + 3).contains(&c) => {
                        edit_config(|cfg| cfg.intensity = tray::INTENSITY_PRESETS[(c - tray::ID_INTENSITY) as usize])
                    }
                    tray::ID_AUTOSTART => tray::set_autostart(!tray::autostart_enabled()),
                    tray::ID_EXIT => unsafe { PostQuitMessage(0) },
                    _ => {}
                }
            }
            LRESULT(0)
        }
        config::WM_SNOWIE_CMD => {
            match wp.0 {
                config::CMD_RELOAD => {
                    if let Ok(mut g) = CONFIG.lock() {
                        *g = Some(Config::load());
                    }
                    RELOAD.store(true, Ordering::Relaxed);
                }
                config::CMD_CLEAR => CLEAR.store(true, Ordering::Relaxed),
                config::CMD_PLOW => PLOW.store(true, Ordering::Relaxed),
                config::CMD_FIREWORKS => FIREWORKS.store(true, Ordering::Relaxed),
                config::CMD_EXIT => unsafe { PostQuitMessage(0) },
                _ => {}
            }
            LRESULT(0)
        }
        WM_DISPLAYCHANGE | WM_SETTINGCHANGE => {
            DISPLAY_CHANGED.store(true, Ordering::Relaxed);
            unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
        }
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        m if m == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            tray::add(hwnd);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

/// `--bench SECONDS [--out FILE] [--fireworks] [--heavy] [--music]`
struct BenchOpts {
    secs: f32,
    out: std::path::PathBuf,
    fireworks: bool,
    heavy: bool,
    music: bool,
    fps: u32,
}

const BENCH_WARMUP: f32 = 5.0;

fn refresh_hz() -> f32 {
    let mut info = DWM_TIMING_INFO { cbSize: std::mem::size_of::<DWM_TIMING_INFO>() as u32, ..Default::default() };
    unsafe {
        if DwmGetCompositionTimingInfo(HWND::default(), &mut info).is_ok() && info.rateRefresh.uiDenominator != 0 {
            info.rateRefresh.uiNumerator as f32 / info.rateRefresh.uiDenominator as f32
        } else {
            60.0
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--export-sounds") {
        let dir = args.get(i + 1).map(String::as_str).unwrap_or("snowie-sounds");
        let _ = audio::export(std::path::Path::new(dir));
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--export-music") {
        // writes the synthesized playlist to a WAV file
        let path = args.get(i + 1).map(String::as_str).unwrap_or("snowie-music.wav");
        let _ = std::fs::write(path, music::render_wav());
        return;
    }
    let bench = args.iter().position(|a| a == "--bench").map(|i| BenchOpts {
        secs: args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(30.0),
        out: args
            .iter()
            .position(|a| a == "--out")
            .and_then(|j| args.get(j + 1))
            .map(Into::into)
            .unwrap_or_else(|| "snowie-bench.md".into()),
        fireworks: args.iter().any(|a| a == "--fireworks"),
        heavy: args.iter().any(|a| a == "--heavy"),
        music: args.iter().any(|a| a == "--music"),
        fps: args
            .iter()
            .position(|a| a == "--fps")
            .and_then(|j| args.get(j + 1))
            .and_then(|v| v.parse().ok())
            .unwrap_or(60),
    });
    if args.iter().any(|a| a == "--settings") {
        let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        settings::run();
        return;
    }
    if let Err(e) = run(bench) {
        let text: Vec<u16> = format!("Snowie не смог запуститься:\n{e}").encode_utf16().chain([0]).collect();
        unsafe {
            MessageBoxW(None, PCWSTR(text.as_ptr()), w!("Snowie"), MB_ICONERROR);
        }
    }
}

/// Music and sound settings, applied to the mixer; everything goes quiet while snow is hidden.
struct SoundState {
    audio: audio::Audio,
    playing: bool,
    rendering: bool,
    volumes: (f32, f32),
    engine: f32,
    engine_pan: f32,
}

impl SoundState {
    fn update(&mut self, cfg: &Config, hidden: bool) {
        let want = cfg.music && !hidden && cfg.volume > 0.0;
        let vols = (cfg.volume, if cfg.sounds { cfg.sound_volume } else { 0.0 });
        if vols != self.volumes {
            self.volumes = vols;
            self.audio.volumes(vols.0, vols.1);
        }
        // the track is synthesized on first use only, so a muted Snowie never holds it in memory
        if want && !self.rendering {
            self.rendering = true;
            std::thread::spawn(|| {
                let _ = MUSIC_PCM.set(std::sync::Arc::new(music::render_samples()));
            });
        }
        match (want, self.playing, MUSIC_PCM.get()) {
            (true, false, Some(pcm)) => {
                self.audio.music(Some(pcm.clone()));
                self.playing = true;
            }
            (false, true, _) => {
                self.audio.music(None);
                self.playing = false;
            }
            _ => {}
        }
        if hidden && self.engine > 0.0 {
            self.engine = 0.0;
            self.audio.engine(0.0, 0.0);
        }
    }

    /// Plays this frame's sounds and keeps the snowplow engine in step with the truck.
    fn frame(&mut self, w: &mut World, cfg: &Config) {
        let vr = w.vrect;
        let pan = |x: f32| ((x - vr.l as f32) / vr.w().max(1) as f32 * 2.0 - 1.0).clamp(-1.0, 1.0);
        for (sfx, x) in w.sounds.drain(..) {
            if cfg.sounds {
                self.audio.play(sfx, pan(x));
            }
        }
        // the engine is loud on screen and fades as the truck drives off the edge
        let (level, at) = w
            .plows
            .first()
            .and_then(|p| {
                let f = w.floors.get(p.floor)?;
                let (l, r) = (f.left as f32, f.right() as f32);
                let inside = ((p.x - l).min(r - p.x) / (150.0 * w.scale) + 1.0).clamp(0.0, 1.0);
                Some((inside, p.x))
            })
            .unwrap_or((0.0, 0.0));
        let level = if cfg.sounds { level } else { 0.0 };
        // follow the truck across the screen: the mixer glides between these updates
        let p = pan(at);
        if (level - self.engine).abs() > 0.01
            || (p - self.engine_pan).abs() > 0.005
            || (level == 0.0 && self.engine != 0.0)
        {
            self.engine = level;
            self.engine_pan = p;
            self.audio.engine(level, p);
        }
    }
}

fn run(bench: Option<BenchOpts>) -> Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _instance = CreateMutexW(None, true, w!("Local\\SnowieSingleInstance"))?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            if bench.is_some() {
                MessageBoxW(None, w!("Закройте Snowie перед замером (--bench)."), w!("Snowie"), MB_ICONWARNING);
            }
            return Ok(());
        }
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);
        // a benchmark runs the default settings, never touches the user's config, and starts
        // with drifts in place so the scene is the same every time
        let mut cfg = match &bench {
            Some(b) => {
                std::env::set_var("SNOWIE_PRESNOW", "1");
                Config {
                    music: b.music,
                    intensity: if b.heavy { 2.5 } else { 1.0 },
                    plow: false,
                    countdown: true,
                    fps: b.fps,
                    ..Config::default()
                }
            }
            None => Config::load(),
        };
        *CONFIG.lock().unwrap() = Some(cfg.clone());

        let hinst: HINSTANCE = GetModuleHandleW(None)?.into();
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            lpszClassName: w!("SnowieWindow"),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(Error::from_thread());
        }

        let mut vr = scan::virtual_rect();
        let overlay = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP
                | WS_EX_TOPMOST
                | WS_EX_TOOLWINDOW
                | WS_EX_LAYERED
                | WS_EX_TRANSPARENT
                | WS_EX_NOACTIVATE,
            w!("SnowieWindow"),
            w!("Snowie"),
            WS_POPUP,
            vr.l,
            vr.t,
            vr.w(),
            vr.h(),
            None,
            None,
            Some(hinst),
            None,
        )?;
        SetLayeredWindowAttributes(overlay, COLORREF(0), 255, LWA_ALPHA)?;
        // hidden window that owns the tray icon, its menu and the command channel
        let tray_hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("SnowieWindow"),
            w!("Snowie tray"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinst),
            None,
        )?;
        if bench.is_none() {
            tray::add(tray_hwnd);
        }

        let scale = GetDpiForSystem() as f32 / 96.0;
        let mut mons = scan::monitors();
        let _ = hook::WORLD.set(Mutex::new(World::new(mons.clone(), vr, scale, cfg.clone())));
        let world = hook::WORLD.get().unwrap();
        if let Some(b) = &bench {
            let mut w = world.lock().unwrap();
            w.seed(2026);
            if b.fireworks {
                w.fireworks.show(b.secs + BENCH_WARMUP + 60.0, 2.5);
            }
        }

        let new_renderer =
            |vr: geom::RectI| Renderer::new(overlay, V2(vr.l as f32, vr.t as f32), vr.w() as u32, vr.h() as u32, scale);
        let mut renderer = new_renderer(vr)?;
        let mut hz = refresh_hz();
        let _ = ShowWindow(overlay, SW_SHOWNOACTIVATE);
        hook::spawn();
        // window moves are reported to this thread's message queue (out of context)
        let _move_hook = windows::Win32::UI::Accessibility::SetWinEventHook(
            EVENT_OBJECT_LOCATIONCHANGE,
            EVENT_OBJECT_LOCATIONCHANGE,
            None,
            Some(win_event),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
        let mut moving = false;
        // debug: SNOWIE_LEAD=<frames> overrides how far ahead moving windows' snow is drawn
        let lead_frames: f32 = std::env::var("SNOWIE_LEAD").ok().and_then(|v| v.parse().ok()).unwrap_or(0.35);
        let mut next_idle = Instant::now();

        // debug: pretend New Year comes in N seconds (SNOWIE_NY_IN=15)
        let fake_ny: Option<(Instant, i64)> =
            std::env::var("SNOWIE_NY_IN").ok().and_then(|v| v.parse().ok()).map(|n| (Instant::now(), n));

        let own = [overlay.0 as isize, tray_hwnd.0 as isize];
        let mut last = Instant::now();
        let mut last_check = Instant::now() - Duration::from_secs(5);
        let mut fullscreen = false;
        let mut was_hidden = false;
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get() as u32);
        let csv = std::env::var_os("SNOWIE_LOG").map(|_| std::env::temp_dir().join("snowie-profile.csv"));
        let mut prof = profiler::Profiler::new(csv);
        if let Some(b) = &bench {
            prof.start_bench(BENCH_WARMUP);
            let _ = b;
        }
        let started = Instant::now();
        let mut frame_start = Instant::now();
        let mut sound = SoundState {
            audio: audio::Audio::start(),
            playing: false,
            rendering: false,
            volumes: (-1.0, -1.0),
            engine: 0.0,
            engine_pan: 0.0,
        };

        'main: loop {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    break 'main;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            if RELOAD.swap(false, Ordering::Relaxed) {
                cfg = current_config();
                world.lock().unwrap().set_config(cfg.clone());
            }
            if DISPLAY_CHANGED.swap(false, Ordering::Relaxed) {
                let new_mons = scan::monitors();
                let new_vr = scan::virtual_rect();
                if new_vr != vr {
                    let _ = SetWindowPos(
                        overlay,
                        Some(HWND_TOPMOST),
                        new_vr.l,
                        new_vr.t,
                        new_vr.w(),
                        new_vr.h(),
                        SWP_NOACTIVATE,
                    );
                    drop(renderer);
                    renderer = new_renderer(new_vr)?;
                }
                hz = refresh_hz();
                world.lock().unwrap().set_monitors(new_mons.clone(), new_vr);
                mons = new_mons;
                vr = new_vr;
            }
            let now = Instant::now();
            if now - last_check > Duration::from_millis(500) {
                last_check = now;
                fullscreen = cfg.hide_fullscreen && scan::foreground_fullscreen(&mons);
                // No periodic "bring to top": the overlay is in the topmost band from creation, and
                // re-raising it would put snow over menus, flyouts and tooltips opened after it.
            }

            let hidden = cfg.paused || fullscreen;
            sound.update(&cfg, hidden);
            if hidden {
                if !was_hidden {
                    let mut w = world.lock().unwrap();
                    w.active = false;
                    w.held = None;
                    let _ = renderer.draw(&w, None);
                    drop(w);
                    let _ = renderer.present();
                    hook::take_inputs();
                }
                was_hidden = true;
                last = Instant::now();
                Sleep(100);
                continue;
            }
            was_hidden = false;

            // Idle pacing: draw at the fps setting, but wake up the moment a window starts moving
            // (WinEvent messages end the wait), so a drag never waits for the next idle tick.
            if !moving {
                loop {
                    let now = Instant::now();
                    if now >= next_idle || WINDOW_MOVED.load(Ordering::Relaxed) {
                        break;
                    }
                    let ms = ((next_idle - now).as_secs_f32() * 1000.0).ceil().max(1.0) as u32;
                    MsgWaitForMultipleObjectsEx(None, ms, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
                    let mut msg = MSG::default();
                    while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                        if msg.message == WM_QUIT {
                            break 'main;
                        }
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            }
            WINDOW_MOVED.store(false, Ordering::Relaxed);
            // sample window positions right before drawing, once the swap chain is ready
            renderer.wait();
            let t_ready = Instant::now();
            let interval = (t_ready - frame_start).as_secs_f32() * 1000.0;
            frame_start = t_ready;
            let dt = (t_ready - last).as_secs_f32().min(0.05);
            last = t_ready;
            let scan::Scan { windows, popups } = scan::scan_windows(&own);
            hook::poll_cursor();
            let inputs = hook::take_inputs();
            let mut clock = fx::local_clock();
            if let Some((start, n)) = fake_ny {
                let left = n - start.elapsed().as_secs() as i64;
                clock.secs_to_ny = if left > 0 { left } else { left + 365 * 86400 };
            }
            let t_scanned = Instant::now();
            let (t_upd0, t_upd1);
            prof.hud_on = cfg.profiler && bench.is_none();
            let hud = prof.hud(cores, hz);
            let drawn = {
                let mut w = world.lock().unwrap();
                w.active = true;
                if CLEAR.swap(false, Ordering::Relaxed) {
                    w.clear();
                }
                if PLOW.swap(false, Ordering::Relaxed) {
                    w.call_plow();
                }
                if FIREWORKS.swap(false, Ordering::Relaxed) {
                    w.fire_salute();
                }
                t_upd0 = Instant::now();
                w.set_popups(popups);
                w.update(dt, windows, inputs, clock);
                sound.frame(&mut w, &cfg);
                // every present waits for one vblank only (lowest latency); at rest the idle wait
                // above spaces frames to the fps setting. While windows move or a ball is in play
                // we draw every vblank and draw moving windows' snow where they will be when the
                // frame reaches the screen: measured ~0.35 of a refresh period (scripts: drag a window
                // at constant speed and compare the drift edge with the window edge in screenshots).
                renderer.sync = 1;
                moving = w.in_motion();
                w.lead = if moving { lead_frames / hz } else { 0.0 };
                next_idle = t_ready + Duration::from_secs_f32(1.0 / cfg.fps.max(15) as f32);
                t_upd1 = Instant::now();
                prof.scene = profiler::Scene {
                    particles: w.particles.len(),
                    sparks: w.fireworks.sparks.len(),
                    windows: w.windows.len(),
                    sync: renderer.sync,
                };
                renderer.draw(&w, hud.as_ref())
            };
            let t_present0 = Instant::now();
            let presented = drawn.and_then(|_| renderer.present());
            let t_present1 = Instant::now();
            let ms = |a: Instant, b: Instant| (b - a).as_secs_f32() * 1000.0;
            let rt = renderer.timing.get();
            prof.push_frame(profiler::Frame {
                scan: ms(t_ready, t_scanned) + ms(t_scanned, t_upd0),
                update: ms(t_upd0, t_upd1),
                record: rt[0],
                submit: rt[1],
                present: ms(t_present0, t_present1),
                interval,
            });
            while let Some(g) = renderer.poll_gpu() {
                prof.push_gpu(g);
            }
            prof.sample(renderer.vram_mb(), render::DRIFT_BUILDS.swap(0, Ordering::Relaxed));
            if let Some(b) = &bench {
                if started.elapsed().as_secs_f32() >= BENCH_WARMUP + b.secs {
                    let env = format!(
                        "- Snowie {}\n- Screen {}x{} on {} monitor(s), {:.0} Hz, scale {:.0}%\n- GPU: {}\n- CPU threads: {}\n- Scenario: default settings, drifts pre-filled{}{}{}, fixed seed",
                        env!("CARGO_PKG_VERSION"),
                        vr.w(),
                        vr.h(),
                        mons.len(),
                        hz,
                        scale * 100.0,
                        renderer.adapter_name,
                        cores,
                        if b.heavy { ", heavy snowfall" } else { "" },
                        if b.fireworks { ", fireworks running" } else { "" },
                        if b.music { ", music on" } else { ", music off" },
                    ) + &format!(", {} fps cap", b.fps);
                    let _ = std::fs::write(&b.out, prof.bench_report(&env));
                    break 'main;
                }
            }
            if presented.is_err() {
                // device lost (driver update, GPU reset): rebuild the renderer
                Sleep(500);
                drop(renderer);
                renderer = new_renderer(vr)?;
            }
        }
        sound.audio.music(None);
        tray::remove(tray_hwnd);
        Ok(())
    }
}
