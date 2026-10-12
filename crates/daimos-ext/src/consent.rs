//! What the user confirmed about an installed extension: the capabilities it declared and the
//! version they belonged to. The record lives in the extension's directory, next to the manifest
//! but outside it, so an update that replaces `extension.toml` cannot grant itself anything.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::dir::{LoadError, MANIFEST_FILE};
use crate::manifest::{Capabilities, Manifest};

/// The file that holds the [`ConsentRecord`].
pub const CONSENT_FILE: &str = ".consent.json";

/// The confirmed capabilities and the version that was confirmed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentRecord {
    pub version: String,
    pub capabilities: Capabilities,
}

/// The consent file of the extension in `dir`.
pub fn consent_path(dir: &Path) -> PathBuf {
    dir.join(CONSENT_FILE)
}

/// Reads the consent of the extension in `dir`. A missing or unreadable file is no consent.
pub fn read(dir: &Path) -> Option<ConsentRecord> {
    let source = std::fs::read_to_string(consent_path(dir)).ok()?;
    serde_json::from_str(&source).ok()
}

/// Records the capabilities of `manifest` as confirmed.
pub fn write(dir: &Path, manifest: &Manifest) -> std::io::Result<()> {
    let record = ConsentRecord {
        version: manifest.extension.version.clone(),
        capabilities: manifest.capabilities.clone(),
    };
    let json = serde_json::to_string_pretty(&record).map_err(std::io::Error::other)?;
    std::fs::write(consent_path(dir), json + "\n")
}

/// The capabilities `manifest` asks for beyond what `consent` confirmed; `None` when nothing is
/// pending. Without a consent record everything is pending, even an empty declaration, because a
/// script still runs with the user's rights.
pub fn pending(manifest: &Manifest, consent: Option<&ConsentRecord>) -> Option<Capabilities> {
    match consent {
        None => Some(manifest.capabilities.clone()),
        Some(record) => {
            let added = manifest.capabilities.diff(&record.capabilities);
            (!added.is_empty()).then_some(added)
        }
    }
}

/// Confirms the extension in `dir`: the capabilities of its current manifest become the consent.
/// The extension is enabled the next time it loads.
pub fn confirm(dir: &Path) -> Result<ConsentRecord, LoadError> {
    let file = dir.join(MANIFEST_FILE);
    let other = |message: String| LoadError::Other { file: file.clone(), message };
    let source = std::fs::read_to_string(&file).map_err(|e| other(e.to_string()))?;
    let manifest = Manifest::parse(&source, &file).map_err(LoadError::Manifest)?;
    write(dir, &manifest).map_err(|e| other(format!("cannot record the consent: {e}")))?;
    Ok(read(dir).expect("the consent was just written"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(capabilities: &str) -> String {
        format!(
            "[extension]\nname = \"ext\"\ntitle = \"T\"\nversion = \"1.0.0\"\nruntime = \"script\"\n\n\
             [[commands]]\nname = \"go\"\ntitle = \"Go\"\n\n\
             [runtime.script]\ncommand = [\"run.sh\"]\n\n[capabilities]\n{capabilities}\n"
        )
    }

    fn caps(capabilities: &str) -> Capabilities {
        Manifest::parse(&manifest(capabilities), Path::new("e.toml")).unwrap().capabilities
    }

    fn added(now: &str, confirmed: &str) -> Capabilities {
        caps(now).diff(&caps(confirmed))
    }

    #[test]
    fn nothing_changed_adds_nothing() {
        let all = "network = [\"a.com\"]\nfilesystem = [\"/x:rw\"]\nclipboard = \"read-write\"\nkeyring = [\"k\"]";
        assert!(added(all, all).is_empty());
        assert!(added("", "").is_empty());
    }

    #[test]
    fn a_new_host_path_or_keyring_entry_is_added() {
        let base = "network = [\"a.com\"]\nfilesystem = [\"/x:ro\"]\nkeyring = [\"k\"]";
        let now = "network = [\"a.com\", \"b.com\"]\nfilesystem = [\"/x:ro\", \"/y:ro\"]\nkeyring = [\"k\", \"j\"]";
        let diff = added(now, base);
        assert_eq!(diff.network, ["b.com"]);
        assert_eq!(diff.filesystem, ["/y:ro"]);
        assert_eq!(diff.keyring, ["j"]);
        assert_eq!(diff.clipboard, None);
    }

    #[test]
    fn a_wider_filesystem_mode_is_added_and_a_narrower_one_is_not() {
        assert_eq!(
            added("filesystem = [\"/x:rw\"]", "filesystem = [\"/x:ro\"]").filesystem,
            ["/x:rw"]
        );
        assert_eq!(added("filesystem = [\"/x\"]", "filesystem = [\"/x:ro\"]").filesystem.len(), 0);
        assert!(added("filesystem = [\"/x:ro\"]", "filesystem = [\"/x:rw\"]").is_empty());
        assert_eq!(added("filesystem = [\"/x:rw\"]", "").filesystem, ["/x:rw"]);
    }

    #[test]
    fn a_wider_clipboard_is_added_and_a_narrower_one_is_not() {
        use crate::manifest::ClipboardAccess::*;
        assert_eq!(added("clipboard = \"write\"", "").clipboard, Some(Write));
        assert_eq!(
            added("clipboard = \"read-write\"", "clipboard = \"read\"").clipboard,
            Some(Write)
        );
        assert_eq!(added("clipboard = \"read\"", "clipboard = \"write\"").clipboard, Some(Read));
        assert!(added("clipboard = \"read\"", "clipboard = \"read-write\"").is_empty());
        assert!(added("", "clipboard = \"read\"").is_empty());
    }

    #[test]
    fn removing_a_capability_adds_nothing() {
        let before = "network = [\"a.com\", \"b.com\"]\nkeyring = [\"k\"]\nclipboard = \"read\"";
        assert!(added("network = [\"a.com\"]", before).is_empty());
    }

    #[test]
    fn without_consent_everything_is_pending_even_when_empty() {
        let m = Manifest::parse(&manifest(""), Path::new("e.toml")).unwrap();
        assert_eq!(pending(&m, None), Some(Capabilities::default()));
        let record =
            ConsentRecord { version: "1.0.0".into(), capabilities: Capabilities::default() };
        assert_eq!(pending(&m, Some(&record)), None);
    }

    #[test]
    fn confirm_writes_the_consent_and_read_returns_it() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(dir.path()), None);
        std::fs::write(dir.path().join(MANIFEST_FILE), manifest("network = [\"a.com\"]")).unwrap();
        let record = confirm(dir.path()).unwrap();
        assert_eq!(record.version, "1.0.0");
        assert_eq!(record.capabilities.network, ["a.com"]);
        assert_eq!(read(dir.path()), Some(record));
    }

    #[test]
    fn confirming_a_broken_extension_fails_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(confirm(dir.path()), Err(LoadError::Other { .. })));
        std::fs::write(dir.path().join(MANIFEST_FILE), "not [valid").unwrap();
        assert!(matches!(confirm(dir.path()), Err(LoadError::Manifest(_))));
        assert!(!consent_path(dir.path()).exists());
    }

    #[test]
    fn a_corrupt_consent_file_is_no_consent() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(consent_path(dir.path()), "{ nope").unwrap();
        assert_eq!(read(dir.path()), None);
    }
}
