use crate::{
    keycodes::windows::key_from_code,
    rdev::{Button, Event, EventType, ListenError},
    windows::common::{convert, get_scan_code, set_key_hook, set_mouse_hook, HookError, WHEEL_DELTA},
};
use std::{
    collections::HashMap,
    mem::{size_of, MaybeUninit},
    os::raw::c_int,
    ptr::null_mut,
    time::SystemTime,
};
use winapi::{
    shared::{
        basetsd::ULONG_PTR,
        hidusage::{HID_USAGE_GENERIC_KEYBOARD, HID_USAGE_GENERIC_MOUSE, HID_USAGE_PAGE_GENERIC},
        minwindef::{LPARAM, LRESULT, UINT, WPARAM},
        ntdef::{NTSTATUS, ULONG, USHORT},
        windef::HWND,
    },
    um::{
        libloaderapi::GetModuleHandleA,
        winuser::{
            CallNextHookEx, CreateWindowExA, DefWindowProcA, DispatchMessageA, GetMessageA,
            GetRawInputData, GetRawInputDeviceInfoA, RegisterClassExA, RegisterRawInputDevices,
            TranslateMessage, CS_HREDRAW, CS_VREDRAW, HC_ACTION, HRAWINPUT, MSG, PKBDLLHOOKSTRUCT,
            PMOUSEHOOKSTRUCT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RIDEV_INPUTSINK,
            RIDI_PREPARSEDDATA, RID_INPUT, RIM_TYPEHID, RIM_TYPEKEYBOARD, RIM_TYPEMOUSE,
            RI_KEY_BREAK, RI_MOUSE_WHEEL, WM_INPUT, WNDCLASSEXA, WS_EX_NOACTIVATE,
            WS_EX_TOOLWINDOW, WS_POPUP,
        },
    },
};

// HID Parser types and functions (from hid.dll)
// These aren't exposed in winapi 0.3.x, so we define them manually

type PHIDP_PREPARSED_DATA = *mut std::ffi::c_void;

#[repr(C)]
#[allow(non_snake_case)]
struct HIDP_CAPS {
    Usage: USHORT,
    UsagePage: USHORT,
    InputReportByteLength: USHORT,
    OutputReportByteLength: USHORT,
    FeatureReportByteLength: USHORT,
    Reserved: [USHORT; 17],
    NumberLinkCollectionNodes: USHORT,
    NumberInputButtonCaps: USHORT,
    NumberInputValueCaps: USHORT,
    NumberInputDataIndices: USHORT,
    NumberOutputButtonCaps: USHORT,
    NumberOutputValueCaps: USHORT,
    NumberOutputDataIndices: USHORT,
    NumberFeatureButtonCaps: USHORT,
    NumberFeatureValueCaps: USHORT,
    NumberFeatureDataIndices: USHORT,
}

// HidP report types
const HIDP_INPUT: i32 = 0;

// HIDP status codes
const HIDP_STATUS_SUCCESS: NTSTATUS = 0x0011_0000u32 as i32;

#[link(name = "hid")]
unsafe extern "system" {
    fn HidP_GetCaps(
        PreparsedData: PHIDP_PREPARSED_DATA,
        Capabilities: *mut HIDP_CAPS,
    ) -> NTSTATUS;

    fn HidP_GetUsageValue(
        ReportType: i32,
        UsagePage: USHORT,
        LinkCollection: USHORT,
        Usage: USHORT,
        UsageValue: *mut ULONG,
        PreparsedData: PHIDP_PREPARSED_DATA,
        Report: *const i8,
        ReportLength: ULONG,
    ) -> NTSTATUS;
}

// Constants not defined in winapi 0.3.9
const RI_MOUSE_HWHEEL: u16 = 0x0800;

// Raw Input mouse button flags
const RI_MOUSE_LEFT_BUTTON_DOWN: u16 = 0x0001;
const RI_MOUSE_LEFT_BUTTON_UP: u16 = 0x0002;
const RI_MOUSE_RIGHT_BUTTON_DOWN: u16 = 0x0004;
const RI_MOUSE_RIGHT_BUTTON_UP: u16 = 0x0008;
const RI_MOUSE_MIDDLE_BUTTON_DOWN: u16 = 0x0010;
const RI_MOUSE_MIDDLE_BUTTON_UP: u16 = 0x0020;

// HID Usage Page for Digitizers (touchpads, touchscreens, etc.)
const HID_USAGE_PAGE_DIGITIZER: u16 = 0x0D;
// HID Usage for Touch Pad within the Digitizer page
const HID_USAGE_DIGITIZER_TOUCH_PAD: u16 = 0x05;

// HID Usages for touchpad data extraction
const HID_USAGE_DIGITIZER_CONTACT_COUNT: u16 = 0x54;
const HID_USAGE_DIGITIZER_CONTACT_ID: u16 = 0x51;
const HID_USAGE_GENERIC_X: u16 = 0x30;
const HID_USAGE_GENERIC_Y: u16 = 0x31;

