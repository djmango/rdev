use crate::{
    rdev::{Event, EventType, GrabError},
    windows::common::{
        HookError, KEYBOARD, convert, get_scan_code, is_keyboard_injected, is_mouse_injected,
    },
};
use parking_lot::Mutex;
use std::sync::{LazyLock, OnceLock};
use std::{io::Error, ptr::null_mut, time::SystemTime};
use tracing::error;

use winapi::{
    shared::{
        basetsd::ULONG_PTR,
        minwindef::{DWORD, FALSE},
        ntdef::NULL,
        windef::{HHOOK, POINT},
    },
    um::{
        errhandlingapi::GetLastError,
        processthreadsapi::GetCurrentThreadId,
        winuser::{
            CallNextHookEx, DispatchMessageA, GetMessageA, HC_ACTION, MSG, PKBDLLHOOKSTRUCT,
            PMOUSEHOOKSTRUCT, PostThreadMessageA, SetWindowsHookExA, TranslateMessage,
            UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_USER,
        },
    },
};

use std::sync::atomic::{AtomicBool, Ordering};

type GrabCallback = Mutex<Box<dyn FnMut(Event) -> Option<Event> + Send>>;

static GLOBAL_CALLBACK: OnceLock<GrabCallback> = OnceLock::new();
static GET_KEY_UNICODE: AtomicBool = AtomicBool::new(true);

static CUR_HOOK_THREAD_ID: LazyLock<Mutex<DWORD>> = LazyLock::new(|| Mutex::new(0));

const WM_USER_EXIT_HOOK: u32 = WM_USER + 1;

pub fn set_get_key_unicode(b: bool) {
    GET_KEY_UNICODE.store(b, Ordering::Relaxed);
}

pub fn set_event_popup(b: bool) {
    KEYBOARD.lock().set_event_popup(b);
}

unsafe fn raw_callback(
    code: i32,
    param: usize,
    lpdata: isize,
    f_get_extra_data: impl FnOnce(isize) -> ULONG_PTR,
    f_is_injected: impl FnOnce(isize) -> bool,
) -> isize {
    unsafe {
        if code == HC_ACTION {
            let (opt, code) = convert(param, lpdata);
            if let Some(event_type) = opt {
                let unicode = if GET_KEY_UNICODE.load(Ordering::Relaxed) {
                    match &event_type {
                        EventType::KeyPress(_key) => {
                            let mut keyboard = KEYBOARD.lock();
                            keyboard.get_unicode(lpdata)
                        }
                        _ => None,
                    }
                } else {
                    None
                };
                let event = Event {
                    event_type,
                    time: SystemTime::now(),
                    unicode,
                    platform_code: code as _,
                    position_code: get_scan_code(lpdata),
                    usb_hid: 0,
                    extra_data: f_get_extra_data(lpdata),
                    is_synthetic: f_is_injected(lpdata),
                };

                if let Some(callback_mutex) = GLOBAL_CALLBACK.get() {
                    let mut callback = callback_mutex.lock();
                    if callback(event).is_none() {
                        // https://stackoverflow.com/questions/42756284/blocking-windows-mouse-click-using-setwindowshookex
                        // https://android.developreference.com/article/14560004/Blocking+windows+mouse+click+using+SetWindowsHookEx()
                        // https://cboard.cprogramming.com/windows-programming/99678-setwindowshookex-wm_keyboard_ll.html
                        // let _result = CallNextHookEx(hhk, code, param, lpdata);
                        return 1;
                    }
                }
            }
        }
        CallNextHookEx(null_mut(), code, param, lpdata)
    }
}

unsafe extern "system" fn raw_callback_mouse(code: i32, param: usize, lpdata: isize) -> isize {
    unsafe {
        raw_callback(
            code,
            param,
            lpdata,
            |data: isize| (*(data as PMOUSEHOOKSTRUCT)).dwExtraInfo,
            |data: isize| is_mouse_injected(data),
        )
    }
}

unsafe extern "system" fn raw_callback_keyboard(code: i32, param: usize, lpdata: isize) -> isize {
    unsafe {
        raw_callback(
            code,
            param,
            lpdata,
            |data: isize| (*(data as PKBDLLHOOKSTRUCT)).dwExtraInfo,
            |data: isize| is_keyboard_injected(data),
        )
    }
}

