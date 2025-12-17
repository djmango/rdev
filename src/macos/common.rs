#![allow(clippy::upper_case_acronyms)]
use crate::keycodes::macos::virtual_keycodes::*;
use crate::macos::keyboard::Keyboard;
use crate::rdev::{Button, Event, EventType, Key};
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventFlags, CGEventSource, CGEventSourceStateID, CGEventType,
    CGKeyCode,
};
use parking_lot::Mutex;
use std::convert::TryInto;
use std::ptr::NonNull;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use crate::keycodes::macos::key_from_code;

pub type FourCharCode = ::std::os::raw::c_uint;
pub type OSType = FourCharCode;
pub type PhysicalKeyboardLayoutType = OSType;
pub type SInt16 = ::std::os::raw::c_short;

#[allow(non_upper_case_globals, dead_code)]
pub const kUnknownType: FourCharCode = 1061109567;
#[allow(non_upper_case_globals, dead_code)]
pub const kKeyboardJIS: PhysicalKeyboardLayoutType = 1246319392;
#[allow(non_upper_case_globals, dead_code)]
pub const kKeyboardANSI: PhysicalKeyboardLayoutType = 1095652169;
#[allow(non_upper_case_globals, dead_code)]
pub const kKeyboardISO: PhysicalKeyboardLayoutType = 1230196512;
#[allow(non_upper_case_globals, dead_code)]
pub const kKeyboardUnknown: PhysicalKeyboardLayoutType = 1061109567;

// Using AtomicU64 for LAST_FLAGS since CGEventFlags is a newtype around u64
// This eliminates mutex overhead and deadlock potential
pub static LAST_FLAGS: AtomicU64 = AtomicU64::new(0);

// Using parking_lot::Mutex which never poisons, eliminating panic potential
pub static KEYBOARD_STATE: LazyLock<Mutex<Option<Keyboard>>> =
    LazyLock::new(|| Mutex::new(Keyboard::new()));

#[allow(improper_ctypes)]
#[allow(non_snake_case)]
#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    pub fn LMGetKbdType() -> u8;
    pub fn KBGetLayoutType(iKeyboardType: SInt16) -> PhysicalKeyboardLayoutType;
}

#[cfg(target_os = "macos")]
#[inline]
fn kb_get_layout_type() -> PhysicalKeyboardLayoutType {
    unsafe { KBGetLayoutType(LMGetKbdType() as _) }
}

#[cfg(target_os = "macos")]
#[allow(non_upper_case_globals)]
pub fn map_keycode(code: CGKeyCode) -> CGKeyCode {
    match code {
        kVK_ISO_Section => {
            if kb_get_layout_type() == kKeyboardISO {
                kVK_ANSI_Grave
            } else {
                kVK_ISO_Section
            }
        }
        kVK_ANSI_Grave => {
            if kb_get_layout_type() == kKeyboardISO {
                kVK_ISO_Section
            } else {
                kVK_ANSI_Grave
            }
        }
        _ => code,
    }
}

#[inline]
unsafe fn get_code(cg_event: &CGEvent) -> Option<CGKeyCode> {
    CGEvent::integer_value_field(Some(cg_event), CGEventField::KeyboardEventKeycode)
        .try_into()
        .ok()
}

