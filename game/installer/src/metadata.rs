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