static mut GLOBAL_CALLBACK: Option<Box<dyn FnMut(Event)>> = None;

// Cache for preparsed data per device (keyed by device handle)
static mut PREPARSED_DATA_CACHE: Option<HashMap<usize, Vec<u8>>> = None;

// State for tracking touchpad finger positions to compute scroll deltas
static mut LAST_TOUCH_X: i32 = 0;
static mut LAST_TOUCH_Y: i32 = 0;
static mut TOUCH_ACTIVE: bool = false;
static mut LAST_CONTACT_COUNT: u32 = 0;

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
            // Hook handles all events including wheel - Raw Input provides additional
            // coverage for precision touchpads that the hook might miss.
            // Some duplicate wheel events may occur, which is acceptable.
            let event = Event {
                event_type,
                time: SystemTime::now(),
                unicode: None,
                platform_code: code as _,
                position_code: get_scan_code(lpdata),
                usb_hid: 0,
                extra_data: f_get_extra_data(lpdata),
            };
            let ptr = &raw mut GLOBAL_CALLBACK;
            if let Some(callback) = &mut *ptr {
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

    match raw.header.dwType {
        RIM_TYPEMOUSE => handle_raw_mouse_input(raw),
        RIM_TYPEKEYBOARD => handle_raw_keyboard_input(raw),
        RIM_TYPEHID => handle_raw_hid_input(raw, &buffer),
        _ => {}
    }
}

/// Handle raw mouse input (traditional mice, some touchpads in mouse mode)
/// Emits raw events that are not breakable by any user-mode application
/// Note: Kernel drivers (anti-cheat, EDR) can still intercept or block these events.
unsafe fn handle_raw_mouse_input(raw: &RAWINPUT) {
    let mouse = &raw.data.mouse();
    let button_flags = mouse.usButtonFlags as u16;

    if button_flags & RI_MOUSE_LEFT_BUTTON_DOWN != 0 {
        emit_raw_event(EventType::ButtonPressRaw(Button::Left));
    }
    if button_flags & RI_MOUSE_LEFT_BUTTON_UP != 0 {
        emit_raw_event(EventType::ButtonReleaseRaw(Button::Left));
    }
    if button_flags & RI_MOUSE_RIGHT_BUTTON_DOWN != 0 {
        emit_raw_event(EventType::ButtonPressRaw(Button::Right));
    }
    if button_flags & RI_MOUSE_RIGHT_BUTTON_UP != 0 {
        emit_raw_event(EventType::ButtonReleaseRaw(Button::Right));
    }
    if button_flags & RI_MOUSE_MIDDLE_BUTTON_DOWN != 0 {
        emit_raw_event(EventType::ButtonPressRaw(Button::Middle));
    }
    if button_flags & RI_MOUSE_MIDDLE_BUTTON_UP != 0 {
        emit_raw_event(EventType::ButtonReleaseRaw(Button::Middle));
    }

    // Emit raw movement events (relative deltas)
    let delta_x = mouse.lLastX;
    let delta_y = mouse.lLastY;
    if delta_x != 0 || delta_y != 0 {
        emit_raw_event(EventType::MouseMoveRaw { delta_x, delta_y });
    }

    // Emit raw wheel events
    if button_flags & RI_MOUSE_WHEEL != 0 {
        let delta = mouse.usButtonData as i16;
        emit_raw_event(EventType::WheelRaw {
            delta_x: 0.0,
            delta_y: delta as f64 / WHEEL_DELTA as f64,
        });
    }
    if button_flags & RI_MOUSE_HWHEEL != 0 {
        let delta = mouse.usButtonData as i16;
        emit_raw_event(EventType::WheelRaw {
            delta_x: delta as f64 / WHEEL_DELTA as f64,
            delta_y: 0.0,
        });
    }
}

/// Handle raw keyboard input
/// Emits KeyPressRaw/KeyReleaseRaw that cannot be blocked by any user-mode application
unsafe fn handle_raw_keyboard_input(raw: &RAWINPUT) {
    let keyboard = &raw.data.keyboard();
    let vkey = keyboard.VKey;
    let flags = keyboard.Flags;

    // RI_KEY_BREAK (0x01) indicates key up, otherwise key down
    let is_release = (flags & RI_KEY_BREAK as u16) != 0;

    // Convert virtual key code to our Key enum
    let key = key_from_code(u32::from(vkey));

    if is_release {
        emit_raw_event(EventType::KeyReleaseRaw(key));
    } else {
        emit_raw_event(EventType::KeyPressRaw(key));
    }
}

