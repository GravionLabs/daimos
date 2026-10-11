//! The Windows application source against the real `shell:AppsFolder` of the runner.
#![cfg(windows)]

use daimos_core::apps::AppSource;
use daimos_platform::apps::windows::WindowsAppSource;

fn scan() -> (tempfile::TempDir, Vec<daimos_core::apps::AppEntry>) {
    let icons = tempfile::tempdir().unwrap();
    let apps = WindowsAppSource::new(icons.path()).scan();
    (icons, apps)
}

#[test]
fn notepad_and_settings_are_found_with_ids_and_exec_paths() {
    let (_icons, apps) = scan();
    let names: Vec<_> = apps.iter().map(|a| a.name.as_str()).collect();
    for wanted in ["Notepad", "Settings"] {
        let app = apps
            .iter()
            .find(|a| a.name == wanted)
            .unwrap_or_else(|| panic!("{wanted} not found among {names:?}"));
        assert!(app.id.starts_with("windows:"), "{}", app.id);
        assert!(app.exec.starts_with(r"shell:AppsFolder\"), "{}", app.exec);
    }
}

#[test]
fn the_list_is_sorted_and_free_of_duplicates() {
    let (_icons, apps) = scan();
    assert!(!apps.is_empty());
    let lower: Vec<_> = apps.iter().map(|a| a.name.to_lowercase()).collect();
    let mut sorted = lower.clone();
    sorted.sort();
    assert_eq!(lower, sorted);
    let mut ids: Vec<_> = apps.iter().map(|a| a.id.clone()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), apps.len());
}

#[test]
fn icons_are_64_pixel_pngs_in_the_cache() {
    let (icons, apps) = scan();
    let with_icon: Vec<_> = apps.iter().filter_map(|a| a.icon.as_ref()).collect();
    assert!(!with_icon.is_empty(), "no app has an icon");
    for icon in with_icon.iter().take(5) {
        assert!(icon.starts_with(icons.path()), "{}", icon.display());
        let bytes = std::fs::read(icon).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "{}", icon.display());
        // The IHDR chunk holds the size right after the 8-byte signature and 8 bytes of chunk header.
        let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
        assert!((16..=256).contains(&width) && width == height, "{width}x{height}");
    }
}

#[test]
fn classic_apps_offer_run_as_administrator_and_packaged_apps_do_not() {
    let (_icons, apps) = scan();
    for app in &apps {
        let packaged = app.id.contains('!');
        let has_runas = app.actions.iter().any(|a| a.id == "runas");
        assert_eq!(has_runas, !packaged, "{} ({})", app.name, app.id);
    }
}

#[test]
fn only_classic_apps_with_a_file_can_be_shown_in_explorer() {
    let (_icons, apps) = scan();
    for app in &apps {
        let reveal = app.actions.iter().find(|a| a.id == "reveal");
        if app.id.contains('!') {
            assert!(reveal.is_none(), "{} ({})", app.name, app.id);
        } else if let Some(reveal) = reveal {
            assert!(std::path::Path::new(&reveal.exec).exists(), "{}: {}", app.name, reveal.exec);
        }
    }
}

#[test]
fn an_unknown_action_is_refused() {
    let (_icons, apps) = scan();
    let error =
        WindowsAppSource::new(std::env::temp_dir()).launch(&apps[0], Some("nope")).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
}
