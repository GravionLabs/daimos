//! The data model shared by built-in providers and extensions (`docs/specs/architecture.md`).

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Icon {
    /// A themed icon name, for example `firefox`.
    Named {
        name: String,
    },
    File {
        path: String,
    },
    DataUrl {
        url: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ActionKind {
    Open,
    Copy,
    Paste,
    Run,
    Push,
    Form,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    pub id: String,
    pub title: String,
    pub shortcut: Option<String>,
    pub kind: ActionKind,
    /// What the action acts on: a path, text to copy, a command line, or a view to push.
    pub payload: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Detail {
    Markdown { text: String },
    Metadata { entries: Vec<(String, String)> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub icon: Option<Icon>,
    pub keywords: Vec<String>,
    pub accessories: Vec<String>,
    pub actions: Vec<Action>,
    pub detail: Option<Detail>,
    /// Positions (in characters) of the title that matched the query, for highlighting.
    #[serde(default)]
    pub highlight: Vec<u32>,
}

impl Item {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            subtitle: None,
            icon: None,
            keywords: Vec::new(),
            accessories: Vec::new(),
            actions: Vec::new(),
            detail: None,
            highlight: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FormField {
    pub id: String,
    pub label: String,
    pub required: bool,
}

/// What a provider answers with. Views are data; the webview renders them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum View {
    List { items: Vec<Item> },
    Detail { detail: Detail },
    Form { fields: Vec<FormField> },
    Push { view_id: String },
}

/// One batch of items from one provider.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ItemBatch {
    pub items: Vec<Item>,
}

impl From<Vec<Item>> for ItemBatch {
    fn from(items: Vec<Item>) -> Self {
        Self { items }
    }
}
