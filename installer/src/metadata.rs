use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    pub version: String,
    pub files: Vec<FileEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub url: String,
}

pub fn files_from_urls(urls: &[String]) -> Vec<FileEntry> {
    urls.iter()
        .map(|url| FileEntry {
            name: filename_of(url),
            url: url.clone(),
        })
        .collect()
}

fn filename_of(url: &str) -> String {
    url.rsplit('/')
        .next()
        .unwrap_or(url)
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_owned()
}
