//! The typed form of `extension.toml` (`docs/specs/extensions.md`).

use std::path::Path;

use serde::Deserialize;

use crate::error::{self, Locator, ManifestError, Step};

/// Which host runs the extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Runtime {
    Script,
    Wasm,
}

impl Runtime {
    /// `script` or `wasm`, as written in the manifest.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Script => "script",
            Self::Wasm => "wasm",
        }
    }
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

/// The safe directory-style identifier rule for extension, command and preference names.
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        && name.len() <= 64
}

impl Manifest {
    /// Parses and validates `source`; `file` is only used to name the file in errors.
    pub fn parse(source: &str, file: &Path) -> Result<Self, ManifestError> {
        let manifest: Self =
            toml::from_str(source).map_err(|e| error::from_toml(&e, source, file))?;
        manifest.validate(source, file)?;
        Ok(manifest)
    }

    /// Checks what the types cannot: names, the runtime tables, and duplicates.
    pub fn validate(&self, source: &str, file: &Path) -> Result<(), ManifestError> {
        let locator = Locator::new(source);
        let fail = |path: Vec<Step>, message: String| ManifestError {
            file: file.to_path_buf(),
            line: locator.line(&path),
            field: error::render(&path),
            message,
        };
        let key = |name: &str| Step::Key(name.to_string());
        let in_list =
            |list: &str, index: usize, field: &str| vec![key(list), Step::Index(index), key(field)];

        let info = &self.extension;
        if !is_identifier(&info.name) {
            return Err(fail(
                vec![key("extension"), key("name")],
                format!(
                    "`{}` is not a safe name: use lowercase letters, digits, `-` and `_` (at most 64)",
                    info.name
                ),
            ));
        }
        for (field, value) in [("title", &info.title), ("version", &info.version)] {
            if value.trim().is_empty() {
                return Err(fail(vec![key("extension"), key(field)], "must not be empty".into()));
            }
        }

        let (wanted, other, wanted_present, other_present) = match info.runtime {
            Runtime::Script => {
                ("script", "wasm", self.runtime.script.is_some(), self.runtime.wasm.is_some())
            }
            Runtime::Wasm => {
                ("wasm", "script", self.runtime.wasm.is_some(), self.runtime.script.is_some())
            }
        };
        if !wanted_present {
            return Err(fail(
                vec![key("extension"), key("runtime")],
                format!("runtime is \"{wanted}\" but there is no [runtime.{wanted}] table"),
            ));
        }
        if other_present {
            return Err(fail(
                vec![key("runtime"), key(other)],
                format!("[runtime.{other}] does not match runtime = \"{wanted}\""),
            ));
        }
        if let Some(script) = &self.runtime.script
            && script.command.first().is_none_or(|program| program.trim().is_empty())
        {
            return Err(fail(
                vec![key("runtime"), key("script"), key("command")],
                "must name the program to run".into(),
            ));
        }
        if let Some(wasm) = &self.runtime.wasm
            && wasm.component.trim().is_empty()
        {
            return Err(fail(
                vec![key("runtime"), key("wasm"), key("component")],
                "must name the component file".into(),
            ));
        }

        if self.commands.is_empty() {
            return Err(fail(
                vec![key("commands")],
                "an extension needs at least one [[commands]] entry".into(),
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for (index, command) in self.commands.iter().enumerate() {
            if !is_identifier(&command.name) {
                return Err(fail(
                    in_list("commands", index, "name"),
                    format!(
                        "`{}` is not a safe name: use lowercase letters, digits, `-` and `_`",
                        command.name
                    ),
                ));
            }
            if !seen.insert(command.name.as_str()) {
                return Err(fail(
                    in_list("commands", index, "name"),
                    format!("command `{}` is defined twice", command.name),
                ));
            }
            if command.title.trim().is_empty() {
                return Err(fail(in_list("commands", index, "title"), "must not be empty".into()));
            }
            if command
                .keywords
                .iter()
                .any(|k| k.trim().is_empty() || k.contains(char::is_whitespace))
            {
                return Err(fail(
                    in_list("commands", index, "keywords"),
                    "keywords must not be empty or contain spaces".into(),
                ));
            }
        }

        let mut seen = std::collections::HashSet::new();
        for (index, preference) in self.preferences.iter().enumerate() {
            if !is_identifier(&preference.name) {
                return Err(fail(
                    in_list("preferences", index, "name"),
                    format!(
                        "`{}` is not a safe name: use lowercase letters, digits, `-` and `_`",
                        preference.name
                    ),
                ));
            }
            if !seen.insert(preference.name.as_str()) {
                return Err(fail(
                    in_list("preferences", index, "name"),
                    format!("preference `{}` is defined twice", preference.name),
                ));
            }
            if preference.title.trim().is_empty() {
                return Err(fail(
                    in_list("preferences", index, "title"),
                    "must not be empty".into(),
                ));
            }
        }
        Ok(())
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

    fn parse(source: &str) -> Result<Manifest, ManifestError> {
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

    /// Parses `SCRIPT` with `from` replaced by `to` and expects an error at `line` and `field`.
    fn rejects(from: &str, to: &str, line: usize, field: &str) -> ManifestError {
        assert!(SCRIPT.contains(from), "{from} is not in the fixture");
        let error = parse(&SCRIPT.replace(from, to)).unwrap_err();
        assert_eq!(error.file, Path::new("ext/extension.toml"));
        assert_eq!((error.line, error.field.as_str()), (line, field), "{error}");
        let text = error.to_string();
        assert!(
            text.starts_with(&format!("ext/extension.toml:{line}: {field}: ")),
            "display names file, line and field: {text}"
        );
        error
    }

    #[test]
    fn malformed_toml_names_the_file_and_the_line() {
        let error = parse("[extension]\nname = \n").unwrap_err();
        assert_eq!(error.file, Path::new("ext/extension.toml"));
        assert_eq!(error.line, 2);
        assert!(error.to_string().starts_with("ext/extension.toml:2:"), "{error}");
    }

    #[test]
    fn an_unknown_field_is_named() {
        let error = rejects("version = \"0.1.0\"", "colour = \"red\"", 5, "extension.colour");
        assert!(error.message.contains("colour"), "{error}");
    }

    #[test]
    fn a_missing_required_field_is_named() {
        let error = rejects("title = \"Notes\"\n", "", 2, "extension.title");
        assert!(error.message.contains("missing field"), "{error}");
        assert_eq!(parse("").unwrap_err().field, "extension");
    }

    #[test]
    fn an_unknown_runtime_is_rejected() {
        rejects("runtime = \"script\"", "runtime = \"java\"", 6, "extension.runtime");
    }

    #[test]
    fn a_mode_other_than_view_or_no_view_is_rejected() {
        let error = rejects("keywords =", "mode = \"popup\"\nkeywords =", 11, "commands[0].mode");
        assert!(error.message.contains("view"), "{error}");
    }

    #[test]
    fn an_unknown_preference_type_is_rejected() {
        let source =
            format!("{SCRIPT}\n[[preferences]]\nname = \"a\"\ntitle = \"A\"\ntype = \"color\"\n");
        let error = parse(&source).unwrap_err();
        assert_eq!((error.line, error.field.as_str()), (20, "preferences[0].type"), "{error}");
    }

    #[test]
    fn the_runtime_table_must_exist() {
        let source = SCRIPT.replace("runtime = \"script\"", "runtime = \"wasm\"").replace(
            "[runtime.script]\ncommand = [\"python3\", \"main.py\"]\nlong_running = true\n",
            "",
        );
        let error = parse(&source).unwrap_err();
        assert_eq!((error.line, error.field.as_str()), (6, "extension.runtime"), "{error}");
        assert!(error.message.contains("[runtime.wasm]"), "{error}");
    }

    #[test]
    fn a_runtime_table_for_the_other_runtime_is_rejected() {
        let source = format!("{SCRIPT}\n[runtime.wasm]\ncomponent = \"main.wasm\"\n");
        let error = parse(&source).unwrap_err();
        assert_eq!((error.line, error.field.as_str()), (17, "runtime.wasm"), "{error}");
    }

    #[test]
    fn a_runtime_without_a_program_or_component_is_rejected() {
        rejects("[\"python3\", \"main.py\"]", "[]", 14, "runtime.script.command");
        let wasm = WASM.replace("main.wasm", " ");
        let error = Manifest::parse(&wasm, Path::new("e.toml")).unwrap_err();
        assert_eq!(error.field, "runtime.wasm.component");
    }

    #[test]
    fn unsafe_names_are_rejected() {
        for name in ["../evil", "Has Space", "UPPER", "", "-lead"] {
            let error = parse(&SCRIPT.replace("name = \"notes\"", &format!("name = \"{name}\"")))
                .unwrap_err();
            assert_eq!(
                (error.line, error.field.as_str()),
                (3, "extension.name"),
                "{name}: {error}"
            );
        }
        rejects("name = \"search\"", "name = \"a/b\"", 9, "commands[0].name");
    }

    #[test]
    fn empty_titles_and_versions_are_rejected() {
        rejects("version = \"0.1.0\"", "version = \" \"", 5, "extension.version");
        rejects("title = \"Notes\"", "title = \"\"", 4, "extension.title");
        rejects("title = \"Search Notes\"", "title = \"\"", 10, "commands[0].title");
    }

    #[test]
    fn an_extension_needs_a_command() {
        let source = SCRIPT.replace("[[commands]]\nname = \"search\"\ntitle = \"Search Notes\"\nkeywords = [\"n\", \"note\"]\n", "");
        let error = parse(&source).unwrap_err();
        assert_eq!(error.field, "commands");
        assert!(error.to_string().contains("ext/extension.toml:"), "{error}");
    }

    #[test]
    fn duplicate_command_names_point_at_the_second_one() {
        let source = format!("{SCRIPT}\n[[commands]]\nname = \"search\"\ntitle = \"Again\"\n");
        let error = parse(&source).unwrap_err();
        assert_eq!((error.line, error.field.as_str()), (18, "commands[1].name"), "{error}");
        assert!(error.message.contains("twice"), "{error}");
    }

    #[test]
    fn duplicate_preference_names_are_rejected() {
        let pref = "\n[[preferences]]\nname = \"a\"\ntitle = \"A\"\ntype = \"text\"\n";
        let error = parse(&format!("{SCRIPT}{pref}{pref}")).unwrap_err();
        assert_eq!(error.field, "preferences[1].name", "{error}");
    }

    #[test]
    fn keywords_without_text_or_with_spaces_are_rejected() {
        rejects("[\"n\", \"note\"]", "[\"n\", \"two words\"]", 11, "commands[0].keywords");
        rejects("[\"n\", \"note\"]", "[\"\"]", 11, "commands[0].keywords");
    }
}
