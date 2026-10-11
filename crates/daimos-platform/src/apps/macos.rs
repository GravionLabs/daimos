//! Applications from `.app` bundles, plus the panes of System Settings.
//!
//! Names come from the bundle (`CFBundleDisplayName`, then `CFBundleName`; `NSBundle` returns the
//! localized value of the current language), icons from `NSWorkspace` drawn at 64 px, launching
//! through `NSWorkspace`. A bundle is a leaf: nothing inside an `.app` is scanned.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

use daimos_core::apps::{AppAction, AppEntry, AppSource};
use objc2::rc::{Retained, autoreleasepool};
use objc2_app_kit::{NSWorkspace, NSWorkspaceOpenConfiguration};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGBitmapContextCreate, CGColorSpace, CGContext, CGImageAlphaInfo};
use objc2_foundation::{NSArray, NSBundle, NSDictionary, NSNumber, NSString, NSURL};

const ICON_SIZE: usize = 64;
const SETTINGS_PANES: &str = "/System/Library/ExtensionKit/Extensions";
const SETTINGS_EXTENSION_POINT: &str = "com.apple.Settings.extension.ui";
const SETTINGS_URL_PREFIX: &str = "x-apple.systempreferences:";

pub struct MacAppSource {
    /// Directories with `.app` bundles and how deep to look for them (folders such as `Utilities`
    /// hold bundles; bundles are never entered).
    dirs: Vec<(PathBuf, usize)>,
    icon_dir: PathBuf,
    settings_panes: bool,
}

impl MacAppSource {
    /// The standard application folders, Homebrew Cask included, and the System Settings panes.
    pub fn new(icon_dir: impl Into<PathBuf>) -> Self {
        let mut dirs = vec![
            (PathBuf::from("/Applications"), 2),
            (PathBuf::from("/System/Applications"), 2),
            (PathBuf::from("/System/Applications/Utilities"), 1),
            (PathBuf::from("/System/Library/CoreServices/Applications"), 1),
            // Casks unpack to Caskroom/<name>/<version>/<App>.app.
            (PathBuf::from("/opt/homebrew/Caskroom"), 3),
            (PathBuf::from("/usr/local/Caskroom"), 3),
        ];
        if let Some(home) = std::env::var_os("HOME") {
            dirs.insert(1, (Path::new(&home).join("Applications"), 2));
        }
        Self { dirs, icon_dir: icon_dir.into(), settings_panes: true }
    }

    /// Scans only `dirs` (to the given depth) and no System Settings panes.
    pub fn with_dirs(dirs: Vec<(PathBuf, usize)>, icon_dir: impl Into<PathBuf>) -> Self {
        Self { dirs, icon_dir: icon_dir.into(), settings_panes: false }
    }

    fn entry(&self, bundle_path: &Path) -> Option<AppEntry> {
        autoreleasepool(|_| {
            let bundle = bundle_for(bundle_path)?;
            if bool_key(&bundle, "LSUIElement") || bool_key(&bundle, "LSBackgroundOnly") {
                return None;
            }
            let name = string_key(&bundle, "CFBundleDisplayName")
                .or_else(|| string_key(&bundle, "CFBundleName"))
                .or_else(|| bundle_path.file_stem().map(|s| s.to_string_lossy().into_owned()))?;
            let identifier =
                bundle_identifier(&bundle).unwrap_or_else(|| bundle_path.display().to_string());
            Some(AppEntry {
                id: format!("macos:{identifier}"),
                name,
                generic_name: None,
                keywords: Vec::new(),
                exec: bundle_path.display().to_string(),
                terminal: false,
                icon: render_icon(bundle_path, &self.icon_dir),
                actions: vec![AppAction {
                    id: "reveal".into(),
                    name: "Show in Finder".into(),
                    exec: bundle_path.display().to_string(),
                }],
            })
        })
    }

    fn settings_panes(&self) -> Vec<AppEntry> {
        let Ok(read) = std::fs::read_dir(SETTINGS_PANES) else { return Vec::new() };
        read.flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "appex"))
            .filter_map(|path| {
                autoreleasepool(|_| {
                    let bundle = bundle_for(&path)?;
                    if extension_point(&bundle).as_deref() != Some(SETTINGS_EXTENSION_POINT) {
                        return None;
                    }
                    let identifier = bundle_identifier(&bundle)?;
                    let name = string_key(&bundle, "CFBundleDisplayName")
                        .or_else(|| string_key(&bundle, "CFBundleName"))?;
                    Some(AppEntry {
                        id: format!("macos:settings:{identifier}"),
                        name: format!("{name} (System Settings)"),
                        generic_name: Some("System Settings".into()),
                        keywords: vec!["settings".into(), "preferences".into()],
                        exec: format!("{SETTINGS_URL_PREFIX}{identifier}"),
                        terminal: false,
                        icon: render_icon(&path, &self.icon_dir),
                        actions: Vec::new(),
                    })
                })
            })
            .collect()
    }
}

