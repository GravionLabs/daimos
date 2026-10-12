//! The `daimos` command: talks to the resident process over its local socket.

mod install;

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use daimos_core::index::IndexStatus;
use daimos_core::ipc::{ClientError, Endpoint, ExtensionList, Request, Response, send};

/// Exit code when no Daimos process listens on the socket.
const NOT_RUNNING: u8 = 2;

#[derive(Parser)]
#[command(name = "daimos", version, about = "Control the running Daimos")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show the window if it is hidden, hide it if it is shown
    Toggle,
    /// Show the window, optionally with text in the search field
    Show { query: Option<String> },
    /// Hide the window
    Hide,
    /// Quit the running Daimos
    Quit,
    /// The application index
    Index {
        #[command(subcommand)]
        command: IndexCommand,
    },
    /// Extensions
    Ext {
        #[command(subcommand)]
        command: ExtCommand,
    },
}

#[derive(Subcommand)]
enum ExtCommand {
    /// List the extensions with their version, runtime and state
    List,
    /// Install an extension from a directory or a git URL after you confirm its capabilities
    Install {
        /// A local directory with an extension.toml, or a git URL
        source: String,
        /// Confirm without asking (for scripts)
        #[arg(long, short)]
        yes: bool,
    },
    /// Review and confirm the capabilities of an extension, for example after an update
    Confirm {
        /// The name of the installed extension
        name: String,
        /// Confirm without asking (for scripts)
        #[arg(long, short)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum IndexCommand {
    /// Scan all application sources now and wait until the index is rebuilt
    Rebuild,
    /// Show how many applications each source contributed
    Status,
}

/// What a command does: talk to the running Daimos, or work on the extension directory.
#[derive(Debug, PartialEq, Eq)]
enum Action {
    Send(Request),
    Local(Local),
}

/// The commands that work on the extension directory.
#[derive(Debug, PartialEq, Eq)]
enum Local {
    Install { source: String, yes: bool },
    Confirm { name: String, yes: bool },
}

impl From<Command> for Action {
    fn from(command: Command) -> Self {
        let send = Action::Send;
        match command {
            Command::Toggle => send(Request::Toggle),
            Command::Show { query } => send(Request::Show { query }),
            Command::Hide => send(Request::Hide),
            Command::Quit => send(Request::Quit),
            Command::Index { command: IndexCommand::Rebuild } => send(Request::IndexRebuild),
            Command::Index { command: IndexCommand::Status } => send(Request::IndexStatus),
            Command::Ext { command: ExtCommand::List } => send(Request::ExtList),
            Command::Ext { command: ExtCommand::Install { source, yes } } => {
                Action::Local(Local::Install { source, yes })
            }
            Command::Ext { command: ExtCommand::Confirm { name, yes } } => {
                Action::Local(Local::Confirm { name, yes })
            }
        }
    }
}

/// `ext install` and `ext confirm`: they do not need the running Daimos.
fn run_local(action: Local) -> ExitCode {
    let Some(root) = daimos_ext::dir::extensions_dir() else {
        eprintln!("daimos: this system has no configuration directory");
        return ExitCode::FAILURE;
    };
    let mut out = std::io::stdout();
    let result = match action {
        Local::Install { source, yes } => {
            install::install(&root, &source, yes, &mut install::Stdin, &mut out)
        }
        Local::Confirm { name, yes } => {
            install::confirm(&root, &name, yes, &mut install::Stdin, &mut out)
        }
    };
    match result {
        Ok(install::Outcome::Confirmed | install::Outcome::NothingToConfirm) => ExitCode::SUCCESS,
        Ok(install::Outcome::Declined(message)) => {
            eprintln!("daimos: {message}");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("daimos: {error}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let request = match Action::from(cli.command) {
        Action::Send(request) => request,
        Action::Local(local) => return run_local(local),
    };
    let endpoint = match Endpoint::default_for_user() {
        Ok(endpoint) => endpoint,
        Err(error) => {
            eprintln!("daimos: {error}");
            return ExitCode::FAILURE;
        }
    };
    match send(&endpoint, &request) {
        Ok(Response::Ok) => ExitCode::SUCCESS,
        Ok(Response::Index(status)) => {
            print!("{}", format_status(&status));
            ExitCode::SUCCESS
        }
        Ok(Response::Extensions(list)) => {
            print!("{}", format_extensions(&list));
            ExitCode::SUCCESS
        }
        Ok(Response::Error { message }) => {
            eprintln!("daimos: {message}");
            ExitCode::FAILURE
        }
        Err(ClientError::NotRunning) => {
            eprintln!("daimos: Daimos is not running; start it first");
            ExitCode::from(NOT_RUNNING)
        }
        Err(error) => {
            eprintln!("daimos: {error}");
            ExitCode::FAILURE
        }
    }
}

fn format_status(status: &IndexStatus) -> String {
    let mut out =
        format!("{} apps{}\n", status.apps, if status.indexing { " (reindexing)" } else { "" });
    for source in &status.sources {
        out.push_str(&format!("  {:<10} {}\n", source.source, source.apps));
    }
    if status.last_indexed > 0 {
        out.push_str(&format!(
            "last indexed at {} (seconds since the epoch)\n",
            status.last_indexed
        ));
    } else {
        out.push_str("not indexed in this run yet\n");
    }
    out
}

fn format_extensions(list: &ExtensionList) -> String {
    if list.extensions.is_empty() {
        return "no extensions installed\n".into();
    }
    let none = "-";
    let cell = |value: &Option<String>| value.clone().unwrap_or_else(|| none.into());
    let width = |header: &str, values: Vec<String>| {
        values.iter().map(|v| v.chars().count()).chain([header.len()]).max().unwrap_or(0)
    };
    let name_w = width("NAME", list.extensions.iter().map(|e| e.name.clone()).collect());
    let version_w = width("VERSION", list.extensions.iter().map(|e| cell(&e.version)).collect());
    let runtime_w = width("RUNTIME", list.extensions.iter().map(|e| cell(&e.runtime)).collect());
    let mut out = format!(
        "{:<name_w$}  {:<version_w$}  {:<runtime_w$}  STATE\n",
        "NAME", "VERSION", "RUNTIME"
    );
    for e in &list.extensions {
        out.push_str(&format!(
            "{:<name_w$}  {:<version_w$}  {:<runtime_w$}  {}\n",
            e.name,
            cell(&e.version),
            cell(&e.runtime),
            e.state
        ));
        if let Some(message) = &e.message {
            out.push_str(&format!("  {message}\n"));
        }
        if e.state == "needs-confirmation" {
            out.push_str(&format!(
                "  run `daimos ext confirm {}` to review its capabilities\n",
                e.name
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use daimos_core::index::SourceStatus;
    use daimos_core::ipc::ExtensionInfo;

    use super::*;

    #[test]
    fn the_status_lists_every_source() {
        let status = IndexStatus {
            apps: 5,
            sources: vec![SourceStatus { source: "linux".into(), apps: 5 }],
            indexing: true,
            last_indexed: 0,
        };
        assert_eq!(
            format_status(&status),
            "5 apps (reindexing)\n  linux      5\nnot indexed in this run yet\n"
        );
    }

    fn info(
        name: &str,
        version: Option<&str>,
        state: &str,
        message: Option<&str>,
    ) -> ExtensionInfo {
        ExtensionInfo {
            name: name.into(),
            version: version.map(Into::into),
            runtime: version.map(|_| "script".into()),
            state: state.into(),
            message: message.map(Into::into),
        }
    }

    #[test]
    fn the_extension_table_shows_an_enabled_and_an_errored_extension() {
        let list = ExtensionList {
            extensions: vec![
                info("broken", None, "error", Some("extension.toml:2: extension.name: bad")),
                info("github-repos", Some("0.2.0"), "enabled", None),
            ],
        };
        assert_eq!(
            format_extensions(&list),
            "NAME          VERSION  RUNTIME  STATE\n\
             broken        -        -        error\n  extension.toml:2: extension.name: bad\n\
             github-repos  0.2.0    script   enabled\n"
        );
    }

    #[test]
    fn an_extension_that_needs_confirmation_shows_the_command_to_run() {
        let list = ExtensionList {
            extensions: vec![info("notes", Some("1.0.0"), "needs-confirmation", None)],
        };
        assert_eq!(
            format_extensions(&list),
            "NAME   VERSION  RUNTIME  STATE\n\
             notes  1.0.0    script   needs-confirmation\n  \
             run `daimos ext confirm notes` to review its capabilities\n"
        );
    }

    #[test]
    fn install_and_confirm_work_without_the_running_daimos() {
        let cli = Cli::try_parse_from(["daimos", "ext", "install", "./notes", "--yes"]).unwrap();
        assert_eq!(
            Action::from(cli.command),
            Action::Local(Local::Install { source: "./notes".into(), yes: true })
        );
        let cli = Cli::try_parse_from(["daimos", "ext", "install", "https://x/y.git"]).unwrap();
        assert_eq!(
            Action::from(cli.command),
            Action::Local(Local::Install { source: "https://x/y.git".into(), yes: false })
        );
        let cli = Cli::try_parse_from(["daimos", "ext", "confirm", "notes"]).unwrap();
        assert_eq!(
            Action::from(cli.command),
            Action::Local(Local::Confirm { name: "notes".into(), yes: false })
        );
        assert!(Cli::try_parse_from(["daimos", "ext", "install"]).is_err());
        assert!(Cli::try_parse_from(["daimos", "ext", "confirm"]).is_err());
    }

    #[test]
    fn an_empty_list_says_so() {
        assert_eq!(
            format_extensions(&ExtensionList { extensions: Vec::new() }),
            "no extensions installed\n"
        );
    }

    #[test]
    fn ext_list_is_a_subcommand() {
        let cli = Cli::try_parse_from(["daimos", "ext", "list"]).unwrap();
        assert_eq!(Action::from(cli.command), Action::Send(Request::ExtList));
        assert!(Cli::try_parse_from(["daimos", "ext"]).is_err());
    }
}
