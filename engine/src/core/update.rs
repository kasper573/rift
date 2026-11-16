use std::io::Read;
use std::path::{Path, PathBuf};

use minisign_verify::{PublicKey, Signature};

use crate::core::os::{FetchFailure, Http};

/// The name the game client's binary has in every release, before the platform's suffix.
pub const CLIENT_BINARY: &str = "game-client";

/// Where a client finds a release, and the key its archives are signed with.
#[derive(Clone, Debug, PartialEq)]
pub struct ReleaseSource {
    /// `<releases_url>/<build>/game-client-<platform>.tar.gz` (`.zip` on Windows) and its `.minisig`.
    pub releases_url: String,
    /// The public half of the release key, in minisign's base64.
    pub key: String,
}

/// What a client does when its server runs another build.
#[derive(Clone, Debug, PartialEq)]
pub enum OtherBuildAnswer {
    /// Installs the server's build from its release, and starts again on it.
    Update(ReleaseSource),
    /// It updated itself already and is still on another build, so it stops rather than loop.
    Updated { from: String },
    /// It can't update itself: the player starts a client of that build.
    ByHand,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateStep {
    Downloading { got: u64, of: Option<u64> },
    Verifying,
    Replacing,
    Restarting,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateFailure {
    /// The archive isn't signed by the release key, so nothing was installed.
    Unsigned,
    /// The server's release is no longer published.
    Gone,
    /// The client's own folder can't be written.
    Unwritable { folder: PathBuf, reason: String },
    /// The archive doesn't hold the client.
    Malformed(String),
    /// The download stopped; trying again may finish it.
    Download(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveKind {
    TarGz,
    Zip,
}

/// The release key this build trusts, built in from `RIFT_RELEASE_KEY`. A build without one never
/// updates itself.
pub fn release_key() -> Option<&'static str> {
    option_env!("RIFT_RELEASE_KEY").filter(|key| !key.is_empty())
}

pub fn answer(
    releases_url: Option<&str>,
    key: Option<&str>,
    updated_from: Option<&str>,
) -> OtherBuildAnswer {
    if let Some(from) = updated_from {
        return OtherBuildAnswer::Updated {
            from: from.to_owned(),
        };
    }
    match (releases_url, key) {
        (Some(releases_url), Some(key)) => OtherBuildAnswer::Update(ReleaseSource {
            releases_url: releases_url.trim_end_matches('/').to_owned(),
            key: key.to_owned(),
        }),
        _ => OtherBuildAnswer::ByHand,
    }
}

impl ReleaseSource {
    pub fn archive_url(&self, build: &str) -> String {
        format!("{}/{build}/{}", self.releases_url, archive_name())
    }

    /// Fetches the client of `build`, checks it is signed by the release key and takes the client
    /// out of it, telling each step and how much has come as it goes.
    pub fn fetch(
        &self,
        http: &Http,
        build: &str,
        step: &dyn Fn(UpdateStep),
    ) -> Result<Vec<u8>, UpdateFailure> {
        let url = self.archive_url(build);
        let signature = http
            .download(&format!("{url}.minisig"), &|_, _| {})
            .map_err(fetch_failure)?;
        let archive = http
            .download(&url, &|got, of| step(UpdateStep::Downloading { got, of }))
            .map_err(fetch_failure)?;
        step(UpdateStep::Verifying);
        let signature = String::from_utf8(signature).map_err(|_| UpdateFailure::Unsigned)?;
        verify(&archive, &signature, &self.key)?;
        unpack(&archive, archive_kind(), &executable_name())
    }
}

/// Whether `archive` is what `signature` signs, under `key`.
pub fn verify(archive: &[u8], signature: &str, key: &str) -> Result<(), UpdateFailure> {
    let key = PublicKey::from_base64(key.trim()).map_err(|_| UpdateFailure::Unsigned)?;
    let signature = Signature::decode(signature).map_err(|_| UpdateFailure::Unsigned)?;
    key.verify(archive, &signature, false)
        .map_err(|_| UpdateFailure::Unsigned)
}

/// The one file named `binary` at the top of `archive`; whatever else it holds is left in it.
pub fn unpack(archive: &[u8], kind: ArchiveKind, binary: &str) -> Result<Vec<u8>, UpdateFailure> {
    let malformed = |reason: String| UpdateFailure::Malformed(reason);
    let mut found = None;
    match kind {
        ArchiveKind::TarGz => {
            let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
            for entry in tar
                .entries()
                .map_err(|error| malformed(error.to_string()))?
            {
                let mut entry = entry.map_err(|error| malformed(error.to_string()))?;
                let named = entry.path().is_ok_and(|path| path == Path::new(binary));
                if named && entry.header().entry_type().is_file() {
                    let mut bytes = Vec::new();
                    entry
                        .read_to_end(&mut bytes)
                        .map_err(|error| malformed(error.to_string()))?;
                    found = Some(bytes);
                }
            }
        }
        ArchiveKind::Zip => {
            let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive))
                .map_err(|error| malformed(error.to_string()))?;
            if let Ok(mut file) = zip.by_name(binary)
                && file.is_file()
            {
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes)
                    .map_err(|error| malformed(error.to_string()))?;
                found = Some(bytes);
            }
        }
    }
    found.ok_or_else(|| malformed(format!("the archive holds no {binary}")))
}

