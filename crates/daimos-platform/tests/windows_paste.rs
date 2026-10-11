//! The SendInput paster, observed with a low-level keyboard hook: it sees every key event the
//! system dispatches, injected ones included, whichever window has the focus. (A CI runner has no
//! window that can take the focus, so pasting into a real text field is a manual check.)
#![cfg(windows)]

use std::sync::Mutex;
use std::time::{Duration, Instant};

use daimos_core::paste::Paster;
use daimos_platform::paste::windows::SendInputPaster;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, PM_REMOVE,
    PeekMessageW, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP,
};

/// `(virtual key, key down, injected)` in the order the hook saw them.
static SEEN: Mutex<Vec<(u32, bool, bool)>> = Mutex::new(Vec::new());

unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        // SAFETY: for WH_KEYBOARD_LL `lparam` points to a KBDLLHOOKSTRUCT.
        let event = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let message = wparam.0 as u32;
        if message == WM_KEYDOWN || message == WM_KEYUP {
            SEEN.lock().unwrap().push((
                event.vkCode,
                message == WM_KEYDOWN,
                event.flags.0 & LLKHF_INJECTED.0 != 0,
            ));
        }
    }
    // SAFETY: passes the arguments of this call on unchanged.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn pump(duration: Duration) {
    let end = Instant::now() + duration;
    while Instant::now() < end {
        let mut message = MSG::default();
        // SAFETY: plain message pump on the thread that installed the hook.
        while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
            unsafe { DispatchMessageW(&message) };
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_paste_shortcut_is_sent_as_ctrl_down_v_down_v_up_ctrl_up() {
    // SAFETY: the hook is removed before the test ends.
    let handle = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), None, 0) }
        .expect("a low-level keyboard hook can be installed");
    SendInputPaster.paste().unwrap();
    pump(Duration::from_secs(2));
    // SAFETY: `handle` came from SetWindowsHookExW.
    unsafe { UnhookWindowsHookEx(handle) }.unwrap();

    // The hook reports the left Control key (VK_LCONTROL) for the generic VK_CONTROL that is sent.
    const CONTROL: u32 = 0xA2;
    const V: u32 = 0x56;
    let seen = SEEN.lock().unwrap().clone();
    assert_eq!(
        seen,
        [(CONTROL, true, true), (V, true, true), (V, false, true), (CONTROL, false, true)]
    );
}
