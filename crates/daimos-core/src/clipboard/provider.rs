//! The clipboard history as a provider: the `Clipboard History` command opens the whole list, and
//! the keyword `clip` searches it (`clip hello` filters the text entries with the fuzzy matcher).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use futures::StreamExt;
use futures::stream::{self, BoxStream};

use super::ClipboardHistory;
use super::history::Entry;
use crate::commands::OPEN_SETTINGS;
use crate::frecency::Frecency;
use crate::model::{Action, ActionKind, Detail, Icon, Item, ItemBatch, View};
use crate::provider::{ActionEffect, Provider, QueryContext};
use crate::ranker::{Candidate, Ranker};

/// The keyword that routes a query to the history.
pub const KEYWORD: &str = "clip";
/// The view the `Clipboard History` command pushes.
pub const HISTORY_VIEW: &str = "history";
const COMMAND_ID: &str = "clip:history";
const ENTRY_PREFIX: &str = "clip:#";
/// Entries shown at most; the history itself keeps more.
const SHOWN: usize = 100;
/// Longest title, in characters.
const TITLE_CHARS: usize = 100;
/// Longest text in the detail pane, in characters.
const DETAIL_CHARS: usize = 2000;
/// Images up to this size are shown in the detail pane; larger ones only give their size.
const PREVIEW_BYTES: usize = 1024 * 1024;

pub struct ClipboardProvider {
    history: Arc<ClipboardHistory>,
    /// Whether the capture is on; while it is off the list says so.
    enabled: Arc<AtomicBool>,
    ranker: std::sync::Mutex<Ranker>,
    frecency: Frecency,
}

impl ClipboardProvider {
    pub fn new(history: Arc<ClipboardHistory>, enabled: Arc<AtomicBool>) -> Self {
        Self {
            history,
            enabled,
            ranker: std::sync::Mutex::new(Ranker::new()),
            frecency: Frecency::in_memory().expect("in-memory frecency"),
        }
    }

    fn command() -> Item {
        let mut item = Item::new(COMMAND_ID, "Clipboard History");
        item.subtitle = Some(format!("Everything you copied; type {KEYWORD} <text> to search"));
        item.keywords =
            ["clipboard", "paste", "copied", "history", KEYWORD].map(str::to_owned).to_vec();
        item.icon = Some(Icon::Named { name: "📋".into() });
        item.actions = vec![Action {
            id: "open".into(),
            title: "Open Clipboard History".into(),
            shortcut: Some("Enter".into()),
            kind: ActionKind::Push,
            payload: Some(HISTORY_VIEW.into()),
        }];
        item
    }

    /// What the list says while there is nothing to list.
    fn notice(&self) -> Item {
        let (title, subtitle) = if self.enabled.load(Ordering::SeqCst) {
            ("Nothing copied yet", "Copy something and it shows up here")
        } else {
            ("The clipboard history is off", "Turn it on in the settings to remember what you copy")
        };
        let mut item = Item::new("clip:notice", title);
        item.subtitle = Some(subtitle.into());
        item.icon = Some(Icon::Named { name: "📋".into() });
        item.actions = vec![Action {
            id: "settings".into(),
            title: "Open Settings".into(),
            shortcut: Some("Enter".into()),
            kind: ActionKind::Push,
            payload: Some(OPEN_SETTINGS.into()),
        }];
        item
    }

    fn entry_items(&self, now: i64) -> Vec<Item> {
        match self.history.list() {
            Ok(entries) => entries.into_iter().take(SHOWN).map(|e| self.item(&e, now)).collect(),
            Err(error) => {
                log::warn!("cannot read the clipboard history: {error}");
                Vec::new()
            }
        }
    }

