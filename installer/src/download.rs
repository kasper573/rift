use std::fmt;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytesize::ByteSize;

use crate::metadata::FileEntry;

/// Downloads land here, under the install dir, so promoting a finished file into place is a same-
/// filesystem rename rather than a copy.
pub const DOWNLOAD_DIR: &str = ".rift-download";

#[derive(Default)]
pub struct Progress {
    pub files: Vec<FileProgress>,
}

pub struct FileProgress {
    pub name: String,
    pub downloaded: ByteSize,
    pub total: Option<ByteSize>,
    pub done: bool,
}

impl Progress {
    /// `None` until at least one total is known, so the header can read "Downloading…" instead of a false 0%.
    pub fn fraction(&self) -> Option<f64> {
        let total: u64 = self
            .files
            .iter()
            .filter_map(|file| file.total)
            .map(|bytes| bytes.as_u64())
            .sum();
        if total == 0 {
            return None;
        }
        let done: u64 = self
            .files
            .iter()
            .filter(|file| file.total.is_some())
            .map(|file| file.downloaded.as_u64())
            .sum();
        Some(done as f64 / total as f64)
    }
}

#[derive(Debug)]
pub struct DownloadError {
    pub file: String,
    pub message: String,
}

impl fmt::Display for DownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.file, self.message)
    }
}

impl std::error::Error for DownloadError {}

/// Downloads every file concurrently into `dir`'s [`DOWNLOAD_DIR`], advancing the shared [`Progress`].
/// The first failure cancels the rest and is returned; on success the download dir is returned.
pub fn run(
    files: &[FileEntry],
    dir: &Path,
    progress: &Arc<Mutex<Progress>>,
) -> Result<PathBuf, DownloadError> {
    {
        let mut state = progress.lock().expect("progress lock");
        state.files = files
            .iter()
            .map(|file| FileProgress {
                name: file.name.clone(),
                downloaded: ByteSize(0),
                total: None,
                done: false,
            })
            .collect();
    }

    let temp = dir.join(DOWNLOAD_DIR);
    std::fs::create_dir_all(&temp).map_err(|error| DownloadError {
        file: DOWNLOAD_DIR.to_owned(),
        message: error.to_string(),
    })?;

    // A 30-minute ceiling bounds a stalled connection without cutting off a large download on a slow link.
    let agent = crate::http_agent(Duration::from_secs(30 * 60));
    let cancel = Arc::new(AtomicBool::new(false));
    let handles: Vec<_> = files
        .iter()
        .enumerate()
        .map(|(index, file)| {
            let agent = agent.clone();
            let file = file.clone();
            let temp = temp.clone();
            let progress = Arc::clone(progress);
            let cancel = Arc::clone(&cancel);
            std::thread::spawn(move || one(&agent, index, &file, &temp, &progress, &cancel))
        })
        .collect();

    let mut failure = None;
    for handle in handles {
        if let Err(Stop::Failed(error)) = handle.join().expect("download thread") {
            cancel.store(true, Ordering::SeqCst);
            failure.get_or_insert(error);
        }
    }
    match failure {
        Some(error) => Err(error),
        None => Ok(temp),
    }
}

enum Stop {
    Cancelled,
    Failed(DownloadError),
}

fn one(
    agent: &ureq::Agent,
    index: usize,
    file: &FileEntry,
    temp: &Path,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
) -> Result<(), Stop> {
    let fail = |message: String| {
        Stop::Failed(DownloadError {
            file: file.name.clone(),
            message,
        })
    };

    let response = agent
        .get(&file.url)
        .call()
        .map_err(|error| fail(error.to_string()))?;
    let total = response
        .headers()
        .get(ureq::http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(ByteSize);
    progress.lock().expect("progress lock").files[index].total = total;

    let path = temp.join(&file.name);
    let mut sink = File::create(&path).map_err(|error| fail(error.to_string()))?;
    let mut reader = response.into_body().into_reader();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Stop::Cancelled);
        }
        let read = reader
            .read(&mut buffer)
            .map_err(|error| fail(error.to_string()))?;
        if read == 0 {
            break;
        }
        sink.write_all(&buffer[..read])
            .map_err(|error| fail(error.to_string()))?;
        let mut state = progress.lock().expect("progress lock");
        let file = &mut state.files[index];
        file.downloaded = ByteSize(file.downloaded.as_u64() + read as u64);
    }
    progress.lock().expect("progress lock").files[index].done = true;
    Ok(())
}
