//! macOS: one transparent, click-through panel per screen above all ordinary windows, and the
//! frame loop. A panel ignores the mouse except where there is snow under the cursor (or while
//! something is held), so every other click reaches the windows below as usual.

use super::render::Renderer;
use super::{scan, tray};
use crate::config::{self, Config};
use crate::geom::V2;
use crate::sound::SoundState;
use crate::world::{Input, World};
use crate::{audio, fx, music, settings};
use objc2::rc::Retained;
use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSColor, NSEvent, NSEventMask, NSPanel, NSScreen,
    NSView, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSPoint, NSRect};
use std::cell::RefCell;
use std::ptr::NonNull;
use std::time::{Duration, Instant};

/// Mouse events from the overlay views, in global top-left points.
enum Mouse {
    Down(V2),
    Drag(V2),
    Up(V2),
}

thread_local! {
    static EVENTS: RefCell<Vec<Mouse>> = const { RefCell::new(Vec::new()) };
}

/// The cursor in global top-left points.
fn cursor(mtm: MainThreadMarker) -> V2 {
    let p = NSEvent::mouseLocation();
    V2(p.x as f32, (scan::primary_height(mtm) - p.y) as f32)
}

define_class!(
    // SAFETY: NSView has no subclassing requirements and this class has no Drop impl.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "SnowieView"]
    struct SnowView;

    impl SnowView {
        // the first click on an inactive panel is a real click, not just activation
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &NSEvent) {
            let p = cursor(self.mtm());
            EVENTS.with(|e| e.borrow_mut().push(Mouse::Down(p)));
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, _event: &NSEvent) {
            let p = cursor(self.mtm());
            EVENTS.with(|e| e.borrow_mut().push(Mouse::Drag(p)));
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _event: &NSEvent) {
            let p = cursor(self.mtm());
            EVENTS.with(|e| e.borrow_mut().push(Mouse::Up(p)));
        }
    }
);

impl SnowView {
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }
}

/// One overlay per screen.
struct Overlay {
    panel: Retained<NSPanel>,
    view: Retained<SnowView>,
    /// the screen in global top-left points and its pixels per point
    rect: crate::geom::RectI,
    scale: f32,
    ignoring: bool,
}

/// Just above the Dock (20) and below the menu bar (24): snow covers ordinary windows and lies
/// on the Dock, while menus, popovers and the menu bar stay on top of it.
const LEVEL: isize = 23;

fn make_overlay(mtm: MainThreadMarker, screen: &NSScreen, primary_h: f64) -> Overlay {
    let frame = screen.frame();
    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        frame,
        NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
        NSBackingStoreType::Buffered,
        false,
    );
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setHasShadow(false);
    panel.setIgnoresMouseEvents(true);
    panel.setLevel(LEVEL);
    panel.setHidesOnDeactivate(false);
    // on every desktop, but not in the Spaces of full-screen apps and games
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::IgnoresCycle
            | NSWindowCollectionBehavior::FullScreenNone,
    );
    unsafe { panel.setReleasedWhenClosed(false) };
    let view = SnowView::new(mtm, NSRect::new(NSPoint::new(0.0, 0.0), frame.size));
    view.setWantsLayer(true);
    panel.setContentView(Some(&view));
    panel.setFrame_display(frame, false);
    panel.orderFrontRegardless();
    Overlay {
        panel,
        view,
        rect: scan::flip(frame, primary_h),
        scale: screen.backingScaleFactor() as f32,
        ignoring: true,
    }
}

fn make_overlays(mtm: MainThreadMarker) -> Vec<Overlay> {
    let h = scan::primary_height(mtm);
    NSScreen::screens(mtm).iter().map(|s| make_overlay(mtm, &s, h)).collect()
}

fn make_renderer(overlays: &[Overlay]) -> Result<Renderer, String> {
    let views: Vec<_> = overlays
        .iter()
        .map(|o| {
            let ptr = NonNull::from(&*o.view).cast::<std::ffi::c_void>();
            let r = o.rect;
            (ptr, V2(r.l as f32, r.t as f32), V2(r.w() as f32, r.h() as f32), o.scale)
        })
        .collect();
    Renderer::new(&views)
}

