#![allow(improper_ctypes_definitions)]
use crate::macos::common::*;
use crate::rdev::{Event, GrabError};
use objc2_core_foundation::{CFMachPort, CFRetained, CFRunLoop, kCFRunLoopCommonModes};
use objc2_core_graphics::{
    CGEvent, CGEventTapCallBack, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventTapProxy, CGEventType, kCGEventMaskForAllEvents,
};
use objc2_foundation::NSAutoreleasePool;
use std::os::raw::c_void;
use std::ptr::{null_mut, NonNull};
use std::sync::atomic::{AtomicBool, Ordering};

static mut GLOBAL_CALLBACK: Option<Box<dyn FnMut(Event) -> Option<Event>>> = None;
static IS_GRABBED: AtomicBool = AtomicBool::new(false);
static mut CURRENT_LOOP: Option<CFRetained<CFRunLoop>> = None;

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
                    // If any event is blocked (callback returns None), block the original event
                    let mut should_block = false;
                    for event in events {
                        if callback(event).is_none() {
                            should_block = true;
                        }
                    }
                    if should_block {
                        CGEvent::set_type(Some(cg_event.as_ref()), CGEventType::Null);
                    }
                }
            }
        }
    cg_event.as_ptr()
}

#[inline]
pub fn is_grabbed() -> bool {
    IS_GRABBED.load(Ordering::SeqCst)
}

pub fn grab<T>(callback: T) -> Result<(), GrabError>
where
    T: FnMut(Event) -> Option<Event> + 'static,
{
    if is_grabbed() {
        return Ok(());
    }

    unsafe {
        GLOBAL_CALLBACK = Some(Box::new(callback));
        let _pool = NSAutoreleasePool::new();
        let tap_callback: CGEventTapCallBack = Some(raw_callback);
        let tap = CGEvent::tap_create(
            CGEventTapLocation::SessionEventTap, // Session for grab (can intercept)
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::Default,
            kCGEventMaskForAllEvents.into(),
            tap_callback,
            null_mut(),
        )
        .ok_or(GrabError::EventTapError)?;

        let loop_source = CFMachPort::new_run_loop_source(None, Some(&tap), 0)
            .ok_or(GrabError::LoopSourceError)?;

        let current_loop = CFRunLoop::current().ok_or(GrabError::LoopSourceError)?;
        current_loop.add_source(Some(&loop_source), kCFRunLoopCommonModes);
        CURRENT_LOOP = Some(current_loop);

        CGEvent::tap_enable(&tap, true);
        IS_GRABBED.store(true, Ordering::SeqCst);
        CFRunLoop::run();
    }
    Ok(())
}

pub fn exit_grab() -> Result<(), GrabError> {
    unsafe {
        if let Some(ref current_loop) = CURRENT_LOOP {
            current_loop.stop();
        }
        CURRENT_LOOP = None;
        IS_GRABBED.store(false, Ordering::SeqCst);
    }
    Ok(())
}
