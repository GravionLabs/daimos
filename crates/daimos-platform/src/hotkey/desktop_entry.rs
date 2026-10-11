//! The portal identifies an application by its desktop file. A packaged install ships
//! `dev.gravionlabs.daimos.desktop`; a build run from a checkout has none, and the portal then
//! answers "An app id is required". The app writes a user-level entry for itself in that case.

use std::io;
use std::path::{Path, PathBuf};

use super::portal::APP_ID;

fn content(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Daimos\nComment=Command palette\n\
         Exec={exe}\nTerminal=false\nNoDisplay=true\nCategories=Utility;\n",
        exe = quote(exe)
    )
}

/// Quotes a path for the `Exec` key (desktop entry specification, "Quoting").
fn quote(path: &Path) -> String {
    let text = path.to_string_lossy();
    if text.chars().any(|c| " \t\n\"'\\><~|&;$*?#()`".contains(c)) {
        let escaped: String = text
            .chars()
            .flat_map(|c| match c {
                '"' | '`' | '$' | '\\' => vec!['\\', c],
                c => vec![c],
            })
            .collect();
        format!("\"{escaped}\"")
    } else {
        text.into_owned()
    }
}

fn file_name() -> String {
    format!("{APP_ID}.desktop")
}

fn exists_in(dirs: &[PathBuf]) -> bool {
    dirs.iter().any(|dir| dir.join("applications").join(file_name()).exists())
}

/// Writes `<data_home>/applications/dev.gravionlabs.daimos.desktop` unless an entry with that
/// name exists in `data_home` or one of `data_dirs`. Returns whether it wrote one.
pub fn ensure(exe: &Path, data_home: &Path, data_dirs: &[PathBuf]) -> io::Result<bool> {
    let mut search = vec![data_home.to_path_buf()];
    search.extend(data_dirs.iter().cloned());
    if exists_in(&search) {
        return Ok(false);
    }
    let dir = data_home.join("applications");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(file_name()), content(exe))?;
    Ok(true)
}

/// [`ensure`] for the running executable and the user's XDG directories.
pub fn ensure_for_current_exe() -> io::Result<bool> {
    let exe = std::env::current_exe()?;
    let home = dirs_data_home()?;
    let system = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".into())
        .split(':')
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    ensure(&exe, &home, &system)
}

fn dirs_data_home() -> io::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME").filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".local/share"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no HOME"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_an_entry_that_points_at_the_executable() {
        let home = tempfile::tempdir().unwrap();
        let wrote = ensure(Path::new("/opt/daimos/daimos-app"), home.path(), &[]).unwrap();
        assert!(wrote);
        let text = std::fs::read_to_string(
            home.path().join("applications/dev.gravionlabs.daimos.desktop"),
        )
        .unwrap();
        assert!(text.contains("Exec=/opt/daimos/daimos-app\n"), "{text}");
        assert!(text.starts_with("[Desktop Entry]\n"));
    }

    #[test]
    fn leaves_a_packaged_entry_alone() {
        let home = tempfile::tempdir().unwrap();
        let system = tempfile::tempdir().unwrap();
        let apps = system.path().join("applications");
        std::fs::create_dir_all(&apps).unwrap();
        std::fs::write(apps.join("dev.gravionlabs.daimos.desktop"), "packaged").unwrap();
        let wrote = ensure(Path::new("/x"), home.path(), &[system.path().to_path_buf()]).unwrap();
        assert!(!wrote);
        assert!(!home.path().join("applications").exists());
    }

    #[test]
    fn quotes_paths_with_spaces() {
        assert_eq!(quote(Path::new("/home/a b/daimos")), "\"/home/a b/daimos\"");
        assert_eq!(quote(Path::new("/usr/bin/daimos")), "/usr/bin/daimos");
    }
}
