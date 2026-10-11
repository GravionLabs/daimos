//! Applications from `shell:AppsFolder`: classic desktop programs (Start Menu shortcuts) and
//! packaged (MSIX/UWP) apps appear in the same folder.
//!
//! Every item has a display name and a parsing name: the `AppUserModelID` of a packaged app
//! (`Package_family!App`), or a known-folder path for a classic one. `shell:AppsFolder\<parsing
//! name>` launches either kind. Icons are rendered with `IShellItemImageFactory` to 64 px PNGs.

use std::io;
use std::path::{Path, PathBuf};

use daimos_core::apps::{AppAction, AppEntry, AppSource};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
    DeleteObject, GetDIBits, GetObjectW, HBITMAP, HGDIOBJ,
};
use windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    CoUninitialize,
};
use windows::Win32::UI::Shell::{
    ApplicationActivationManager, BHID_EnumItems, FOLDERID_AppsFolder,
    IApplicationActivationManager, IEnumShellItems, IShellItem, IShellItemImageFactory,
    KF_FLAG_DEFAULT, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, SHGetKnownFolderItem, SIGDN_FILESYSPATH,
    SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
    ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR};

/// The id of the action that selects the shortcut in Explorer; its `exec` is the file path.
const REVEAL: &str = "reveal";
const ICON_SIZE: i32 = 64;
const AO_NONE: windows::Win32::UI::Shell::ACTIVATEOPTIONS =
    windows::Win32::UI::Shell::ACTIVATEOPTIONS(0);

pub struct WindowsAppSource {
    /// PNG icons, named by the hash of their pixels.
    icon_dir: PathBuf,
}

impl WindowsAppSource {
    pub fn new(icon_dir: impl Into<PathBuf>) -> Self {
        Self { icon_dir: icon_dir.into() }
    }
}

impl AppSource for WindowsAppSource {
    fn id(&self) -> &str {
        "windows"
    }

    fn scan(&self) -> Vec<AppEntry> {
        let icon_dir = self.icon_dir.clone();
        on_sta_thread(move || enumerate(&icon_dir)).unwrap_or_else(|error| {
            log::warn!("cannot list the applications folder: {error}");
            Vec::new()
        })
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        for (var, sub) in [
            ("ProgramData", r"Microsoft\Windows\Start Menu\Programs"),
            ("APPDATA", r"Microsoft\Windows\Start Menu\Programs"),
        ] {
            if let Some(base) = std::env::var_os(var) {
                let path = Path::new(&base).join(sub);
                if path.is_dir() {
                    paths.push(path);
                }
            }
        }
        paths
    }

    fn launch(&self, app: &AppEntry, action: Option<&str>) -> io::Result<()> {
        let parsing_name = app
            .id
            .strip_prefix("windows:")
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a Windows app id"))?
            .to_owned();
        if action == Some(REVEAL)
            && let Some(reveal) = app.actions.iter().find(|a| a.id == REVEAL)
        {
            return reveal_in_explorer(&reveal.exec);
        }
        let runas = match action {
            None => false,
            Some("runas") if app.actions.iter().any(|a| a.id == "runas") => true,
            Some(other) => {
                return Err(io::Error::new(io::ErrorKind::NotFound, format!("no action {other}")));
            }
        };
        on_sta_thread(move || launch_item(&parsing_name, runas))
            .map_err(io::Error::other)?
            .map_err(io::Error::other)
    }
}

/// Runs `work` on a new thread with COM initialized as a single-threaded apartment, which the
/// shell objects require.
fn on_sta_thread<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> windows::core::Result<T> {
    std::thread::spawn(move || {
        // SAFETY: balanced with CoUninitialize below; the thread owns no COM objects past it.
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
        let result = work();
        // SAFETY: matches the successful CoInitializeEx above.
        unsafe { CoUninitialize() };
        Ok(result)
    })
    .join()
    .map_err(|_| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))?
}

