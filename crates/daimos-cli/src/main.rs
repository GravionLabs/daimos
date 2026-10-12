//! The `daimos` command: talks to the resident process over its local socket.

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
}

#[derive(Subcommand)]
enum IndexCommand {
    /// Scan all application sources now and wait until the index is rebuilt
    Rebuild,
    /// Show how many applications each source contributed
    Status,
}

impl From<Command> for Request {
    fn from(command: Command) -> Self {
        match command {
            Command::Toggle => Request::Toggle,
            Command::Show { query } => Request::Show { query },
            Command::Hide => Request::Hide,
            Command::Quit => Request::Quit,
            Command::Index { command: IndexCommand::Rebuild } => Request::IndexRebuild,
            Command::Index { command: IndexCommand::Status } => Request::IndexStatus,
            Command::Ext { command: ExtCommand::List } => Request::ExtList,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let endpoint = match Endpoint::default_for_user() {
        Ok(endpoint) => endpoint,
        Err(error) => {
            eprintln!("daimos: {error}");
            return ExitCode::FAILURE;
        }
    };
    match send(&endpoint, &cli.command.into()) {
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
    fn an_empty_list_says_so() {
        assert_eq!(
            format_extensions(&ExtensionList { extensions: Vec::new() }),
            "no extensions installed\n"
        );
    }

    #[test]
    fn ext_list_is_a_subcommand() {
        let cli = Cli::try_parse_from(["daimos", "ext", "list"]).unwrap();
        assert_eq!(Request::from(cli.command), Request::ExtList);
        assert!(Cli::try_parse_from(["daimos", "ext"]).is_err());
    }
}
