use crate::{
    rdev::{Event, EventType, ListenError},
    windows::common::{convert, get_scan_code, set_key_hook, set_mouse_hook, HookError, WHEEL_DELTA},
};
use std::{
    mem::{size_of, MaybeUninit},
    os::raw::c_int,
    ptr::null_mut,
    time::SystemTime,
};
use winapi::{
    shared::{
        basetsd::ULONG_PTR,
        hidusage::{HID_USAGE_GENERIC_MOUSE, HID_USAGE_PAGE_GENERIC},
        minwindef::{LPARAM, LRESULT, UINT, WPARAM},
        windef::HWND,
    },
    um::{
        libloaderapi::GetModuleHandleA,
        winuser::{
            CallNextHookEx, CreateWindowExA, DefWindowProcA, DispatchMessageA, GetMessageA,
            GetRawInputData, RegisterClassExA, RegisterRawInputDevices, TranslateMessage,
            CS_HREDRAW, CS_VREDRAW, HC_ACTION, HRAWINPUT, HWND_MESSAGE, MSG, PKBDLLHOOKSTRUCT,
            PMOUSEHOOKSTRUCT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RIDEV_INPUTSINK,
            RID_INPUT, RIM_TYPEMOUSE, RI_MOUSE_HWHEEL, RI_MOUSE_WHEEL, WM_INPUT, WNDCLASSEXA,
        },
    },
};

static mut GLOBAL_CALLBACK: Option<Box<dyn FnMut(Event)>> = None;

impl From<HookError> for ListenError {
    fn from(error: HookError) -> Self {
        match error {
            HookError::Mouse(code) => ListenError::MouseHookError(code),
            HookError::Key(code) => ListenError::KeyHookError(code),
        }
    }
}

unsafe fn raw_callback(
    code: c_int,
    param: WPARAM,
    lpdata: LPARAM,
    f_get_extra_data: impl FnOnce(isize) -> ULONG_PTR,
) -> LRESULT {
    if code == HC_ACTION {
        let (opt, code) = convert(param, lpdata);
        if let Some(event_type) = opt {
            // Skip wheel events from the hook - they're handled exclusively by Raw Input
            // This avoids duplicates and ensures we capture precision touchpad events
            if matches!(event_type, EventType::Wheel { .. }) {
                return CallNextHookEx(null_mut(), code as i32, param, lpdata);
            }

            let event = Event {
                event_type,
                time: SystemTime::now(),
                unicode: None,
                platform_code: code as _,
                position_code: get_scan_code(lpdata),
                usb_hid: 0,
                extra_data: f_get_extra_data(lpdata),
            };
            if let Some(callback) = &mut GLOBAL_CALLBACK {
                callback(event);
            }
        }
    }
    CallNextHookEx(null_mut(), code, param, lpdata)
}

unsafe extern "system" fn raw_callback_mouse(code: i32, param: usize, lpdata: isize) -> isize {
    raw_callback(code, param, lpdata, |data: isize| unsafe {
        (*(data as PMOUSEHOOKSTRUCT)).dwExtraInfo
    })
}

unsafe extern "system" fn raw_callback_keyboard(code: i32, param: usize, lpdata: isize) -> isize {
    raw_callback(code, param, lpdata, |data: isize| unsafe {
        (*(data as PKBDLLHOOKSTRUCT)).dwExtraInfo
    })
}

/// Window procedure for the hidden message window
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_INPUT {
        handle_raw_input(lparam);
        return 0;
    }
    DefWindowProcA(hwnd, msg, wparam, lparam)
}

