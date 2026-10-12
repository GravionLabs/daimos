//! Conformance of the two script modes: the same program, installed once as a one-shot and once
//! as a long-running extension, answers every query with identical items. The tests skip when
//! there is no Python.
//!
//! `Mode`, `install` and `items` are written to be reused by the conformance suite over the
//! example extensions: pass another `fixture` and `args` and compare the results.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use daimos_core::model::Item;
use daimos_core::provider::QueryContext;
use daimos_ext::dir::load_extension;
use daimos_ext::{Backend, ScriptBackend};
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

/// The Python of this machine: `python` on Windows, `python3` elsewhere, with the other as a
/// fallback. Resolved once.
fn python() -> Option<&'static str> {
    static PYTHON: OnceLock<Option<&'static str>> = OnceLock::new();
    *PYTHON.get_or_init(|| {
        let candidates: &[&str] =
            if cfg!(windows) { &["python", "py", "python3"] } else { &["python3", "python"] };
        candidates.iter().copied().find(|name| {
            Command::new(name).arg("--version").output().is_ok_and(|out| out.status.success())
        })
    })
}

macro_rules! python_or_skip {
    () => {
        match python() {
            Some(python) => python,
            None => {
                eprintln!("skipped: no python3 or python on PATH");
                return;
            }
        }
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    OneShot,
    LongRunning,
}

/// An extension called `echo` with the command `search`, installed in a temporary directory.
struct Installed {
    _root: tempfile::TempDir,
    backend: ScriptBackend,
}

/// Installs `fixture` (a file in `tests/fixtures`) as the extension `echo`. `args` follow the
/// script on the command line; a long-running extension gets `loop_arg` as well.
fn install(python: &str, fixture: &str, loop_arg: &str, mode: Mode) -> Installed {
    let root = tempfile::tempdir().unwrap();
    let dir: PathBuf = root.path().join("echo");
    std::fs::create_dir_all(&dir).unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(fixture);
    std::fs::copy(source, dir.join(fixture)).unwrap();
    let (loop_arg, long_running) = match mode {
        Mode::OneShot => (String::new(), false),
        Mode::LongRunning => (format!(", {loop_arg:?}"), true),
    };
    let manifest = format!(
        "[extension]\nname = \"echo\"\ntitle = \"Echo\"\nversion = \"1.0.0\"\nruntime = \"script\"\n\n\
         [[commands]]\nname = \"search\"\ntitle = \"Search\"\nkeywords = [\"e\"]\n\n\
         [runtime.script]\ncommand = [{python:?}, {fixture:?}{loop_arg}]\nlong_running = {long_running}\n"
    );
    std::fs::write(dir.join("extension.toml"), manifest).unwrap();
    let backend = ScriptBackend::new();
    backend.load(&load_extension(&dir).unwrap());
    Installed { _root: root, backend }
}

/// Every item the command `search` returns for `text`, in order.
async fn items(installed: &Installed, text: &str) -> Vec<Item> {
    let ctx =
        QueryContext { text: text.into(), keyword_routed: true, cancel: CancellationToken::new() };
    installed
        .backend
        .query("echo", "search", ctx)
        .flat_map(|batch| futures::stream::iter(batch.items))
        .collect()
        .await
}

const QUERIES: [&str; 5] = ["a", "hello world", "", "ünï ☃", "x\"y\\z"];

#[tokio::test]
async fn both_modes_return_identical_items() {
    let python = python_or_skip!();
    let one_shot = install(python, "echo.py", "loop", Mode::OneShot);
    let long_running = install(python, "echo.py", "loop", Mode::LongRunning);
    for text in QUERIES {
        let expected = items(&one_shot, text).await;
        assert_eq!(expected.len(), 3, "{text:?}");
        assert_eq!(items(&long_running, text).await, expected, "{text:?}");
    }
    // Asked again, the warm process still answers the same.
    let expected = items(&one_shot, "a").await;
    assert_eq!(items(&long_running, "a").await, expected);
}

#[tokio::test]
async fn the_items_carry_titles_subtitles_and_actions_in_both_modes() {
    let python = python_or_skip!();
    for mode in [Mode::OneShot, Mode::LongRunning] {
        let installed = install(python, "echo.py", "loop", mode);
        let found = items(&installed, "q").await;
        let titles: Vec<_> = found.iter().map(|item| item.title.as_str()).collect();
        assert_eq!(titles, ["echo q", "again q", "last q"], "{mode:?}");
        assert_eq!(found[0].subtitle.as_deref(), Some("from echo.py"), "{mode:?}");
        let actions: Vec<_> = found[0].actions.iter().map(|action| action.id.as_str()).collect();
        assert_eq!(actions, ["open", "star"], "{mode:?}");
    }
}

#[test]
fn both_modes_answer_a_custom_action_with_the_same_toast() {
    let python = python_or_skip!();
    for mode in [Mode::OneShot, Mode::LongRunning] {
        let installed = install(python, "echo.py", "loop", mode);
        assert_eq!(
            installed.backend.custom_action("echo", "search", "item-7", "star"),
            Ok(Some("Starred item-7".to_owned())),
            "{mode:?}"
        );
        assert_eq!(installed.backend.custom_action("echo", "search", "x", "quiet"), Ok(None));
    }
}

/// The 30 ms budget itself is checked by the benchmark (`benches/script_warm.rs`, p95 with a
/// tolerance). This guards against a warm query that is slow by an order of magnitude, with a
/// margin that shared CI runners keep: the median of 20 warm queries stays below 250 ms.
#[tokio::test]
async fn a_warm_long_running_query_is_fast() {
    let python = python_or_skip!();
    let installed = install(python, "echo.py", "loop", Mode::LongRunning);
    assert_eq!(items(&installed, "warm up").await.len(), 3);
    let mut times = Vec::new();
    for _ in 0..20 {
        let started = std::time::Instant::now();
        assert_eq!(items(&installed, "lau").await.len(), 3);
        times.push(started.elapsed());
    }
    times.sort();
    assert!(times[10] < std::time::Duration::from_millis(250), "{times:?}");
}
