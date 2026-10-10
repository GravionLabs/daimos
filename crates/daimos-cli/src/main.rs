//! The `daimos` command: talks to the resident process over its local socket.

use std::process::ExitCode;

use clap::{Parser, Subcommand};
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
}

impl From<Command> for Request {
    fn from(command: Command) -> Self {
        match command {
            Command::Toggle => Request::Toggle,
            Command::Show { query } => Request::Show { query },
            Command::Hide => Request::Hide,
            Command::Quit => Request::Quit,
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
