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

/// Why an extension could not be installed or found.
#[derive(Debug)]
pub enum InstallError {
    /// The source is not a directory with an `extension.toml`, or `git` failed.
    Source(String),
    /// The manifest of the source is invalid.
    Manifest(ManifestError),
    /// An extension of that name is already installed.
    Exists(PathBuf),
    /// Copying or removing files failed.
    Io(std::io::Error),
    /// `find` found no extension of that name.
    NotFound(String),
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(message) => f.write_str(message),
            Self::Manifest(error) => error.fmt(f),
            Self::Exists(dir) => write!(
                f,
                "{} already exists; remove it first to install again, or use `daimos ext confirm` \
                 after an update",
                dir.display()
            ),
            Self::Io(error) => write!(f, "cannot copy the extension: {error}"),
            Self::NotFound(name) => write!(f, "no extension named `{name}` is installed"),
        }
    }
}

impl std::error::Error for InstallError {}

impl From<std::io::Error> for InstallError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Whether `source` names a git repository to clone rather than a local directory.
pub fn is_git_url(source: &str) -> bool {
    ["http://", "https://", "ssh://", "git://", "file://", "git@"]
        .iter()
        .any(|prefix| source.starts_with(prefix))
}

/// Copies `from` to `to`; symlinks are skipped (they could point out of the extension), and so is
/// a `.git` folder and any `.consent.json`, because a consent must come from the user and never
/// from the package.
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        let kind = entry.file_type()?;
        if kind.is_symlink() || name == ".git" || name == crate::consent::CONSENT_FILE {
            continue;
        }
        if kind.is_dir() {
            copy_tree(&entry.path(), &to.join(&name))?;
        } else {
            std::fs::copy(entry.path(), to.join(&name))?;
        }
    }
    Ok(())
}

/// Installs the extension in `source` (a local directory, or a git URL that is cloned) into `root`
/// as `<root>/<name>`. The copy has no consent, so it stays `needs-confirmation` until
/// [`crate::consent::confirm`]. Nothing is left behind when it fails.
pub fn install(source: &str, root: &Path) -> Result<LoadedExtension, InstallError> {
    let staging = tempfile::Builder::new().prefix("daimos-install-").tempdir()?;
    let origin = if is_git_url(source) {
        let target = staging.path().join("clone");
        let output = std::process::Command::new("git")
            .args(["clone", "--quiet", "--depth", "1", "--", source])
            .arg(&target)
            .output()
            .map_err(|e| InstallError::Source(format!("cannot run git to clone {source}: {e}")))?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr);
            return Err(InstallError::Source(format!(
                "git clone {source} failed: {}",
                detail.trim()
            )));
        }
        target
    } else {
        PathBuf::from(source)
    };
    let manifest_file = origin.join(MANIFEST_FILE);
    let text = std::fs::read_to_string(&manifest_file).map_err(|e| {
        InstallError::Source(format!("{} is not an extension: {e}", manifest_file.display()))
    })?;
    let manifest = Manifest::parse(&text, &manifest_file).map_err(InstallError::Manifest)?;

    std::fs::create_dir_all(root)?;
    let target = root.join(&manifest.extension.name);
    if target.exists() {
        return Err(InstallError::Exists(target));
    }
    // Copy to a hidden sibling first, so a half-copied extension is never scanned or run.
    let partial = root.join(format!(".installing-{}", manifest.extension.name));
    let _ = std::fs::remove_dir_all(&partial);
    let copied = copy_tree(&origin, &partial).and_then(|()| std::fs::rename(&partial, &target));
    if let Err(error) = copied {
        let _ = std::fs::remove_dir_all(&partial);
        return Err(error.into());
    }
    load_extension(&target).map_err(|error| {
        let _ = std::fs::remove_dir_all(&target);
        match error {
            LoadError::Manifest(error) => InstallError::Manifest(error),
            LoadError::Other { message, .. } => InstallError::Source(message),
        }
    })
}

/// Deletes an installed extension's directory (a declined install).
pub fn remove(extension: &LoadedExtension) -> std::io::Result<()> {
    std::fs::remove_dir_all(&extension.dir)
}

