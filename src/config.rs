//! Settings shared between the overlay and the settings window (`%APPDATA%\Snowie\config.json`
//! on Windows, `~/Library/Application Support/Snowie/config.json` on macOS), plus the small
//! command channel the settings window uses to poke the running overlay.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub paused: bool,
    /// snowfall density multiplier
    pub intensity: f32,
    pub flake_size: f32,
    pub wind: f32,
    /// how fast drifts grow
    pub accumulation: f32,
    pub snow_on_windows: bool,
    /// max drift height on windows / on the taskbar, px at 100% scale
    pub window_drift: f32,
    pub floor_drift: f32,
    pub fps: u32,
    pub hide_fullscreen: bool,
    /// no snow over games, in a window too (found automatically)
    pub hide_over_games: bool,
    /// programs that never get snow over their windows: exe or app names, e.g. "notepad.exe"
    pub no_snow_apps: Vec<String>,
    pub music: bool,
    pub volume: f32,
    /// sounds when playing with snow, fireworks, the snowplow
    pub sounds: bool,
    pub sound_volume: f32,
    pub plow: bool,
    pub plow_minutes: f32,
    pub fireworks_box: bool,
    /// box position along the primary taskbar, 0..1
    pub box_pos: f32,
    pub countdown: bool,
    pub new_year_show: bool,
    /// extra fireworks shows, "HH:MM" every day
    pub timers: Vec<String>,
    /// on-screen profiler HUD
    pub profiler: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            paused: false,
            intensity: 1.0,
            flake_size: 1.0,
            wind: 1.0,
            accumulation: 1.0,
            snow_on_windows: true,
            window_drift: 28.0,
            floor_drift: 60.0,
            fps: 60,
            hide_fullscreen: true,
            hide_over_games: true,
            no_snow_apps: Vec::new(),
            music: true,
            volume: 0.35,
            sounds: true,
            sound_volume: 0.6,
            plow: true,
            plow_minutes: 10.0,
            fireworks_box: true,
            box_pos: 0.82,
            countdown: true,
            new_year_show: true,
            timers: Vec::new(),
            profiler: false,
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        dir().join("config.json")
    }

    pub fn load() -> Config {
        std::fs::read_to_string(Self::path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(tmp, path)
    }
}

/// Parses "HH:MM".
pub fn parse_timer(s: &str) -> Option<(u32, u32)> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m) = (h.trim().parse::<u32>().ok()?, m.trim().parse::<u32>().ok()?);
    (h < 24 && m < 60).then_some((h, m))
}

/// Where Snowie keeps its files.
pub fn dir() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    #[cfg(not(windows))]
    let base = std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join("Library").join("Application Support"))
        .unwrap_or_else(std::env::temp_dir);
    base.join("Snowie")
}

#[cfg(windows)]
pub const WM_SNOWIE_CMD: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 2;
pub const CMD_RELOAD: usize = 1;
pub const CMD_CLEAR: usize = 2;
pub const CMD_PLOW: usize = 3;
pub const CMD_FIREWORKS: usize = 4;
pub const CMD_EXIT: usize = 5;

/// Sends a command to the running overlay. Returns false if Snowie is not running.
#[cfg(windows)]
pub fn send(cmd: usize) -> bool {
    use windows::core::w;
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};
    unsafe {
        match FindWindowW(w!("SnowieWindow"), w!("Snowie tray")) {
            Ok(h) if !h.is_invalid() => PostMessageW(Some(h), WM_SNOWIE_CMD, WPARAM(cmd), LPARAM(0)).is_ok(),
            _ => false,
        }
    }
}

#[cfg(windows)]
pub fn overlay_running() -> bool {
    use windows::core::w;
    use windows::Win32::UI::WindowsAndMessaging::FindWindowW;
    unsafe { FindWindowW(w!("SnowieWindow"), w!("Snowie tray")).is_ok_and(|h| !h.is_invalid()) }
}

// macOS: the overlay writes its pid to `overlay.pid`; commands are appended to `commands`,
// which the overlay reads and clears a few times a second.

#[cfg(not(windows))]
pub fn pid_file() -> PathBuf {
    dir().join("overlay.pid")
}

#[cfg(not(windows))]
pub fn commands_file() -> PathBuf {
    dir().join("commands")
}

#[cfg(not(windows))]
pub fn send(cmd: usize) -> bool {
    use std::io::Write;
    if !overlay_running() {
        return false;
    }
    let ok = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(commands_file())
        .and_then(|mut f| writeln!(f, "{cmd}"));
    ok.is_ok()
}

#[cfg(not(windows))]
pub fn overlay_running() -> bool {
    let Some(pid) = std::fs::read_to_string(pid_file()).ok().and_then(|s| s.trim().parse::<i32>().ok()) else {
        return false;
    };
    // signal 0 only checks that the process exists
    pid > 0 && unsafe { libc::kill(pid, 0) } == 0
}

/// Takes the commands sent since the last call.
#[cfg(not(windows))]
pub fn take_commands() -> Vec<usize> {
    let path = commands_file();
    let Ok(text) = std::fs::read_to_string(&path) else { return Vec::new() };
    let _ = std::fs::remove_file(&path);
    text.lines().filter_map(|l| l.trim().parse().ok()).collect()
}
