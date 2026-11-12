//! Global low-level mouse hook on its own thread. The overlay itself is click-through;
//! the hook swallows a left click only when it lands on snow, and then feeds the
//! drag to the simulation until the button is released.

use crate::geom::V2;
use crate::world::{Input, World};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    GetCurrentThreadId, GetThreadTimes, OpenThread, THREAD_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::*;

pub static WORLD: OnceLock<Mutex<World>> = OnceLock::new();
pub static INPUT: Mutex<Vec<Input>> = Mutex::new(Vec::new());
static GRABBING: AtomicBool = AtomicBool::new(false);
/// mouse events seen by the hook (profiler)
pub static EVENTS: AtomicU64 = AtomicU64::new(0);
static THREAD_ID: AtomicU32 = AtomicU32::new(0);

/// CPU seconds spent by the hook thread so far (profiler).
pub fn thread_cpu_seconds() -> f64 {
    let tid = THREAD_ID.load(Ordering::Relaxed);
    if tid == 0 {
        return 0.0;
    }
    unsafe {
        let Ok(h) = OpenThread(THREAD_QUERY_LIMITED_INFORMATION, false, tid) else { return 0.0 };
        let (mut c, mut e, mut k, mut u) =
            (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let _ = GetThreadTimes(h, &mut c, &mut e, &mut k, &mut u);
        let _ = CloseHandle(h);
        let t = |f: FILETIME| ((f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64) as f64 / 1e7;
        t(k) + t(u)
    }
}

/// While snow is held, also sample the cursor every frame: some input sources
/// (pens, remote desktop, SetCursorPos) move the cursor without hook events.
pub fn poll_cursor() {
    if !GRABBING.load(Ordering::SeqCst) {
        return;
    }
    let mut p = POINT::default();
    if unsafe { GetCursorPos(&mut p) }.is_ok() {
        push(Input::Move(V2(p.x as f32, p.y as f32)));
    }
}

pub fn take_inputs() -> Vec<Input> {
    std::mem::take(&mut *INPUT.lock().unwrap())
}

fn push(i: Input) {
    if let Ok(mut q) = INPUT.lock() {
        q.push(i);
    }
}

unsafe extern "system" fn proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        EVENTS.fetch_add(1, Ordering::Relaxed);
        let ms = unsafe { &*(lp.0 as *const MSLLHOOKSTRUCT) };
        let p = V2(ms.pt.x as f32, ms.pt.y as f32);
        match wp.0 as u32 {
            WM_LBUTTONDOWN => {
                if GRABBING.swap(false, Ordering::SeqCst) {
                    // we missed a button-up (e.g. secure desktop); finish the old grab
                    push(Input::Up(p));
                } else if let Some(w) = WORLD.get() {
                    let grabbed = w.lock().map(|mut w| w.try_grab(p)).unwrap_or(false);
                    if grabbed {
                        GRABBING.store(true, Ordering::SeqCst);
                        return LRESULT(1);
                    }
                }
            }
            WM_LBUTTONUP => {
                if GRABBING.swap(false, Ordering::SeqCst) {
                    push(Input::Up(p));
                    return LRESULT(1);
                }
            }
            WM_MOUSEMOVE if GRABBING.load(Ordering::SeqCst) => {
                push(Input::Move(p));
            }
            _ => {}
        }
    }
    unsafe { CallNextHookEx(None, code, wp, lp) }
}

pub fn spawn() {
    std::thread::spawn(|| unsafe {
        THREAD_ID.store(GetCurrentThreadId(), Ordering::Relaxed);
        let hinst = GetModuleHandleW(None).ok();
        let Ok(hook) = SetWindowsHookExW(WH_MOUSE_LL, Some(proc), hinst.map(|h| HINSTANCE(h.0)), 0) else {
            return;
        };
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = UnhookWindowsHookEx(hook);
    });
}