/// Puts `executable` in the place of the running client, so its next start runs it.
pub fn replace_running(executable: &[u8]) -> Result<(), UpdateFailure> {
    let running = std::env::current_exe().map_err(|error| UpdateFailure::Unwritable {
        folder: PathBuf::new(),
        reason: error.to_string(),
    })?;
    let folder = running
        .parent()
        .map_or_else(PathBuf::new, Path::to_path_buf);
    let unwritable = |error: std::io::Error| UpdateFailure::Unwritable {
        folder: folder.clone(),
        reason: error.to_string(),
    };
    let staged = folder.join(format!(".{}.update", executable_name()));
    std::fs::write(&staged, executable).map_err(unwritable)?;
    let replaced = std::fs::metadata(&running)
        .and_then(|running| std::fs::set_permissions(&staged, running.permissions()))
        .and_then(|()| self_replace::self_replace(&staged));
    let _ = std::fs::remove_file(&staged);
    replaced.map_err(unwritable)
}

/// Starts the client again as it was started, telling it which build it updated from.
pub fn restart(updated_from_var: &str, from: &str) -> std::io::Result<()> {
    std::process::Command::new(std::env::current_exe()?)
        .args(std::env::args_os().skip(1))
        .env(updated_from_var, from)
        .spawn()
        .map(|_| ())
}

fn fetch_failure(failure: FetchFailure) -> UpdateFailure {
    match failure {
        FetchFailure::Status { status: 404, .. } => UpdateFailure::Gone,
        other => UpdateFailure::Download(other.to_string()),
    }
}

/// This machine's platform as releases name it, such as `linux-x86_64`.
pub fn platform() -> String {
    format!(
        "{}-{}",
        std::env::consts::OS,
        match std::env::consts::ARCH {
            "aarch64" => "arm64",
            arch => arch,
        }
    )
}

fn archive_name() -> String {
    let platform = platform();
    let extension = match archive_kind() {
        ArchiveKind::TarGz => "tar.gz",
        ArchiveKind::Zip => "zip",
    };
    format!("{CLIENT_BINARY}-{platform}.{extension}")
}

fn archive_kind() -> ArchiveKind {
    if std::env::consts::OS == "windows" {
        ArchiveKind::Zip
    } else {
        ArchiveKind::TarGz
    }
}

fn executable_name() -> String {
    format!("{CLIENT_BINARY}{}", std::env::consts::EXE_SUFFIX)
}