    fn item(&self, entry: &Entry, now: i64) -> Item {
        let age = age(now - entry.copied_at);
        let mut item = if let Some(text) = &entry.text {
            let mut item = Item::new(format!("{ENTRY_PREFIX}{}", entry.id), preview(text));
            let lines = text.lines().count().max(1);
            item.subtitle = Some(format!(
                "Text · {lines} {} · {age}",
                if lines == 1 { "line" } else { "lines" }
            ));
            item.icon = Some(Icon::Named { name: "📝".into() });
            item.detail = Some(Detail::Markdown { text: fenced(text) });
            item
        } else {
            let mut item = Item::new(format!("{ENTRY_PREFIX}{}", entry.id), "Image");
            item.subtitle = Some(format!("Image · {} · {age}", size(entry.size)));
            item.icon = Some(Icon::Named { name: "🖼".into() });
            item.detail = Some(self.image_detail(entry));
            item
        };
        if entry.pinned {
            item.accessories.push("pinned".into());
        }
        item.actions = actions(entry);
        item
    }

    fn image_detail(&self, entry: &Entry) -> Detail {
        let caption = format!("Image, {}", size(entry.size));
        if entry.size <= PREVIEW_BYTES
            && let Ok(Some(bytes)) = self.history.image(entry.id)
        {
            return Detail::Image {
                data_url: format!("data:image/png;base64,{}", base64(&bytes)),
                caption,
            };
        }
        Detail::Metadata {
            entries: vec![("Type".into(), "Image".into()), ("Size".into(), size(entry.size))],
        }
    }

    fn rank(&self, text: &str, items: Vec<Item>) -> Vec<Item> {
        let candidates: Vec<_> = items.iter().map(|item| Candidate { item, weight: 1.0 }).collect();
        let ranked = self.ranker.lock().unwrap_or_else(|e| e.into_inner()).rank(
            text,
            &candidates,
            &self.frecency,
            0,
            SHOWN,
        );
        ranked
            .into_iter()
            .map(|r| Item { highlight: r.title_indices, ..items[r.index].clone() })
            .collect()
    }
}

fn actions(entry: &Entry) -> Vec<Action> {
    let act = |id: &str, title: &str, shortcut: &str, kind, payload: Option<String>| Action {
        id: id.into(),
        title: title.into(),
        shortcut: Some(shortcut.into()),
        kind,
        payload,
    };
    let mut actions = Vec::new();
    if let Some(text) = &entry.text {
        actions.push(act("paste", "Paste", "Enter", ActionKind::Paste, Some(text.clone())));
        actions.push(act("copy", "Copy", "Ctrl+Enter", ActionKind::Copy, Some(text.clone())));
    }
    let pin = if entry.pinned { ("unpin", "Unpin") } else { ("pin", "Pin") };
    actions.push(act(pin.0, pin.1, "Ctrl+P", ActionKind::Run, None));
    actions.push(act("delete", "Delete", "Ctrl+Backspace", ActionKind::Run, None));
    actions
}

impl Provider for ClipboardProvider {
    fn id(&self) -> &str {
        "clipboard"
    }

    fn keyword(&self) -> Option<&str> {
        Some(KEYWORD)
    }

    /// Below the apps: a query for an app should not list the history first.
    fn weight(&self) -> f32 {
        0.5
    }

    fn query(&self, ctx: QueryContext) -> BoxStream<'static, ItemBatch> {
        let text = ctx.text.trim().to_owned();
        let items = if ctx.keyword_routed {
            let entries = self.entry_items(now());
            let found = if text.is_empty() { entries } else { self.rank(&text, entries) };
            if found.is_empty() && text.is_empty() { vec![self.notice()] } else { found }
        } else if text.is_empty() {
            Vec::new()
        } else {
            self.rank(&text, vec![Self::command()])
        };
        stream::once(async move { ItemBatch::from(items) }).boxed()
    }

    fn view(&self, view_id: &str) -> Option<View> {
        if view_id != HISTORY_VIEW {
            return None;
        }
        let mut items = self.entry_items(now());
        if items.is_empty() {
            items.push(self.notice());
        }
        Some(View::List { items })
    }

    fn run_action(&self, item_id: &str, action_id: &str) -> Option<Result<ActionEffect, String>> {
        let id: i64 = item_id.strip_prefix(ENTRY_PREFIX)?.parse().ok()?;
        let result = match action_id {
            "pin" => self.history.set_pinned(id, true),
            "unpin" => self.history.set_pinned(id, false),
            "delete" => self.history.remove(id),
            other => return Some(Err(format!("no action {other}"))),
        };
        Some(result.map(|()| ActionEffect::Stay).map_err(|e| e.to_string()))
    }
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// The first line that is not blank, cut to a title.
fn preview(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    if line.chars().count() > TITLE_CHARS {
        let cut: String = line.chars().take(TITLE_CHARS).collect();
        format!("{cut}…")
    } else {
        line.to_owned()
    }
}

