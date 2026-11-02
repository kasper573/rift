use std::sync::OnceLock;

use jsonc_parser::ast::Value;
use jsonc_parser::common::Ranged;
use jsonc_parser::{CollectOptions, ParseOptions, parse_to_ast};

/// Where each value and key of a JSON file sits, by JSON pointer. A file is only indexed once a
/// place in it is asked for, which only a problem in it does.
pub struct JsonIndex {
    text: String,
    values: OnceLock<Vec<IndexedValue>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedValue {
    pub pointer: String,
    pub line: usize,
    pub column: usize,
    pub string: Option<String>,
    pub key: bool,
}

impl JsonIndex {
    pub fn of(text: &str) -> JsonIndex {
        JsonIndex {
            text: text.to_owned(),
            values: OnceLock::new(),
        }
    }

    /// `text` indexed at once, for a caller that asks for places in all of it anyway.
    pub fn indexed(text: &str) -> JsonIndex {
        let index = JsonIndex::of(text);
        index.values();
        index
    }

    pub fn position(&self, pointer: &str) -> Option<(usize, usize)> {
        let mut wanted = pointer.to_owned();
        loop {
            if let Some(value) = self
                .values()
                .iter()
                .find(|value| !value.key && value.pointer == wanted)
            {
                return Some((value.line, value.column));
            }
            let cut = wanted.rfind('/')?;
            wanted.truncate(cut);
        }
    }

    pub fn strings(&self) -> impl Iterator<Item = &IndexedValue> {
        self.values()
            .iter()
            .filter(|value| !value.key && value.string.is_some())
    }

    // Text that isn't JSON indexes nothing: its parse error carries a place of its own.
    fn values(&self) -> &[IndexedValue] {
        self.values.get_or_init(|| {
            let mut values = Vec::new();
            let parsed = parse_to_ast(&self.text, &CollectOptions::default(), &strict_json());
            if let Some(value) = parsed.ok().and_then(|parsed| parsed.value) {
                visit(&mut values, &value, String::new(), &Lines::of(&self.text));
            }
            values
        })
    }
}

fn visit(values: &mut Vec<IndexedValue>, value: &Value, pointer: String, lines: &Lines) {
    let (line, column) = lines.place(value.start());
    let string = match value {
        Value::StringLit(literal) => Some(literal.value.to_string()),
        _ => None,
    };
    values.push(IndexedValue {
        pointer: pointer.clone(),
        line,
        column,
        string,
        key: false,
    });
    match value {
        Value::Object(object) => {
            for property in &object.properties {
                let name = property.name.as_str();
                let inner = format!("{pointer}/{}", escape(name));
                let (line, column) = lines.place(property.name.start());
                values.push(IndexedValue {
                    pointer: inner.clone(),
                    line,
                    column,
                    string: Some(name.to_owned()),
                    key: true,
                });
                visit(values, &property.value, inner, lines);
            }
        }
        Value::Array(array) => {
            for (index, element) in array.elements.iter().enumerate() {
                visit(values, element, format!("{pointer}/{index}"), lines);
            }
        }
        _ => {}
    }
}

pub fn escape(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

pub fn unescape(segment: &str) -> String {
    segment.replace("~1", "/").replace("~0", "~")
}

fn strict_json() -> ParseOptions {
    ParseOptions {
        allow_comments: false,
        allow_loose_object_property_names: false,
        allow_trailing_commas: false,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
        allow_bare_decimal_point_numbers: false,
        allow_non_finite_numbers: false,
        allow_extended_string_escapes: false,
    }
}

struct Lines<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    fn of(text: &'a str) -> Lines<'a> {
        let starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(at, _)| at + 1))
            .collect();
        Lines { text, starts }
    }

    fn place(&self, offset: usize) -> (usize, usize) {
        let line = self.starts.partition_point(|&start| start <= offset);
        let column = self.text[self.starts[line - 1]..offset].chars().count() + 1;
        (line, column)
    }
}
