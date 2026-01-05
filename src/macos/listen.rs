#![allow(improper_ctypes_definitions)]
use crate::macos::common::{convert, KEYBOARD_STATE};
use crate::rdev::{Event, ListenError};
use crossbeam_channel::{Sender, unbounded};
use objc2_core_graphics::{CGEvent, CGEventType};
use std::ffi::c_void;
use std::ptr::{null, null_mut, NonNull};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicPtr, Ordering};
use tracing::{debug, error, warn};

static EVENT_SENDER: OnceLock<Sender<Event>> = OnceLock::new();
static EVENT_TAP: AtomicPtr<c_void> = AtomicPtr::new(null_mut());

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(tap: u32, place: u32, options: u32, mask: u64, cb: CGEventTapCallBack, info: *mut c_void) -> *mut c_void;
    fn CGEventTapEnable(tap: *mut c_void, enable: bool);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFMachPortCreateRunLoopSource(alloc: *const c_void, port: *mut c_void, order: i64) -> *mut c_void;
    fn CFRunLoopGetCurrent() -> *mut c_void;
    fn CFRunLoopAddSource(rl: *mut c_void, src: *mut c_void, mode: *const c_void);
    fn CFRunLoopRun();
    fn CFMachPortIsValid(port: *const c_void) -> bool;
    static kCFRunLoopCommonModes: *const c_void;
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" { fn AXIsProcessTrusted() -> bool; }

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" { fn IOHIDCheckAccess(req: u32) -> u32; }

type CGEventTapCallBack = Option<unsafe extern "C" fn(*mut c_void, u32, *mut c_void, *mut c_void) -> *mut c_void>;

const TAP_DISABLED_TIMEOUT: u32 = 0xFFFFFFFE;
const TAP_DISABLED_USER: u32 = 0xFFFFFFFF;

unsafe extern "C" fn tap_callback(_: *mut c_void, event_type: u32, event: *mut c_void, _: *mut c_void) -> *mut c_void {
    // Re-enable if macOS disabled the tap
    if event_type == TAP_DISABLED_TIMEOUT || event_type == TAP_DISABLED_USER {
        warn!("Event tap disabled, re-enabling");
        let tap = EVENT_TAP.load(Ordering::Acquire);
        if !tap.is_null() { unsafe { CGEventTapEnable(tap, true); } }
        return null_mut();
    }

    if event_type == 0 || event.is_null() { return event; }

    let sender = match EVENT_SENDER.get() { Some(s) => s, None => return event };

    if let Some(cg_event) = NonNull::new(event as *mut CGEvent) {
        let mut guard = KEYBOARD_STATE.lock();
        if let Some(kb) = guard.as_mut() {
            for ev in unsafe { convert(CGEventType(event_type), cg_event, kb) } {
                let _ = sender.send(ev);
            }
        }
    }
    event
}

pub fn listen<T: FnMut(Event) + Send + 'static>(mut user_callback: T) -> Result<(), ListenError> {
    let mask: u64 = if crate::keyboard_only() { (1 << 10) | (1 << 11) | (1 << 12) } else { !0 };

    let (tx, rx) = unbounded();
    if EVENT_SENDER.set(tx).is_err() { return Err(ListenError::AlreadyListening); }

    std::thread::spawn(move || { while let Ok(ev) = rx.recv() { user_callback(ev); } });

    if !unsafe { AXIsProcessTrusted() } {
        error!("Accessibility permission not granted");
        return Err(ListenError::EventTapError);
    }
    if unsafe { IOHIDCheckAccess(1) } != 0 {
        error!("Input Monitoring permission not granted");
        return Err(ListenError::EventTapError);
    }

    unsafe {
        // ListenOnly (1) = passive, never blocks input
        let tap = CGEventTapCreate(1, 0, 1, mask, Some(tap_callback), null_mut());
        if tap.is_null() || !CFMachPortIsValid(tap) { return Err(ListenError::EventTapError); }
        EVENT_TAP.store(tap, Ordering::Release);

        let src = CFMachPortCreateRunLoopSource(null(), tap, 0);
        if src.is_null() { return Err(ListenError::LoopSourceError); }

        CFRunLoopAddSource(CFRunLoopGetCurrent(), src, kCFRunLoopCommonModes);
        CGEventTapEnable(tap, true);
        debug!("Event tap started (ListenOnly mode)");
        CFRunLoopRun();
    }
    Ok(())
}
