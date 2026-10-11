//! The `daimos` command: talks to the resident process over its local socket.

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use daimos_core::index::IndexStatus;
use daimos_core::ipc::{ClientError, Endpoint, Request, Response, send};

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

#[cfg(test)]
mod tests {
    use daimos_core::index::SourceStatus;

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
}
