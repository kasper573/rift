use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fltk::prelude::*;
use fltk::{app, enums, frame::Frame, window::Window};
use installer::download::{self, FileProgress, Progress};
use installer::metadata::{FileEntry, Metadata};
use installer::version;
use serde::Deserialize;

fn main() {
    let bundle = Bundle::beside_exe();
    let progress = Arc::new(Mutex::new(Progress::default()));
    let status = Arc::new(Mutex::new(Status::default()));

    {
        let progress = Arc::clone(&progress);
        let status = Arc::clone(&status);
        std::thread::spawn(move || bundle.run(&progress, &status));
    }

    let app = app::App::default();
    let mut window = Window::new(100, 100, 480, 220, "Rift");
    let mut view = Frame::new(15, 15, 450, 190, "");
    view.set_align(
        enums::Align::Left | enums::Align::Top | enums::Align::Inside | enums::Align::Wrap,
    );
    window.end();
    window.show();

    app::add_timeout3(0.1, move |handle| {
        view.set_label(&render(&status, &progress));
        view.redraw();
        // A failure leaves `done` false, so the window stays up for the user to read the error.
        let finished = {
            let status = status.lock().expect("status lock");
            status.error.is_none() && status.done
        };
        if finished {
            app::quit();
        } else {
            app::repeat_timeout3(0.1, handle);
        }
    });
    app.run().expect("fltk event loop");
}

struct Bundle {
    metadata_url: String,
    dir: PathBuf,
    client: PathBuf,
}

#[derive(Deserialize)]
struct Env {
    rift_installer_metadata_url: String,
    rift_client_executable: String,
}

impl Bundle {
    fn beside_exe() -> Self {
        let exe = std::env::current_exe().expect("locate the running executable");
        let dir = exe
            .parent()
            .expect("the executable has a parent directory")
            .to_path_buf();
        let _ = dotenvy::from_path(dir.join(".env"));
        let env: Env = envy::from_env()
            .expect("RIFT_INSTALLER_METADATA_URL and RIFT_CLIENT_EXECUTABLE in the .env beside the installer");
        Self {
            metadata_url: env.rift_installer_metadata_url,
            client: dir.join(env.rift_client_executable),
            dir,
        }
    }

    /// Runs on the worker thread and never touches fltk — the UI thread owns that.
    fn run(&self, progress: &Arc<Mutex<Progress>>, status: &Arc<Mutex<Status>>) {
        match self.patch_and_launch(progress, status) {
            Ok(()) => status.lock().expect("status lock").done = true,
            Err(error) => status.lock().expect("status lock").error = Some(error),
        }
    }

    fn patch_and_launch(
        &self,
        progress: &Arc<Mutex<Progress>>,
        status: &Arc<Mutex<Status>>,
    ) -> Result<(), String> {
        let manifest = self.fetch_manifest()?;
        let remote = version::parse(&manifest.version).map_err(|error| {
            format!("unreadable release version {:?}: {error}", manifest.version)
        })?;
        if remote > version::installed(&self.dir) {
            set_phase(status, Phase::Downloading);
            let downloads = download::run(&manifest.files, &self.dir, progress)
                .map_err(|error| error.to_string())?;
            set_phase(status, Phase::Installing);
            self.install(&manifest.files, &downloads)
                .map_err(|error| format!("installing the update failed: {error}"))?;
            version::record(&self.dir, &manifest.version)
                .map_err(|error| format!("recording the version failed: {error}"))?;
        }
        set_phase(status, Phase::Launching);
        std::process::Command::new(&self.client)
            .spawn()
            .map(drop)
            .map_err(|error| format!("could not start {}: {error}", self.client.display()))
    }

    fn fetch_manifest(&self) -> Result<Metadata, String> {
        let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
        installer::http_agent(Duration::from_secs(30))
            .get(&self.metadata_url)
            .query("platform", &platform)
            .call()
            .map_err(|error| format!("update check failed: {error}"))?
            .body_mut()
            .read_json::<Metadata>()
            .map_err(|error| format!("update manifest invalid: {error}"))
    }

    /// The installer's own new binary (the entry matching the running executable) is swapped in via
    /// `self_replace`, which handles platforms that refuse to overwrite a running executable in place.
    fn install(&self, files: &[FileEntry], downloads: &Path) -> std::io::Result<()> {
        let exe = std::env::current_exe()?;
        let stem = exe
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default();
        for file in files {
            let src = downloads.join(&file.name);
            if file.name.starts_with(stem) {
                self.install_self(&file.name, &src, &exe)?;
            } else {
                installer::archive::extract_or_write(&file.name, &src, &self.dir)?;
            }
        }
        let _ = std::fs::remove_dir_all(downloads);
        Ok(())
    }

    fn install_self(&self, name: &str, src: &Path, exe: &Path) -> std::io::Result<()> {
        let staging = self.dir.join(".rift-self-update");
        let _ = std::fs::remove_dir_all(&staging);
        installer::archive::extract_or_write(name, src, &staging)?;
        let exe_name = exe.file_name();
        for entry in std::fs::read_dir(&staging)? {
            let entry = entry?;
            if Some(entry.file_name().as_os_str()) == exe_name {
                self_replace::self_replace(entry.path())?;
            } else {
                let target = self.dir.join(entry.file_name());
                let _ = std::fs::remove_file(&target);
                std::fs::rename(entry.path(), target)?;
            }
        }
        let _ = std::fs::remove_dir_all(&staging);
        Ok(())
    }
}

#[derive(Default)]
struct Status {
    phase: Phase,
    error: Option<String>,
    done: bool,
}

#[derive(Default, Clone, Copy)]
enum Phase {
    #[default]
    Checking,
    Downloading,
    Installing,
    Launching,
}

fn set_phase(status: &Arc<Mutex<Status>>, phase: Phase) {
    status.lock().expect("status lock").phase = phase;
}

fn render(status: &Arc<Mutex<Status>>, progress: &Arc<Mutex<Progress>>) -> String {
    let status = status.lock().expect("status lock");
    if let Some(error) = &status.error {
        return format!("Update failed:\n\n{error}\n\nClose this window to exit.");
    }
    match status.phase {
        Phase::Checking => "Checking for updates…".to_owned(),
        Phase::Installing => "Installing…".to_owned(),
        Phase::Launching => "Starting Rift…".to_owned(),
        Phase::Downloading => {
            let progress = progress.lock().expect("progress lock");
            let header = match progress.fraction() {
                Some(fraction) => format!("Downloading ({:.1}%):", fraction * 100.0),
                None => "Downloading…".to_owned(),
            };
            let rows: Vec<String> = progress.files.iter().map(row).collect();
            format!("{header}\n{}", rows.join("\n"))
        }
    }
}

fn row(file: &FileProgress) -> String {
    match file.total {
        Some(total) => format!(
            "- {} {}/{} ({:.0}%)",
            file.name,
            file.downloaded,
            total,
            100.0 * file.downloaded.as_u64() as f64 / total.as_u64().max(1) as f64,
        ),
        None => format!("- {} {}", file.name, file.downloaded),
    }
}
