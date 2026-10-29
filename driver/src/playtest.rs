use std::path::{Path, PathBuf};
use std::time::Duration;

use std::sync::Arc;

use engine::core::content::{ContentFile, FilesystemFolder, FolderReader, WrittenFiles};
use engine::systems::launch;
use engine::systems::player::snapshot::CharacterSnapshot;
use engine::systems::script::Place;
use engine::systems::{GameSettings, registry};
use walkdir::WalkDir;

use crate::content;
use crate::desktop::{Desktop, WaitBudget};
use crate::memory::Bytes;
use crate::player::{Pace, Player};
use crate::seen::in_world;
use crate::server::Server;

pub struct PlaytestSetup {
    pub server_binary: PathBuf,
    pub client_binary: PathBuf,
    /// The content the server starts on, and that the driver reads names and places from.
    pub content: PathBuf,
    /// Where the playtest keeps what it leaves behind: logs, recordings and profiles.
    pub folder: PathBuf,
    pub pace: Pace,
    /// Without one, the server draws its own and logs it.
    pub seed: Option<u64>,
    pub recorded: bool,
}

/// A content file, and how a deploy changes it: read as its type, changed, and written back as
/// the editor writes it.
pub struct DeployEdit<'a> {
    apply: Box<dyn Fn(&FolderReader) -> WrittenFiles + 'a>,
}

impl<'a> DeployEdit<'a> {
    pub fn game(change: impl Fn(&mut GameSettings) + 'a) -> DeployEdit<'a> {
        DeployEdit {
            apply: Box::new(move |reader| {
                let mut settings: GameSettings = reader.settings().expect("the game's settings");
                change(&mut settings);
                WrittenFiles {
                    files: reader
                        .write_settings(&settings)
                        .expect("the game's settings written"),
                    removed: Vec::new(),
                }
            }),
        }
    }

    /// An edit of the one content file of its type named `name`.
    pub fn file<Row: ContentFile>(name: &str, change: impl Fn(&mut Row) + 'a) -> DeployEdit<'a> {
        let path = content::path_named(Row::TABLE, name);
        DeployEdit {
            apply: Box::new(move |reader| {
                let mut row: Row = reader.read(path).expect("a content file to deploy");
                change(&mut row);
                reader.write(path, &row).expect("a deployed content file")
            }),
        }
    }
}

/// One game server and the players who join it. What they leave behind is kept should anything
/// fail, and cleared otherwise.
pub struct Playtest {
    server: Server,
    client_binary: PathBuf,
    content: PathBuf,
    folder: PathBuf,
    pace: Pace,
    recorded: bool,
    joined: usize,
}

impl Playtest {
    /// The memory the game server holds.
    pub fn server_memory(&self) -> Bytes {
        self.server.memory()
    }

    pub fn start(setup: PlaytestSetup) -> Playtest {
        content::load(&setup.content);
        // What an earlier playtest left behind there, its players' profiles among it, plays no
        // part.
        if setup.folder.exists() {
            std::fs::remove_dir_all(&setup.folder).expect("the earlier playtest's folder cleared");
        }
        std::fs::create_dir_all(&setup.folder).expect("a folder for the playtest");
        let server = Server::start(
            &setup.server_binary,
            &setup.content,
            setup.seed,
            &setup.folder.join("server.log"),
        );
        Playtest {
            server,
            client_binary: setup.client_binary,
            content: setup.content,
            folder: setup.folder,
            pace: setup.pace,
            recorded: setup.recorded,
            joined: 0,
        }
    }

    /// A player whose first sign-in plays the test character, starting at `start` or where the
    /// character would.
    pub fn join_as(&mut self, character: &str, start: Option<Place>) -> Player {
        let account = self.next_account();
        let character = content::id::<CharacterSnapshot>(character).path(content::content());
        self.server.hand_over(&account, character, start);
        self.enter_world(&account)
    }

    /// A player new to the game, on a character of their own.
    pub fn join_fresh(&mut self) -> Player {
        let account = self.next_account();
        self.enter_world(&account)
    }

    /// A desktop client started without a sign-in, on the screen that asks for one.
    pub fn start_signed_out(&mut self) -> Player {
        let account = self.next_account();
        self.start_client(&account, None)
    }

    /// A content deploy: rows of the content change and the server restarts on them, as a deploy
    /// does, running `while_down` in between. Clients reconnect, sync the changed files and rejoin.
    /// Returns how many files changed.
    pub fn deploy(&mut self, edits: &[DeployEdit], while_down: impl FnOnce()) -> usize {
        let deployed = self.folder.join("deployed");
        if !deployed.exists() {
            copy_folder(&self.content, &deployed);
        }
        for edit in edits {
            let reader =
                FolderReader::new(registry(), Arc::new(FilesystemFolder(deployed.clone())));
            let written = (edit.apply)(&reader);
            for (file, bytes) in written.files {
                std::fs::write(deployed.join(file), bytes).expect("a deployed content file");
            }
            for file in written.removed {
                std::fs::remove_file(deployed.join(file)).expect("a part no longer deployed");
            }
        }
        self.server.restart_on(&deployed, while_down);
        edits.len()
    }

    fn next_account(&mut self) -> String {
        self.joined += 1;
        format!("player{}", self.joined)
    }

    fn enter_world(&mut self, account: &str) -> Player {
        let token = launch::fake_token(account);
        let mut player = self.start_client(account, Some(token));
        player.wait_within("the player to stand in the world", JOINING, |snapshot| {
            in_world(snapshot).then_some(())
        });
        player
    }

    fn start_client(&mut self, account: &str, token: Option<String>) -> Player {
        let desktop = Desktop::start(
            &self.server,
            &self.client_binary,
            token,
            &self.folder.join(account),
            self.recorded,
        );
        let mut player = Player::new(desktop, self.pace);
        player.snapshot();
        player
    }
}

impl Drop for Playtest {
    fn drop(&mut self) {
        self.server.stop();
        if std::thread::panicking() {
            eprintln!(
                "what the playtest left behind is in {}",
                self.folder.display()
            );
            return;
        }
        let _ = std::fs::remove_dir_all(&self.folder);
        if let Some(parent) = self.folder.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
}

// Joining downloads the content first.
const JOINING: WaitBudget = WaitBudget {
    time: Duration::from_secs(120),
    frames: 1200,
};

fn copy_folder(from: &Path, to: &Path) {
    for entry in WalkDir::new(from).follow_links(true) {
        let entry = entry.expect("a folder entry");
        let target = to.join(
            entry
                .path()
                .strip_prefix(from)
                .expect("an entry within the folder"),
        );
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target).expect("a folder to copy into");
        } else {
            std::fs::copy(entry.path(), &target).expect("a copied file");
        }
    }
}
