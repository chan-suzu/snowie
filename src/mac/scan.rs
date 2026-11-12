//! macOS: screens and the on-screen windows (front to back), in global top-left coordinates
//! (points), the same layout the simulation uses on Windows.

use crate::geom::RectI;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSRunningApplication, NSScreen};
use objc2_core_graphics::{kCGNullWindowID, CGWindowListCopyWindowInfo, CGWindowListOption};
use objc2_foundation::{NSArray, NSBundle, NSDictionary, NSNumber, NSRect, NSString};

#[derive(Clone, Copy, Debug)]
pub struct WinInfo {
    /// the window number; stable while the window exists
    pub hwnd: isize,
    pub rect: RectI,
    /// the app that owns the window
    pub pid: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Monitor {
    pub rect: RectI,
    /// the screen without the menu bar and the Dock: drifts lie on its bottom edge
    pub work: RectI,
}

pub struct Scan {
    /// ordinary windows of other apps, frontmost first
    pub windows: Vec<WinInfo>,
    /// menus and popups: on macOS they sit above the overlay, so nothing is masked
    pub popups: Vec<RectI>,
}

/// Height of the primary screen: AppKit counts y up from its bottom edge, CoreGraphics and
/// the simulation count down from its top.
pub fn primary_height(mtm: MainThreadMarker) -> f64 {
    NSScreen::screens(mtm).firstObject().map_or(0.0, |s| s.frame().size.height)
}

/// An AppKit rectangle (bottom-left origin) in top-left global coordinates.
pub fn flip(r: NSRect, primary_h: f64) -> RectI {
    let t = primary_h - (r.origin.y + r.size.height);
    RectI {
        l: r.origin.x.round() as i32,
        t: t.round() as i32,
        r: (r.origin.x + r.size.width).round() as i32,
        b: (t + r.size.height).round() as i32,
    }
}

pub fn monitors(mtm: MainThreadMarker) -> Vec<Monitor> {
    let h = primary_height(mtm);
    NSScreen::screens(mtm)
        .iter()
        .map(|s| Monitor { rect: flip(s.frame(), h), work: flip(s.visibleFrame(), h) })
        .collect()
}

pub fn virtual_rect(mons: &[Monitor]) -> RectI {
    let mut v = mons.first().map_or(RectI { l: 0, t: 0, r: 1, b: 1 }, |m| m.rect);
    for m in mons {
        v.l = v.l.min(m.rect.l);
        v.t = v.t.min(m.rect.t);
        v.r = v.r.max(m.rect.r);
        v.b = v.b.max(m.rect.b);
    }
    v
}

fn number(d: &NSDictionary<NSString, AnyObject>, key: &str) -> Option<f64> {
    let v = d.objectForKey(&NSString::from_str(key))?;
    v.downcast_ref::<NSNumber>().map(|n| n.doubleValue())
}

/// Visible ordinary windows of other apps, frontmost first. Window bounds are readable without
/// the screen recording permission (only titles need it, and we don't use them).
pub fn scan_windows(own_pid: i32) -> Scan {
    let mut windows = Vec::with_capacity(64);
    let opts = CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements;
    if let Some(list) = CGWindowListCopyWindowInfo(opts, kCGNullWindowID) {
        // CFArray of CFDictionary is toll-free bridged to NSArray of NSDictionary
        let arr: &NSArray<NSDictionary<NSString, AnyObject>> =
            unsafe { &*(objc2_core_foundation::CFRetained::as_ptr(&list).as_ptr() as *const _) };
        for d in arr.iter() {
            let d: Retained<NSDictionary<NSString, AnyObject>> = d;
            // layer 0 is ordinary windows; the Dock, the menu bar and panels live on other layers
            if number(&d, "kCGWindowLayer") != Some(0.0) {
                continue;
            }
            let pid = number(&d, "kCGWindowOwnerPID").map_or(0, |p| p as i32);
            if pid == own_pid {
                continue;
            }
            if number(&d, "kCGWindowAlpha").is_some_and(|a| a < 0.15) {
                continue;
            }
            let Some(id) = number(&d, "kCGWindowNumber") else { continue };
            let Some(b) = d.objectForKey(&NSString::from_str("kCGWindowBounds")) else { continue };
            let Some(b) = b.downcast_ref::<NSDictionary>() else { continue };
            let b: &NSDictionary<NSString, AnyObject> = unsafe { &*(b as *const NSDictionary as *const _) };
            let (Some(x), Some(y), Some(w), Some(h)) =
                (number(b, "X"), number(b, "Y"), number(b, "Width"), number(b, "Height"))
            else {
                continue;
            };
            if w < 60.0 || h < 24.0 {
                continue;
            }
            let rect = RectI { l: x as i32, t: y as i32, r: (x + w) as i32, b: (y + h) as i32 };
            windows.push(WinInfo { hwnd: id as isize, rect, pid: pid.max(0) as u32 });
        }
    }
    Scan { windows, popups: Vec::new() }
}

// ---------------------------------------------------------------- games

/// Is the app a game (by the category in its Info.plist, or by where it is installed), or on
/// the user's no-snow list (by app or executable name, lower case)?
fn app_is_game(pid: u32, auto: bool, apps: &[String]) -> bool {
    let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid as libc::pid_t) else {
        return false;
    };
    let lower = |s: Option<Retained<NSString>>| s.map(|s| s.to_string().to_lowercase()).unwrap_or_default();
    if !apps.is_empty() {
        let name = lower(app.localizedName());
        let exe = lower(app.executableURL().and_then(|u| u.lastPathComponent()));
        let bundle = lower(app.bundleURL().and_then(|u| u.lastPathComponent()));
        let listed = apps.iter().any(|a| {
            !a.is_empty() && (*a == name || *a == exe || *a == bundle || bundle.trim_end_matches(".app") == a.as_str())
        });
        if listed {
            return true;
        }
    }
    if !auto {
        return false;
    }
    let Some(url) = app.bundleURL() else { return false };
    let path = lower(url.path());
    if path.contains("/steamapps/") || path.contains("/epic games/") || path.contains("/gog games/") {
        return true;
    }
    let Some(bundle) = NSBundle::bundleWithURL(&url) else { return false };
    bundle
        .objectForInfoDictionaryKey(&NSString::from_str("LSApplicationCategoryType"))
        .and_then(|v| v.downcast_ref::<NSString>().map(|s| s.to_string()))
        .is_some_and(|c| c.contains("games"))
}

