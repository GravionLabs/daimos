//! The accent color of the OS, as `#rrggbb`: `UISettings` on Windows and `NSColor` on macOS.
//! Linux desktops have no common API for it (GNOME 47 added one that few themes honor), so the
//! UI keeps its fixed accent there.

/// The system accent color, or `None` where the OS has none or does not tell.
pub fn system_accent() -> Option<String> {
    imp::system_accent()
}

/// `#rrggbb` from 8-bit channels.
pub fn hex(red: u8, green: u8, blue: u8) -> String {
    format!("#{red:02x}{green:02x}{blue:02x}")
}

#[cfg(windows)]
mod imp {
    use windows::UI::ViewManagement::{UIColorType, UISettings};
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};

    pub fn system_accent() -> Option<String> {
        // Command threads have no COM apartment; S_FALSE and a changed mode are fine.
        // SAFETY: plain initialization of the calling thread.
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        let color = UISettings::new().ok()?.GetColorValue(UIColorType::Accent).ok()?;
        Some(super::hex(color.R, color.G, color.B))
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::{NSColor, NSColorSpace};

    pub fn system_accent() -> Option<String> {
        autoreleasepool(|_| {
            let color = NSColor::controlAccentColor()
                .colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
            let channel = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
            Some(super::hex(
                channel(color.redComponent()),
                channel(color.greenComponent()),
                channel(color.blueComponent()),
            ))
        })
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    pub fn system_accent() -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_are_formatted_as_lowercase_hex() {
        assert_eq!(hex(0, 120, 215), "#0078d7");
        assert_eq!(hex(255, 255, 255), "#ffffff");
    }

    #[test]
    fn the_accent_is_a_hex_color_or_none() {
        if let Some(color) = system_accent() {
            assert!(color.len() == 7 && color.starts_with('#'), "{color}");
            assert!(color[1..].chars().all(|c| c.is_ascii_hexdigit()), "{color}");
        }
        // On Linux there is none.
        #[cfg(not(any(windows, target_os = "macos")))]
        assert_eq!(system_accent(), None);
    }
}
