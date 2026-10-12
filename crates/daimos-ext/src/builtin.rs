//! The built-in action kinds that the daimos runs for an extension: `open` and `run`.
//!
//! `copy` and `paste` need the clipboard of the window and are run by the window itself.

use std::io;
use std::process::{Command, Stdio};

/// Opens things and runs command lines on behalf of an extension's actions.
pub trait Opener: Send + Sync {
    /// Opens a URL or a path with the default application.
    fn open(&self, target: &str) -> io::Result<()>;

    /// Starts a command line (a program and its arguments, quotes group words) without a shell.
    fn run(&self, command_line: &str) -> io::Result<()>;
}

/// Opens things with the tools of the operating system.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemOpener;

impl Opener for SystemOpener {
    fn open(&self, target: &str) -> io::Result<()> {
        if target.trim().is_empty() || target.starts_with('-') {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "nothing to open"));
        }
        let argv: Vec<String> = if cfg!(target_os = "macos") {
            vec!["open".into(), target.into()]
        } else if cfg!(windows) {
            vec!["rundll32".into(), "url.dll,FileProtocolHandler".into(), target.into()]
        } else {
            vec!["xdg-open".into(), target.into()]
        };
        spawn_detached(&argv)
    }

    fn run(&self, command_line: &str) -> io::Result<()> {
        let argv = split_command_line(command_line);
        if argv.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "nothing to run"));
        }
        spawn_detached(&argv)
    }
}

/// Starts `argv` without waiting for it; a helper thread reaps it when it ends.
fn spawn_detached(argv: &[String]) -> io::Result<()> {
    let mut command = Command::new(&argv[0]);
    command.args(&argv[1..]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own process group: closing Daimos must not take the program with it.
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command.spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Splits a command line into words. Whitespace separates words; single or double quotes group
/// words (and are dropped); there is no escaping, so a Windows path keeps its backslashes.
pub fn split_command_line(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    for c in line.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => word.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            (None, c) => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(word);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_line_is_split_at_whitespace() {
        assert_eq!(split_command_line("git status  -s"), ["git", "status", "-s"]);
        assert_eq!(split_command_line("  "), Vec::<String>::new());
        assert_eq!(split_command_line(""), Vec::<String>::new());
    }

    #[test]
    fn quotes_group_words_and_keep_backslashes() {
        assert_eq!(
            split_command_line(r#"code "my project" 'a b' C:\dir\file.txt"#),
            ["code", "my project", "a b", r"C:\dir\file.txt"]
        );
        assert_eq!(split_command_line(r#"echo "" x"#), ["echo", "", "x"]);
        assert_eq!(split_command_line(r#"say "it's""#), ["say", "it's"]);
        assert_eq!(split_command_line(r#"a "unclosed b"#), ["a", "unclosed b"]);
    }

    #[test]
    fn the_system_opener_refuses_empty_targets_and_options() {
        for target in ["", "  ", "--help"] {
            let error = SystemOpener.open(target).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{target:?}");
        }
        assert_eq!(SystemOpener.run("  ").unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn running_a_program_that_does_not_exist_is_an_error() {
        assert!(SystemOpener.run("daimos-no-such-program --flag").is_err());
    }
}