/// The installed extension called `name` (its manifest name or its folder name).
pub fn find(root: &Path, name: &str) -> Result<LoadedExtension, InstallError> {
    if let Ok(extension) = load_extension(&root.join(name))
        && extension.manifest.extension.name == name
    {
        return Ok(extension);
    }
    scan(root)
        .loaded
        .into_iter()
        .find(|extension| extension.manifest.extension.name == name)
        .ok_or_else(|| InstallError::NotFound(name.to_owned()))
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

    fn package(parent: &Path, name: &str) -> PathBuf {
        let dir = parent.join("package");
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(dir.join(MANIFEST_FILE), manifest(name)).unwrap();
        std::fs::write(dir.join("lib").join("run.sh"), "#!/bin/sh\n").unwrap();
        dir
    }

    #[test]
    fn a_local_directory_is_copied_and_waits_for_the_confirmation() {
        let work = tempfile::tempdir().unwrap();
        let root = work.path().join("extensions");
        let source = package(work.path(), "notes");
        // A consent that ships with the package is not trusted.
        std::fs::write(source.join(".consent.json"), r#"{"version":"1.0.0","capabilities":{}}"#)
            .unwrap();
        let installed = install(source.to_str().unwrap(), &root).unwrap();
        assert_eq!(installed.dir, root.join("notes"));
        assert_eq!(installed.manifest.extension.name, "notes");
        assert!(root.join("notes").join("lib").join("run.sh").is_file());
        assert!(!root.join("notes").join(".consent.json").exists());
        assert!(installed.pending().is_some());
        assert!(source.join(MANIFEST_FILE).exists(), "the source is untouched");
        assert!(scan(&root).failed.is_empty());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1, "no staging leftovers");
    }

    #[test]
    fn installing_twice_or_from_a_bad_source_changes_nothing() {
        let work = tempfile::tempdir().unwrap();
        let root = work.path().join("extensions");
        let source = package(work.path(), "notes");
        install(source.to_str().unwrap(), &root).unwrap();
        let again = install(source.to_str().unwrap(), &root).unwrap_err();
        assert!(matches!(again, InstallError::Exists(_)), "{again}");

        let empty = work.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        let error = install(empty.to_str().unwrap(), &root).unwrap_err();
        assert!(matches!(error, InstallError::Source(_)), "{error}");
        let error = install(work.path().join("missing").to_str().unwrap(), &root).unwrap_err();
        assert!(matches!(error, InstallError::Source(_)), "{error}");

        std::fs::write(source.join(MANIFEST_FILE), manifest("Bad Name")).unwrap();
        let error = install(source.to_str().unwrap(), &root).unwrap_err();
        assert!(matches!(error, InstallError::Manifest(_)), "{error}");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_in_a_package_are_not_copied() {
        let work = tempfile::tempdir().unwrap();
        let root = work.path().join("extensions");
        let source = package(work.path(), "notes");
        std::os::unix::fs::symlink("/etc/passwd", source.join("passwd")).unwrap();
        install(source.to_str().unwrap(), &root).unwrap();
        assert!(!root.join("notes").join("passwd").exists());
    }

    #[test]
    fn a_git_repository_is_cloned_without_its_history() {
        let git = |dir: &Path, args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
                .args(args)
                .output()
        };
        let work = tempfile::tempdir().unwrap();
        let repo = package(work.path(), "cloned");
        if git(&repo, &["init", "--quiet"]).is_err() {
            return; // no git on this machine
        }
        git(&repo, &["add", "."]).unwrap();
        git(&repo, &["commit", "--quiet", "-m", "init"]).unwrap();
        let root = work.path().join("extensions");
        let url = format!("file://{}", repo.display());
        assert!(is_git_url(&url));
        let installed = install(&url, &root).unwrap();
        assert_eq!(installed.manifest.extension.name, "cloned");
        assert!(root.join("cloned").join("lib").join("run.sh").is_file());
        assert!(!root.join("cloned").join(".git").exists());

        let error = install("file:///does/not/exist", &root).unwrap_err();
        assert!(matches!(error, InstallError::Source(m) if m.contains("git clone")));
    }

    #[test]
    fn urls_are_told_from_paths() {
        for url in ["https://github.com/a/b", "git@github.com:a/b.git", "ssh://h/r", "file:///r"] {
            assert!(is_git_url(url), "{url}");
        }
        for path in ["./ext", "/home/me/ext", "C:\\ext", "ext"] {
            assert!(!is_git_url(path), "{path}");
        }
    }

    #[test]
    fn find_matches_the_manifest_name_or_the_folder() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), "folder", MANIFEST_FILE, &manifest("real"));
        assert_eq!(find(root.path(), "real").unwrap().dir, root.path().join("folder"));
        write(root.path(), "plain", MANIFEST_FILE, &manifest("plain"));
        assert_eq!(find(root.path(), "plain").unwrap().dir, root.path().join("plain"));
        assert!(matches!(find(root.path(), "nope"), Err(InstallError::NotFound(_))));
    }

    #[test]
    fn remove_deletes_the_installed_directory() {
        let work = tempfile::tempdir().unwrap();
        let root = work.path().join("extensions");
        let source = package(work.path(), "notes");
        let installed = install(source.to_str().unwrap(), &root).unwrap();
        remove(&installed).unwrap();
        assert!(!root.join("notes").exists());
    }
}