fn enumerate(icon_dir: &Path) -> Vec<AppEntry> {
    let items = match app_items() {
        Ok(items) => items,
        Err(error) => {
            log::warn!("cannot open shell:AppsFolder: {error}");
            return Vec::new();
        }
    };
    if let Err(error) = std::fs::create_dir_all(icon_dir) {
        log::warn!("cannot create the icon cache {}: {error}", icon_dir.display());
    }
    let mut apps: Vec<AppEntry> = items
        .iter()
        .filter_map(|item| {
            let name = display_name(item, SIGDN_NORMALDISPLAY)?.trim().to_owned();
            let parsing_name = display_name(item, SIGDN_PARENTRELATIVEPARSING)?;
            if name.is_empty() || parsing_name.is_empty() {
                return None;
            }
            let packaged = is_packaged(&parsing_name);
            Some(AppEntry {
                id: format!("windows:{parsing_name}"),
                name,
                generic_name: None,
                keywords: Vec::new(),
                exec: apps_folder_path(&parsing_name),
                terminal: false,
                icon: render_icon(item, icon_dir),
                actions: if packaged {
                    Vec::new()
                } else {
                    let mut actions = vec![AppAction {
                        id: "runas".into(),
                        name: "Run as administrator".into(),
                        exec: apps_folder_path(&parsing_name),
                    }];
                    // Only items that are a file (a Start Menu shortcut) can be shown in Explorer.
                    if let Some(path) =
                        display_name(item, SIGDN_FILESYSPATH).filter(|p| !p.is_empty())
                    {
                        actions.push(AppAction {
                            id: REVEAL.into(),
                            name: "Show in Explorer".into(),
                            exec: path,
                        });
                    }
                    actions
                },
            })
        })
        .collect();
    apps.sort_by(|a, b| {
        a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.id.cmp(&b.id))
    });
    apps.dedup_by(|a, b| a.id == b.id);
    apps
}

fn app_items() -> windows::core::Result<Vec<IShellItem>> {
    // SAFETY: COM is initialized on this thread; the folder id is a static GUID.
    unsafe {
        let folder: IShellItem = SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)?;
        let children: IEnumShellItems = folder.BindToHandler(None, &BHID_EnumItems)?;
        let mut items = Vec::new();
        loop {
            let mut batch = [None];
            let mut fetched = 0;
            children.Next(&mut batch, Some(&mut fetched))?;
            match batch[0].take() {
                Some(item) if fetched == 1 => items.push(item),
                _ => break,
            }
        }
        Ok(items)
    }
}

fn display_name(item: &IShellItem, kind: windows::Win32::UI::Shell::SIGDN) -> Option<String> {
    // SAFETY: the returned string is allocated by the shell and freed here.
    unsafe {
        let name = item.GetDisplayName(kind).ok()?;
        let text = name.to_string().ok();
        CoTaskMemFree(Some(name.0 as *const _));
        text
    }
}

/// Packaged apps have an `AppUserModelID` of the form `PackageFamilyName!AppId`.
pub fn is_packaged(parsing_name: &str) -> bool {
    parsing_name.contains('!')
}

pub fn apps_folder_path(parsing_name: &str) -> String {
    format!(r"shell:AppsFolder\{parsing_name}")
}

/// Renders the icon of `item` to a PNG in `icon_dir`; the file name is the hash of the pixels, so
/// equal icons share a file and an unchanged icon is not written again.
fn render_icon(item: &IShellItem, icon_dir: &Path) -> Option<PathBuf> {
    // SAFETY: the bitmap handle is owned here and deleted before returning.
    unsafe {
        let factory: IShellItemImageFactory = windows::core::Interface::cast(item).ok()?;
        let size = windows::Win32::Foundation::SIZE { cx: ICON_SIZE, cy: ICON_SIZE };
        let bitmap = factory.GetImage(size, SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK).ok()?;
        let pixels = bitmap_rgba(bitmap);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let (width, height, rgba) = pixels?;
        let path = icon_dir.join(format!("{:016x}.png", fnv1a(&rgba)));
        if !path.exists() {
            write_png(&path, width, height, &rgba).ok()?;
        }
        Some(path)
    }
}

