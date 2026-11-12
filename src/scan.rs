//! Enumerates monitors and the visible top-level windows (in z-order, topmost first).

use crate::geom::RectI;
use windows::core::BOOL;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Copy, Debug)]
pub struct WinInfo {
    pub hwnd: isize,
    pub rect: RectI,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Monitor {
    pub rect: RectI,
    pub work: RectI,
}

fn to_rect(r: RECT) -> RectI {
    RectI { l: r.left, t: r.top, r: r.right, b: r.bottom }
}

struct Ctx {
    out: Vec<WinInfo>,
    popups: Vec<RectI>,
    skip: Vec<isize>,
}

/// Result of one scan.
pub struct Scan {
    /// visible top-level windows that collect snow, topmost first
    pub windows: Vec<WinInfo>,
    /// menus, flyouts, tooltips, the Start menu: snow is never drawn over these
    pub popups: Vec<RectI>,
}

/// The desktop and the taskbar: neither windows nor popups.
const SKIP_CLASSES: &[&str] = &["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd", "SysShadow"];

/// Menus, tooltips and shell flyouts. Windows may place them below our topmost overlay,
/// so instead of relying on z-order their area is cut out of the snow.
const POPUP_CLASSES: &[&str] = &[
    "#32768",
    "tooltips_class32",
    "Xaml_WindowedPopupClass",
    "DropDown",
    "Windows.UI.Core.CoreWindow",
    "XamlExplorerHostIslandWindow",
    "TopLevelWindowForOverflowXamlIsland",
];

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lp: LPARAM) -> BOOL {
    let ctx = unsafe { &mut *(lp.0 as *mut Ctx) };
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return BOOL(1);
        }
        if ctx.skip.contains(&(hwnd.0 as isize)) {
            return BOOL(1);
        }
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex & WS_EX_TRANSPARENT.0 != 0 {
            return BOOL(1);
        }
        let mut cloaked: u32 = 0;
        if DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4).is_ok() && cloaked != 0 {
            return BOOL(1);
        }
        if ex & WS_EX_LAYERED.0 != 0 {
            let mut alpha = 255u8;
            let mut flags = LAYERED_WINDOW_ATTRIBUTES_FLAGS(0);
            if GetLayeredWindowAttributes(hwnd, None, Some(&mut alpha), Some(&mut flags)).is_ok()
                && flags.0 & LWA_ALPHA.0 != 0
                && alpha < 40
            {
                return BOOL(1);
            }
        }
        let mut r = RECT::default();
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut _ as *mut _,
            std::mem::size_of::<RECT>() as u32,
        )
        .is_err()
            && GetWindowRect(hwnd, &mut r).is_err()
        {
            return BOOL(1);
        }
        let rect = to_rect(r);
        let cls = class_name(hwnd);
        if SKIP_CLASSES.contains(&cls.as_str()) {
            return BOOL(1);
        }
        // topmost tool windows are menus and flyouts (the taskbar's own menus among them)
        let popup =
            POPUP_CLASSES.contains(&cls.as_str()) || (ex & WS_EX_TOPMOST.0 != 0 && ex & WS_EX_TOOLWINDOW.0 != 0);
        if popup {
            if rect.w() >= 8 && rect.h() >= 8 {
                ctx.popups.push(rect);
            }
            return BOOL(1);
        }
        if rect.w() < 60 || rect.h() < 24 {
            return BOOL(1);
        }
        ctx.out.push(WinInfo { hwnd: hwnd.0 as isize, rect });
    }
    BOOL(1)
}

/// Visible top-level windows (topmost first) and popups.
pub fn scan_windows(skip: &[isize]) -> Scan {
    let mut ctx = Ctx { out: Vec::with_capacity(64), popups: Vec::new(), skip: skip.to_vec() };
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut Ctx as isize));
    }
    Scan { windows: ctx.out, popups: ctx.popups }
}

unsafe extern "system" fn mon_proc(hm: HMONITOR, _: HDC, _: *mut RECT, lp: LPARAM) -> BOOL {
    let v = unsafe { &mut *(lp.0 as *mut Vec<Monitor>) };
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    if unsafe { GetMonitorInfoW(hm, &mut mi) }.as_bool() {
        v.push(Monitor { rect: to_rect(mi.rcMonitor), work: to_rect(mi.rcWork) });
    }
    BOOL(1)
}

pub fn monitors() -> Vec<Monitor> {
    let mut v: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(mon_proc), LPARAM(&mut v as *mut _ as isize));
    }
    v
}

pub fn virtual_rect() -> RectI {
    unsafe {
        let l = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let t = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        RectI { l, t, r: l + w, b: t + h }
    }
}

/// True when the foreground window is a borderless full-screen app (game, video, presentation).
pub fn foreground_fullscreen(mons: &[Monitor]) -> bool {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_invalid() {
            return false;
        }
        let cls = class_name(fg);
        if SKIP_CLASSES.contains(&cls.as_str()) {
            return false;
        }
        let style = GetWindowLongW(fg, GWL_STYLE) as u32;
        if style & WS_CAPTION.0 == WS_CAPTION.0 {
            return false;
        }
        let mut r = RECT::default();
        if GetWindowRect(fg, &mut r).is_err() {
            return false;
        }
        let r = to_rect(r);
        mons.iter().any(|m| r.l <= m.rect.l && r.t <= m.rect.t && r.r >= m.rect.r && r.b >= m.rect.b)
    }
}
