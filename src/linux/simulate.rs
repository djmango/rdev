use crate::keycodes::linux::code_from_key;
use crate::linux::common::{FALSE, TRUE};
use crate::rdev::{Button, EventType, RawKey, SimulateError};
use std::os::raw::c_int;
use std::ptr::null;
use x11::xlib;
use x11::xtest;

// X11 exposes wheel clicks rather than a continuous axis. Bound the native requests per call.
const MAX_WHEEL_STEPS: f64 = 1024.0;

fn wheel_steps(delta: f64) -> Option<u16> {
    let magnitude = delta.abs();
    if !magnitude.is_finite() || magnitude > MAX_WHEEL_STEPS {
        return None;
    }
    Some(if magnitude == 0.0 {
        0
    } else {
        magnitude.round().max(1.0) as u16
    })
}

unsafe fn send_native(event_type: &EventType, display: *mut xlib::Display) -> Option<()> {
    unsafe {
        let res = match event_type {
            EventType::KeyPress(key) => match key {
                crate::Key::RawKey(rawkey) => {
                    if let RawKey::LinuxXorgKeycode(keycode) = rawkey {
                        xtest::XTestFakeKeyEvent(display, *keycode as _, TRUE, 0)
                    } else {
                        return None;
                    }
                }
                _ => {
                    let code = code_from_key(*key)?;
                    xtest::XTestFakeKeyEvent(display, code, TRUE, 0)
                }
            },
            EventType::KeyRelease(key) => match key {
                crate::Key::RawKey(rawkey) => {
                    if let RawKey::LinuxXorgKeycode(keycode) = rawkey {
                        xtest::XTestFakeKeyEvent(display, *keycode as _, FALSE, 0)
                    } else {
                        return None;
                    }
                }
                _ => {
                    let code = code_from_key(*key)?;
                    xtest::XTestFakeKeyEvent(display, code, FALSE, 0)
                }
            },
            EventType::ButtonPress(button) => match button {
                Button::Left => xtest::XTestFakeButtonEvent(display, 1, TRUE, 0),
                Button::Middle => xtest::XTestFakeButtonEvent(display, 2, TRUE, 0),
                Button::Right => xtest::XTestFakeButtonEvent(display, 3, TRUE, 0),
                Button::Unknown(code) => {
                    xtest::XTestFakeButtonEvent(display, (*code).into(), TRUE, 0)
                }
            },
            EventType::ButtonRelease(button) => match button {
                Button::Left => xtest::XTestFakeButtonEvent(display, 1, FALSE, 0),
                Button::Middle => xtest::XTestFakeButtonEvent(display, 2, FALSE, 0),
                Button::Right => xtest::XTestFakeButtonEvent(display, 3, FALSE, 0),
                Button::Unknown(code) => {
                    xtest::XTestFakeButtonEvent(display, (*code).into(), FALSE, 0)
                }
            },
            EventType::MouseMove { x, y } => {
                let x = if x.is_finite() {
                    x.clamp(c_int::MIN.into(), c_int::MAX.into()).round() as c_int
                } else {
                    0
                };
                let y = if y.is_finite() {
                    y.clamp(c_int::MIN.into(), c_int::MAX.into()).round() as c_int
                } else {
                    0
                };
                xtest::XTestFakeMotionEvent(display, 0, x, y, 0)
                //     xlib::XWarpPointer(display, 0, root, 0, 0, 0, 0, *x as i32, *y as i32);
            }
            EventType::Wheel { delta_x, delta_y } => {
                // Validate both axes before emitting any part of the event.
                let x = wheel_steps(*delta_x)?;
                let y = wheel_steps(*delta_y)?;
                for (button, count) in [
                    (if *delta_x > 0.0 { 7 } else { 6 }, x),
                    (if *delta_y > 0.0 { 4 } else { 5 }, y),
                ] {
                    for _ in 0..count {
                        let down = xtest::XTestFakeButtonEvent(display, button, TRUE, 0);
                        let up = xtest::XTestFakeButtonEvent(display, button, FALSE, 0);
                        if down == 0 || up == 0 {
                            return None;
                        }
                    }
                }
                1
            }
            // Raw events are for capture only, not simulation
            EventType::MouseMoveRaw { .. }
            | EventType::ButtonPressRaw(_)
            | EventType::ButtonReleaseRaw(_)
            | EventType::WheelRaw { .. }
            | EventType::KeyPressRaw(_)
            | EventType::KeyReleaseRaw(_) => return None,
        };
        if res == 0 { None } else { Some(()) }
    }
}

pub fn simulate(event_type: &EventType) -> Result<(), SimulateError> {
    unsafe {
        let dpy = xlib::XOpenDisplay(null());
        if dpy.is_null() {
            return Err(SimulateError);
        }
        match send_native(event_type, dpy) {
            Some(_) => {
                xlib::XFlush(dpy);
                xlib::XSync(dpy, 0);
                xlib::XCloseDisplay(dpy);
                Ok(())
            }
            None => {
                xlib::XCloseDisplay(dpy);
                Err(SimulateError)
            }
        }
    }
}

unsafe fn send_native_char(chr: char, pressed: bool, display: *mut xlib::Display) -> Option<()> {
    unsafe {
        // unuse keycode: F24 -> 194
        let keycode: u32 = 194;

        // char to keysym
        let ordinal: u32 = chr.into();
        let mut keysym = if ordinal < 0x100 {
            ordinal
        } else {
            ordinal | 0x01000000
        } as libc::c_ulong;

        // remap keycode to keysym
        x11::xlib::XChangeKeyboardMapping(display, keycode as _, 1, &mut keysym, 1);

        let res = if pressed {
            xtest::XTestFakeKeyEvent(display, keycode as _, TRUE, 0)
        } else {
            xtest::XTestFakeKeyEvent(display, keycode as _, FALSE, 0)
        };

        if res == 0 { None } else { Some(()) }
    }
}

pub fn simulate_char(chr: char, pressed: bool) -> Result<(), SimulateError> {
    unsafe {
        let dpy = xlib::XOpenDisplay(null());
        if dpy.is_null() {
            return Err(SimulateError);
        }
        match send_native_char(chr, pressed, dpy) {
            Some(_) => {
                xlib::XFlush(dpy);
                xlib::XSync(dpy, 0);
                xlib::XCloseDisplay(dpy);
                Ok(())
            }
            None => {
                xlib::XCloseDisplay(dpy);
                Err(SimulateError)
            }
        }
    }
}

pub fn simulate_unicode(_unicode: u16) -> Result<(), SimulateError> {
    Err(SimulateError)
}
