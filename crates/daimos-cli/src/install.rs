//! `daimos ext install` and `daimos ext confirm`: they work on the extension directory and need no
//! running Daimos. A running one picks the change up through its file watcher.

use std::io::{BufRead, Write};
use std::path::Path;

use daimos_ext::dir::{self, LoadedExtension};
use daimos_ext::manifest::{Capabilities, ClipboardAccess, Runtime};

/// Where the answer to the prompt comes from.
pub trait Answer {
    /// Whether stdin can answer a question at all.
    fn interactive(&self) -> bool;
    /// One line, without the line break; `None` at the end of the input.
    fn read_line(&mut self) -> Option<String>;
}

/// The real stdin.
pub struct Stdin;

impl Answer for Stdin {
    fn interactive(&self) -> bool {
        std::io::IsTerminal::is_terminal(&std::io::stdin())
    }

    fn read_line(&mut self) -> Option<String> {
        let mut line = String::new();
        match std::io::stdin().lock().read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(line.trim_end().to_owned()),
        }
    }
}

/// How an install or confirm ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Confirmed,
    /// Already confirmed, nothing to do (`ext confirm` only).
    NothingToConfirm,
    /// Not confirmed; for an install the copy is removed again.
    Declined(String),
}

/// One capability per line, `added` entries marked.
pub fn describe(capabilities: &Capabilities, added: Option<&Capabilities>) -> Vec<String> {
    let mark = |new: bool| if new { "  (new)" } else { "" };
    let mut lines = Vec::new();
    for entry in &capabilities.filesystem {
        let (path, mode) = match entry.rsplit_once(':') {
            Some((path, "rw")) => (path, "read and write"),
            Some((path, "ro")) => (path, "read only"),
            _ => (entry.as_str(), "read only"),
        };
        let new = added.is_some_and(|a| a.filesystem.contains(entry));
        lines.push(format!("filesystem  {path} ({mode}){}", mark(new)));
    }
    for host in &capabilities.network {
        let new = added.is_some_and(|a| a.network.contains(host));
        lines.push(format!("network     {host}{}", mark(new)));
    }
    if let Some(access) = capabilities.clipboard {
        let text = match access {
            ClipboardAccess::Read => "read",
            ClipboardAccess::Write => "write",
            ClipboardAccess::ReadWrite => "read and write",
        };
        let new = added.is_some_and(|a| a.clipboard.is_some());
        lines.push(format!("clipboard   {text}{}", mark(new)));
    }
    for entry in &capabilities.keyring {
        let new = added.is_some_and(|a| a.keyring.contains(entry));
        lines.push(format!("keyring     {entry}{}", mark(new)));
    }
    lines
}

/// What the user is asked to confirm.
fn summary(extension: &LoadedExtension) -> String {
    let manifest = &extension.manifest;
    let info = &manifest.extension;
    let mut out = format!("{} {} ({})", info.name, info.version, info.runtime.as_str());
    if let Some(author) = &info.author {
        out.push_str(&format!(" by {author}"));
    }
    out.push('\n');
    out.push_str(match info.runtime {
        Runtime::Script => {
            "Runs unsandboxed: a script extension runs with your rights and can do anything you can.\n"
        }
        Runtime::Wasm => "Sandboxed: it can only use the capabilities listed here.\n",
    });
    let lines = describe(
        &manifest.capabilities,
        extension.pending().as_ref().filter(|_| extension.consent.is_some()),
    );
    if lines.is_empty() {
        out.push_str("It declares no capabilities.\n");
    } else {
        out.push_str("It declares these capabilities:\n");
        for line in lines {
            out.push_str(&format!("  {line}\n"));
        }
    }
    out
}

/// Prints `extension` and asks. `yes` skips the question; without it and without a terminal the
/// answer is no.
fn ask(
    extension: &LoadedExtension,
    yes: bool,
    question: &str,
    input: &mut dyn Answer,
    out: &mut dyn Write,
) -> Result<bool, std::io::Error> {
    write!(out, "{}", summary(extension))?;
    if yes {
        return Ok(true);
    }
    if !input.interactive() {
        writeln!(
            out,
            "Not confirmed: there is no terminal to ask. Run it again with --yes to accept."
        )?;
        return Ok(false);
    }
    write!(out, "{question} [y/N] ")?;
    out.flush()?;
    Ok(input.read_line().is_some_and(|line| matches!(line.trim(), "y" | "Y" | "yes" | "Yes")))
}

/// Installs `source` into `root`, shows its capabilities and enables it on a yes. On a no the copy
/// is removed.
pub fn install(
    root: &Path,
    source: &str,
    yes: bool,
    input: &mut dyn Answer,
    out: &mut dyn Write,
) -> Result<Outcome, Box<dyn std::error::Error>> {
    let extension = dir::install(source, root)?;
    let accepted = ask(&extension, yes, "Enable this extension?", input, out)?;
    if !accepted {
        dir::remove(&extension)?;
        return Ok(Outcome::Declined(format!(
            "{} was not installed",
            extension.manifest.extension.name
        )));
    }
    daimos_ext::consent::confirm(&extension.dir)?;
    writeln!(out, "Installed and enabled {}.", extension.manifest.extension.name)?;
    Ok(Outcome::Confirmed)
}

