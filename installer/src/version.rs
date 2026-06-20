use std::path::Path;

use semver::Version;

pub const VERSION_FILE: &str = ".rift-version";

pub fn parse(tag: &str) -> Result<Version, semver::Error> {
    Version::parse(&normalize(tag))
}

pub fn installed(dir: &Path) -> Version {
    std::fs::read_to_string(dir.join(VERSION_FILE))
        .ok()
        .and_then(|raw| parse(&raw).ok())
        .unwrap_or_else(|| Version::new(0, 0, 0))
}

pub fn record(dir: &Path, tag: &str) -> std::io::Result<()> {
    std::fs::write(dir.join(VERSION_FILE), tag.trim())
}

fn normalize(tag: &str) -> String {
    let core = tag.trim().trim_start_matches('v');
    match core.split('.').count() {
        1 => format!("{core}.0.0"),
        2 => format!("{core}.0"),
        _ => core.to_owned(),
    }
}