/// The text as a Markdown code block, so the detail pane shows it as it is.
fn fenced(text: &str) -> String {
    let shown: String = text.chars().take(DETAIL_CHARS).collect();
    let more = if text.chars().count() > DETAIL_CHARS { "\n…" } else { "" };
    // A fence of backticks inside the text would end the block early.
    format!("```\n{}{more}\n```", shown.replace("```", "``\u{200b}`"))
}

pub fn age(seconds: i64) -> String {
    match seconds.max(0) {
        s if s < 60 => "just now".into(),
        s if s < 3600 => format!("{} min ago", s / 60),
        s if s < 86_400 => format!("{} h ago", s / 3600),
        s => format!("{} d ago", s / 86_400),
    }
}

pub fn size(bytes: usize) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.1} KB", b as f64 / 1024.0),
        b => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::clipboard::{ClipboardContent, ClipboardSnapshot, Limits};

    fn provider() -> (ClipboardProvider, Arc<ClipboardHistory>, Arc<AtomicBool>) {
        let history = Arc::new(ClipboardHistory::in_memory(Limits::default()).unwrap());
        let enabled = Arc::new(AtomicBool::new(true));
        (ClipboardProvider::new(Arc::clone(&history), Arc::clone(&enabled)), history, enabled)
    }

    fn text(history: &ClipboardHistory, s: &str) -> i64 {
        let snapshot =
            ClipboardSnapshot { content: ClipboardContent::Text(s.into()), concealed: false };
        history.record(&snapshot).unwrap().unwrap()
    }

    async fn query(provider: &ClipboardProvider, text: &str, keyword_routed: bool) -> Vec<Item> {
        let ctx =
            QueryContext { text: text.into(), keyword_routed, cancel: CancellationToken::new() };
        provider.query(ctx).collect::<Vec<_>>().await.into_iter().flat_map(|b| b.items).collect()
    }

    #[tokio::test]
    async fn the_command_is_found_by_its_name_and_keywords() {
        let (provider, ..) = provider();
        for typed in ["clipboard", "hist", "paste"] {
            let items = query(&provider, typed, false).await;
            assert_eq!(items.len(), 1, "{typed}");
            assert_eq!(items[0].id, "clip:history");
        }
        assert!(query(&provider, "", false).await.is_empty());
        assert!(query(&provider, "firefox", false).await.is_empty());
    }

    #[tokio::test]
    async fn the_keyword_lists_the_newest_first_and_filters_with_the_matcher() {
        let (provider, history, _) = provider();
        let first = text(&history, "alpha text");
        std::thread::sleep(std::time::Duration::from_millis(1100));
        text(&history, "beta text");
        let all = query(&provider, "", true).await;
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].title, "beta text");
        let found = query(&provider, "alp", true).await;
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, format!("clip:#{first}"));
        assert_eq!(found[0].highlight, [0, 1, 2]);
    }

    #[tokio::test]
    async fn an_entry_pastes_with_enter_and_has_the_shortcut_actions() {
        let (provider, history, _) = provider();
        text(&history, "hello\nworld");
        let item = query(&provider, "", true).await.remove(0);
        assert_eq!(item.title, "hello");
        assert!(item.subtitle.as_deref().unwrap().starts_with("Text · 2 lines"));
        let actions: Vec<_> = item
            .actions
            .iter()
            .map(|a| (a.id.as_str(), a.shortcut.as_deref().unwrap(), a.kind))
            .collect();
        assert_eq!(
            actions,
            [
                ("paste", "Enter", ActionKind::Paste),
                ("copy", "Ctrl+Enter", ActionKind::Copy),
                ("pin", "Ctrl+P", ActionKind::Run),
                ("delete", "Ctrl+Backspace", ActionKind::Run),
            ]
        );
        assert_eq!(item.actions[0].payload.as_deref(), Some("hello\nworld"));
    }

    #[tokio::test]
    async fn pin_unpin_and_delete_change_the_list_and_keep_the_window() {
        let (provider, history, _) = provider();
        let a = text(&history, "a");
        text(&history, "b");
        let id = format!("clip:#{a}");
        assert_eq!(provider.run_action(&id, "pin"), Some(Ok(ActionEffect::Stay)));
        let items = query(&provider, "", true).await;
        assert_eq!(items[0].id, id, "pinned entries stay on top");
        assert_eq!(items[0].accessories, ["pinned"]);
        assert_eq!(items[0].actions[2].id, "unpin");
        assert_eq!(provider.run_action(&id, "unpin"), Some(Ok(ActionEffect::Stay)));
        assert_eq!(provider.run_action(&id, "delete"), Some(Ok(ActionEffect::Stay)));
        assert_eq!(query(&provider, "", true).await.len(), 1);
        assert!(provider.run_action(&id, "explode").unwrap().is_err());
        assert_eq!(provider.run_action("app:firefox", "pin"), None);
    }

    #[tokio::test]
    async fn images_show_their_size_and_only_offer_pin_and_delete() {
        let (provider, history, _) = provider();
        let snapshot = ClipboardSnapshot {
            content: ClipboardContent::Image(vec![0x89, b'P', b'N', b'G']),
            concealed: false,
        };
        history.record(&snapshot).unwrap();
        let item = query(&provider, "", true).await.remove(0);
        assert_eq!(item.title, "Image");
        assert_eq!(item.subtitle.as_deref().map(|s| s.starts_with("Image · 4 B")), Some(true));
        assert_eq!(
            item.actions.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            ["pin", "delete"]
        );
        let Some(Detail::Image { data_url, caption }) = item.detail else { panic!("no image") };
        assert_eq!(data_url, "data:image/png;base64,iVBORw==");
        assert_eq!(caption, "Image, 4 B");
    }

    #[tokio::test]
    async fn an_empty_history_says_whether_it_is_off() {
        let (provider, _, enabled) = provider();
        assert_eq!(query(&provider, "", true).await[0].title, "Nothing copied yet");
        enabled.store(false, Ordering::SeqCst);
        let items = query(&provider, "", true).await;
        assert_eq!(items[0].title, "The clipboard history is off");
        assert_eq!(items[0].actions[0].payload.as_deref(), Some(OPEN_SETTINGS));
    }

    #[test]
    fn the_history_view_lists_everything_and_unknown_views_are_none() {
        let (provider, history, _) = provider();
        text(&history, "x");
        let Some(View::List { items }) = provider.view("history") else { panic!("no list") };
        assert_eq!(items.len(), 1);
        assert!(provider.view("other").is_none());
    }

    #[test]
    fn titles_ages_sizes_and_base64() {
        assert_eq!(preview("\n  first line  \nsecond"), "first line");
        assert_eq!(preview(&"x".repeat(150)).chars().count(), TITLE_CHARS + 1);
        assert_eq!(
            [age(5), age(120), age(7200), age(200_000)],
            ["just now", "2 min ago", "2 h ago", "2 d ago"]
        );
        assert_eq!([size(10), size(2048), size(3 * 1024 * 1024)], ["10 B", "2.0 KB", "3.0 MB"]);
        assert_eq!([base64(b"M"), base64(b"Ma"), base64(b"Man")], ["TQ==", "TWE=", "TWFu"]);
        assert_eq!(fenced("a ``` b"), "```\na ``\u{200b}` b\n```");
    }
}
