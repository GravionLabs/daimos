//! The SendInput paster against an `EDIT` control of its own that has the focus; the clipboard is
//! filled with PowerShell first.
#![cfg(windows)]

use std::process::Command;
use std::time::{Duration, Instant};

use daimos_core::paste::Paster;
use daimos_platform::paste::windows::SendInputPaster;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, DispatchMessageW, GetWindowTextW, MSG, PM_REMOVE, PeekMessageW,
    SW_SHOW, SetForegroundWindow, ShowWindow, WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};
use windows::core::w;

fn pump(duration: Duration) {
    let end = Instant::now() + duration;
    while Instant::now() < end {
        let mut message = MSG::default();
        // SAFETY: plain message pump on the thread that owns the window.
        while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
            unsafe { DispatchMessageW(&message) };
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn text(edit: HWND) -> String {
    let mut buffer = [0u16; 256];
    // SAFETY: the buffer is valid for its length.
    let len = unsafe { GetWindowTextW(edit, &mut buffer) } as usize;
    String::from_utf16_lossy(&buffer[..len])
}

#[test]
fn ctrl_v_pastes_the_clipboard_into_the_focused_edit_control() {
    let status = Command::new("powershell")
        .args(["-NoProfile", "-Command", "Set-Clipboard -Value 'pasted by daimos'"])
        .status()
        .unwrap();
    assert!(status.success());

    // SAFETY: plain Win32 calls on this thread; the window is destroyed at the end.
    unsafe {
        let instance = GetModuleHandleW(None).unwrap();
        let edit = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("EDIT"),
            w!(""),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            100,
            100,
            400,
            100,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .unwrap();
        let _ = ShowWindow(edit, SW_SHOW);
        let _ = SetForegroundWindow(edit);
        let _ = SetFocus(Some(edit));
        pump(Duration::from_millis(500));

        SendInputPaster.paste().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while text(edit).is_empty() && Instant::now() < deadline {
            pump(Duration::from_millis(50));
        }
        let pasted = text(edit);
        let _ = DestroyWindow(edit);
        assert_eq!(pasted, "pasted by daimos");
    }
}