impl From<HookError> for GrabError {
    fn from(error: HookError) -> Self {
        match error {
            HookError::Mouse(code) => GrabError::MouseHookError(code),
            HookError::Key(code) => GrabError::KeyHookError(code),
        }
    }
}

fn do_hook<T>(callback: T) -> Result<(HHOOK, HHOOK), GrabError>
where
    T: FnMut(Event) -> Option<Event> + Send + 'static,
{
    let mut cur_hook_thread_id = CUR_HOOK_THREAD_ID.lock();
    if *cur_hook_thread_id != 0 {
        // already hooked
        return Ok((null_mut(), null_mut()));
    }

    let hook_keyboard;
    let mut hook_mouse = null_mut();

    // Initialize callback in OnceLock
    if GLOBAL_CALLBACK.set(Mutex::new(Box::new(callback))).is_err() {
        error!(
            "grab() called multiple times - this is not allowed. Only one grab can be active at a time."
        );
        return Err(GrabError::AlreadyGrabbing);
    }

    unsafe {
        hook_keyboard =
            SetWindowsHookExA(WH_KEYBOARD_LL, Some(raw_callback_keyboard), null_mut(), 0);
        if hook_keyboard.is_null() {
            return Err(GrabError::KeyHookError(GetLastError()));
        }

        if !crate::keyboard_only() {
            hook_mouse = SetWindowsHookExA(WH_MOUSE_LL, Some(raw_callback_mouse), null_mut(), 0);
            if hook_mouse.is_null() {
                if FALSE == UnhookWindowsHookEx(hook_keyboard) {
                    // Fatal error
                    error!(
                        "UnhookWindowsHookEx keyboard failed: {}",
                        Error::last_os_error()
                    );
                }
                return Err(GrabError::MouseHookError(GetLastError()));
            }
        }
        *cur_hook_thread_id = GetCurrentThreadId();
    }

    Ok((hook_keyboard, hook_mouse))
}

#[inline]
pub fn is_grabbed() -> bool {
    *CUR_HOOK_THREAD_ID.lock() != 0
}

pub fn grab<T>(callback: T) -> Result<(), GrabError>
where
    T: FnMut(Event) -> Option<Event> + Send + 'static,
{
    if is_grabbed() {
        return Ok(());
    }

    unsafe {
        let (mut hook_keyboard, hook_mouse) = do_hook(callback)?;
        if hook_keyboard.is_null() && hook_mouse.is_null() {
            return Ok(());
        }

        let mut msg = MSG {
            hwnd: NULL as _,
            message: 0 as _,
            wParam: 0 as _,
            lParam: 0 as _,
            time: 0 as _,
            pt: POINT {
                x: 0 as _,
                y: 0 as _,
            },
        };
        while FALSE != GetMessageA(&mut msg, NULL as _, 0, 0) {
            if msg.message == WM_USER_EXIT_HOOK {
                if !hook_keyboard.is_null() {
                    if FALSE == UnhookWindowsHookEx(hook_keyboard as _) {
                        error!(
                            "Failed UnhookWindowsHookEx keyboard: {}",
                            Error::last_os_error()
                        );
                        continue;
                    }
                    hook_keyboard = null_mut();
                }

                if !hook_mouse.is_null() && FALSE == UnhookWindowsHookEx(hook_mouse as _) {
                    error!(
                        "Failed UnhookWindowsHookEx mouse: {}",
                        Error::last_os_error()
                    );
                    continue;
                }
                // hook_mouse = null_mut();
                break;
            }

            TranslateMessage(&msg);
            DispatchMessageA(&msg);
        }

        *CUR_HOOK_THREAD_ID.lock() = 0;
    }
    Ok(())
}

pub fn exit_grab() -> Result<(), GrabError> {
    unsafe {
        let mut cur_hook_thread_id = CUR_HOOK_THREAD_ID.lock();
        if *cur_hook_thread_id != 0
            && FALSE == PostThreadMessageA(*cur_hook_thread_id, WM_USER_EXIT_HOOK, 0, 0)
        {
            return Err(GrabError::ExitGrabError(format!(
                "Failed to post message to exit hook, {}",
                GetLastError()
            )));
        }
        *cur_hook_thread_id = 0;
    }
    Ok(())
}
