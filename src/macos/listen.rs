#![allow(improper_ctypes_definitions)]
use crate::macos::common::*;
use crate::rdev::{Event, ListenError};
use parking_lot::Mutex;
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicPtr, Ordering};
use tracing::{debug, error, warn};

type ListenCallbackType = Mutex<Box<dyn FnMut(Event) + Send>>;

static GLOBAL_CALLBACK: OnceLock<ListenCallbackType> = OnceLock::new();
static EVENT_TAP: AtomicPtr<c_void> = AtomicPtr::new(null_mut());

// Raw FFI declarations - we use raw FFI instead of objc2 bindings because
// the objc2 bindings have issues with event tap enable/disable on modern macOS
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: CGEventTapCallBack,
        user_info: *mut c_void,
    ) -> *mut c_void;

    fn CGEventTapEnable(tap: *mut c_void, enable: bool);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFMachPortCreateRunLoopSource(
        allocator: *const c_void,
        port: *mut c_void,
        order: i64,
    ) -> *mut c_void;

    fn CFRunLoopGetCurrent() -> *mut c_void;
    fn CFRunLoopAddSource(rl: *mut c_void, source: *mut c_void, mode: *const c_void);
    fn CFRunLoopRun();
    fn CFMachPortIsValid(port: *const c_void) -> bool;

    static kCFRunLoopCommonModes: *const c_void;
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

// IOKit HID API for checking Input Monitoring permission
#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOHIDCheckAccess(request_type: u32) -> u32;
}

// IOHIDRequestType
const K_IOHID_REQUEST_TYPE_LISTEN_EVENT: u32 = 1;

// IOHIDAccessType
const K_IOHID_ACCESS_TYPE_GRANTED: u32 = 0;

// CGEventTapLocation
const K_CG_SESSION_EVENT_TAP: u32 = 1;

// CGEventTapPlacement
const K_CG_HEAD_INSERT_EVENT_TAP: u32 = 0;

// CGEventTapOptions
// IMPORTANT: Must use Default (0), not ListenOnly (1) - ListenOnly doesn't work on modern macOS
const K_CG_EVENT_TAP_OPTION_DEFAULT: u32 = 0;

// CGEventType values
const K_CG_EVENT_NULL: u32 = 0;
const K_CG_EVENT_KEY_DOWN: u32 = 10;
const K_CG_EVENT_KEY_UP: u32 = 11;
const K_CG_EVENT_FLAGS_CHANGED: u32 = 12;
const K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFFFFFE;
const K_CG_EVENT_TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFFFFFF;

// Event mask for all events
const K_CG_EVENT_MASK_FOR_ALL_EVENTS: u64 = !0u64;

type CGEventRef = *mut c_void;
type CGEventTapProxy = *mut c_void;
type CGEventTapCallBack = Option<
    unsafe extern "C" fn(
        proxy: CGEventTapProxy,
        event_type: u32,
        event: CGEventRef,
        user_info: *mut c_void,
    ) -> CGEventRef,
>;

// Import objc2 types only for event conversion (convert function uses them)
use objc2_core_graphics::{CGEvent, CGEventType};
use std::ptr::NonNull;

unsafe extern "C" fn raw_callback(
    _proxy: CGEventTapProxy,
    event_type: u32,
    event: CGEventRef,
    _user_info: *mut c_void,
) -> CGEventRef {
    // Handle tap disabled events - re-enable the tap
    if event_type == K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT
        || event_type == K_CG_EVENT_TAP_DISABLED_BY_USER_INPUT
    {
        warn!("Event tap disabled by macOS, re-enabling");
        let tap = EVENT_TAP.load(Ordering::Acquire);
        if !tap.is_null() {
            unsafe { CGEventTapEnable(tap, true) };
        }
        return null_mut();
    }

    // Skip null events
    if event_type == K_CG_EVENT_NULL || event.is_null() {
        return event;
    }

    // Convert raw pointer to objc2 type for event processing
    if let Some(cg_event_ptr) = NonNull::new(event as *mut CGEvent) {
        let cg_event_type = CGEventType(event_type);

        let mut guard = KEYBOARD_STATE.lock();
        if let Some(keyboard) = guard.as_mut() {
            let events = unsafe { convert(cg_event_type, cg_event_ptr, keyboard) };
            drop(guard); // Release lock before calling user callback

            if let Some(callback_mutex) = GLOBAL_CALLBACK.get() {
                let mut callback = callback_mutex.lock();
                for ev in events {
                    callback(ev);
                }
            }
        } else {
            drop(guard);
        }
    }

    event
}