/// Convert a CGEvent to rdev Events
/// Returns a Vec because we emit both raw events and absolute events
pub unsafe fn convert(
    _type: CGEventType,
    cg_event: NonNull<CGEvent>,
    keyboard_state: &mut Keyboard,
) -> Vec<Event> {
    unsafe {
        let cg_event_ref = cg_event.as_ref();
        let mut events = Vec::new();

        // Get common event data
        let extra_data =
            CGEvent::integer_value_field(Some(cg_event_ref), CGEventField::EventSourceUserData);
        let time = SystemTime::now();

        // Detect if event is synthetic (programmatically generated) by checking the source state ID.
        // Hardware events have HIDSystemState, while synthetic events have Private or CombinedSessionState.
        let is_synthetic_by_source = CGEvent::new_source_from_event(Some(cg_event_ref))
            .map(|src| {
                CGEventSource::source_state_id(Some(&src)) != CGEventSourceStateID::HIDSystemState
            })
            .unwrap_or(false);

        // Fallback: check for enigo's EVENT_SOURCE_USER_DATA marker (100)
        // This catches events where source info is lost after posting to HID
        let is_synthetic = is_synthetic_by_source || extra_data == 100;

        match _type {
            CGEventType::LeftMouseDown => {
                // Raw event
                events.push(Event {
                    event_type: EventType::ButtonPressRaw(Button::Left),
                    time,
                    unicode: None,
                    platform_code: 0,
                    position_code: 0,
                    usb_hid: 0,
                    extra_data,
                    is_synthetic,
                });
                // Absolute event
                events.push(Event {
                    event_type: EventType::ButtonPress(Button::Left),
                    time,
                    unicode: None,
                    platform_code: 0,
                    position_code: 0,
                    usb_hid: 0,
                    extra_data,
                    is_synthetic,
                });
            }
            CGEventType::LeftMouseUp => {
                events.push(Event {
                    event_type: EventType::ButtonReleaseRaw(Button::Left),
                    time,
                    unicode: None,
                    platform_code: 0,
                    position_code: 0,
                    usb_hid: 0,
                    extra_data,
                    is_synthetic,
                });
                events.push(Event {
                    event_type: EventType::ButtonRelease(Button::Left),
                    time,
                    unicode: None,
                    platform_code: 0,
                    position_code: 0,
                    usb_hid: 0,
                    extra_data,
                    is_synthetic,
                });
            }
            CGEventType::RightMouseDown => {
                events.push(Event {
                    event_type: EventType::ButtonPressRaw(Button::Right),
                    time,
                    unicode: None,
                    platform_code: 0,
                    position_code: 0,
                    usb_hid: 0,
                    extra_data,
                    is_synthetic,
                });
                events.push(Event {
                    event_type: EventType::ButtonPress(Button::Right),
                    time,
                    unicode: None,
                    platform_code: 0,
                    position_code: 0,
                    usb_hid: 0,
                    extra_data,
                    is_synthetic,
                });
            }
            CGEventType::RightMouseUp => {
                events.push(Event {
                    event_type: EventType::ButtonReleaseRaw(Button::Right),
                    time,
                    unicode: None,
                    platform_code: 0,
                    position_code: 0,
                    usb_hid: 0,
                    extra_data,
                    is_synthetic,
                });
                events.push(Event {
                    event_type: EventType::ButtonRelease(Button::Right),
                    time,
                    unicode: None,
                    platform_code: 0,
                    position_code: 0,
                    usb_hid: 0,
                    extra_data,
                    is_synthetic,
                });
            }
            CGEventType::OtherMouseDown => {
                let button_num = CGEvent::integer_value_field(
                    Some(cg_event_ref),
                    CGEventField::MouseEventButtonNumber,
                );
                let button = if button_num == 2 {
                    Button::Middle
                } else {
                    Button::Unknown(button_num as u8)
                };
                events.push(Event {
                    event_type: EventType::ButtonPressRaw(button),
                    time,
                    unicode: None,
                    platform_code: 0,
                    position_code: 0,
                    usb_hid: 0,
                    extra_data,
                    is_synthetic,
                });
                events.push(Event {
                    event_type: EventType::ButtonPress(button),
                    time,
                    unicode: None,
                    platform_code: 0,
                    position_code: 0,
                    usb_hid: 0,
                    extra_data,
                    is_synthetic,
                });
            }
            CGEventType::OtherMouseUp => {
                let button_num = CGEvent::integer_value_field(
                    Some(cg_event_ref),
                    CGEventField::MouseEventButtonNumber,
                );
                let button = if button_num == 2 {
                    Button::Middle
                } else {
                    Button::Unknown(button_num as u8)
                };
                events.push(Event {
                    event_type: EventType::ButtonReleaseRaw(button),
                    time,
                    unicode: None,
                    platform_code: 0,
                    position_code: 0,
                    usb_hid: 0,
                    extra_data,
                    is_synthetic,
                });
                events.push(Event {
                    event_type: EventType::ButtonRelease(button),
                    time,
                    unicode: None,
                    platform_code: 0,
                    position_code: 0,
                    usb_hid: 0,
                    extra_data,
                    is_synthetic,
                });
            }
            CGEventType::MouseMoved
            | CGEventType::LeftMouseDragged
            | CGEventType::RightMouseDragged => {
                // Raw deltas (unaccelerated)
                let delta_x = CGEvent::integer_value_field(
                    Some(cg_event_ref),
                    CGEventField::MouseEventDeltaX,
                ) as i32;
                let delta_y = CGEvent::integer_value_field(
                    Some(cg_event_ref),
                    CGEventField::MouseEventDeltaY,
                ) as i32;
                if delta_x != 0 || delta_y != 0 {
                    events.push(Event {
                        event_type: EventType::MouseMoveRaw { delta_x, delta_y },
                        time,
                        unicode: None,
                        platform_code: 0,
                        position_code: 0,
                        usb_hid: 0,
                        extra_data,
                        is_synthetic,
                    });
                }
                // Absolute position
                let point = CGEvent::location(Some(cg_event_ref));
                events.push(Event {
                    event_type: EventType::MouseMove {
                        x: point.x,
                        y: point.y,
                    },
                    time,
                    unicode: None,
                    platform_code: 0,
                    position_code: 0,
                    usb_hid: 0,
                    extra_data,
                    is_synthetic,
                });
            }
            CGEventType::KeyDown => {
                if let Some(code) = get_code(cg_event_ref) {
                    let key = key_from_code(code);
                    let key_code = code as u32;
                    #[allow(non_upper_case_globals)]
                    let skip_unicode =
                        matches!(code, kVK_Shift | kVK_RightShift | kVK_ForwardDelete);
                    let unicode = if skip_unicode {
                        None
                    } else {
                        let flags = CGEvent::flags(Some(cg_event_ref));
                        keyboard_state.create_unicode_for_key(key_code, flags)
                    };
                    // Raw event
                    events.push(Event {
                        event_type: EventType::KeyPressRaw(key),
                        time,
                        unicode: unicode.clone(),
                        platform_code: code as _,
                        position_code: 0,
                        usb_hid: 0,
                        extra_data,
                        is_synthetic,
                    });
                    // Regular event
                    events.push(Event {
                        event_type: EventType::KeyPress(key),
                        time,
                        unicode,
                        platform_code: code as _,
                        position_code: 0,
                        usb_hid: 0,
                        extra_data,
                        is_synthetic,
                    });
                }
            }
            CGEventType::KeyUp => {
                if let Some(code) = get_code(cg_event_ref) {
                    let key = key_from_code(code);
                    // Raw event
                    events.push(Event {
                        event_type: EventType::KeyReleaseRaw(key),
                        time,
                        unicode: None,
                        platform_code: code as _,
                        position_code: 0,
                        usb_hid: 0,
                        extra_data,
                        is_synthetic,
                    });
                    // Regular event
                    events.push(Event {
                        event_type: EventType::KeyRelease(key),
                        time,
                        unicode: None,
                        platform_code: code as _,
                        position_code: 0,
                        usb_hid: 0,
                        extra_data,
                        is_synthetic,
                    });
                }
            }
            CGEventType::FlagsChanged => {
                if let Some(code) = get_code(cg_event_ref) {
                    let key = key_from_code(code);
                    let flags = CGEvent::flags(Some(cg_event_ref));
                    let flags_u64 = flags.0;

                    // Use atomic compare-and-swap to update flags without locking
                    let last_flags_u64 = LAST_FLAGS.load(Ordering::Acquire);
                    let last_flags = CGEventFlags(last_flags_u64);

                    let (event_type, raw_event_type) = if flags < last_flags {
                        LAST_FLAGS.store(flags_u64, Ordering::Release);
                        (EventType::KeyRelease(key), EventType::KeyReleaseRaw(key))
                    } else {
                        LAST_FLAGS.store(flags_u64, Ordering::Release);
                        (EventType::KeyPress(key), EventType::KeyPressRaw(key))
                    };
                    // Raw event
                    events.push(Event {
                        event_type: raw_event_type,
                        time,
                        unicode: None,
                        platform_code: code as _,
                        position_code: 0,
                        usb_hid: 0,
                        extra_data,
                        is_synthetic,
                    });
                    // Regular event
                    events.push(Event {
                        event_type,
                        time,
                        unicode: None,
                        platform_code: code as _,
                        position_code: 0,
                        usb_hid: 0,
                        extra_data,
                        is_synthetic,
                    });
                }
            }
            CGEventType::ScrollWheel => {
                let delta_y = CGEvent::integer_value_field(
                    Some(cg_event_ref),
                    CGEventField::ScrollWheelEventPointDeltaAxis1,
                );
                let delta_x = CGEvent::integer_value_field(
                    Some(cg_event_ref),
                    CGEventField::ScrollWheelEventPointDeltaAxis2,
                );
                // Raw wheel event
                events.push(Event {
                    event_type: EventType::WheelRaw {
                        delta_x: delta_x as f64,
                        delta_y: delta_y as f64,
                    },
                    time,
                    unicode: None,
                    platform_code: 0,
                    position_code: 0,
                    usb_hid: 0,
                    extra_data,
                    is_synthetic,
                });
                // Absolute wheel event (for compatibility)
                events.push(Event {
                    event_type: EventType::Wheel {
                        delta_x: delta_x as f64,
                        delta_y: delta_y as f64,
                    },
                    time,
                    unicode: None,
                    platform_code: 0,
                    position_code: 0,
                    usb_hid: 0,
                    extra_data,
                    is_synthetic,
                });
            }
            _ => {}
        }
        events
    }
}