/// Get or cache preparsed data for a HID device
unsafe fn get_preparsed_data(device_handle: usize) -> Option<PHIDP_PREPARSED_DATA> {
    // Initialize cache if needed using raw pointer
    let cache_ptr = &raw mut PREPARSED_DATA_CACHE;
    if (*cache_ptr).is_none() {
        *cache_ptr = Some(HashMap::new());
    }

    let cache = (*cache_ptr).as_mut().unwrap();

    // Check if already cached
    if let Some(data) = cache.get(&device_handle) {
        return Some(data.as_ptr() as PHIDP_PREPARSED_DATA);
    }

    // Get preparsed data size
    let mut preparsed_size: UINT = 0;
    if GetRawInputDeviceInfoA(
        device_handle as _,
        RIDI_PREPARSEDDATA,
        null_mut(),
        &mut preparsed_size,
    ) != 0
    {
        return None;
    }

    if preparsed_size == 0 {
        return None;
    }

    // Allocate and get preparsed data
    let mut preparsed_data: Vec<u8> = vec![0u8; preparsed_size as usize];
    if GetRawInputDeviceInfoA(
        device_handle as _,
        RIDI_PREPARSEDDATA,
        preparsed_data.as_mut_ptr() as *mut _,
        &mut preparsed_size,
    ) == u32::MAX
    {
        return None;
    }

    let ptr = preparsed_data.as_ptr() as PHIDP_PREPARSED_DATA;
    cache.insert(device_handle, preparsed_data);

    Some(ptr)
}

/// Handle raw HID input (precision touchpads)
/// Uses HidP_* functions to properly parse touchpad reports
unsafe fn handle_raw_hid_input(raw: &RAWINPUT, buffer: &[u8]) {
    let hid = &raw.data.hid();

    if hid.dwCount == 0 || hid.dwSizeHid == 0 {
        return;
    }

    // Get preparsed data for this device
    let device_handle = raw.header.hDevice as usize;
    let preparsed_data = match get_preparsed_data(device_handle) {
        Some(data) => data,
        None => return,
    };

    // Get device capabilities
    let mut caps: HIDP_CAPS = MaybeUninit::zeroed().assume_init();
    if HidP_GetCaps(preparsed_data, &mut caps) != HIDP_STATUS_SUCCESS {
        return;
    }

    // Calculate where the HID report data starts
    // The bRawData field in RAWHID is at offset after dwSizeHid and dwCount
    let header_size = size_of::<RAWINPUTHEADER>();
    let rawhid_header_size = size_of::<ULONG>() * 2; // dwSizeHid + dwCount
    let report_start = header_size + rawhid_header_size;

    // Process each HID report
    for i in 0..hid.dwCount {
        let report_offset = report_start + (i as usize * hid.dwSizeHid as usize);
        if report_offset + hid.dwSizeHid as usize > buffer.len() {
            break;
        }

        let report = &buffer[report_offset..report_offset + hid.dwSizeHid as usize];

        // Try to extract touchpad scroll data using HidP functions
        if let Some((delta_x, delta_y)) = parse_touchpad_with_hidp(preparsed_data, report, &caps) {
            if delta_x != 0.0 || delta_y != 0.0 {
                emit_raw_event(EventType::WheelRaw { delta_x, delta_y });
            }
        }
    }
}

