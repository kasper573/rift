use installer::archive::{ArchiveKind, kind};
use installer::metadata::{FileEntry, Metadata, select_files};
use installer::version;

fn entry(name: &str) -> FileEntry {
    FileEntry {
        name: name.to_owned(),
        url: format!("https://example.test/{name}"),
    }
}

#[test]
fn two_component_release_tags_normalize_and_compare() {
    assert!(version::parse("0.111").unwrap() > version::parse("0.110").unwrap());
    assert_eq!(
        version::parse("0.110").unwrap(),
        version::parse("v0.110.0").unwrap()
    );
}

#[test]
fn a_fresh_install_reads_as_the_lowest_version_then_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        version::installed(dir.path()),
        version::parse("0.0.0").unwrap()
    );
    version::record(dir.path(), "0.110").unwrap();
    assert_eq!(
        version::installed(dir.path()),
        version::parse("0.110").unwrap()
    );
}

#[test]
fn files_are_classified_by_extension() {
    assert_eq!(kind("rift-assets.zip"), ArchiveKind::Zip);
    assert_eq!(kind("rift-linux-x86_64.tar.gz"), ArchiveKind::TarGz);
    assert_eq!(kind("bundle.tgz"), ArchiveKind::TarGz);
    assert_eq!(kind("bundle.tar"), ArchiveKind::Tar);
    assert_eq!(kind("rift"), ArchiveKind::Raw);
    assert_eq!(kind("rift.exe"), ArchiveKind::Raw);
}

#[test]
fn select_keeps_this_platform_and_shared_assets_only() {
    let all = [
        entry("rift-installer-linux-x86_64.tar.gz"),
        entry("rift-linux-x86_64.tar.gz"),
        entry("rift-installer-macos-aarch64.tar.gz"),
        entry("rift-macos-aarch64.tar.gz"),
        entry("rift-installer-windows-x86_64.zip"),
        entry("rift-windows-x86_64.zip"),
        entry("rift-assets.zip"),
    ];

    let linux: Vec<_> = select_files(&all, "linux", "x86_64")
        .into_iter()
        .map(|file| file.name)
        .collect();
    assert_eq!(
        linux,
        [
            "rift-installer-linux-x86_64.tar.gz",
            "rift-linux-x86_64.tar.gz",
            "rift-assets.zip",
        ]
    );

    // Same arch, different os: windows must not pick up the linux archives.
    let windows: Vec<_> = select_files(&all, "windows", "x86_64")
        .into_iter()
        .map(|file| file.name)
        .collect();
    assert_eq!(
        windows,
        [
            "rift-installer-windows-x86_64.zip",
            "rift-windows-x86_64.zip",
            "rift-assets.zip",
        ]
    );
}

#[test]
fn manifest_round_trips_as_json_and_carries_no_size() {
    let manifest = Metadata {
        version: "0.110".to_owned(),
        files: vec![entry("rift-assets.zip")],
    };
    let json = serde_json::to_string(&manifest).unwrap();
    assert!(!json.contains("size"));
    assert_eq!(serde_json::from_str::<Metadata>(&json).unwrap(), manifest);
}
