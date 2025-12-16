#![allow(improper_ctypes_definitions)]
use crate::macos::common::*;
use crate::rdev::{Event, ListenError};
use objc2_core_foundation::{CFMachPort, CFRunLoop, kCFRunLoopCommonModes};
use objc2_core_graphics::{
    CGEvent, CGEventTapCallBack, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventTapProxy, CGEventType, kCGEventMaskForAllEvents,
};
use objc2_foundation::NSAutoreleasePool;
use parking_lot::Mutex;
use std::os::raw::c_void;
use std::ptr::{NonNull, null_mut};
use std::sync::OnceLock;

type ListenCallbackType = Mutex<Box<dyn FnMut(Event) + Send>>;

static GLOBAL_CALLBACK: OnceLock<ListenCallbackType> = OnceLock::new();

#[link(name = "Cocoa", kind = "framework")]
unsafe extern "C" {}

unsafe extern "C-unwind" fn raw_callback(
    _proxy: CGEventTapProxy,
    _type: CGEventType,
    cg_event: NonNull<CGEvent>,
    _user_info: *mut c_void,
) -> *mut CGEvent {
    // parking_lot::Mutex never poisons, so we can safely lock
    let mut guard = KEYBOARD_STATE.lock();
    if let Some(keyboard) = guard.as_mut() {
        unsafe {
            let events = convert(_type, cg_event, keyboard);
            // Get callback from OnceLock and lock it
            if let Some(callback_mutex) = GLOBAL_CALLBACK.get() {
                let mut callback = callback_mutex.lock();
                for event in events {
                    callback(event);
                }
            }
        }
    }
    cg_event.as_ptr()
}

pub fn listen<T>(callback: T) -> Result<(), ListenError>
where
    T: FnMut(Event) + Send + 'static,
{
    // Compute event mask based on keyboard_only setting
    let event_mask: u64 = if crate::keyboard_only() {
        (1 << CGEventType::KeyDown.0 as u64)
            + (1 << CGEventType::KeyUp.0 as u64)
            + (1 << CGEventType::FlagsChanged.0 as u64)
    } else {
        kCGEventMaskForAllEvents.into()
    };

    // Initialize callback in OnceLock
    // If already set, this returns Err, meaning listen was called multiple times
    if GLOBAL_CALLBACK.set(Mutex::new(Box::new(callback))).is_err() {
        log::warn!("listen() called multiple times, ignoring previous callback");
    }

    unsafe {
        let _pool = NSAutoreleasePool::new();
        let tap_callback: CGEventTapCallBack = Some(raw_callback);
        let tap = CGEvent::tap_create(
            CGEventTapLocation::SessionEventTap,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::ListenOnly,
            event_mask,
            tap_callback,
            null_mut(),
        )
        .ok_or(ListenError::EventTapError)?;

        let loop_source = CFMachPort::new_run_loop_source(None, Some(&tap), 0)
            .ok_or(ListenError::LoopSourceError)?;

        let current_loop = CFRunLoop::current().ok_or(ListenError::LoopSourceError)?;
        current_loop.add_source(Some(&loop_source), kCFRunLoopCommonModes);

        CGEvent::tap_enable(&tap, true);
        CFRunLoop::run();
    }
    Ok(())
}
