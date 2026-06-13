use std::fs::File;
use std::io::Read;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    Zip,
    TarGz,
    Tar,
    Raw,
}

pub fn kind(name: &str) -> ArchiveKind {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        ArchiveKind::TarGz
    } else if lower.ends_with(".zip") {
        ArchiveKind::Zip
    } else if lower.ends_with(".tar") {
        ArchiveKind::Tar
    } else {
        ArchiveKind::Raw
    }
}

/// Unpacking is rooted at `dir`, so an archive entry can only ever land inside it.
pub fn extract_or_write(name: &str, src: &Path, dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    match kind(name) {
        ArchiveKind::Zip => extract_zip(src, dir),
        ArchiveKind::TarGz => unpack_tar(flate2::read::GzDecoder::new(File::open(src)?), dir),
        ArchiveKind::Tar => unpack_tar(File::open(src)?, dir),
        ArchiveKind::Raw => std::fs::copy(src, dir.join(name)).map(drop),
    }
}

fn unpack_tar(reader: impl Read, dir: &Path) -> std::io::Result<()> {
    tar::Archive::new(reader).unpack(dir)
}

fn extract_zip(src: &Path, dir: &Path) -> std::io::Result<()> {
    let invalid =
        |error: zip::result::ZipError| std::io::Error::new(std::io::ErrorKind::InvalidData, error);
    zip::ZipArchive::new(File::open(src)?)
        .map_err(invalid)?
        .extract(dir)
        .map_err(invalid)
}
