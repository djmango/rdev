use crate::keycodes::macos::{code_from_key, virtual_keycodes::*};
use crate::rdev::{Button, EventType, RawKey, SimulateError};
use objc2_core_foundation::{CFRetained, CGPoint};
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventFlags, CGEventSource, CGEventSourceStateID, CGEventTapLocation,
    CGEventType, CGKeyCode, CGMouseButton, CGScrollEventUnit,
};

use crate::macos::common::LAST_FLAGS;

static mut MOUSE_EXTRA_INFO: i64 = 0;
static mut KEYBOARD_EXTRA_INFO: i64 = 0;

pub fn set_mouse_extra_info(extra: i64) {
    unsafe { MOUSE_EXTRA_INFO = extra }
}

pub fn set_keyboard_extra_info(extra: i64) {
    unsafe { KEYBOARD_EXTRA_INFO = extra }
}

#[allow(non_upper_case_globals)]
fn workaround_fn(event: &CGEvent, keycode: CGKeyCode) {
    // https://github.com/rustdesk/rustdesk/issues/10126
    // https://stackoverflow.com/questions/74938870/sticky-fn-after-home-is-simulated-programmatically-macos
    match keycode {
        kVK_F1 | kVK_F2 | kVK_F3 | kVK_F4 | kVK_F5 | kVK_F6 | kVK_F7 | kVK_F8 | kVK_F9
        | kVK_F10 | kVK_F11 | kVK_F12 | kVK_F13 | kVK_F14 | kVK_F15 | kVK_F16 | kVK_F17
        | kVK_F18 | kVK_F19 | kVK_ANSI_KeypadClear | kVK_ForwardDelete | kVK_Home
        | kVK_End | kVK_PageDown | kVK_PageUp
        | 129 // Spotlight Search
        | 130 // Application
        | 131 // Launchpad
        | 144 // Brightness Up
        | 145 // Brightness Down
        => {
            let flags = CGEvent::flags(Some(event));
            // Remove SecondaryFn flag
            let new_flags = CGEventFlags(flags.0 & !(CGEventFlags::MaskSecondaryFn.0));
            CGEvent::set_flags(Some(event), new_flags);
        }
        kVK_UpArrow | kVK_DownArrow | kVK_LeftArrow | kVK_RightArrow => {
            let flags = CGEvent::flags(Some(event));
            // Remove SecondaryFn and NumericPad flags
            let new_flags = CGEventFlags(
                flags.0 & !(CGEventFlags::MaskSecondaryFn.0 | CGEventFlags::MaskNumericPad.0)
            );
            CGEvent::set_flags(Some(event), new_flags);
        }
        kVK_Help => {
            let flags = CGEvent::flags(Some(event));
            // Remove SecondaryFn and Help flags  
            let new_flags = CGEventFlags(
                flags.0 & !(CGEventFlags::MaskSecondaryFn.0 | CGEventFlags::MaskHelp.0)
            );
            CGEvent::set_flags(Some(event), new_flags);
        }
        _ => {}
    }
}

