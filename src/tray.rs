//! Notification-area icon with a small menu, plus "start with Windows".

use crate::config::Config;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const WM_TRAY: u32 = WM_APP + 1;

pub const ID_PAUSE: u32 = 1;
pub const ID_CLEAR: u32 = 2;
pub const ID_INTENSITY: u32 = 10; // 10, 11, 12
pub const ID_AUTOSTART: u32 = 20;
pub const ID_SETTINGS: u32 = 30;
pub const ID_MUSIC: u32 = 31;
pub const ID_FIREWORKS: u32 = 32;
pub const ID_PLOW: u32 = 33;
pub const ID_PROFILER: u32 = 34;
pub const ID_EXIT: u32 = 99;

pub const INTENSITY_PRESETS: [f32; 3] = [0.45, 1.0, 2.3];

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE: PCWSTR = w!("Snowie");

pub use crate::icon::snowflake_rgba;

fn make_icon() -> Result<HICON> {
    const N: usize = 32;
    let rgba = snowflake_rgba(N);
    let mut bgra = vec![0u8; N * N * 4];
    for (d, s) in bgra.as_chunks_mut::<4>().0.iter_mut().zip(rgba.as_chunks::<4>().0.iter()) {
        let a = s[3] as u32;
        d[0] = (s[2] as u32 * a / 255) as u8;
        d[1] = (s[1] as u32 * a / 255) as u8;
        d[2] = (s[0] as u32 * a / 255) as u8;
        d[3] = s[3];
    }
    let and = [0u8; N * N / 8];
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        CreateIcon(Some(hinst.into()), N as i32, N as i32, 1, 32, and.as_ptr(), bgra.as_ptr())
    }
}

fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    };
    let tip: Vec<u16> = "Snowie — снег на рабочем столе".encode_utf16().collect();
    nid.szTip[..tip.len()].copy_from_slice(&tip);
    nid
}

pub fn add(hwnd: HWND) {
    unsafe {
        let mut nid = icon_data(hwnd);
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY;
        nid.hIcon = make_icon().unwrap_or_else(|_| LoadIconW(None, IDI_APPLICATION).unwrap_or_default());
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
    }
}

pub fn remove(hwnd: HWND) {
    unsafe {
        let nid = icon_data(hwnd);
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

pub fn autostart_enabled() -> bool {
    unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, RRF_RT_REG_SZ, None, None, None).is_ok() }
}

pub fn set_autostart(on: bool) {
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, None, KEY_SET_VALUE, &mut key).is_err() {
            return;
        }
        if on {
            if let Ok(exe) = std::env::current_exe() {
                let s = format!("\"{}\"", exe.display());
                let wide: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
                let bytes = std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2);
                let _ = RegSetValueExW(key, RUN_VALUE, None, REG_SZ, Some(bytes));
            }
        } else {
            let _ = RegDeleteValueW(key, RUN_VALUE);
        }
        let _ = RegCloseKey(key);
    }
}

pub fn open_settings() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg("--settings").spawn();
    }
}

/// Shows the context menu and returns the chosen command id (0 if none).
pub fn show_menu(hwnd: HWND, cfg: &Config) -> u32 {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return 0 };
        let check = |on: bool| if on { MF_CHECKED } else { MF_UNCHECKED };
        let _ = AppendMenuW(menu, MF_STRING, ID_SETTINGS as usize, w!("Настройки…"));
        let _ = SetMenuDefaultItem(menu, ID_SETTINGS, 0);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING | check(cfg.paused), ID_PAUSE as usize, w!("Пауза"));
        let _ = AppendMenuW(menu, MF_STRING | check(cfg.music), ID_MUSIC as usize, w!("Музыка"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let names = [w!("Снегопад: лёгкий"), w!("Снегопад: обычный"), w!("Снегопад: метель")];
        let cur = INTENSITY_PRESETS
            .iter()
            .enumerate()
            .min_by(|a, b| (a.1 - cfg.intensity).abs().total_cmp(&(b.1 - cfg.intensity).abs()))
            .map(|x| x.0)
            .unwrap_or(1);
        for (i, n) in names.iter().enumerate() {
            let _ = AppendMenuW(menu, MF_STRING | check(i == cur), ID_INTENSITY as usize + i, *n);
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, ID_FIREWORKS as usize, w!("Салют!"));
        let _ = AppendMenuW(menu, MF_STRING, ID_PLOW as usize, w!("Вызвать снегоуборщик"));
        let _ = AppendMenuW(menu, MF_STRING, ID_CLEAR as usize, w!("Убрать снег"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING | check(cfg.profiler), ID_PROFILER as usize, w!("Профайлер"));
        let _ = AppendMenuW(
            menu,
            MF_STRING | check(autostart_enabled()),
            ID_AUTOSTART as usize,
            w!("Запускать вместе с Windows"),
        );
        let _ = AppendMenuW(menu, MF_STRING, ID_EXIT as usize, w!("Выход"));

        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, p.x, p.y, None, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        cmd.0 as u32
    }
}
