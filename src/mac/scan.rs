//! macOS: screens and the on-screen windows (front to back), in global top-left coordinates
//! (points), the same layout the simulation uses on Windows.

use crate::geom::RectI;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::MainThreadMarker;
use objc2_app_kit::NSScreen;
use objc2_core_graphics::{kCGNullWindowID, CGWindowListCopyWindowInfo, CGWindowListOption};
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSRect, NSString};

#[derive(Clone, Copy, Debug)]
pub struct WinInfo {
    /// the window number; stable while the window exists
    pub hwnd: isize,
    pub rect: RectI,
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
            if number(&d, "kCGWindowOwnerPID").map(|p| p as i32) == Some(own_pid) {
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
            windows.push(WinInfo { hwnd: id as isize, rect });
        }
    }
    Scan { windows, popups: Vec::new() }
}
