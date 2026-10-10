//! A real Win32 window goes through what `show` does: the palette style, the foreground, and
//! hiding again. Runs on the Windows job of the CI matrix.
#![cfg(windows)]

use daimos_platform::window::PreviousWindow;
use daimos_platform::windows::{apply_palette_style, foreground, take_foreground};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GWL_EXSTYLE, GetWindowLongPtrW, IsWindowVisible, SW_HIDE,
    SW_SHOW, ShowWindow, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::w;

#[test]
fn shows_as_a_tool_window_takes_the_foreground_and_hides() {
    // SAFETY: plain Win32 calls on a window created and destroyed by this test.
    unsafe {
        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW | WS_EX_NOACTIVATE,
            w!("STATIC"),
            w!("daimos-test"),
            WS_POPUP,
            0,
            0,
            200,
            100,
            None,
            None,
            None,
            None,
        )
        .expect("create a window");

        apply_palette_style(hwnd);
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        assert_ne!(style & WS_EX_TOOLWINDOW.0, 0, "no taskbar button, not in Alt+Tab");
        assert_eq!(style & WS_EX_APPWINDOW.0, 0);
        assert_eq!(style & WS_EX_NOACTIVATE.0, 0, "must be able to take the foreground");

        let _ = ShowWindow(hwnd, SW_SHOW);
        assert!(IsWindowVisible(hwnd).as_bool());
        assert!(take_foreground(hwnd), "SetForegroundWindow (with the fallback) failed");
        assert_eq!(foreground(), Some(PreviousWindow::Windows(hwnd.0 as isize)));

        let _ = ShowWindow(hwnd, SW_HIDE);
        assert!(!IsWindowVisible(hwnd).as_bool());
        let _ = DestroyWindow(hwnd);
    }
}
