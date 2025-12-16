#![allow(improper_ctypes_definitions)]
use crate::macos::common::*;
use crate::rdev::{Event, GrabError};
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
use std::sync::atomic::{AtomicBool, Ordering};

type GrabCallbackType = Mutex<Box<dyn FnMut(Event) -> Option<Event> + Send>>;

static GLOBAL_CALLBACK: OnceLock<GrabCallbackType> = OnceLock::new();
static IS_GRABBED: AtomicBool = AtomicBool::new(false);
// CFRunLoop is not Send/Sync, don't store it - exit_grab won't be able to stop it from another thread anyway
// Users must call exit_grab from the same thread that called grab

#[link(name = "Cocoa", kind = "framework")]
unsafe extern "C" {}

unsafe extern "C-unwind" fn raw_callback(
    _proxy: CGEventTapProxy,
    _type: CGEventType,
    cg_event: NonNull<CGEvent>,
    _user_info: *mut c_void,
) -> *mut CGEvent {
    // parking_lot::Mutex never poisons
    let mut guard = KEYBOARD_STATE.lock();
    if let Some(keyboard) = guard.as_mut() {
        unsafe {
            let events = convert(_type, cg_event, keyboard);
            // Get callback from OnceLock
            if let Some(callback_mutex) = GLOBAL_CALLBACK.get() {
                let mut callback = callback_mutex.lock();
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
    T: FnMut(Event) -> Option<Event> + Send + 'static,
{
    if is_grabbed() {
        return Ok(());
    }

    // Initialize callback in OnceLock
    if GLOBAL_CALLBACK.set(Mutex::new(Box::new(callback))).is_err() {
        log::warn!("grab() called multiple times, ignoring previous callback");
    }

    unsafe {
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

        CGEvent::tap_enable(&tap, true);
        IS_GRABBED.store(true, Ordering::SeqCst);
        CFRunLoop::run();
    }
    Ok(())
}

pub fn exit_grab() -> Result<(), GrabError> {
    IS_GRABBED.store(false, Ordering::SeqCst);
    // Stop the current run loop (must be called from the same thread as grab())
    if let Some(current_loop) = CFRunLoop::current() {
        current_loop.stop();
    }
    Ok(())
}
