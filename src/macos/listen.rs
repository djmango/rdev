#![allow(improper_ctypes_definitions)]
use crate::macos::common::*;
use crate::rdev::{Event, ListenError};
use objc2_core_foundation::{CFMachPort, CFRunLoop, kCFRunLoopCommonModes};
use objc2_core_graphics::{
    CGEvent, CGEventTapCallBack, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventTapProxy, CGEventType, kCGEventMaskForAllEvents,
};
use objc2_foundation::NSAutoreleasePool;
use std::os::raw::c_void;
use std::ptr::{null_mut, NonNull};

static mut GLOBAL_CALLBACK: Option<Box<dyn FnMut(Event)>> = None;

#[link(name = "Cocoa", kind = "framework")]
unsafe extern "C" {}

unsafe extern "C-unwind" fn raw_callback(
    _proxy: CGEventTapProxy,
    _type: CGEventType,
    cg_event: NonNull<CGEvent>,
    _user_info: *mut c_void,
) -> *mut CGEvent {
    let opt = KEYBOARD_STATE.lock();
    if let Ok(mut guard) = opt
        && let Some(keyboard) = guard.as_mut() {
            unsafe {
                let events = convert(_type, cg_event, keyboard);
                // Reborrowing the global callback pointer.
                let ptr = &raw mut GLOBAL_CALLBACK;
                if let Some(callback) = &mut *ptr {
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
    T: FnMut(Event) + 'static,
{
    // Compute event mask based on keyboard_only setting
    let event_mask: u64 = if crate::keyboard_only() {
        (1 << CGEventType::KeyDown.0 as u64)
            + (1 << CGEventType::KeyUp.0 as u64)
            + (1 << CGEventType::FlagsChanged.0 as u64)
    } else {
        kCGEventMaskForAllEvents.into()
    };

    unsafe {
        GLOBAL_CALLBACK = Some(Box::new(callback));
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
