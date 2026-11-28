use std::io::{Cursor, Read, Write};

use zip::write::SimpleFileOptions;

#[derive(Clone, Copy)]
pub(crate) enum ArchiveKind {
    TarGz,
    Zip,
}

/// The release's `archive` of the bare game client remade for a player: every file it holds, and
/// `rift_env` beside them, in one `folder`, so extracting it is all it takes to play.
pub(crate) fn bundle(
    archive: &[u8],
    kind: ArchiveKind,
    folder: &str,
    rift_env: &str,
) -> Result<Vec<u8>, String> {
    let failed = |error: &dyn std::fmt::Display| format!("the release's archive: {error}");
    match kind {
        ArchiveKind::TarGz => {
            let mut release = tar::Archive::new(flate2::read::GzDecoder::new(archive));
            let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            let mut bundle = tar::Builder::new(gz);
            let mut written_at = 0;
            for entry in release.entries().map_err(|error| failed(&error))? {
                let mut entry = entry.map_err(|error| failed(&error))?;
                let path = entry.path().map_err(|error| failed(&error))?.into_owned();
                let mode = entry.header().mode().map_err(|error| failed(&error))?;
                let mtime = entry.header().mtime().unwrap_or(0);
                written_at = written_at.max(mtime);
                let mut bytes = Vec::new();
                entry
                    .read_to_end(&mut bytes)
                    .map_err(|error| failed(&error))?;
                // A header of its own rather than the release's: reading a sparse entry fills in
                // its holes, so the release's header would describe less than is written.
                let mut header = tar::Header::new_gnu();
                header.set_size(bytes.len() as u64);
                header.set_mode(mode & 0o7777);
                header.set_mtime(mtime);
                bundle
                    .append_data(&mut header, format!("{folder}/{}", path.display()), &*bytes)
                    .map_err(|error| failed(&error))?;
            }
            let mut header = tar::Header::new_gnu();
            header.set_size(rift_env.len() as u64);
            header.set_mode(0o644);
            header.set_mtime(written_at);
            bundle
                .append_data(
                    &mut header,
                    format!("{folder}/{RIFT_ENV}"),
                    rift_env.as_bytes(),
                )
                .map_err(|error| failed(&error))?;
            let gz = bundle.into_inner().map_err(|error| failed(&error))?;
            gz.finish().map_err(|error| failed(&error))
        }
        ArchiveKind::Zip => {
            let mut release =
                zip::ZipArchive::new(Cursor::new(archive)).map_err(|error| failed(&error))?;
            let mut bundle = zip::ZipWriter::new(Cursor::new(Vec::new()));
            for index in 0..release.len() {
                let file = release
                    .by_index_raw(index)
                    .map_err(|error| failed(&error))?;
                let name = format!("{folder}/{}", file.name().map_err(|error| failed(&error))?);
                bundle
                    .raw_copy_file_rename(file, name)
                    .map_err(|error| failed(&error))?;
            }
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            bundle
                .start_file(format!("{folder}/{RIFT_ENV}"), options)
                .and_then(|()| Ok(bundle.write_all(rift_env.as_bytes())?))
                .map_err(|error| failed(&error))?;
            let written = bundle.finish().map_err(|error| failed(&error))?;
            Ok(written.into_inner())
        }
    }
}

/// The settings file the game client reads from its own folder.
const RIFT_ENV: &str = "rift.env";