/// Tells which windows belong to games or to apps on the no-snow list, cached per app.
pub struct Games {
    cache: std::collections::HashMap<u32, (bool, std::time::Instant)>,
}

impl Games {
    pub fn new() -> Games {
        Games { cache: Default::default() }
    }

    /// Splits off the windows that must stay free of snow.
    pub fn split(&mut self, windows: Vec<WinInfo>, auto: bool, apps: &[String]) -> (Vec<WinInfo>, Vec<RectI>) {
        use std::time::{Duration, Instant};
        if !auto && apps.is_empty() {
            return (windows, Vec::new());
        }
        let mut keep = Vec::with_capacity(windows.len());
        let mut shields = Vec::new();
        for w in windows {
            let fresh = self.cache.get(&w.pid).is_some_and(|c| c.1.elapsed() < Duration::from_secs(10));
            if !fresh {
                self.cache.insert(w.pid, (app_is_game(w.pid, auto, apps), Instant::now()));
            }
            if self.cache.get(&w.pid).is_some_and(|c| c.0) {
                shields.push(w.rect);
            } else {
                keep.push(w);
            }
        }
        self.cache.retain(|_, c| c.1.elapsed() < Duration::from_secs(60));
        (keep, shields)
    }

    /// Forget the answers, e.g. after the no-snow list changed.
    pub fn reset(&mut self) {
        self.cache.clear();
    }
}
