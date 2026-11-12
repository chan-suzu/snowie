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
    /// the process that owns the window
    pub pid: u32,
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
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        ctx.out.push(WinInfo { hwnd: hwnd.0 as isize, rect, pid });
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

/// True when a game or another full-screen app is in front: Windows itself says so (exclusive
/// Direct3D full screen, presentation mode, a full-screen app that wants no interruptions), or
/// the foreground window covers a whole monitor, taskbar included. Many games keep the title
/// bar style bits in "borderless window" mode, so the style is not checked; a maximized window
/// is not full screen even when an auto-hidden taskbar lets it cover the monitor.
pub fn foreground_fullscreen(mons: &[Monitor]) -> bool {
    use windows::Win32::UI::Shell::{
        SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
    };
    unsafe {
        if let Ok(state) = SHQueryUserNotificationState() {
            if state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE || state == QUNS_BUSY {
                return true;
            }
        }
        let fg = GetForegroundWindow();
        if fg.is_invalid() {
            return false;
        }
        let root = GetAncestor(fg, GA_ROOT);
        let fg = if root.is_invalid() { fg } else { root };
        let cls = class_name(fg);
        if SKIP_CLASSES.contains(&cls.as_str()) {
            return false;
        }
        if IsZoomed(fg).as_bool() {
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

// ---------------------------------------------------------------- games

/// Libraries that only games load: store and launcher SDKs, the Steam overlay, game engines,
/// video codecs and physics made for games, old XInput.
const GAME_MODULES: &[&str] = &[
    "steam_api.dll",
    "steam_api64.dll",
    "gameoverlayrenderer.dll",
    "gameoverlayrenderer64.dll",
    "unityplayer.dll",
    "gameassembly.dll",
    "eossdk-win32-shipping.dll",
    "eossdk-win64-shipping.dll",
    "galaxy.dll",
    "galaxy64.dll",
    "binkw32.dll",
    "binkw64.dll",
    "bink2w32.dll",
    "bink2w64.dll",
    "physx3_x64.dll",
    "physx3common_x64.dll",
    "nvngx_dlss.dll",
    "xinput1_3.dll",
    "xinput9_1_0.dll",
];

/// Folders where launchers install games.
const GAME_DIRS: &[&str] = &[
    "\\steamapps\\common\\",
    "\\epic games\\",
    "\\gog galaxy\\games\\",
    "\\riot games\\",
    "\\xboxgames\\",
    "\\ea games\\",
    "\\ubisoft game launcher\\games\\",
    "\\battle.net\\",
];

/// The executable path (lower case) and loaded module names of a process.
fn process_info(pid: u32) -> Option<(String, Vec<String>)> {
    use windows::Win32::System::ProcessStatus::{EnumProcessModulesEx, GetModuleBaseNameW, LIST_MODULES_ALL};
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
    };
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let path =
            if QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len)
                .is_ok()
            {
                String::from_utf16_lossy(&buf[..len as usize]).to_lowercase()
            } else {
                String::new()
            };
        let mut mods = vec![HMODULE::default(); 1024];
        let mut needed = 0u32;
        let mut names = Vec::new();
        let size = (mods.len() * std::mem::size_of::<HMODULE>()) as u32;
        if EnumProcessModulesEx(h, mods.as_mut_ptr(), size, &mut needed, LIST_MODULES_ALL).is_ok() {
            let n = (needed as usize / std::mem::size_of::<HMODULE>()).min(mods.len());
            for m in &mods[..n] {
                let mut name = [0u16; 260];
                let l = GetModuleBaseNameW(h, Some(*m), &mut name) as usize;
                if l > 0 {
                    names.push(String::from_utf16_lossy(&name[..l]).to_lowercase());
                }
            }
        }
        let _ = CloseHandle(h);
        Some((path, names))
    }
}

