//! The Windows clipboard: a message-only window registered with `AddClipboardFormatListener`
//! receives `WM_CLIPBOARDUPDATE` for every change. Content that carries the format
//! `ExcludeClipboardContentFromMonitorProcessing` (password managers set it) is concealed.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use daimos_core::clipboard::{
    ClipboardBackend, ClipboardContent, ClipboardSink, ClipboardSnapshot, ClipboardSupport,
};
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, GetClipboardData, IsClipboardFormatAvailable,
    OpenClipboard, RegisterClipboardFormatW, RemoveClipboardFormatListener,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWLP_USERDATA,
    GetWindowLongPtrW, HWND_MESSAGE, MSG, MsgWaitForMultipleObjects, PM_REMOVE, PeekMessageW,
    QS_ALLINPUT, RegisterClassW, SetWindowLongPtrW, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_CLIPBOARDUPDATE, WNDCLASSW,
};
use windows::core::{PCWSTR, w};

pub struct WindowsClipboard;

impl ClipboardBackend for WindowsClipboard {
    fn id(&self) -> &str {
        "windows"
    }

    fn support(&self) -> ClipboardSupport {
        ClipboardSupport::Full
    }

    fn watch(&self, sink: ClipboardSink, stop: Arc<AtomicBool>) -> io::Result<()> {
        let class = w!("DaimosClipboardListener");
        // SAFETY: plain Win32 calls on this thread; the window is destroyed before returning.
        unsafe {
            let instance = GetModuleHandleW(None).map_err(io::Error::other)?;
            let class_info = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance.into(),
                lpszClassName: class,
                ..Default::default()
            };
            // Registering twice fails with "class already exists", which is fine.
            RegisterClassW(&class_info);
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                w!(""),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(instance.into()),
                None,
            )
            .map_err(io::Error::other)?;

            let mut listener = Listener { sink, skip_initial: true };
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, &mut listener as *mut Listener as isize);
            if let Err(error) = AddClipboardFormatListener(hwnd) {
                let _ = DestroyWindow(hwnd);
                return Err(io::Error::other(error));
            }
            while !stop.load(Ordering::SeqCst) {
                MsgWaitForMultipleObjects(None, false, 200, QS_ALLINPUT);
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    DispatchMessageW(&message);
                }
            }
            let _ = RemoveClipboardFormatListener(hwnd);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            let _ = DestroyWindow(hwnd);
        }
        Ok(())
    }
}

struct Listener {
    sink: ClipboardSink,
    /// Adding the listener posts one update for the content that is already there.
    skip_initial: bool,
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_CLIPBOARDUPDATE {
        // SAFETY: the pointer is set by `watch` and cleared before the `Listener` is dropped; both
        // happen on this thread, which is the only one that dispatches messages to the window.
        let listener =
            unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Listener).as_mut() };
        if let Some(listener) = listener {
            if std::mem::take(&mut listener.skip_initial) {
                return LRESULT(0);
            }
            if let Some(snapshot) = read_clipboard(hwnd) {
                (listener.sink)(snapshot);
            }
        }
        return LRESULT(0);
    }
    // SAFETY: forwards the arguments of this call unchanged.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

fn format(name: PCWSTR) -> u32 {
    // SAFETY: `name` is a valid NUL-terminated string.
    unsafe { RegisterClipboardFormatW(name) }
}

/// Another process may hold the clipboard open for a moment.
fn open(hwnd: HWND) -> bool {
    for _ in 0..10 {
        // SAFETY: plain Win32 call.
        if unsafe { OpenClipboard(Some(hwnd)) }.is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn read_clipboard(hwnd: HWND) -> Option<ClipboardSnapshot> {
    if !open(hwnd) {
        return None;
    }
    let png = format(w!("PNG"));
    let concealed = format(w!("ExcludeClipboardContentFromMonitorProcessing"));
    // SAFETY: the clipboard is open until `CloseClipboard` below.
    let snapshot = unsafe {
        let concealed = IsClipboardFormatAvailable(concealed).is_ok();
        if IsClipboardFormatAvailable(png).is_ok() {
            global_bytes(png).map(|bytes| (ClipboardContent::Image(bytes), concealed))
        } else if IsClipboardFormatAvailable(CF_UNICODETEXT.0.into()).is_ok() {
            global_bytes(CF_UNICODETEXT.0.into()).map(|bytes| {
                let units: Vec<u16> =
                    bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
                (ClipboardContent::Text(String::from_utf16_lossy(&units[..end])), concealed)
            })
        } else {
            None
        }
    };
    // SAFETY: closes what `open` opened.
    let _ = unsafe { CloseClipboard() };
    snapshot.map(|(content, concealed)| ClipboardSnapshot { content, concealed })
}

/// A copy of the global memory behind clipboard format `format`; the clipboard must be open.
unsafe fn global_bytes(format: u32) -> Option<Vec<u8>> {
    // SAFETY: the caller keeps the clipboard open; the lock is released before returning.
    unsafe {
        let handle: HANDLE = GetClipboardData(format).ok()?;
        let global = HGLOBAL(handle.0);
        let size = GlobalSize(global);
        let pointer = GlobalLock(global) as *const u8;
        if pointer.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(pointer, size).to_vec();
        let _ = GlobalUnlock(global);
        Some(bytes)
    }
}
