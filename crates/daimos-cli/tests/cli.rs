//! Runs the real `daimos` binary against a fake resident process.
#![cfg(unix)]

use std::process::Command;
#[cfg(target_os = "linux")]
use std::sync::{Arc, Mutex};

#[cfg(target_os = "linux")]
use daimos_core::ipc::{Endpoint, Request, Response, Server};

fn daimos(runtime_dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_daimos"))
        .args(args)
        .env("XDG_RUNTIME_DIR", runtime_dir)
        .env("HOME", runtime_dir)
        .output()
        .expect("run daimos")
}

#[test]
fn without_a_running_process_the_exit_code_is_2_and_the_message_is_clear() {
    let dir = tempfile::tempdir().unwrap();
    let output = daimos(dir.path(), &["toggle"]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("not running"));
}

#[cfg(target_os = "linux")]
#[test]
fn every_subcommand_reaches_the_process() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    let server = Server::start(Endpoint::Path(dir.path().join("daimos.sock")), move |request| {
        log.lock().unwrap().push(request);
        Response::Ok
    })
    .unwrap();

    for args in [&["toggle"][..], &["show"], &["show", "calc 2+2"], &["hide"], &["quit"]] {
        let output = daimos(dir.path(), args);
        assert!(output.status.success(), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
    }

    assert_eq!(
        *seen.lock().unwrap(),
        [
            Request::Toggle,
            Request::Show { query: None },
            Request::Show { query: Some("calc 2+2".into()) },
            Request::Hide,
            Request::Quit,
        ]
    );
    server.stop();
}
