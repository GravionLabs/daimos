//! The typed form of `extension.toml` (`docs/specs/extensions.md`).

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Which host runs the extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Runtime {
    Script,
    Wasm,
}

/// Whether a command shows a list or runs and exits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommandMode {
    #[default]
    View,
    NoView,
}

/// The `[extension]` table.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionInfo {
    pub name: String,
    pub title: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    pub runtime: Runtime,
}

/// One `[[commands]]` entry.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub name: String,
    pub title: String,
    #[serde(default)]
    pub mode: CommandMode,
    #[serde(default)]
    pub keywords: Vec<String>,
}

/// `[runtime.script]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptRuntime {
    pub command: Vec<String>,
    #[serde(default)]
    pub long_running: bool,
}

/// `[runtime.wasm]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WasmRuntime {
    pub component: String,
}

/// The `[runtime.*]` tables; the one that matches `extension.runtime` must be present.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeTables {
    #[serde(default)]
    pub script: Option<ScriptRuntime>,
    #[serde(default)]
    pub wasm: Option<WasmRuntime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClipboardAccess {
    Read,
    Write,
    ReadWrite,
}

/// The `[capabilities]` table: enforced for wasm, displayed for script.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    #[serde(default)]
    pub network: Vec<String>,
    #[serde(default)]
    pub filesystem: Vec<String>,
    #[serde(default)]
    pub clipboard: Option<ClipboardAccess>,
    #[serde(default)]
    pub keyring: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PreferenceKind {
    Text,
    /// Stored in the OS keyring.
    Password,
    Number,
    Checkbox,
}

/// One `[[preferences]]` entry.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preference {
    pub name: String,
    pub title: String,
    #[serde(rename = "type")]
    pub kind: PreferenceKind,
    #[serde(default)]
    pub required: bool,
}

/// A parsed `extension.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub extension: ExtensionInfo,
    #[serde(default)]
    pub commands: Vec<Command>,
    #[serde(default)]
    pub runtime: RuntimeTables,
    #[serde(default)]
    pub capabilities: Capabilities,
    #[serde(default)]
    pub preferences: Vec<Preference>,
}

/// The TOML is malformed or does not fit the manifest types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub file: PathBuf,
    /// 1-based line of the problem, when known.
    pub line: Option<usize>,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "{}:{line}: {}", self.file.display(), self.message),
            None => write!(f, "{}: {}", self.file.display(), self.message),
        }
    }
}

impl std::error::Error for ParseError {}

/// 1-based line of a byte offset.
pub(crate) fn line_of(source: &str, offset: usize) -> usize {
    let end = offset.min(source.len());
    source.as_bytes()[..end].iter().filter(|&&b| b == b'\n').count() + 1
}

impl Manifest {
    /// Parses `source`; `file` is only used to name the file in errors.
    pub fn parse(source: &str, file: &Path) -> Result<Self, ParseError> {
        toml::from_str(source).map_err(|error| ParseError {
            file: file.to_path_buf(),
            line: error.span().map(|span| line_of(source, span.start)),
            message: error.message().to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = r#"
[extension]
name = "notes"
title = "Notes"
version = "0.1.0"
runtime = "script"

[[commands]]
name = "search"
title = "Search Notes"
keywords = ["n", "note"]

[runtime.script]
command = ["python3", "main.py"]
long_running = true
"#;

    const WASM: &str = r#"
[extension]
name = "github-repos"
title = "GitHub Repos"
version = "0.2.0"
description = "Search and open your GitHub repositories"
author = "you"
runtime = "wasm"

[[commands]]
name = "search"
title = "Search Repos"
mode = "view"
keywords = ["gh", "repo"]

[[commands]]
name = "sync"
title = "Sync"
mode = "no-view"

[runtime.wasm]
component = "main.wasm"

[capabilities]
network = ["api.github.com"]
filesystem = ["$HOME/.config/gh:ro"]
clipboard = "write"
keyring = ["github-token"]

[[preferences]]
name = "token"
title = "GitHub token"
type = "password"
required = true
"#;

    fn parse(source: &str) -> Result<Manifest, ParseError> {
        Manifest::parse(source, Path::new("ext/extension.toml"))
    }

    #[test]
    fn a_script_manifest_parses() {
        let manifest = parse(SCRIPT).unwrap();
        assert_eq!(manifest.extension.name, "notes");
        assert_eq!(manifest.extension.runtime, Runtime::Script);
        assert_eq!(manifest.commands[0].keywords, ["n", "note"]);
        assert_eq!(manifest.commands[0].mode, CommandMode::View);
        let script = manifest.runtime.script.unwrap();
        assert_eq!(script.command, ["python3", "main.py"]);
        assert!(script.long_running);
        assert!(manifest.runtime.wasm.is_none());
        assert_eq!(manifest.capabilities, Capabilities::default());
    }

    #[test]
    fn a_wasm_manifest_parses_with_capabilities_and_preferences() {
        let manifest = parse(WASM).unwrap();
        assert_eq!(manifest.extension.runtime, Runtime::Wasm);
        assert_eq!(manifest.extension.author.as_deref(), Some("you"));
        assert_eq!(manifest.commands.len(), 2);
        assert_eq!(manifest.commands[1].mode, CommandMode::NoView);
        assert_eq!(manifest.runtime.wasm.unwrap().component, "main.wasm");
        assert_eq!(manifest.capabilities.network, ["api.github.com"]);
        assert_eq!(manifest.capabilities.clipboard, Some(ClipboardAccess::Write));
        assert_eq!(manifest.capabilities.keyring, ["github-token"]);
        assert_eq!(manifest.preferences[0].kind, PreferenceKind::Password);
        assert!(manifest.preferences[0].required);
    }

    #[test]
    fn malformed_toml_names_the_file_and_the_line() {
        let error = parse("[extension]\nname = \n").unwrap_err();
        assert_eq!(error.file, Path::new("ext/extension.toml"));
        assert_eq!(error.line, Some(2));
        assert!(error.to_string().starts_with("ext/extension.toml:2:"), "{error}");
    }

    #[test]
    fn unknown_fields_and_missing_tables_are_parse_errors() {
        let error = parse(&SCRIPT.replace("version = \"0.1.0\"", "colour = \"red\"")).unwrap_err();
        assert!(error.message.contains("colour"), "{error}");
        assert_eq!(error.line, Some(5));
        assert!(parse("").is_err());
    }

    #[test]
    fn line_numbers_count_from_one() {
        assert_eq!(line_of("a\nb\nc", 0), 1);
        assert_eq!(line_of("a\nb\nc", 2), 2);
        assert_eq!(line_of("a\nb\nc", 99), 3);
    }
}
