//! The JSON Lines protocol of script extensions (`docs/specs/extensions.md`).
//!
//! This module is a codec only: the types of the messages, the conversion of items to
//! [`daimos_core::model::Item`], and reading and writing single lines. Starting the process is the
//! job of the script host.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{self, BufRead, Write};

use daimos_core::model::{self, ActionKind, Detail, Icon};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// A request from the daimos to the extension. Every request carries an `id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Request {
    /// Asks for the items of `command` for the text the user typed.
    Query {
        id: u64,
        command: String,
        text: String,
        #[serde(default)]
        preferences: BTreeMap<String, String>,
    },
    /// Runs an action that is not a built-in kind.
    Action { id: u64, command: String, item: String, action: String },
    /// Drops the work of an earlier request.
    Cancel { id: u64, cancels: u64 },
}

/// A response from the extension. It repeats the `id` of the request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Response {
    /// A batch of items; may come several times for one request, `done` ends the stream.
    Items {
        id: u64,
        #[serde(default)]
        items: Vec<Item>,
        #[serde(default)]
        done: bool,
    },
    /// The outcome of an `action` request.
    Result {
        id: u64,
        #[serde(default)]
        toast: Option<String>,
        /// A view to show; opaque until views are supported.
        #[serde(default)]
        view: Option<serde_json::Value>,
    },
}

impl Response {
    /// The id of the request this answers.
    pub fn id(&self) -> u64 {
        match self {
            Self::Items { id, .. } | Self::Result { id, .. } => *id,
        }
    }
}

/// The kind of an action. `custom` is sent back to the extension; the others the daimos runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Open,
    Copy,
    Paste,
    Run,
    Custom,
}

impl From<Kind> for ActionKind {
    fn from(kind: Kind) -> Self {
        match kind {
            Kind::Open => Self::Open,
            Kind::Copy => Self::Copy,
            Kind::Paste => Self::Paste,
            Kind::Run => Self::Run,
            Kind::Custom => Self::Custom,
        }
    }
}

/// One action of an item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    pub id: String,
    pub title: String,
    pub kind: Kind,
    /// What the action acts on: the URL or path of `open`, the text of `copy` and `paste`, the
    /// command line of `run`. `url` is accepted as another name for it.
    #[serde(default, alias = "url", skip_serializing_if = "Option::is_none")]
    pub payload: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut: Option<String>,
}

impl From<Action> for model::Action {
    fn from(action: Action) -> Self {
        Self {
            id: action.id,
            title: action.title,
            shortcut: action.shortcut,
            kind: action.kind.into(),
            payload: action.payload,
        }
    }
}

/// The detail of an item: markdown, or else a list of key and value pairs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetailSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub markdown: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metadata: Vec<(String, String)>,
}

/// One item as an extension writes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    /// A themed icon name, a `data:` URL, or a file path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<DetailSpec>,
}

fn icon_of(spec: &str) -> Icon {
    if spec.starts_with("data:") {
        Icon::DataUrl { url: spec.to_owned() }
    } else if spec.contains(['/', '\\']) {
        Icon::File { path: spec.to_owned() }
    } else {
        Icon::Named { name: spec.to_owned() }
    }
}

impl From<Item> for model::Item {
    fn from(item: Item) -> Self {
        let mut out = model::Item::new(item.id, item.title);
        out.subtitle = item.subtitle;
        out.icon = item.icon.as_deref().map(icon_of);
        out.keywords = item.keywords;
        out.actions = item.actions.into_iter().map(Into::into).collect();
        out.detail = item.detail.and_then(|detail| match detail.markdown {
            Some(text) => Some(Detail::Markdown { text }),
            None if !detail.metadata.is_empty() => {
                Some(Detail::Metadata { entries: detail.metadata })
            }
            None => None,
        });
        out
    }
}