impl AppSource for MacAppSource {
    fn id(&self) -> &str {
        "macos"
    }

    fn scan(&self) -> Vec<AppEntry> {
        if let Err(error) = std::fs::create_dir_all(&self.icon_dir) {
            log::warn!("cannot create the icon cache {}: {error}", self.icon_dir.display());
        }
        let mut seen = HashSet::new();
        let mut apps = Vec::new();
        // Earlier directories win, so a copy in ~/Applications hides one in a Caskroom.
        for (dir, depth) in &self.dirs {
            for bundle in app_bundles(dir, *depth) {
                if let Some(app) = self.entry(&bundle)
                    && seen.insert(app.id.clone())
                {
                    apps.push(app);
                }
            }
        }
        if self.settings_panes {
            apps.extend(self.settings_panes());
        }
        apps.sort_by(|a, b| {
            a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.id.cmp(&b.id))
        });
        apps
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        self.dirs.iter().map(|(d, _)| d.clone()).filter(|d| d.is_dir()).collect()
    }

    fn launch(&self, app: &AppEntry, action: Option<&str>) -> io::Result<()> {
        match action {
            Some("reveal") if app.actions.iter().any(|a| a.id == "reveal") => {
                reveal_in_finder(Path::new(&app.exec))
            }
            Some(other) => {
                Err(io::Error::new(io::ErrorKind::NotFound, format!("no action {other}")))
            }
            None if app.exec.starts_with(SETTINGS_URL_PREFIX) => open_url(&app.exec),
            None => open_application(Path::new(&app.exec)),
        }
    }
}

/// `.app` bundles below `dir`, looking into at most `depth` levels of ordinary folders.
fn app_bundles(dir: &Path, depth: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(read) = std::fs::read_dir(dir) else { return found };
    let mut children: Vec<_> = read.flatten().map(|e| e.path()).collect();
    children.sort();
    for path in children {
        if path.extension().is_some_and(|e| e == "app") {
            found.push(path);
        } else if depth > 1 && path.is_dir() {
            found.extend(app_bundles(&path, depth - 1));
        }
    }
    found
}

fn bundle_for(path: &Path) -> Option<Retained<NSBundle>> {
    NSBundle::bundleWithPath(&NSString::from_str(&path.to_string_lossy()))
}

fn bundle_identifier(bundle: &NSBundle) -> Option<String> {
    bundle.bundleIdentifier().map(|s| s.to_string()).filter(|s| !s.is_empty())
}

/// The (localized) string under `key` in the bundle's `Info.plist`.
fn string_key(bundle: &NSBundle, key: &str) -> Option<String> {
    let value = bundle.objectForInfoDictionaryKey(&NSString::from_str(key))?;
    let text = value.downcast::<NSString>().ok()?.to_string();
    let text = text.trim().to_owned();
    (!text.is_empty()).then_some(text)
}

fn bool_key(bundle: &NSBundle, key: &str) -> bool {
    let Some(value) = bundle.objectForInfoDictionaryKey(&NSString::from_str(key)) else {
        return false;
    };
    match value.downcast::<NSNumber>() {
        Ok(number) => number.boolValue(),
        Err(other) => other
            .downcast::<NSString>()
            .is_ok_and(|s| matches!(s.to_string().as_str(), "1" | "YES" | "true")),
    }
}

/// `EXAppExtensionAttributes.EXExtensionPointIdentifier` of an app extension.
fn extension_point(bundle: &NSBundle) -> Option<String> {
    let attributes =
        bundle.objectForInfoDictionaryKey(&NSString::from_str("EXAppExtensionAttributes"))?;
    let attributes = attributes.downcast::<NSDictionary>().ok()?;
    let value = attributes.objectForKey(&NSString::from_str("EXExtensionPointIdentifier"))?;
    Some(value.downcast::<NSString>().ok()?.to_string())
}

