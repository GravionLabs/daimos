//! The Linux application source against fixture `.desktop` files.
#![cfg(target_os = "linux")]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use daimos_core::apps::{AppEntry, AppSource};
use daimos_platform::apps::linux::LinuxAppSource;

fn write(dir: &Path, name: &str, content: &str) {
    let path = dir.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn executable(dir: &Path, name: &str, script: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, script).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn source(dir: &Path) -> LinuxAppSource {
    LinuxAppSource::with_dirs(vec![dir.to_owned()])
        .locales(vec!["en".into()])
        .desktops(vec!["GNOME".into()])
        .search_path(vec![dir.to_owned()])
}

fn names(apps: &[AppEntry]) -> Vec<&str> {
    apps.iter().map(|a| a.name.as_str()).collect()
}

const FIREFOX: &str = "[Desktop Entry]
Type=Application
Name=Firefox
Name[de]=Feuerfuchs
GenericName=Web Browser
GenericName[de]=Webbrowser
Keywords=web;internet;
Keywords[de]=netz;
Exec=firefox %u
Actions=new-window;private;

[Desktop Action new-window]
Name=Open a New Window
Name[de]=Neues Fenster
Exec=firefox --new-window %u

[Desktop Action private]
Name=Open a New Private Window
Exec=firefox --private-window %u
";

#[test]
fn names_keywords_and_exec_are_read_and_localized() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "firefox.desktop", FIREFOX);

    let apps = source(dir.path()).scan();
    assert_eq!(apps.len(), 1);
    let app = &apps[0];
    assert_eq!(app.id, "linux:firefox.desktop");
    assert_eq!(app.name, "Firefox");
    assert_eq!(app.generic_name.as_deref(), Some("Web Browser"));
    assert_eq!(app.keywords, ["web", "internet"]);
    assert_eq!(app.exec, "firefox");
    assert!(!app.terminal);

    let german = LinuxAppSource::with_dirs(vec![dir.path().to_owned()])
        .locales(vec!["de_DE".into(), "de".into()])
        .desktops(vec!["GNOME".into()])
        .scan();
    assert_eq!(german[0].name, "Feuerfuchs");
    assert_eq!(german[0].generic_name.as_deref(), Some("Webbrowser"));
    assert_eq!(german[0].keywords, ["netz"]);
    // The id does not follow the display name, so frecency survives a translation change.
    assert_eq!(german[0].id, "linux:firefox.desktop");
}

#[test]
fn desktop_actions_become_actions() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "firefox.desktop", FIREFOX);
    let app = &source(dir.path()).scan()[0];
    let actions: Vec<_> =
        app.actions.iter().map(|a| (a.id.as_str(), a.name.as_str(), a.exec.as_str())).collect();
    assert_eq!(
        actions,
        [
            ("new-window", "Open a New Window", "firefox --new-window"),
            ("private", "Open a New Private Window", "firefox --private-window"),
        ]
    );
}

#[test]
fn hidden_entries_are_not_listed() {
    let dir = tempfile::tempdir().unwrap();
    let entry =
        |extra: &str| format!("[Desktop Entry]\nType=Application\nName=X\nExec=x\n{extra}\n");
    write(dir.path(), "shown.desktop", &entry("").replace("Name=X", "Name=Shown"));
    write(dir.path(), "nodisplay.desktop", &entry("NoDisplay=true"));
    write(dir.path(), "hidden.desktop", &entry("Hidden=true"));
    write(dir.path(), "kde-only.desktop", &entry("OnlyShowIn=KDE;"));
    write(dir.path(), "not-gnome.desktop", &entry("NotShowIn=GNOME;"));
    write(
        dir.path(),
        "link.desktop",
        "[Desktop Entry]\nType=Link\nName=Link\nURL=https://example.org\n",
    );
    write(dir.path(), "noexec.desktop", "[Desktop Entry]\nType=Application\nName=No Exec\n");

    assert_eq!(names(&source(dir.path()).scan()), ["Shown"]);

    // On KDE the KDE-only entry appears and the GNOME-excluding one too.
    let kde = source(dir.path()).desktops(vec!["KDE".into()]).scan();
    assert_eq!(names(&kde), ["Shown", "X", "X"]);
}

#[test]
fn only_show_in_matches_any_listed_desktop_of_a_stacked_session() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "a.desktop",
        "[Desktop Entry]\nType=Application\nName=A\nExec=a\nOnlyShowIn=Unity;GNOME;\n",
    );
    let apps = source(dir.path()).desktops(vec!["ubuntu".into(), "gnome".into()]).scan();
    assert_eq!(names(&apps), ["A"]);
}

#[test]
fn try_exec_must_exist() {
    let dir = tempfile::tempdir().unwrap();
    let bin = tempfile::tempdir().unwrap();
    executable(bin.path(), "present", "#!/bin/sh\n");
    let entry = |name: &str, try_exec: &str| {
        format!("[Desktop Entry]\nType=Application\nName={name}\nExec=x\nTryExec={try_exec}\n")
    };
    write(dir.path(), "a.desktop", &entry("OnPath", "present"));
    write(dir.path(), "b.desktop", &entry("Missing", "absent-program"));
    write(
        dir.path(),
        "c.desktop",
        &entry("Absolute", bin.path().join("present").to_str().unwrap()),
    );
    write(dir.path(), "d.desktop", &entry("AbsoluteMissing", "/nonexistent/program"));

    let apps = source(dir.path()).search_path(vec![bin.path().to_owned()]).scan();
    assert_eq!(names(&apps), ["Absolute", "OnPath"]);
}