/// Handle WM_INPUT messages to capture scroll events from all mice and touchpads
unsafe fn handle_raw_input(lparam: LPARAM) {
    let mut size: UINT = 0;

    // Get required buffer size
    if GetRawInputData(
        lparam as HRAWINPUT,
        RID_INPUT,
        null_mut(),
        &mut size,
        size_of::<RAWINPUTHEADER>() as UINT,
    ) != 0
    {
        return;
    }

    if size == 0 {
        return;
    }

    // Allocate buffer and get the data
    let mut buffer: Vec<u8> = vec![0u8; size as usize];
    let bytes_copied = GetRawInputData(
        lparam as HRAWINPUT,
        RID_INPUT,
        buffer.as_mut_ptr() as *mut _,
        &mut size,
        size_of::<RAWINPUTHEADER>() as UINT,
    );

    if bytes_copied != size {
        return;
    }

    let raw = &*(buffer.as_ptr() as *const RAWINPUT);

    // Only handle mouse input
    if raw.header.dwType != RIM_TYPEMOUSE {
        return;
    }

    let mouse = &raw.data.mouse();
    let button_flags = mouse.usButtonFlags as u32;

    // Check for wheel events
    let (delta_x, delta_y) = if button_flags & RI_MOUSE_WHEEL != 0 {
        // Vertical scroll
        let delta = mouse.usButtonData as i16;
        (0.0, delta as f64 / WHEEL_DELTA as f64)
    } else if button_flags & RI_MOUSE_HWHEEL != 0 {
        // Horizontal scroll
        let delta = mouse.usButtonData as i16;
        (delta as f64 / WHEEL_DELTA as f64, 0.0)
    } else {
        return; // No wheel event
    };

    let event = Event {
        event_type: EventType::Wheel { delta_x, delta_y },
        time: SystemTime::now(),
        unicode: None,
        platform_code: 0,
        position_code: 0,
        usb_hid: 0,
        extra_data: 0,
    };

    if let Some(callback) = &mut GLOBAL_CALLBACK {
        callback(event);
    }
}

/// Create a hidden message-only window for receiving Raw Input
unsafe fn create_message_window() -> Option<HWND> {
    let class_name = b"RdevRawInputClass\0";
    let hinstance = GetModuleHandleA(null_mut());

    let mut wc: WNDCLASSEXA = MaybeUninit::zeroed().assume_init();
    wc.cbSize = size_of::<WNDCLASSEXA>() as UINT;
    wc.style = CS_HREDRAW | CS_VREDRAW;
    wc.lpfnWndProc = Some(window_proc);
    wc.hInstance = hinstance;
    wc.lpszClassName = class_name.as_ptr() as *const i8;

    if RegisterClassExA(&wc) == 0 {
        // Class might already be registered, continue anyway
    }

    let hwnd = CreateWindowExA(
        0,
        class_name.as_ptr() as *const i8,
        b"RdevRawInput\0".as_ptr() as *const i8,
        0,
        0,
        0,
        0,
        0,
        HWND_MESSAGE, // Message-only window - completely invisible
        null_mut(),
        hinstance,
        null_mut(),
    );

    if hwnd.is_null() {
        return None;
    }

    Some(hwnd)
}

/// Register for Raw Input mouse events
unsafe fn register_raw_input(hwnd: HWND) -> bool {
    let mut rid = RAWINPUTDEVICE {
        usUsagePage: HID_USAGE_PAGE_GENERIC,
        usUsage: HID_USAGE_GENERIC_MOUSE,
        dwFlags: RIDEV_INPUTSINK, // Receive input even when not in foreground
        hwndTarget: hwnd,
    };

    RegisterRawInputDevices(&mut rid, 1, size_of::<RAWINPUTDEVICE>() as UINT) != 0
}

pub fn listen<T>(callback: T) -> Result<(), ListenError>
where
    T: FnMut(Event) + 'static,
{
    unsafe {
        GLOBAL_CALLBACK = Some(Box::new(callback));
        set_key_hook(raw_callback_keyboard)?;

        let using_mouse = !crate::keyboard_only();
        if using_mouse {
            set_mouse_hook(raw_callback_mouse)?;

            // Create hidden window and register for Raw Input to capture all scroll events
            // Raw Input handles wheel events from ALL devices (traditional mice, gaming mice,
            // precision touchpads) - the hook skips wheel events to avoid duplicates
            if let Some(hwnd) = create_message_window() {
                if !register_raw_input(hwnd) {
                    log::warn!("Failed to register for Raw Input - scroll events may not work");
                }
            } else {
                log::warn!("Failed to create message window for Raw Input");
            }
        }

        // Message loop - handles both hook messages and WM_INPUT
        let mut msg: MSG = MaybeUninit::zeroed().assume_init();
        while GetMessageA(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageA(&msg);
        }
    }
    Ok(())
}
