//! Windows: a tool window (no taskbar button, not in Alt+Tab) that reliably takes the foreground
//! after the hotkey, and a note of the window that had it before.
//!
//! `SetForegroundWindow` only works for a process that handled the last input event, which the
//! registered hotkey counts as. When Windows refuses anyway, the fallback attaches this thread's
//! input to the foreground thread and tries once more.

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetForegroundWindow, GetWindowLongPtrW, GetWindowThreadProcessId,
    SetForegroundWindow, SetWindowLongPtrW, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};

use crate::window::PreviousWindow;

/// The window in the foreground right now.
pub fn foreground() -> Option<PreviousWindow> {
    // SAFETY: no arguments; returns a null handle when there is no foreground window.
    let hwnd = unsafe { GetForegroundWindow() };
    (!hwnd.is_invalid()).then_some(PreviousWindow::Windows(hwnd.0 as isize))
}

/// Computes the extended style of the Daimos window: a tool window that can be activated.
pub fn palette_style(current: isize) -> isize {
    let set = WS_EX_TOOLWINDOW.0 as isize;
    let clear = (WS_EX_APPWINDOW.0 | WS_EX_NOACTIVATE.0) as isize;
    (current | set) & !clear
}

/// Applies [`palette_style`] to `hwnd`.
pub fn apply_palette_style(hwnd: HWND) {
    // SAFETY: `hwnd` is a window of this process; the calls only read and write its style.
    unsafe {
        let current = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, palette_style(current));
    }
}

/// Brings `hwnd` to the foreground; returns whether it is there afterwards.
pub fn take_foreground(hwnd: HWND) -> bool {
    // SAFETY: `hwnd` is a window of this process.
    unsafe {
        if SetForegroundWindow(hwnd).as_bool() {
            return true;
        }
        // Retry once through the input queue of the thread that owns the foreground.
        let foreground = GetForegroundWindow();
        if foreground.is_invalid() {
            return false;
        }
        let foreground_thread = GetWindowThreadProcessId(foreground, None);
        let this_thread = GetCurrentThreadId();
        if foreground_thread == 0 || foreground_thread == this_thread {
            return false;
        }
        let attached = AttachThreadInput(this_thread, foreground_thread, true).as_bool();
        let result = SetForegroundWindow(hwnd).as_bool();
        if attached {
            let _ = AttachThreadInput(this_thread, foreground_thread, false);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_palette_style_is_a_tool_window_that_can_be_activated() {
        let all = (WS_EX_APPWINDOW.0 | WS_EX_NOACTIVATE.0) as isize;
        let style = palette_style(all);
        assert_ne!(style & WS_EX_TOOLWINDOW.0 as isize, 0);
        assert_eq!(style & WS_EX_APPWINDOW.0 as isize, 0);
        assert_eq!(style & WS_EX_NOACTIVATE.0 as isize, 0);
    }
}
