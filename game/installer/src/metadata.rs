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

/// Builds a [`FileEntry`] per URL, naming each by the URL's last path segment. The pipeline decides
/// where releases are hosted; the manifest only needs the filename the installer writes locally.
pub fn files_from_urls(urls: &[String]) -> Vec<FileEntry> {
    urls.iter()
        .map(|url| FileEntry {
            name: filename_of(url),
            url: url.clone(),
        })
        .collect()
}

/// A platform with no `{os}-{arch}` file is unsupported and yields nothing — the shared assets alone are
/// useless without a binary, so the caller can treat an empty result as "no release for this platform".
pub fn select_files(all: &[FileEntry], os: &str, arch: &str) -> Vec<FileEntry> {
    let marker = format!("{os}-{arch}");
    let platform: Vec<FileEntry> = all
        .iter()
        .filter(|file| file.name.contains(&marker))
        .cloned()
        .collect();
    if platform.is_empty() {
        return Vec::new();
    }
    let shared = all
        .iter()
        .filter(|file| SHARED_FILES.contains(&file.name.as_str()))
        .cloned();
    platform.into_iter().chain(shared).collect()
}

const SHARED_FILES: &[&str] = &["rift-assets.zip"];

fn filename_of(url: &str) -> String {
    url.rsplit('/')
        .next()
        .unwrap_or(url)
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_owned()
}