/// The pixels of a 32-bit bitmap as top-down RGBA.
///
/// # Safety
/// `bitmap` must be a valid bitmap handle.
unsafe fn bitmap_rgba(bitmap: HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
    // SAFETY: per the contract; the device context is created and deleted here.
    unsafe {
        let mut info = BITMAP::default();
        if GetObjectW(
            HGDIOBJ(bitmap.0),
            size_of::<BITMAP>() as i32,
            Some(&mut info as *mut _ as *mut _),
        ) == 0
        {
            return None;
        }
        let (width, height) = (info.bmWidth, info.bmHeight);
        if width <= 0 || height <= 0 {
            return None;
        }
        let mut header = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                // A negative height asks for a top-down bitmap.
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bgra = vec![0u8; (width * height * 4) as usize];
        let dc = CreateCompatibleDC(None);
        let lines = GetDIBits(
            dc,
            bitmap,
            0,
            height as u32,
            Some(bgra.as_mut_ptr() as *mut _),
            &mut header,
            DIB_RGB_COLORS,
        );
        let _ = DeleteDC(dc);
        (lines != 0).then(|| (width as u32, height as u32, bgra_to_rgba(bgra)))
    }
}

/// Swaps blue and red. Icons from the shell carry straight alpha; a bitmap without any alpha
/// (all zero) is opaque.
pub fn bgra_to_rgba(mut pixels: Vec<u8>) -> Vec<u8> {
    let has_alpha = pixels.chunks_exact(4).any(|p| p[3] != 0);
    for p in pixels.chunks_exact_mut(4) {
        p.swap(0, 2);
        if !has_alpha {
            p[3] = 255;
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

fn reveal_in_explorer(path: &str) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    // `raw_arg`: Explorer wants `/select,"C:\a b"`, not a quoted `"/select,C:\a b"`.
    std::process::Command::new("explorer.exe").raw_arg(select_argument(path)).spawn().map(drop)
}

/// The Explorer argument that opens the folder of `path` with the file selected.
pub fn select_argument(path: &str) -> String {
    format!("/select,\"{path}\"")
}

fn launch_item(parsing_name: &str, runas: bool) -> windows::core::Result<()> {
    let target = apps_folder_path(parsing_name);
    if !runas && is_packaged(parsing_name) {
        // SAFETY: COM is initialized on this thread.
        let activated = unsafe {
            CoCreateInstance::<_, IApplicationActivationManager>(
                &ApplicationActivationManager,
                None,
                CLSCTX_LOCAL_SERVER,
            )
            .and_then(|manager| {
                manager.ActivateApplication(&HSTRING::from(parsing_name), PCWSTR::null(), AO_NONE)
            })
        };
        match activated {
            Ok(_process_id) => return Ok(()),
            Err(error) => log::warn!("activating {parsing_name} failed ({error}); using the shell"),
        }
    }
    shell_execute(&target, runas)
}

fn shell_execute(target: &str, runas: bool) -> windows::core::Result<()> {
    let file = HSTRING::from(target);
    let verb = HSTRING::from("runas");
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC,
        hwnd: HWND::default(),
        lpVerb: if runas { PCWSTR(verb.as_ptr()) } else { PCWSTR::null() },
        lpFile: PCWSTR(file.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: `info` and the strings it points to outlive the call.
    unsafe { ShellExecuteExW(&mut info) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explorer_selects_the_quoted_path() {
        assert_eq!(select_argument(r"C:\Users\me\a b.lnk"), r#"/select,"C:\Users\me\a b.lnk""#);
    }

    #[test]
    fn packaged_apps_have_a_bang_in_their_app_user_model_id() {
        assert!(is_packaged("Microsoft.WindowsNotepad_8wekyb3d8bbwe!App"));
        assert!(!is_packaged(
            r"{7C5A40EF-A0FB-4BFC-874A-C0F2E0B9FA8E}\Mozilla Firefox\firefox.exe"
        ));
    }

    #[test]
    fn the_launch_path_is_in_the_apps_folder() {
        assert_eq!(apps_folder_path("A!B"), r"shell:AppsFolder\A!B");
    }

    #[test]
    fn colors_are_swapped_and_opaque_when_the_bitmap_has_no_alpha() {
        assert_eq!(bgra_to_rgba(vec![1, 2, 3, 0, 4, 5, 6, 0]), [3, 2, 1, 255, 6, 5, 4, 255]);
        assert_eq!(bgra_to_rgba(vec![1, 2, 3, 128, 4, 5, 6, 0]), [3, 2, 1, 128, 6, 5, 4, 0]);
    }

    #[test]
    fn equal_pixels_give_equal_names() {
        assert_eq!(fnv1a(b"abc"), fnv1a(b"abc"));
        assert_ne!(fnv1a(b"abc"), fnv1a(b"abd"));
    }
}
