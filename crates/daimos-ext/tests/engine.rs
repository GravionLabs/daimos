//! Extensions in a running `QueryEngine`: commands are found by keyword, reloads apply live, and
//! a broken manifest shows one error item.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use daimos_core::QueryEngine;
use daimos_core::model::{Item, ItemBatch};
use daimos_core::provider::{Provider, QueryContext};
use daimos_core::query::{ResultBatch, Sink};
use daimos_ext::{Backend, ExtensionHost, Registry};
use futures::StreamExt;
use futures::stream::{self, BoxStream};
use tokio::runtime::Handle;
use tokio::sync::mpsc;

/// Answers every query with one item that echoes the text.
struct Echo;

impl Backend for Echo {
    fn query(&self, ext: &str, _: &str, ctx: QueryContext) -> BoxStream<'static, ItemBatch> {
        let item = Item::new(format!("{ext}/echo"), ctx.text);
        stream::once(async move { ItemBatch::from(vec![item]) }).boxed()
    }
}

fn manifest(keyword: &str) -> String {
    format!(
        "[extension]\nname = \"ext\"\ntitle = \"T\"\nversion = \"1.0.0\"\nruntime = \"script\"\n\n\
         [[commands]]\nname = \"go\"\ntitle = \"Go\"\nkeywords = [\"{keyword}\"]\n\n\
         [runtime.script]\ncommand = [\"run.sh\"]\n"
    )
}

fn write(root: &Path, content: &str) {
    std::fs::create_dir_all(root.join("ext")).unwrap();
    std::fs::write(root.join("ext").join("extension.toml"), content).unwrap();
    // The empty set of capabilities is confirmed, so the extension may run.
    std::fs::write(
        root.join("ext").join(".consent.json"),
        r#"{"version":"1.0.0","capabilities":{}}"#,
    )
    .unwrap();
}

struct Setup {
    engine: QueryEngine,
    host: Arc<ExtensionHost>,
    rx: mpsc::UnboundedReceiver<ResultBatch>,
}

fn setup(root: &Path) -> Setup {
    let (tx, rx) = mpsc::unbounded_channel();
    let sink: Sink = Arc::new(move |b| {
        let _ = tx.send(b);
    });
    let registry = Arc::new(Registry::new());
    let source = Arc::clone(&registry);
    let engine = QueryEngine::new(Vec::<Arc<dyn Provider>>::new(), sink, Handle::current())
        .with_dynamic(Arc::new(move || source.providers()));
    let host = Arc::new(ExtensionHost::new(root.to_path_buf(), Arc::new(Echo), registry));
    host.reload_all();
    Setup { engine, host, rx }
}

/// The titles of the items of one query, after every provider reported `done`.
async fn titles(s: &mut Setup, session: u32, text: &str, providers: usize) -> Vec<String> {
    s.engine.query(session, text);
    let mut done = 0;
    let mut out = Vec::new();
    while done < providers {
        let b = tokio::time::timeout(Duration::from_secs(2), s.rx.recv()).await.unwrap().unwrap();
        done += usize::from(b.done);
        out.extend(b.items.into_iter().map(|i| i.title));
    }
    out
}

#[tokio::test]
async fn a_command_is_found_by_keyword_and_again_after_a_manifest_change() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), &manifest("old"));
    let mut s = setup(root.path());
    assert_eq!(titles(&mut s, 1, "old hello", 1).await, ["hello"]);

    write(root.path(), &manifest("new"));
    s.host.reload("ext");
    assert_eq!(titles(&mut s, 2, "new again", 1).await, ["again"]);
    assert_eq!(s.engine.providers().len(), 1);
}

#[tokio::test]
async fn a_broken_manifest_shows_one_error_item_with_the_message() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "[extension]\nname = \n");
    let mut s = setup(root.path());
    let titles_found = titles(&mut s, 1, "", 1).await;
    assert_eq!(titles_found, ["ext could not be loaded"]);

    s.engine.query(2, "");
    let mut subtitle = None;
    loop {
        let b = tokio::time::timeout(Duration::from_secs(2), s.rx.recv()).await.unwrap().unwrap();
        if let Some(item) = b.items.first() {
            subtitle = item.subtitle.clone();
        }
        if b.done {
            break;
        }
    }
    assert!(subtitle.unwrap().contains("extension.toml:2"));

    // Fixing the file removes the error item and brings the command back.
    write(root.path(), &manifest("k"));
    s.host.reload("ext");
    assert!(titles(&mut s, 3, "", 1).await.iter().all(|t| !t.contains("could not")));
    assert_eq!(titles(&mut s, 4, "k x", 1).await, ["x"]);
}
