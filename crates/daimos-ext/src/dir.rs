//! Finding extensions in the per-platform extension directory.
//!
//! - Linux: `~/.config/daimos/extensions`
//! - Windows: `%APPDATA%\daimos\extensions`
//! - macOS: `~/Library/Application Support/daimos/extensions`

use std::fmt;
use std::path::{Path, PathBuf};

use crate::consent::{self, ConsentRecord};
use crate::error::ManifestError;
use crate::manifest::{Capabilities, Manifest};

/// The file that makes a directory an extension.
pub const MANIFEST_FILE: &str = "extension.toml";

/// `<config dir>/daimos/extensions` for a given base config directory.
pub fn extensions_dir_in(config_dir: &Path) -> PathBuf {
    config_dir.join("daimos").join("extensions")
}

/// The extension directory of the current user, `None` when the platform has no config directory.
pub fn extensions_dir() -> Option<PathBuf> {
    // `dirs::config_dir` is `~/.config`, `%APPDATA%` (Roaming) and `~/Library/Application Support`.
    dirs::config_dir().map(|dir| extensions_dir_in(&dir))
}

/// `<data dir>/logs/<extension>.log`: where the stderr of a script extension goes.
pub fn log_path_in(data_dir: &Path, extension: &str) -> PathBuf {
    data_dir.join("logs").join(format!("{extension}.log"))
}

/// An extension that loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedExtension {
    /// The extension's own directory.
    pub dir: PathBuf,
    pub manifest: Manifest,
    /// What the user confirmed, `None` before the first confirmation.
    pub consent: Option<ConsentRecord>,
}

impl LoadedExtension {
    /// The capabilities that still wait for a confirmation: everything before the first one, then
    /// what an update added. `None` when the extension may run.
    pub fn pending(&self) -> Option<Capabilities> {
        consent::pending(&self.manifest, self.consent.as_ref())
    }
}

/// Why an extension did not load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// `extension.toml` is invalid.
    Manifest(ManifestError),
    /// `extension.toml` cannot be read, or the extension clashes with another one.
    Other { file: PathBuf, message: String },
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manifest(error) => error.fmt(f),
            Self::Other { file, message } => write!(f, "{}: {message}", file.display()),
        }
    }
}

impl std::error::Error for LoadError {}

/// An extension directory that did not load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedExtension {
    pub dir: PathBuf,
    pub error: LoadError,
}

/// What a scan found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Scan {
    pub loaded: Vec<LoadedExtension>,
    pub failed: Vec<FailedExtension>,
}

/// Loads the extension in `dir`, which must contain `extension.toml`. The result carries the
/// consent, so the caller can tell whether it may run ([`LoadedExtension::pending`]).
pub fn load_extension(dir: &Path) -> Result<LoadedExtension, LoadError> {
    let file = dir.join(MANIFEST_FILE);
    let source = std::fs::read_to_string(&file)
        .map_err(|error| LoadError::Other { file: file.clone(), message: error.to_string() })?;
    let manifest = Manifest::parse(&source, &file).map_err(LoadError::Manifest)?;
    Ok(LoadedExtension { dir: dir.to_path_buf(), manifest, consent: consent::read(dir) })
}