#[allow(dead_code)]
#[inline]
fn key_to_name(key: Key) -> &'static str {
    use Key::*;
    match key {
        KeyA => "a",
        KeyB => "b",
        KeyC => "c",
        KeyD => "d",
        KeyE => "e",
        KeyF => "f",
        KeyG => "g",
        KeyH => "h",
        KeyI => "i",
        KeyJ => "j",
        KeyK => "k",
        KeyL => "l",
        KeyM => "m",
        KeyN => "n",
        KeyO => "o",
        KeyP => "p",
        KeyQ => "q",
        KeyR => "r",
        KeyS => "s",
        KeyT => "t",
        KeyU => "u",
        KeyV => "v",
        KeyW => "w",
        KeyX => "x",
        KeyY => "y",
        KeyZ => "z",
        Num0 => "0",
        Num1 => "1",
        Num2 => "2",
        Num3 => "3",
        Num4 => "4",
        Num5 => "5",
        Num6 => "6",
        Num7 => "7",
        Num8 => "8",
        Num9 => "9",
        Minus => "-",
        Equal => "=",
        LeftBracket => "[",
        RightBracket => "]",
        BackSlash => "\\",
        SemiColon => ";",
        Quote => "\"",
        Comma => ",",
        Dot => ".",
        Slash => "/",
        BackQuote => "`",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[allow(non_snake_case)]
    fn test_KBGetLayoutType() {
        unsafe {
            let t1 = LMGetKbdType();
            let t2 = KBGetLayoutType(t1 as _);
            println!("LMGetKbdType: {}, KBGetLayoutType: {}", t1, t2);
        }
    }
}