/// Why a line could not be read.
#[derive(Debug)]
pub enum ProtocolError {
    /// The line is not a valid message; carries the line so the author can find it.
    Malformed {
        line: String,
        message: String,
    },
    Io(io::Error),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed { line, message } => {
                write!(f, "malformed message ({message}): {}", shorten(line))
            }
            Self::Io(error) => write!(f, "cannot read the output: {error}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

impl From<io::Error> for ProtocolError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// The first 200 characters of `line`, for messages.
fn shorten(line: &str) -> String {
    match line.char_indices().nth(200) {
        Some((end, _)) => format!("{}...", &line[..end]),
        None => line.to_owned(),
    }
}

/// Parses one line. A blank line is `Ok(None)`; unknown fields are ignored.
pub fn parse_line<T: DeserializeOwned>(line: &str) -> Result<Option<T>, ProtocolError> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    serde_json::from_str(trimmed).map(Some).map_err(|error| ProtocolError::Malformed {
        line: trimmed.to_owned(),
        message: error.to_string(),
    })
}

/// One compact line for `message`, ending with a newline.
pub fn encode_line<T: Serialize>(message: &T) -> String {
    // Serializing these plain types cannot fail.
    let mut line = serde_json::to_string(message).unwrap_or_default();
    line.push('\n');
    line
}

/// Writes `message` as one line.
pub fn write_message<W: Write, T: Serialize>(out: &mut W, message: &T) -> io::Result<()> {
    out.write_all(encode_line(message).as_bytes())
}

/// Reads the next message from `input`, skipping blank lines. `Ok(None)` is the end of the input.
pub fn read_message<R: BufRead, T: DeserializeOwned>(
    input: &mut R,
) -> Result<Option<T>, ProtocolError> {
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        if let Some(message) = parse_line(&line)? {
            return Ok(Some(message));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip<T: Serialize + DeserializeOwned + PartialEq + fmt::Debug>(message: T) {
        let line = encode_line(&message);
        assert!(line.ends_with('\n') && !line[..line.len() - 1].contains('\n'), "{line:?}");
        assert_eq!(parse_line::<T>(&line).unwrap(), Some(message));
    }

    fn item() -> Item {
        Item {
            id: "r1".into(),
            title: "daimos".into(),
            subtitle: Some("gravionlabs".into()),
            icon: Some("folder".into()),
            keywords: vec!["repo".into()],
            actions: vec![Action {
                id: "open".into(),
                title: "Open in browser".into(),
                kind: Kind::Open,
                payload: Some("https://example.com".into()),
                shortcut: None,
            }],
            detail: Some(DetailSpec { markdown: Some("# hi".into()), metadata: vec![] }),
        }
    }

    #[test]
    fn every_request_round_trips() {
        round_trip(Request::Query {
            id: 1,
            command: "search".into(),
            text: "lau".into(),
            preferences: BTreeMap::from([("token".to_owned(), "x".to_owned())]),
        });
        round_trip(Request::Action {
            id: 2,
            command: "search".into(),
            item: "r1".into(),
            action: "star".into(),
        });
        round_trip(Request::Cancel { id: 3, cancels: 1 });
    }

    #[test]
    fn every_response_round_trips() {
        round_trip(Response::Items { id: 1, items: vec![item()], done: false });
        round_trip(Response::Items { id: 1, items: vec![], done: true });
        round_trip(Response::Result { id: 2, toast: Some("Starred".into()), view: None });
        round_trip(Response::Result {
            id: 2,
            toast: None,
            view: Some(serde_json::json!({"kind": "list"})),
        });
    }

    #[test]
    fn the_requests_have_the_wire_format_of_the_spec() {
        let query = Request::Query {
            id: 1,
            command: "search".into(),
            text: "lau".into(),
            preferences: BTreeMap::new(),
        };
        assert_eq!(
            encode_line(&query),
            "{\"type\":\"query\",\"id\":1,\"command\":\"search\",\"text\":\"lau\",\"preferences\":{}}\n"
        );
        assert_eq!(
            encode_line(&Request::Cancel { id: 3, cancels: 1 }),
            "{\"type\":\"cancel\",\"id\":3,\"cancels\":1}\n"
        );
    }

    #[test]
    fn the_spec_examples_parse() {
        let items = r#"{"id": 1, "type": "items", "items": [{"id": "r1", "title": "daimos", "subtitle": "gravionlabs", "actions": [{"id": "open", "title": "Open in browser", "kind": "open", "url": "https://x.y"}]}], "done": false}"#;
        let Some(Response::Items { id, items, done }) = parse_line(items).unwrap() else {
            panic!("not items");
        };
        assert_eq!((id, done), (1, false));
        assert_eq!(items[0].actions[0].payload.as_deref(), Some("https://x.y"));
        let result = r#"{"id": 2, "type": "result", "toast": "Starred", "view": null}"#;
        assert_eq!(
            parse_line::<Response>(result).unwrap(),
            Some(Response::Result { id: 2, toast: Some("Starred".into()), view: None })
        );
    }

    #[test]
    fn missing_optional_fields_default_and_unknown_fields_are_ignored() {
        let line =
            r#"{"type":"items","id":4,"future":true,"items":[{"id":"a","title":"A","extra":1}]}"#;
        let Some(Response::Items { items, done, .. }) = parse_line(line).unwrap() else {
            panic!("not items");
        };
        assert!(!done);
        assert_eq!(items[0].actions, []);
        assert_eq!(parse_line::<Response>(r#"{"type":"items","id":4}"#).unwrap().unwrap().id(), 4);
    }

    #[test]
    fn blank_lines_are_skipped() {
        assert_eq!(parse_line::<Response>("").unwrap(), None);
        assert_eq!(parse_line::<Response>("  \r\n").unwrap(), None);
        let text = "\n\n{\"type\":\"result\",\"id\":1}\n\n";
        let mut input = text.as_bytes();
        let first: Option<Response> = read_message(&mut input).unwrap();
        assert_eq!(first.unwrap().id(), 1);
        assert!(read_message::<_, Response>(&mut input).unwrap().is_none());
    }

    #[test]
    fn a_malformed_line_is_reported_with_its_content() {
        for line in [
            "not json",
            r#"{"type":"items"}"#,
            r#"{"type":"nope","id":1}"#,
            r#"{"type":"items","id":"x"}"#,
            r#"{"type":"items","id":1,"items":[{"id":"a"}]}"#,
            r#"{"type":"items","id":1,"items":[{"id":"a","title":"A","actions":[{"id":"x","title":"X","kind":"fly"}]}]}"#,
        ] {
            let Err(ProtocolError::Malformed { line: got, message }) = parse_line::<Response>(line)
            else {
                panic!("accepted {line}");
            };
            assert_eq!(got, line);
            assert!(!message.is_empty());
            assert!(ProtocolError::Malformed { line: got, message }.to_string().contains(line));
        }
    }

    #[test]
    fn a_long_malformed_line_is_shortened_in_the_message() {
        let line = "x".repeat(1000);
        let error = parse_line::<Response>(&line).unwrap_err();
        assert!(error.to_string().len() < 400, "{error}");
    }

    #[test]
    fn a_writer_and_reader_agree_on_several_messages() {
        let mut buffer = Vec::new();
        write_message(&mut buffer, &Request::Cancel { id: 1, cancels: 0 }).unwrap();
        write_message(&mut buffer, &Request::Cancel { id: 2, cancels: 1 }).unwrap();
        let mut input = buffer.as_slice();
        let a: Request = read_message(&mut input).unwrap().unwrap();
        let b: Request = read_message(&mut input).unwrap().unwrap();
        assert_eq!(
            (a, b),
            (Request::Cancel { id: 1, cancels: 0 }, Request::Cancel { id: 2, cancels: 1 })
        );
        assert!(read_message::<_, Request>(&mut input).unwrap().is_none());
    }

    #[test]
    fn items_convert_to_core_items_with_every_action_kind() {
        let mut source = item();
        source.actions = [Kind::Open, Kind::Copy, Kind::Paste, Kind::Run, Kind::Custom]
            .into_iter()
            .map(|kind| Action {
                id: format!("{kind:?}"),
                title: "t".into(),
                kind,
                payload: None,
                shortcut: Some("Ctrl+K".into()),
            })
            .collect();
        let core: model::Item = source.into();
        assert_eq!((core.id.as_str(), core.title.as_str()), ("r1", "daimos"));
        assert_eq!(core.subtitle.as_deref(), Some("gravionlabs"));
        assert_eq!(core.icon, Some(Icon::Named { name: "folder".into() }));
        assert_eq!(core.keywords, ["repo"]);
        let kinds: Vec<_> = core.actions.iter().map(|a| a.kind).collect();
        assert_eq!(
            kinds,
            [
                ActionKind::Open,
                ActionKind::Copy,
                ActionKind::Paste,
                ActionKind::Run,
                ActionKind::Custom
            ]
        );
        assert_eq!(core.actions[0].shortcut.as_deref(), Some("Ctrl+K"));
        assert_eq!(core.detail, Some(Detail::Markdown { text: "# hi".into() }));
    }

    #[test]
    fn a_custom_action_keeps_its_id_and_payload_opaque() {
        let line = r#"{"id":"star","title":"Star","kind":"custom","payload":"{\"any\":1}"}"#;
        let action: Action = parse_line(line).unwrap().unwrap();
        let core: model::Action = action.into();
        assert_eq!((core.id.as_str(), core.kind), ("star", ActionKind::Custom));
        assert_eq!(core.payload.as_deref(), Some("{\"any\":1}"));
    }

    #[test]
    fn icons_are_named_files_or_data_urls() {
        assert_eq!(icon_of("firefox"), Icon::Named { name: "firefox".into() });
        assert_eq!(icon_of("/a/b.png"), Icon::File { path: "/a/b.png".into() });
        assert_eq!(icon_of(r"C:\a\b.png"), Icon::File { path: r"C:\a\b.png".into() });
        assert_eq!(
            icon_of("data:image/png;base64,AA"),
            Icon::DataUrl { url: "data:image/png;base64,AA".into() }
        );
    }

    #[test]
    fn details_become_markdown_metadata_or_nothing() {
        let mut source = item();
        source.detail =
            Some(DetailSpec { markdown: None, metadata: vec![("k".into(), "v".into())] });
        let core: model::Item = source.clone().into();
        assert_eq!(core.detail, Some(Detail::Metadata { entries: vec![("k".into(), "v".into())] }));
        source.detail = Some(DetailSpec::default());
        assert_eq!(model::Item::from(source).detail, None);
    }
}