/// Loads every `<name>/extension.toml` below `root`, in directory-name order. Entries that are not
/// extensions (files, directories without a manifest, hidden directories) are skipped; a broken
/// extension is reported in `failed` and never hides the others. A missing `root` is an empty scan.
pub fn scan(root: &Path) -> Scan {
    let mut scan = Scan::default();
    let Ok(entries) = std::fs::read_dir(root) else { return scan };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            let hidden = path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'));
            !hidden && path.is_dir() && path.join(MANIFEST_FILE).exists()
        })
        .collect();
    dirs.sort();
    for dir in dirs {
        match load_extension(&dir) {
            Ok(extension) => {
                let name = &extension.manifest.extension.name;
                match scan.loaded.iter().find(|other| other.manifest.extension.name == *name) {
                    Some(other) => scan.failed.push(FailedExtension {
                        error: LoadError::Other {
                            file: dir.join(MANIFEST_FILE),
                            message: format!(
                                "the name `{name}` is already used by {}",
                                other.dir.display()
                            ),
                        },
                        dir,
                    }),
                    None => scan.loaded.push(extension),
                }
            }
            Err(error) => scan.failed.push(FailedExtension { dir, error }),
        }
    }
    scan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(name: &str) -> String {
        format!(
            "[extension]\nname = \"{name}\"\ntitle = \"T\"\nversion = \"1.0.0\"\nruntime = \"script\"\n\n\
             [[commands]]\nname = \"go\"\ntitle = \"Go\"\nkeywords = [\"{name}\"]\n\n\
             [runtime.script]\ncommand = [\"run.sh\"]\n"
        )
    }

    #[test]
    fn the_log_of_an_extension_is_in_the_logs_folder_of_the_data_dir() {
        assert_eq!(
            log_path_in(Path::new("data"), "notes"),
            Path::new("data").join("logs").join("notes.log")
        );
    }

    fn write(root: &Path, dir: &str, file: &str, content: &str) {
        let dir = root.join(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(file), content).unwrap();
    }

    #[test]
    fn the_directory_is_daimos_extensions_below_the_config_dir() {
        let base = Path::new("base");
        assert_eq!(extensions_dir_in(base), base.join("daimos").join("extensions"));
    }

    #[test]
    fn the_default_directory_follows_the_platform() {
        let dir = extensions_dir().expect("a config directory");
        let tail: Vec<_> = dir.iter().rev().take(2).collect();
        assert_eq!(tail, ["extensions", "daimos"]);
        if cfg!(target_os = "macos") {
            assert!(dir.ends_with("Library/Application Support/daimos/extensions"), "{dir:?}");
        } else if cfg!(windows) {
            assert!(dir.to_string_lossy().contains("AppData"), "{dir:?}");
        } else {
            assert!(dir.ends_with(".config/daimos/extensions"), "{dir:?}");
        }
    }

    #[test]
    fn valid_broken_and_foreign_entries_are_sorted_out() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), "alpha", MANIFEST_FILE, &manifest("alpha"));
        write(root.path(), "broken", MANIFEST_FILE, "[extension]\nname = \n");
        write(root.path(), "invalid", MANIFEST_FILE, &manifest("Bad Name"));
        write(root.path(), "notes-only", "README.md", "not an extension");
        write(root.path(), ".hidden", MANIFEST_FILE, &manifest("hidden"));
        std::fs::write(root.path().join("stray.txt"), "x").unwrap();
        write(root.path(), "zeta", MANIFEST_FILE, &manifest("zeta"));

        let scan = scan(root.path());
        let names: Vec<_> =
            scan.loaded.iter().map(|e| e.manifest.extension.name.as_str()).collect();
        assert_eq!(names, ["alpha", "zeta"]);
        assert_eq!(scan.loaded[0].dir, root.path().join("alpha"));
        let failed: Vec<_> =
            scan.failed.iter().map(|f| f.dir.file_name().unwrap().to_string_lossy()).collect();
        assert_eq!(failed, ["broken", "invalid"]);
        assert!(scan.failed[0].error.to_string().contains("extension.toml:2"), "{:?}", scan.failed);
        assert!(scan.failed[1].error.to_string().contains("extension.name"), "{:?}", scan.failed);
    }

    #[test]
    fn scan_reports_what_waits_for_a_confirmation() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), "new", MANIFEST_FILE, &manifest("new"));
        write(root.path(), "known", MANIFEST_FILE, &manifest("known"));
        consent::confirm(&root.path().join("known")).unwrap();
        let scan = scan(root.path());
        assert_eq!(scan.loaded[0].manifest.extension.name, "known");
        assert_eq!(scan.loaded[0].pending(), None);
        assert_eq!(scan.loaded[1].pending(), Some(Capabilities::default()));
    }

    #[test]
    fn a_missing_directory_is_an_empty_scan() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(scan(&root.path().join("nope")), Scan::default());
    }

    #[test]
    fn a_second_extension_with_the_same_name_fails() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), "one", MANIFEST_FILE, &manifest("same"));
        write(root.path(), "two", MANIFEST_FILE, &manifest("same"));
        let scan = scan(root.path());
        assert_eq!(scan.loaded.len(), 1);
        assert_eq!(scan.failed.len(), 1);
        assert!(scan.failed[0].error.to_string().contains("already used"), "{:?}", scan.failed);
    }

    #[test]
    fn an_unreadable_manifest_is_a_failure_not_a_panic() {
        let root = tempfile::tempdir().unwrap();
        // A directory where the manifest file should be.
        std::fs::create_dir_all(root.path().join("odd").join(MANIFEST_FILE)).unwrap();
        let scan = scan(root.path());
        assert!(scan.loaded.is_empty());
        assert!(matches!(scan.failed[0].error, LoadError::Other { .. }));
    }
}
