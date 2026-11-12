//! macOS: the snowflake in the menu bar with Snowie's menu, autostart through a LaunchAgent,
//! and the settings window as a separate process (like on Windows).

use crate::config::Config;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSMenu, NSMenuItem, NSStatusBar, NSStatusItem, NSVariableStatusItemLength};
use objc2_foundation::NSString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicIsize, Ordering};

pub use crate::icon::snowflake_rgba;

pub const ID_PAUSE: isize = 1;
pub const ID_CLEAR: isize = 2;
pub const ID_INTENSITY: isize = 10; // 10, 11, 12
pub const ID_AUTOSTART: isize = 20;
pub const ID_SETTINGS: isize = 30;
pub const ID_MUSIC: isize = 31;
pub const ID_FIREWORKS: isize = 32;
pub const ID_PLOW: isize = 33;
pub const ID_EXIT: isize = 99;

pub const INTENSITY_PRESETS: [f32; 3] = [0.45, 1.0, 2.3];

/// The last menu item picked (0: none). Read and cleared by the main loop.
static PICKED: AtomicIsize = AtomicIsize::new(0);

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this class has no Drop impl.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "SnowieMenuTarget"]
    struct MenuTarget;

    impl MenuTarget {
        #[unsafe(method(pick:))]
        fn pick(&self, sender: &NSMenuItem) {
            PICKED.store(sender.tag(), Ordering::Relaxed);
        }
    }
);

impl MenuTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

pub struct Tray {
    _item: Retained<NSStatusItem>,
    _target: Retained<MenuTarget>,
    pause: Retained<NSMenuItem>,
    music: Retained<NSMenuItem>,
    autostart: Retained<NSMenuItem>,
    intensity: Vec<Retained<NSMenuItem>>,
}

impl Tray {
    pub fn new(mtm: MainThreadMarker) -> Tray {
        let bar = NSStatusBar::systemStatusBar();
        let item = bar.statusItemWithLength(NSVariableStatusItemLength);
        if let Some(button) = item.button(mtm) {
            button.setTitle(&NSString::from_str("❄"));
            button.setToolTip(Some(&NSString::from_str("Snowie — снег на рабочем столе")));
        }
        let target = MenuTarget::new(mtm);
        let menu = NSMenu::new(mtm);
        let add = |title: &str, tag: isize| -> Retained<NSMenuItem> {
            let it = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(title),
                    Some(sel!(pick:)),
                    &NSString::from_str(""),
                )
            };
            unsafe { it.setTarget(Some(&target as &AnyObject)) };
            it.setTag(tag);
            menu.addItem(&it);
            it
        };
        add("Настройки…", ID_SETTINGS);
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        let pause = add("Пауза", ID_PAUSE);
        let music = add("Музыка", ID_MUSIC);
        let intensity = ["Лёгкий снег", "Обычный снег", "Метель"]
            .iter()
            .enumerate()
            .map(|(i, t)| add(t, ID_INTENSITY + i as isize))
            .collect();
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        add("Убрать снег", ID_CLEAR);
        add("Вызвать снегоуборщик", ID_PLOW);
        add("Запустить салют", ID_FIREWORKS);
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        let autostart = add("Запускать при входе", ID_AUTOSTART);
        add("Выход", ID_EXIT);
        item.setMenu(Some(&menu));
        Tray { _item: item, _target: target, pause, music, autostart, intensity }
    }

    /// Check marks in the menu follow the settings.
    pub fn sync(&self, cfg: &Config) {
        let mark = |it: &NSMenuItem, on: bool| it.setState(if on { 1 } else { 0 });
        mark(&self.pause, cfg.paused);
        mark(&self.music, cfg.music);
        mark(&self.autostart, autostart_enabled());
        for (it, v) in self.intensity.iter().zip(INTENSITY_PRESETS) {
            mark(it, (cfg.intensity - v).abs() < 0.01);
        }
    }

    /// The menu item picked since the last call.
    pub fn take_pick(&self) -> Option<isize> {
        match PICKED.swap(0, Ordering::Relaxed) {
            0 => None,
            id => Some(id),
        }
    }
}

fn launch_agent() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/LaunchAgents/band.tems.snowie.plist"))
}

pub fn autostart_enabled() -> bool {
    launch_agent().is_some_and(|p| p.exists())
}

/// Starts Snowie at login through a LaunchAgent, or removes it.
pub fn set_autostart(on: bool) {
    let Some(path) = launch_agent() else { return };
    if !on {
        let _ = std::fs::remove_file(path);
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let exe = exe.to_string_lossy().replace('&', "&amp;").replace('<', "&lt;");
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>band.tems.snowie</string>
    <key>ProgramArguments</key><array><string>{exe}</string></array>
    <key>RunAtLoad</key><true/>
</dict>
</plist>
"#
    );
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, plist);
}

pub fn open_settings() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg("--settings").spawn();
    }
}
