//! Loading is assertive — a malformed row or dangling reference panics with the offending file,
//! so broken content can never load.

use serde::de::DeserializeOwned;

use crate::assets;

pub fn load<T: DeserializeOwned>(file: &str) -> Vec<T> {
    let json = assets::text(file).unwrap_or_else(|| panic!("missing asset {file}"));
    serde_json::from_str(&json).unwrap_or_else(|error| panic!("{file}: {error}"))
}

pub fn unique_ids<'a>(ids: impl Iterator<Item = &'a str>, file: &str) {
    let mut seen: Vec<&str> = Vec::new();
    for id in ids {
        if seen.contains(&id) {
            panic!("{file}: duplicate id '{id}'");
        }
        seen.push(id);
    }
}