/// Draws the icon `NSWorkspace` has for `path` (the generic one when the bundle has none) into a
/// 64 x 64 bitmap and saves it as PNG, named by the hash of its pixels.
fn render_icon(path: &Path, icon_dir: &Path) -> Option<PathBuf> {
    autoreleasepool(|_| {
        let image = NSWorkspace::sharedWorkspace()
            .iconForFile(&NSString::from_str(&path.to_string_lossy()));
        let mut rect =
            CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(ICON_SIZE as f64, ICON_SIZE as f64));
        // SAFETY: `rect` is a valid pointer for the duration of the call; no context or hints.
        let cg_image =
            unsafe { image.CGImageForProposedRect_context_hints(&mut rect, None, None)? };

        let mut pixels = vec![0u8; ICON_SIZE * ICON_SIZE * 4];
        let space = CGColorSpace::new_device_rgb()?;
        // SAFETY: the buffer holds ICON_SIZE rows of ICON_SIZE * 4 bytes and outlives the context,
        // which is dropped before the buffer is read.
        let context = unsafe {
            CGBitmapContextCreate(
                pixels.as_mut_ptr().cast(),
                ICON_SIZE,
                ICON_SIZE,
                8,
                ICON_SIZE * 4,
                Some(&space),
                CGImageAlphaInfo::PremultipliedLast.0,
            )?
        };
        CGContext::draw_image(Some(&context), rect, Some(&cg_image));
        drop(context);

        let rgba = unpremultiply(pixels);
        let file = icon_dir.join(format!("{:016x}.png", fnv1a(&rgba)));
        if !file.exists() {
            write_png(&file, ICON_SIZE as u32, ICON_SIZE as u32, &rgba).ok()?;
        }
        Some(file)
    })
}

/// Drawing gives premultiplied alpha; PNG wants straight alpha.
pub fn unpremultiply(mut pixels: Vec<u8>) -> Vec<u8> {
    for p in pixels.chunks_exact_mut(4) {
        let alpha = u32::from(p[3]);
        if alpha != 0 && alpha != 255 {
            for channel in &mut p[..3] {
                *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    pixels
}

fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), png::EncodingError> {
    let file = std::fs::File::create(path)?;
    let mut encoder = png::Encoder::new(io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(rgba)
}

/// FNV-1a: a stable file name for equal pixels, without a hashing dependency.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn open_application(path: &Path) -> io::Result<()> {
    if !path.exists() {
        return Err(io::Error::new(io::ErrorKind::NotFound, format!("{} is gone", path.display())));
    }
    autoreleasepool(|_| {
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        let configuration = NSWorkspaceOpenConfiguration::configuration();
        NSWorkspace::sharedWorkspace().openApplicationAtURL_configuration_completionHandler(
            &url,
            &configuration,
            None,
        );
    });
    Ok(())
}

fn open_url(url: &str) -> io::Result<()> {
    autoreleasepool(|_| {
        let url = NSURL::URLWithString(&NSString::from_str(url))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a URL"))?;
        if NSWorkspace::sharedWorkspace().openURL(&url) {
            Ok(())
        } else {
            Err(io::Error::other("the system refused to open the URL"))
        }
    })
}

fn reveal_in_finder(path: &Path) -> io::Result<()> {
    autoreleasepool(|_| {
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        NSWorkspace::sharedWorkspace()
            .activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[url]));
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn straight_alpha_is_restored() {
        // 50 % alpha: a premultiplied 64 is a straight 128 (rounded).
        assert_eq!(unpremultiply(vec![64, 32, 0, 128]), [128, 64, 0, 128]);
        assert_eq!(unpremultiply(vec![10, 20, 30, 255]), [10, 20, 30, 255]);
        assert_eq!(unpremultiply(vec![0, 0, 0, 0]), [0, 0, 0, 0]);
    }

    #[test]
    fn bundles_are_found_in_folders_but_never_inside_bundles() {
        let dir = tempfile::tempdir().unwrap();
        for path in [
            "A.app/Contents",
            "Utilities/B.app/Contents",
            "A.app/Contents/Resources/Nested.app",
            "Deep/More/C.app",
        ] {
            std::fs::create_dir_all(dir.path().join(path)).unwrap();
        }
        let names = |depth| -> Vec<String> {
            app_bundles(dir.path(), depth)
                .iter()
                .map(|p| p.strip_prefix(dir.path()).unwrap().display().to_string())
                .collect()
        };
        assert_eq!(names(1), ["A.app"]);
        assert_eq!(names(2), ["A.app", "Utilities/B.app"]);
        assert_eq!(names(3), ["A.app", "Deep/More/C.app", "Utilities/B.app"]);
    }

    #[test]
    fn equal_pixels_give_equal_names() {
        assert_eq!(fnv1a(b"abc"), fnv1a(b"abc"));
        assert_ne!(fnv1a(b"abc"), fnv1a(b"abd"));
    }
}