/// Shows what the installed extension `name` asks for, with the additions of an update marked, and
/// confirms it on a yes.
pub fn confirm(
    root: &Path,
    name: &str,
    yes: bool,
    input: &mut dyn Answer,
    out: &mut dyn Write,
) -> Result<Outcome, Box<dyn std::error::Error>> {
    let extension = dir::find(root, name)?;
    if extension.pending().is_none() {
        writeln!(out, "{name} is already confirmed.")?;
        return Ok(Outcome::NothingToConfirm);
    }
    if !ask(&extension, yes, "Enable this extension with these capabilities?", input, out)? {
        return Ok(Outcome::Declined(format!("{name} stays disabled")));
    }
    daimos_ext::consent::confirm(&extension.dir)?;
    writeln!(out, "Confirmed {name}.")?;
    Ok(Outcome::Confirmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scripted {
        interactive: bool,
        lines: Vec<String>,
        asked: usize,
    }

    impl Scripted {
        fn says(line: &str) -> Self {
            Self { interactive: true, lines: vec![line.into()], asked: 0 }
        }
    }

    impl Answer for Scripted {
        fn interactive(&self) -> bool {
            self.interactive
        }

        fn read_line(&mut self) -> Option<String> {
            self.asked += 1;
            self.lines.pop()
        }
    }

    const SCRIPT: &str = "[extension]\nname = \"notes\"\ntitle = \"Notes\"\nversion = \"1.0.0\"\n\
        author = \"me\"\nruntime = \"script\"\n\n[[commands]]\nname = \"go\"\ntitle = \"Go\"\n\n\
        [runtime.script]\ncommand = [\"run.sh\"]\n\n[capabilities]\n\
        network = [\"api.example.com\"]\nfilesystem = [\"$HOME/.config/gh:ro\", \"/tmp/x:rw\"]\n\
        clipboard = \"write\"\nkeyring = [\"token\"]\n";

    struct Setup {
        _work: tempfile::TempDir,
        root: std::path::PathBuf,
        source: String,
    }

    fn setup(manifest: &str) -> Setup {
        let work = tempfile::tempdir().unwrap();
        let source = work.path().join("src");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("extension.toml"), manifest).unwrap();
        Setup {
            root: work.path().join("extensions"),
            source: source.to_str().unwrap().to_owned(),
            _work: work,
        }
    }

    fn text(out: Vec<u8>) -> String {
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn the_prompt_lists_the_capabilities_and_the_unsandboxed_line() {
        let s = setup(SCRIPT);
        let mut out = Vec::new();
        install(&s.root, &s.source, false, &mut Scripted::says("y"), &mut out).unwrap();
        let out = text(out);
        assert!(out.contains("notes 1.0.0 (script) by me"), "{out}");
        assert!(out.contains("Runs unsandboxed"), "{out}");
        assert!(out.contains("filesystem  $HOME/.config/gh (read only)"), "{out}");
        assert!(out.contains("filesystem  /tmp/x (read and write)"), "{out}");
        assert!(out.contains("network     api.example.com"), "{out}");
        assert!(out.contains("clipboard   write"), "{out}");
        assert!(out.contains("keyring     token"), "{out}");
        assert!(out.contains("Enable this extension? [y/N]"), "{out}");
    }

    #[test]
    fn a_wasm_extension_is_labeled_sandboxed() {
        let manifest = SCRIPT.replace("runtime = \"script\"", "runtime = \"wasm\"").replace(
            "[runtime.script]\ncommand = [\"run.sh\"]",
            "[runtime.wasm]\ncomponent = \"main.wasm\"",
        );
        let s = setup(&manifest);
        let mut out = Vec::new();
        install(&s.root, &s.source, true, &mut Scripted::says(""), &mut out).unwrap();
        let out = text(out);
        assert!(out.contains("Sandboxed"), "{out}");
        assert!(!out.contains("Runs unsandboxed"), "{out}");
    }

    #[test]
    fn confirming_enables_the_extension() {
        let s = setup(SCRIPT);
        let mut out = Vec::new();
        let outcome =
            install(&s.root, &s.source, false, &mut Scripted::says("y"), &mut out).unwrap();
        assert_eq!(outcome, Outcome::Confirmed);
        let loaded = dir::load_extension(&s.root.join("notes")).unwrap();
        assert_eq!(loaded.pending(), None);
        assert_eq!(loaded.consent.unwrap().capabilities.network, ["api.example.com"]);
    }

    #[test]
    fn declining_leaves_nothing_installed() {
        let s = setup(SCRIPT);
        for answer in ["n", "", "maybe"] {
            let mut out = Vec::new();
            let outcome =
                install(&s.root, &s.source, false, &mut Scripted::says(answer), &mut out).unwrap();
            assert!(matches!(outcome, Outcome::Declined(_)), "{answer}");
            assert!(!s.root.join("notes").exists(), "{answer}");
        }
        // The end of the input is a no too.
        let mut closed = Scripted { interactive: true, lines: Vec::new(), asked: 0 };
        let outcome = install(&s.root, &s.source, false, &mut closed, &mut Vec::new()).unwrap();
        assert!(matches!(outcome, Outcome::Declined(_)));
    }

    #[test]
    fn without_a_terminal_and_without_yes_the_answer_is_no_and_nothing_is_asked() {
        let s = setup(SCRIPT);
        let mut input = Scripted { interactive: false, lines: vec!["y".into()], asked: 0 };
        let mut out = Vec::new();
        let outcome = install(&s.root, &s.source, false, &mut input, &mut out).unwrap();
        assert!(matches!(outcome, Outcome::Declined(_)));
        assert_eq!(input.asked, 0);
        assert!(!s.root.join("notes").exists());
        let out = text(out);
        assert!(out.contains("--yes"), "{out}");
        assert!(out.contains("network     api.example.com"), "the user still sees what was asked");
    }

    #[test]
    fn yes_skips_the_prompt() {
        let s = setup(SCRIPT);
        let mut input = Scripted { interactive: false, lines: Vec::new(), asked: 0 };
        let mut out = Vec::new();
        let outcome = install(&s.root, &s.source, true, &mut input, &mut out).unwrap();
        assert_eq!(outcome, Outcome::Confirmed);
        assert_eq!(input.asked, 0);
        assert!(!text(out).contains("[y/N]"));
        assert_eq!(dir::load_extension(&s.root.join("notes")).unwrap().pending(), None);
    }

    #[test]
    fn a_broken_or_existing_extension_is_an_error() {
        let s = setup("[extension]\nname = \n");
        assert!(
            install(&s.root, &s.source, true, &mut Scripted::says("y"), &mut Vec::new()).is_err()
        );
        let s = setup(SCRIPT);
        install(&s.root, &s.source, true, &mut Scripted::says("y"), &mut Vec::new()).unwrap();
        let again = install(&s.root, &s.source, true, &mut Scripted::says("y"), &mut Vec::new());
        assert!(again.unwrap_err().to_string().contains("already exists"));
    }

    #[test]
    fn confirm_re_enables_an_updated_extension_and_marks_what_is_new() {
        let s = setup(SCRIPT);
        install(&s.root, &s.source, true, &mut Scripted::says(""), &mut Vec::new()).unwrap();
        // Update: one more host, one more keyring entry.
        let manifest = s.root.join("notes").join("extension.toml");
        let updated = SCRIPT
            .replace(
                "network = [\"api.example.com\"]",
                "network = [\"api.example.com\", \"evil.example\"]",
            )
            .replace("keyring = [\"token\"]", "keyring = [\"token\", \"other\"]");
        std::fs::write(&manifest, updated).unwrap();
        assert!(dir::load_extension(&s.root.join("notes")).unwrap().pending().is_some());

        let mut out = Vec::new();
        let mut declined = Scripted::says("n");
        let outcome = confirm(&s.root, "notes", false, &mut declined, &mut out).unwrap();
        assert!(matches!(outcome, Outcome::Declined(_)));
        assert!(dir::load_extension(&s.root.join("notes")).unwrap().pending().is_some());
        let out = text(out);
        assert!(out.contains("network     evil.example  (new)"), "{out}");
        assert!(out.contains("keyring     other  (new)"), "{out}");
        assert!(out.contains("network     api.example.com\n"), "old entries are not marked: {out}");

        let mut out = Vec::new();
        let outcome = confirm(&s.root, "notes", false, &mut Scripted::says("y"), &mut out).unwrap();
        assert_eq!(outcome, Outcome::Confirmed);
        assert_eq!(dir::load_extension(&s.root.join("notes")).unwrap().pending(), None);
    }

    #[test]
    fn confirm_of_a_confirmed_or_unknown_extension() {
        let s = setup(SCRIPT);
        install(&s.root, &s.source, true, &mut Scripted::says(""), &mut Vec::new()).unwrap();
        let mut out = Vec::new();
        let outcome = confirm(&s.root, "notes", true, &mut Scripted::says(""), &mut out).unwrap();
        assert_eq!(outcome, Outcome::NothingToConfirm);
        assert!(text(out).contains("already confirmed"));
        let error = confirm(&s.root, "nope", true, &mut Scripted::says(""), &mut Vec::new());
        assert!(error.unwrap_err().to_string().contains("no extension named `nope`"));
    }

    #[test]
    fn an_extension_without_capabilities_says_so() {
        let manifest = SCRIPT.split("[capabilities]").next().unwrap();
        let s = setup(manifest);
        let mut out = Vec::new();
        install(&s.root, &s.source, true, &mut Scripted::says(""), &mut out).unwrap();
        assert!(text(out).contains("It declares no capabilities."));
    }
}