/// Parse touchpad data using HidP_* functions
/// Returns scroll deltas if this is a two-finger scroll gesture
unsafe fn parse_touchpad_with_hidp(
    preparsed_data: PHIDP_PREPARSED_DATA,
    report: &[u8],
    _caps: &HIDP_CAPS,
) -> Option<(f64, f64)> {
    // Get contact count from the digitizer page
    let mut contact_count: ULONG = 0;
    let status = HidP_GetUsageValue(
        HIDP_INPUT,
        HID_USAGE_PAGE_DIGITIZER,
        0, // Link collection
        HID_USAGE_DIGITIZER_CONTACT_COUNT,
        &mut contact_count,
        preparsed_data,
        report.as_ptr() as *mut i8,
        report.len() as ULONG,
    );

    // If we can't get contact count, this might not be a touchpad report
    if status != HIDP_STATUS_SUCCESS {
        // Reset tracking state
        if TOUCH_ACTIVE {
            TOUCH_ACTIVE = false;
            LAST_CONTACT_COUNT = 0;
        }
        return None;
    }

    // Only interested in two-finger gestures (scrolling)
    if contact_count != 2 {
        if contact_count == 0 && TOUCH_ACTIVE {
            TOUCH_ACTIVE = false;
            LAST_CONTACT_COUNT = 0;
        }
        LAST_CONTACT_COUNT = contact_count;
        return None;
    }

    // Get X position from Generic Desktop page
    let mut x_value: ULONG = 0;
    let x_status = HidP_GetUsageValue(
        HIDP_INPUT,
        HID_USAGE_PAGE_GENERIC,
        0,
        HID_USAGE_GENERIC_X,
        &mut x_value,
        preparsed_data,
        report.as_ptr() as *mut i8,
        report.len() as ULONG,
    );

    // Get Y position from Generic Desktop page
    let mut y_value: ULONG = 0;
    let y_status = HidP_GetUsageValue(
        HIDP_INPUT,
        HID_USAGE_PAGE_GENERIC,
        0,
        HID_USAGE_GENERIC_Y,
        &mut y_value,
        preparsed_data,
        report.as_ptr() as *mut i8,
        report.len() as ULONG,
    );

    // Need both X and Y for scroll tracking
    if x_status != HIDP_STATUS_SUCCESS || y_status != HIDP_STATUS_SUCCESS {
        return None;
    }

    let x = x_value as i32;
    let y = y_value as i32;

    if TOUCH_ACTIVE && LAST_CONTACT_COUNT == 2 {
        // Calculate delta from last position
        let dx = x - LAST_TOUCH_X;
        let dy = y - LAST_TOUCH_Y;

        // Update position
        LAST_TOUCH_X = x;
        LAST_TOUCH_Y = y;
        LAST_CONTACT_COUNT = contact_count;

        // Convert to scroll units
        // Touchpad coordinates are typically in device units (e.g., 0-1000 or larger)
        // Scale factor tuned for reasonable scroll speed
        const SCROLL_SCALE: f64 = 0.01;

        // Only report if there's meaningful movement (filter noise)
        if dx.abs() > 5 || dy.abs() > 5 {
            // Invert Y for natural scrolling (moving fingers down scrolls content up)
            return Some((dx as f64 * SCROLL_SCALE, -dy as f64 * SCROLL_SCALE));
        }
    } else {
        // Start tracking new gesture
        TOUCH_ACTIVE = true;
        LAST_TOUCH_X = x;
        LAST_TOUCH_Y = y;
        LAST_CONTACT_COUNT = contact_count;
    }

    None
}

/// Emit a raw event to the callback
unsafe fn emit_raw_event(event_type: EventType) {
    let event = Event {
        event_type,
        time: SystemTime::now(),
        unicode: None,
        platform_code: 0,
        position_code: 0,
        usb_hid: 0,
        extra_data: 0,
    };

    let ptr = &raw mut GLOBAL_CALLBACK;
    if let Some(callback) = &mut *ptr {
        callback(event);
    }
}

/// Create a hidden window for receiving Raw Input
/// Uses a real invisible window (not HWND_MESSAGE) because message-only windows
/// may not properly receive WM_INPUT messages on all Windows versions.
unsafe fn create_hidden_window() -> Option<HWND> {
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

    // Create a real invisible window instead of HWND_MESSAGE
    // WS_EX_TOOLWINDOW: Won't appear in taskbar
    // WS_EX_NOACTIVATE: Won't steal focus
    // WS_POPUP: No window decorations
    let hwnd = CreateWindowExA(
        WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
        class_name.as_ptr() as *const i8,
        b"RdevRawInput\0".as_ptr() as *const i8,
        WS_POPUP,
        0,
        0,
        1,
        1,
        null_mut(), // No parent - this is a real top-level window
        null_mut(),
        hinstance,
        null_mut(),
    );

    if hwnd.is_null() {
        return None;
    }

    Some(hwnd)
}

/// Register for Raw Input from both mice and precision touchpads
unsafe fn register_raw_input(hwnd: HWND) -> bool {
    let devices = [
        // Traditional mouse input
        RAWINPUTDEVICE {
            usUsagePage: HID_USAGE_PAGE_GENERIC,
            usUsage: HID_USAGE_GENERIC_MOUSE,
            dwFlags: RIDEV_INPUTSINK, // Receive input even when not in foreground
            hwndTarget: hwnd,
        },
        // Precision touchpad input (HID digitizer)
        RAWINPUTDEVICE {
            usUsagePage: HID_USAGE_PAGE_DIGITIZER,
            usUsage: HID_USAGE_DIGITIZER_TOUCH_PAD,
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: hwnd,
        },
        // Keyboard input
        RAWINPUTDEVICE {
            usUsagePage: HID_USAGE_PAGE_GENERIC,
            usUsage: HID_USAGE_GENERIC_KEYBOARD,
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: hwnd,
        },
    ];

    RegisterRawInputDevices(devices.as_ptr(), devices.len() as UINT, size_of::<RAWINPUTDEVICE>() as UINT) != 0
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
            // precision touchpads) - the hook also handles wheel for apps where Raw Input fails
            if let Some(hwnd) = create_hidden_window() {
                if register_raw_input(hwnd) {
                    log::debug!("Raw Input registered successfully for scroll events");
                } else {
                    log::warn!("Failed to register for Raw Input - scroll events may not work");
                }
            } else {
                log::warn!("Failed to create hidden window for Raw Input");
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