fn alert(text: &str) {
    eprintln!("Snowie: {text}");
}

pub fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--export-sounds") {
        let dir = args.get(i + 1).map(String::as_str).unwrap_or("snowie-sounds");
        let _ = audio::export(std::path::Path::new(dir));
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--export-music") {
        let path = args.get(i + 1).map(String::as_str).unwrap_or("snowie-music.wav");
        let _ = std::fs::write(path, music::render_wav());
        return;
    }
    if args.iter().any(|a| a == "--settings") {
        settings::run();
        return;
    }
    if config::overlay_running() {
        // already running: open the settings instead, like a second click on the app would
        tray::open_settings();
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else {
        alert("must run on the main thread");
        return;
    };
    run(mtm);
}

fn run(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    // no Dock icon and no menu of our own: just the snowflake in the menu bar
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    app.finishLaunching();

    let _ = std::fs::create_dir_all(config::dir());
    let _ = std::fs::write(config::pid_file(), std::process::id().to_string());
    let _ = std::fs::remove_file(config::commands_file());

    let mut cfg = Config::load();
    let mut mons = scan::monitors(mtm);
    let mut vr = scan::virtual_rect(&mons);
    let mut overlays = make_overlays(mtm);
    let mut renderer = match make_renderer(&overlays) {
        Ok(r) => r,
        Err(e) => {
            alert(&format!("cannot draw with Metal ({e})"));
            return;
        }
    };
    let tray = tray::Tray::new(mtm);
    tray.sync(&cfg);

    // the simulation works in points: on macOS one point is what one pixel is at 100% on Windows
    let mut world = World::new(mons.clone(), vr, 1.0, cfg.clone());
    let mut sound = SoundState::new();
    let mut games = scan::Games::new();
    let own_pid = std::process::id() as i32;
    let fake_ny: Option<(Instant, i64)> =
        std::env::var("SNOWIE_NY_IN").ok().and_then(|v| v.parse().ok()).map(|n| (Instant::now(), n));

    let mut last = Instant::now();
    let mut last_poll = Instant::now();
    let mut next_frame = Instant::now();
    let mut was_hidden = false;
    let mut holding = false;
    let mut quit = false;

    while !quit {
        // ---- events: wait in the run loop until the next frame is due (or an event comes)
        let wait = next_frame.saturating_duration_since(Instant::now()).as_secs_f64();
        let mut until = NSDate::dateWithTimeIntervalSinceNow(wait);
        loop {
            let ev = unsafe {
                app.nextEventMatchingMask_untilDate_inMode_dequeue(
                    NSEventMask::Any,
                    Some(&until),
                    NSDefaultRunLoopMode,
                    true,
                )
            };
            let Some(ev) = ev else { break };
            app.sendEvent(&ev);
            // after the first event, only drain what is already queued
            until = NSDate::distantPast();
        }

        // ---- menu, settings window, screens
        if let Some(id) = tray.take_pick() {
            match id {
                tray::ID_SETTINGS => tray::open_settings(),
                tray::ID_PAUSE => edit(&mut cfg, |c| c.paused = !c.paused),
                tray::ID_MUSIC => edit(&mut cfg, |c| c.music = !c.music),
                tray::ID_CLEAR => world.clear(),
                tray::ID_PLOW => world.call_plow(),
                tray::ID_FIREWORKS => world.fire_salute(),
                tray::ID_AUTOSTART => tray::set_autostart(!tray::autostart_enabled()),
                tray::ID_EXIT => quit = true,
                c if (tray::ID_INTENSITY..tray::ID_INTENSITY + 3).contains(&c) => {
                    edit(&mut cfg, |cfg| cfg.intensity = tray::INTENSITY_PRESETS[(c - tray::ID_INTENSITY) as usize])
                }
                _ => {}
            }
            world.set_config(cfg.clone());
            tray.sync(&cfg);
        }
        if last_poll.elapsed() > Duration::from_millis(250) {
            last_poll = Instant::now();
            for cmd in config::take_commands() {
                match cmd {
                    config::CMD_RELOAD => {
                        cfg = Config::load();
                        games.reset();
                        world.set_config(cfg.clone());
                        tray.sync(&cfg);
                    }
                    config::CMD_CLEAR => world.clear(),
                    config::CMD_PLOW => world.call_plow(),
                    config::CMD_FIREWORKS => world.fire_salute(),
                    config::CMD_EXIT => quit = true,
                    _ => {}
                }
            }
            let new_mons = scan::monitors(mtm);
            if new_mons != mons {
                // screens were added, removed or rearranged: new overlays and surfaces
                for o in &overlays {
                    o.panel.orderOut(None);
                }
                drop(renderer);
                overlays = make_overlays(mtm);
                renderer = match make_renderer(&overlays) {
                    Ok(r) => r,
                    Err(e) => {
                        alert(&format!("cannot draw with Metal ({e})"));
                        break;
                    }
                };
                vr = scan::virtual_rect(&new_mons);
                world.set_monitors(new_mons.clone(), vr);
                mons = new_mons;
            }
        }

        // a game or video in a borderless window that covers a whole screen (menu bar included)
        // hides the snow, like a full-screen app in its own Space does
        let scan::Scan { windows, popups } = scan::scan_windows(own_pid);
        let (windows, shields) = games.split(windows, cfg.hide_over_games, &cfg.no_snow_apps);
        let fullscreen = cfg.hide_fullscreen
            && windows.first().is_some_and(|w| {
                mons.iter().any(|m| {
                    let (r, s) = (w.rect, m.rect);
                    r.l <= s.l && r.t <= s.t && r.r >= s.r && r.b >= s.b
                })
            });
        let hidden = cfg.paused || fullscreen;
        sound.update(&cfg, hidden);
        if hidden {
            if !was_hidden {
                world.active = false;
                world.held = None;
                world.carried = None;
                renderer.draw(None);
                for o in overlays.iter_mut() {
                    o.panel.setIgnoresMouseEvents(true);
                    o.ignoring = true;
                }
            }
            was_hidden = true;
            last = Instant::now();
            next_frame = Instant::now() + Duration::from_millis(200);
            continue;
        }
        was_hidden = false;

        let now = Instant::now();
        let dt = (now - last).as_secs_f32().min(0.05);
        last = now;

        // ---- mouse: take it only over snow, or while something is in hand
        let mut inputs = Vec::new();
        for ev in EVENTS.with(|e| std::mem::take(&mut *e.borrow_mut())) {
            match ev {
                Mouse::Down(p) => {
                    holding = world.try_grab(p);
                }
                Mouse::Drag(p) if holding => inputs.push(Input::Move(p)),
                Mouse::Up(p) if holding => {
                    inputs.push(Input::Up(p));
                    holding = false;
                }
                _ => {}
            }
        }
        let p = cursor(mtm);
        if holding {
            inputs.push(Input::Move(p));
        }
        let over_snow = holding || world.hit(p);
        for o in overlays.iter_mut() {
            let want_ignore = !over_snow;
            if o.ignoring != want_ignore {
                o.panel.setIgnoresMouseEvents(want_ignore);
                o.ignoring = want_ignore;
            }
        }

        // ---- simulate and draw
        let mut clock = fx::local_clock();
        if let Some((start, n)) = fake_ny {
            let left = n - start.elapsed().as_secs() as i64;
            clock.secs_to_ny = if left > 0 { left } else { left + 365 * 86400 };
        }
        world.active = true;
        world.set_popups(popups);
        world.shields = shields;
        world.update(dt, windows, inputs, clock);
        sound.frame(&mut world, &cfg);
        world.lead = 0.0;
        renderer.draw(Some(&world));

        // full frame rate while things move or the cursor is over snow, the fps setting at rest
        let fast = world.in_motion() || over_snow;
        let fps = if fast { 60 } else { cfg.fps.clamp(15, 60) };
        next_frame = now + Duration::from_secs_f32(1.0 / fps as f32);
    }

    sound.audio.music(None);
    for o in &overlays {
        o.panel.orderOut(None);
    }
    let _ = std::fs::remove_file(config::pid_file());
}

fn edit(cfg: &mut Config, f: impl FnOnce(&mut Config)) {
    f(cfg);
    let _ = cfg.save();
}
