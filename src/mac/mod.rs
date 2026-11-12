//! macOS version: the same simulation, sounds and settings window as on Windows, with its own
//! overlay windows, window list, mouse handling, menu bar item and GPU renderer.

mod app;
mod render;
pub mod scan;
pub mod tray;

pub use app::main;