unsafe fn convert_native_with_source(
    event_type: &EventType,
    source: &CFRetained<CGEventSource>,
) -> Option<CFRetained<CGEvent>> {
    unsafe {
        match event_type {
            EventType::KeyPress(key) => match key {
                crate::Key::RawKey(rawkey) => {
                    if let RawKey::MacVirtualKeycode(keycode) = rawkey {
                        let event = CGEvent::new_keyboard_event(Some(source), *keycode, true)?;
                        // Apply current modifier flags
                        CGEvent::set_flags(Some(&event), *LAST_FLAGS.lock().unwrap());
                        Some(event)
                    } else {
                        None
                    }
                }
                _ => {
                    let code = code_from_key(*key)?;
                    let event = CGEvent::new_keyboard_event(Some(source), code, true)?;
                    CGEvent::set_flags(Some(&event), *LAST_FLAGS.lock().unwrap());
                    Some(event)
                }
            },
            EventType::KeyRelease(key) => match key {
                crate::Key::RawKey(rawkey) => {
                    if let RawKey::MacVirtualKeycode(keycode) = rawkey {
                        let event = CGEvent::new_keyboard_event(Some(source), *keycode, false)?;
                        CGEvent::set_flags(Some(&event), *LAST_FLAGS.lock().unwrap());
                        workaround_fn(&event, *keycode);
                        Some(event)
                    } else {
                        None
                    }
                }
                _ => {
                    let code = code_from_key(*key)?;
                    let event = CGEvent::new_keyboard_event(Some(source), code, false)?;
                    CGEvent::set_flags(Some(&event), *LAST_FLAGS.lock().unwrap());
                    workaround_fn(&event, code);
                    Some(event)
                }
            },
            EventType::ButtonPress(button) => {
                let point = get_current_mouse_location()?;
                let event_type = match button {
                    Button::Left => CGEventType::LeftMouseDown,
                    Button::Right => CGEventType::RightMouseDown,
                    _ => return None,
                };
                CGEvent::new_mouse_event(
                    Some(source),
                    event_type,
                    point,
                    CGMouseButton::Left, // ignored because we don't use OtherMouse EventType
                )
            }
            EventType::ButtonRelease(button) => {
                let point = get_current_mouse_location()?;
                let event_type = match button {
                    Button::Left => CGEventType::LeftMouseUp,
                    Button::Right => CGEventType::RightMouseUp,
                    _ => return None,
                };
                CGEvent::new_mouse_event(
                    Some(source),
                    event_type,
                    point,
                    CGMouseButton::Left,
                )
            }
            EventType::MouseMove { x, y } => {
                let point = CGPoint { x: *x, y: *y };
                CGEvent::new_mouse_event(
                    Some(source),
                    CGEventType::MouseMoved,
                    point,
                    CGMouseButton::Left,
                )
            }
            EventType::Wheel { delta_x, delta_y } => {
                let wheel_count = 2;
                CGEvent::new_scroll_wheel_event2(
                    Some(source),
                    CGScrollEventUnit::Pixel,
                    wheel_count,
                    (*delta_y).round() as i32,
                    (*delta_x).round() as i32,
                    0,
                )
            }
            // Raw events are capture-only, they cannot be simulated
            // Use the non-raw variants (ButtonPress, MouseMove, Wheel, KeyPress) for simulation
            EventType::MouseMoveRaw { .. }
            | EventType::ButtonPressRaw(_)
            | EventType::ButtonReleaseRaw(_)
            | EventType::WheelRaw { .. }
            | EventType::KeyPressRaw(_)
            | EventType::KeyReleaseRaw(_) => None,
        }
    }
}

unsafe fn convert_native(event_type: &EventType) -> Option<CFRetained<CGEvent>> {
    unsafe {
        let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)?;
        convert_native_with_source(event_type, &source)
    }
}

unsafe fn get_current_mouse_location() -> Option<CGPoint> {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)?;
    let event = CGEvent::new(Some(&source))?;
    Some(CGEvent::location(Some(&event)))
}

#[link(name = "Cocoa", kind = "framework")]
unsafe extern "C" {}

pub fn simulate(event_type: &EventType) -> Result<(), SimulateError> {
    if let Some(cg_event) = unsafe { convert_native(event_type) } {
        CGEvent::set_integer_value_field(
            Some(&cg_event),
            CGEventField::EventSourceUserData,
            unsafe { MOUSE_EXTRA_INFO },
        );
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&cg_event));
        Ok(())
    } else {
        Err(SimulateError)
    }
}

pub struct VirtualInput {
    source: CFRetained<CGEventSource>,
    tap_loc: CGEventTapLocation,
}

impl VirtualInput {
    pub fn new(state_id: CGEventSourceStateID, tap_loc: CGEventTapLocation) -> Option<Self> {
        let source = CGEventSource::new(state_id)?;
        Some(Self { source, tap_loc })
    }

    pub fn simulate(&self, event_type: &EventType) -> Result<(), SimulateError> {
        unsafe {
            if let Some(cg_event) = convert_native_with_source(event_type, &self.source) {
                CGEvent::post(self.tap_loc, Some(&cg_event));
                Ok(())
            } else {
                Err(SimulateError)
            }
        }
    }

    // keycode is defined in rdev::macos::virtual_keycodes
    pub fn get_key_state(state_id: CGEventSourceStateID, keycode: CGKeyCode) -> bool {
        CGEventSource::key_state(state_id, keycode)
    }
}
