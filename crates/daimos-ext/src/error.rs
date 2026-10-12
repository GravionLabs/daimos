//! Manifest errors that name the file, the line and the field.

use std::fmt;
use std::ops::Range;
use std::path::{Path, PathBuf};

use toml::Spanned;
use toml::de::{DeTable, DeValue};

/// What is wrong with an `extension.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestError {
    pub file: PathBuf,
    /// 1-based line.
    pub line: usize,
    /// The field, for example `commands[1].mode`; `(document)` when it is the file as a whole.
    pub field: String,
    pub message: String,
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}: {}", self.file.display(), self.line, self.field, self.message)
    }
}

impl std::error::Error for ManifestError {}

/// One step of a path into the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Step {
    Key(String),
    Index(usize),
}

/// Renders `commands[1].mode`.
pub(crate) fn render(path: &[Step]) -> String {
    let mut out = String::new();
    for step in path {
        match step {
            Step::Key(key) if out.is_empty() => out.push_str(key),
            Step::Key(key) => {
                out.push('.');
                out.push_str(key);
            }
            Step::Index(index) => out.push_str(&format!("[{index}]")),
        }
    }
    if out.is_empty() { "(document)".into() } else { out }
}

/// Finds fields and lines in a document.
pub(crate) struct Locator<'a> {
    source: &'a str,
    root: Option<Spanned<DeTable<'a>>>,
}

fn contains(span: &Range<usize>, offset: usize) -> bool {
    span.start <= offset && offset < span.end.max(span.start + 1)
}

impl<'a> Locator<'a> {
    pub(crate) fn new(source: &'a str) -> Self {
        Self { source, root: DeTable::parse(source).ok() }
    }

    pub(crate) fn line_of(&self, offset: usize) -> usize {
        let end = offset.min(self.source.len());
        self.source.as_bytes()[..end].iter().filter(|&&b| b == b'\n').count() + 1
    }

    /// The line of the key at `path`, or of the deepest ancestor that exists.
    pub(crate) fn line(&self, path: &[Step]) -> usize {
        let Some(root) = &self.root else { return 1 };
        let mut table = root.get_ref();
        let mut value: Option<&Spanned<DeValue<'a>>> = None;
        let mut start = 0;

        for step in path {
            let next = match (step, value.map(|v| v.get_ref())) {
                (Step::Key(key), None) => {
                    table.get_key_value(key.as_str()).map(|(k, v)| (k.span().start, v))
                }
                (Step::Key(key), Some(DeValue::Table(t))) => {
                    t.get_key_value(key.as_str()).map(|(k, v)| (k.span().start, v))
                }
                (Step::Index(index), Some(DeValue::Array(a))) => {
                    a.get(*index).map(|v| (v.span().start, v))
                }
                _ => None,
            };
            let Some((offset, found)) = next else { break };
            start = offset;
            // An array element or table starts at its own value for lines inside arrays of tables.
            if matches!(step, Step::Index(_)) {
                start = found.span().start;
            }
            value = Some(found);
            if let DeValue::Table(t) = found.get_ref() {
                table = t;
            }
        }
        self.line_of(start)
    }

    /// The path of the innermost key or value that contains `offset`.
    pub(crate) fn path_at(&self, offset: usize) -> Vec<Step> {
        let mut path = Vec::new();
        let Some(root) = &self.root else { return path };
        descend_table(root.get_ref(), offset, &mut path);
        path
    }
}

// A table's span covers only its header, so the children are searched whatever the span says.
fn descend_table(table: &DeTable<'_>, offset: usize, path: &mut Vec<Step>) -> bool {
    for (key, value) in table {
        path.push(Step::Key(key.get_ref().to_string()));
        if contains(&key.span(), offset)
            || descend_value(value.get_ref(), offset, path)
            || contains(&value.span(), offset)
        {
            return true;
        }
        path.pop();
    }
    false
}

fn descend_value(value: &DeValue<'_>, offset: usize, path: &mut Vec<Step>) -> bool {
    match value {
        DeValue::Table(table) => descend_table(table, offset, path),
        DeValue::Array(array) => {
            for (index, item) in array.iter().enumerate() {
                path.push(Step::Index(index));
                if descend_value(item.get_ref(), offset, path) || contains(&item.span(), offset) {
                    return true;
                }
                path.pop();
            }
            false
        }
        _ => false,
    }
}

/// Turns a serde/TOML error into a [`ManifestError`].
pub(crate) fn from_toml(error: &toml::de::Error, source: &str, file: &Path) -> ManifestError {
    let locator = Locator::new(source);
    let message = error.message().to_string();
    let (line, mut path) = match error.span() {
        Some(span) => (locator.line_of(span.start), locator.path_at(span.start)),
        None => (1, Vec::new()),
    };
    // "missing field `name`" points at the table, the field is the missing key.
    if let Some(rest) = message.strip_prefix("missing field `")
        && let Some(name) = rest.strip_suffix('`')
    {
        path.push(Step::Key(name.to_string()));
    }
    ManifestError { file: file.to_path_buf(), line, field: render(&path), message }
}