#[test]
fn the_first_directory_wins_and_subdirectories_join_the_id_with_a_dash() {
    let user = tempfile::tempdir().unwrap();
    let system = tempfile::tempdir().unwrap();
    let entry = |name: &str| format!("[Desktop Entry]\nType=Application\nName={name}\nExec=x\n");
    write(user.path(), "app.desktop", &entry("User App"));
    write(system.path(), "app.desktop", &entry("System App"));
    write(system.path(), "kde/tool.desktop", &entry("KDE Tool"));

    let apps = LinuxAppSource::with_dirs(vec![user.path().to_owned(), system.path().to_owned()])
        .locales(vec!["en".into()])
        .scan();
    let found: Vec<_> = apps.iter().map(|a| (a.id.as_str(), a.name.as_str())).collect();
    assert_eq!(found, [("linux:kde-tool.desktop", "KDE Tool"), ("linux:app.desktop", "User App")]);
}

#[test]
fn the_result_is_sorted_by_name_and_missing_directories_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let entry = |name: &str| format!("[Desktop Entry]\nType=Application\nName={name}\nExec=x\n");
    write(dir.path(), "1.desktop", &entry("zeta"));
    write(dir.path(), "2.desktop", &entry("Alpha"));
    write(dir.path(), "3.desktop", &entry("beta"));
    let apps =
        LinuxAppSource::with_dirs(vec![PathBuf::from("/nonexistent/dir"), dir.path().to_owned()])
            .locales(vec!["en".into()])
            .scan();
    assert_eq!(names(&apps), ["Alpha", "beta", "zeta"]);
}

#[test]
fn an_absolute_icon_path_is_kept_and_a_missing_one_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let icon = dir.path().join("icon.png");
    fs::write(&icon, b"png").unwrap();
    let entry = |name: &str, icon: &str| {
        format!("[Desktop Entry]\nType=Application\nName={name}\nExec=x\nIcon={icon}\n")
    };
    write(dir.path(), "a.desktop", &entry("With", icon.to_str().unwrap()));
    write(dir.path(), "b.desktop", &entry("Without", "/nonexistent/icon.png"));

    let apps = source(dir.path()).scan();
    assert_eq!(apps[0].icon.as_deref(), Some(icon.as_path()));
    // The fallback comes from the icon theme of the machine: a file or nothing, never the missing path.
    assert_ne!(apps[1].icon.as_deref(), Some(Path::new("/nonexistent/icon.png")));
}

fn wait_for(path: &Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(text) = fs::read_to_string(path)
            && !text.is_empty()
        {
            return text;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("{} was not written", path.display());
}

#[test]
fn launch_runs_the_command_and_its_actions() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.txt");
    let script =
        executable(dir.path(), "app", &format!("#!/bin/sh\necho \"$@\" > {}\n", out.display()));
    write(
        dir.path(),
        "app.desktop",
        &format!(
            "[Desktop Entry]\nType=Application\nName=App\nExec={s} plain %U\nActions=extra;\n\n[Desktop Action extra]\nName=Extra\nExec={s} --extra \"two words\"\n",
            s = script.display()
        ),
    );
    let source = source(dir.path());
    let app = source.scan().remove(0);

    source.launch(&app, None).unwrap();
    assert_eq!(wait_for(&out).trim(), "plain");
    fs::remove_file(&out).unwrap();

    source.launch(&app, Some("extra")).unwrap();
    assert_eq!(wait_for(&out).trim(), "--extra two words");

    let missing = source.launch(&app, Some("nope")).unwrap_err();
    assert_eq!(missing.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn a_terminal_app_runs_inside_the_terminal_emulator() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.txt");
    // A fake emulator: `x-terminal-emulator -e <command...>` runs the command.
    executable(dir.path(), "x-terminal-emulator", "#!/bin/sh\nshift\nexec \"$@\"\n");
    let tool = executable(
        dir.path(),
        "tool",
        &format!("#!/bin/sh\necho \"in terminal $1\" > {}\n", out.display()),
    );
    write(
        dir.path(),
        "tool.desktop",
        &format!(
            "[Desktop Entry]\nType=Application\nName=Tool\nExec={} arg\nTerminal=true\n",
            tool.display()
        ),
    );
    let source = source(dir.path());
    let app = source.scan().remove(0);
    assert!(app.terminal);

    // `$TERMINAL` could point elsewhere on the machine running the test.
    // SAFETY: no other test in this binary reads or writes TERMINAL.
    unsafe { std::env::remove_var("TERMINAL") };
    source.launch(&app, None).unwrap();
    assert_eq!(wait_for(&out).trim(), "in terminal arg");
}

#[test]
fn a_terminal_app_without_any_emulator_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "t.desktop",
        "[Desktop Entry]\nType=Application\nName=T\nExec=t\nTerminal=true\n",
    );
    // SAFETY: no other test in this binary reads or writes TERMINAL.
    unsafe { std::env::remove_var("TERMINAL") };
    let source = source(dir.path());
    let app = source.scan().remove(0);
    assert_eq!(source.launch(&app, None).unwrap_err().kind(), std::io::ErrorKind::NotFound);
}