/// Start listening for input events.
///
/// This function blocks the current thread and calls the callback for each event.
/// Only one listener can be active at a time.
///
/// # Permissions Required
/// On macOS, the following permissions are required in System Settings > Privacy & Security:
/// - **Accessibility**: For mouse events and modifier keys
/// - **Input Monitoring**: For keyboard alphanumeric/symbol keys
///
/// # Errors
/// Returns `ListenError::EventTapError` if:
/// - Accessibility permission is not granted
/// - Failed to create the event tap
///
/// Returns `ListenError::AlreadyListening` if listen() was already called.
pub fn listen<T>(callback: T) -> Result<(), ListenError>
where
    T: FnMut(Event) + Send + 'static,
{
    // Compute event mask
    let event_mask: u64 = if crate::keyboard_only() {
        (1u64 << K_CG_EVENT_KEY_DOWN)
            | (1u64 << K_CG_EVENT_KEY_UP)
            | (1u64 << K_CG_EVENT_FLAGS_CHANGED)
    } else {
        K_CG_EVENT_MASK_FOR_ALL_EVENTS
    };

    // Initialize callback - only one listener allowed
    if GLOBAL_CALLBACK.set(Mutex::new(Box::new(callback))).is_err() {
        error!("listen() called multiple times - only one listener allowed");
        return Err(ListenError::AlreadyListening);
    }
    debug!("Callback registered");

    // Check Accessibility permission (required for mouse events and modifier keys)
    let is_trusted = unsafe { AXIsProcessTrusted() };
    if !is_trusted {
        error!(
            "Accessibility permission not granted. \
             Go to: System Settings > Privacy & Security > Accessibility"
        );
        return Err(ListenError::EventTapError);
    }
    debug!("Accessibility permission granted");

    // Check Input Monitoring permission (required for keyboard alphanumeric keys)
    // This API is available on macOS 10.15+
    let input_access = unsafe { IOHIDCheckAccess(K_IOHID_REQUEST_TYPE_LISTEN_EVENT) };
    if input_access != K_IOHID_ACCESS_TYPE_GRANTED {
        error!(
            "Input Monitoring permission not granted. \
             Go to: System Settings > Privacy & Security > Input Monitoring"
        );
        return Err(ListenError::EventTapError);
    }
    debug!("Input Monitoring permission granted");

    unsafe {
        // Create event tap using raw FFI
        // IMPORTANT: Use kCGEventTapOptionDefault (0), not ListenOnly (1)
        // ListenOnly doesn't work correctly on modern macOS (Monterey+)
        let tap = CGEventTapCreate(
            K_CG_SESSION_EVENT_TAP,
            K_CG_HEAD_INSERT_EVENT_TAP,
            K_CG_EVENT_TAP_OPTION_DEFAULT,
            event_mask,
            Some(raw_callback),
            null_mut(),
        );

        if tap.is_null() {
            error!("Failed to create event tap - check Accessibility permissions");
            return Err(ListenError::EventTapError);
        }

        if !CFMachPortIsValid(tap) {
            error!("Event tap mach port is invalid");
            return Err(ListenError::EventTapError);
        }
        debug!("Event tap created successfully");

        // Store tap pointer for re-enabling in callback if macOS disables it
        EVENT_TAP.store(tap, Ordering::Release);

        // Create run loop source from the mach port
        let source = CFMachPortCreateRunLoopSource(null(), tap, 0);
        if source.is_null() {
            error!("Failed to create run loop source");
            return Err(ListenError::LoopSourceError);
        }

        // Add source to current run loop
        let run_loop = CFRunLoopGetCurrent();
        CFRunLoopAddSource(run_loop, source, kCFRunLoopCommonModes);

        // Enable the event tap
        CGEventTapEnable(tap, true);
        debug!("Event tap enabled, starting run loop");

        // Run the event loop - this blocks until CFRunLoopStop is called
        CFRunLoopRun();
    }

    Ok(())
}
