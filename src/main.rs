#![cfg_attr(windows, windows_subsystem = "windows")]

mod audio;
mod config;
mod fx;
mod geom;
mod icon;
mod music;
mod settings;
mod sound;
mod world;

#[cfg(windows)]
mod hook;
#[cfg(windows)]
mod profiler;
#[cfg(windows)]
mod render;
#[cfg(windows)]
mod scan;
#[cfg(windows)]
mod tray;
#[cfg(windows)]
mod win;

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
use mac::{scan, tray};

fn main() {
    #[cfg(windows)]
    win::main();
    #[cfg(target_os = "macos")]
    mac::main();
}
