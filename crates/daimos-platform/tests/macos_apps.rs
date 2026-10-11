//! The macOS application source against the real machine and against fixture bundles.
#![cfg(target_os = "macos")]

use std::fs;
use std::path::{Path, PathBuf};

use daimos_core::apps::{AppEntry, AppSource};
use daimos_platform::apps::macos::MacAppSource;

fn png_size(path: &Path) -> (u32, u32) {
    let bytes = fs::read(path).unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "{}", path.display());
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    (width, height)
}

fn bundle(dir: &Path, folder: &str, plist_body: &str) -> PathBuf {
    let app = dir.join(folder);
    fs::create_dir_all(app.join("Contents")).unwrap();
    fs::write(
        app.join("Contents/Info.plist"),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>{plist_body}</dict></plist>\n"
        ),
    )
    .unwrap();
    app
}

fn find<'a>(apps: &'a [AppEntry], name: &str) -> &'a AppEntry {
    let names: Vec<_> = apps.iter().map(|a| a.name.as_str()).collect();
    apps.iter()
        .find(|a| a.name == name)
        .unwrap_or_else(|| panic!("{name} not found among {names:?}"))
}

#[test]
fn safari_and_system_settings_are_found_with_icons() {
    let icons = tempfile::tempdir().unwrap();
    let apps = MacAppSource::new(icons.path()).scan();
    for name in ["Safari", "System Settings"] {
        let app = find(&apps, name);
        let icon = app.icon.as_ref().unwrap_or_else(|| panic!("{name} has no icon"));
        assert!(icon.starts_with(icons.path()));
        assert_eq!(png_size(icon), (64, 64), "{name}");
        assert!(app.id.starts_with("macos:com.apple."), "{}", app.id);
        assert!(app.actions.iter().any(|a| a.id == "reveal" && a.name == "Show in Finder"));
    }
}

#[test]
fn the_list_is_sorted_and_free_of_duplicates() {
    let icons = tempfile::tempdir().unwrap();
    let apps = MacAppSource::new(icons.path()).scan();
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
fn system_settings_panes_are_items_with_the_settings_url() {
    let icons = tempfile::tempdir().unwrap();
    let apps = MacAppSource::new(icons.path()).scan();
    let panes: Vec<_> =
        apps.iter().filter(|a| a.exec.starts_with("x-apple.systempreferences:")).collect();
    assert!(!panes.is_empty(), "no System Settings pane found");
    assert!(
        panes
            .iter()
            .all(|p| p.id.starts_with("macos:settings:") && p.name.ends_with("(System Settings)"))
    );
}

#[test]
fn names_identifiers_and_hidden_agents_come_from_the_bundle() {
    let dir = tempfile::tempdir().unwrap();
    let icons = tempfile::tempdir().unwrap();
    bundle(
        dir.path(),
        "Plain.app",
        "<key>CFBundleName</key><string>Plain Name</string><key>CFBundleIdentifier</key><string>dev.example.plain</string>",
    );
    bundle(
        dir.path(),
        "Display.app",
        "<key>CFBundleName</key><string>Short</string><key>CFBundleDisplayName</key><string>Display Name</string><key>CFBundleIdentifier</key><string>dev.example.display</string>",
    );
    bundle(
        dir.path(),
        "Agent.app",
        "<key>CFBundleName</key><string>Agent</string><key>CFBundleIdentifier</key><string>dev.example.agent</string><key>LSUIElement</key><true/>",
    );
    bundle(
        dir.path(),
        "NoName.app",
        "<key>CFBundleIdentifier</key><string>dev.example.noname</string>",
    );
    // A bundle inside a bundle is not listed.
    bundle(
        &dir.path().join("Plain.app/Contents"),
        "Nested.app",
        "<key>CFBundleName</key><string>Nested</string>",
    );
    // A folder of apps is entered.
    bundle(
        &dir.path().join("Utilities"),
        "Tool.app",
        "<key>CFBundleName</key><string>Tool</string><key>CFBundleIdentifier</key><string>dev.example.tool</string>",
    );

    let apps = MacAppSource::with_dirs(vec![(dir.path().to_owned(), 2)], icons.path()).scan();
    let found: Vec<(String, String)> =
        apps.iter().map(|a| (a.name.clone(), a.id.clone())).collect();
    let expected = [
        ("Display Name".to_owned(), "macos:dev.example.display".to_owned()),
        ("NoName".to_owned(), format!("macos:{}", dir.path().join("NoName.app").display())),
        ("Plain Name".to_owned(), "macos:dev.example.plain".to_owned()),
        ("Tool".to_owned(), "macos:dev.example.tool".to_owned()),
    ];
    assert_eq!(found, expected);
    // Apps without an icon get the generic one.
    assert!(apps.iter().all(|a| a.icon.is_some()));
}

#[test]
fn the_first_directory_wins_for_the_same_bundle_identifier() {
    let user = tempfile::tempdir().unwrap();
    let system = tempfile::tempdir().unwrap();
    let icons = tempfile::tempdir().unwrap();
    let plist = |name: &str| {
        format!(
            "<key>CFBundleName</key><string>{name}</string><key>CFBundleIdentifier</key><string>dev.example.same</string>"
        )
    };
    bundle(user.path(), "App.app", &plist("User Copy"));
    bundle(system.path(), "App.app", &plist("System Copy"));
    let apps = MacAppSource::with_dirs(
        vec![(user.path().to_owned(), 1), (system.path().to_owned(), 1)],
        icons.path(),
    )
    .scan();
    assert_eq!(apps.len(), 1);
    assert_eq!(apps[0].name, "User Copy");
}

#[test]
fn an_unknown_action_and_a_missing_bundle_are_errors() {
    let icons = tempfile::tempdir().unwrap();
    let source = MacAppSource::with_dirs(vec![], icons.path());
    let app = AppEntry {
        id: "macos:gone".into(),
        name: "Gone".into(),
        generic_name: None,
        keywords: vec![],
        exec: "/nonexistent/Gone.app".into(),
        terminal: false,
        icon: None,
        actions: vec![],
    };
    assert_eq!(source.launch(&app, Some("nope")).unwrap_err().kind(), std::io::ErrorKind::NotFound);
    assert_eq!(source.launch(&app, None).unwrap_err().kind(), std::io::ErrorKind::NotFound);
}