/// Executables Windows itself knows as games (Game Bar and Game Mode keep this list).
fn known_games() -> Vec<String> {
    use windows::Win32::System::Registry::*;
    let mut out = Vec::new();
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            windows::core::w!("System\\GameConfigStore\\Children"),
            Some(0),
            KEY_READ,
            &mut key,
        )
        .is_err()
        {
            return out;
        }
        for i in 0.. {
            let mut name = [0u16; 256];
            let mut len = name.len() as u32;
            if RegEnumKeyExW(key, i, Some(windows::core::PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None)
                .is_err()
            {
                break;
            }
            let mut val = [0u16; 1024];
            let mut bytes = (val.len() * 2) as u32;
            let ok = RegGetValueW(
                key,
                windows::core::PCWSTR(name.as_ptr()),
                windows::core::w!("MatchedExeFullPath"),
                RRF_RT_REG_SZ,
                None,
                Some(val.as_mut_ptr() as *mut _),
                Some(&mut bytes),
            )
            .is_ok();
            if ok {
                let n = (bytes as usize / 2).saturating_sub(1).min(val.len());
                let p = String::from_utf16_lossy(&val[..n]).to_lowercase();
                if !p.is_empty() {
                    out.push(p);
                }
            }
        }
        let _ = RegCloseKey(key);
    }
    out
}

/// Debug (`snowie.exe --games`): every visible window with its program and whether it counts
/// as a game, one per line.
pub fn describe_games() -> String {
    let known = known_games();
    let mut out = String::new();
    for w in scan_windows(&[]).windows {
        let Some((path, mods)) = process_info(w.pid) else {
            out += &format!(
                "pid {:>6}  (no access)
",
                w.pid
            );
            continue;
        };
        let mut why = Vec::new();
        if known.contains(&path) {
            why.push("known to Windows as a game".to_string());
        }
        if let Some(d) = GAME_DIRS.iter().find(|d| path.contains(*d)) {
            why.push(format!("installed in {d}"));
        }
        if path.ends_with("-shipping.exe") {
            why.push("Unreal build".into());
        }
        why.extend(mods.iter().filter(|m| GAME_MODULES.contains(&m.as_str())).map(|m| format!("loads {m}")));
        let verdict = if why.is_empty() { "-".to_string() } else { format!("GAME: {}", why.join(", ")) };
        out += &format!(
            "pid {:>6}  {:?}  {}  {}
",
            w.pid, w.rect, path, verdict
        );
    }
    out
}

/// Tells which windows belong to games (or to programs on the user's no-snow list), caching
/// the answer per process: a process that does not look like a game yet is looked at again
/// after a while, since games load their libraries a little after the window appears.
pub struct Games {
    cache: std::collections::HashMap<u32, (bool, std::time::Instant)>,
    known: Vec<String>,
    known_at: std::time::Instant,
}

impl Games {
    pub fn new() -> Games {
        Games { cache: Default::default(), known: known_games(), known_at: std::time::Instant::now() }
    }

    /// Splits off the windows that must stay free of snow.
    pub fn split(&mut self, windows: Vec<WinInfo>, auto: bool, apps: &[String]) -> (Vec<WinInfo>, Vec<RectI>) {
        use std::time::{Duration, Instant};
        if !auto && apps.is_empty() {
            return (windows, Vec::new());
        }
        if self.known_at.elapsed() > Duration::from_secs(300) {
            self.known = known_games();
            self.known_at = Instant::now();
            self.cache.clear();
        }
        let own = std::process::id();
        let mut keep = Vec::with_capacity(windows.len());
        let mut shields = Vec::new();
        for w in windows {
            if w.pid == own || w.pid == 0 {
                keep.push(w);
                continue;
            }
            let fresh = match self.cache.get(&w.pid) {
                Some(&(game, at)) => game || at.elapsed() < Duration::from_secs(15),
                None => false,
            };
            if !fresh {
                let game = process_info(w.pid).is_some_and(|(path, mods)| {
                    let file = path.rsplit('\\').next().unwrap_or("").to_string();
                    let listed = apps
                        .iter()
                        .any(|a| !a.is_empty() && (file == *a || file.trim_end_matches(".exe") == a.as_str()));
                    let auto_game = auto
                        && (self.known.contains(&path)
                            || GAME_DIRS.iter().any(|d| path.contains(d))
                            || path.ends_with("-shipping.exe")
                            || mods.iter().any(|m| GAME_MODULES.contains(&m.as_str())));
                    listed || auto_game
                });
                self.cache.insert(w.pid, (game, Instant::now()));
            }
            if self.cache.get(&w.pid).is_some_and(|c| c.0) {
                shields.push(w.rect);
            } else {
                keep.push(w);
            }
        }
        self.cache.retain(|_, c| c.0 || c.1.elapsed() < Duration::from_secs(60));
        (keep, shields)
    }
}
