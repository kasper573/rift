use std::path::{Path, PathBuf};
use std::time::Duration;

use engine::systems::launch;
use engine::systems::script::Place;
use serde_json::Value;

use crate::content;
use crate::desktop::{Desktop, SCREEN, WaitBudget};
use crate::player::{Pace, Player};
use crate::seen::in_world;
use crate::server::Server;

pub struct PlaytestSetup {
    pub server_binary: PathBuf,
    pub client_binary: PathBuf,
    // The content the server starts on, and that the driver reads names and places from.
    pub content: PathBuf,
    // Where the playtest keeps what it leaves behind: logs, recordings and profiles.
    pub folder: PathBuf,
    pub pace: Pace,
    // Without one, the server draws its own and logs it.
    pub seed: Option<u64>,
    pub recorded: bool,
}

// A content file, and how a deploy changes it.
pub type ContentEdit<'a> = (&'a str, &'a dyn Fn(&mut Value));

// One game server and the players who join it. What they leave behind is kept should anything
// fail, and cleared otherwise.
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
    pub fn start(setup: PlaytestSetup) -> Playtest {
        content::load(&setup.content);
        // What an earlier playtest left behind there, its players' profiles among it, plays no part.
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

    // A player whose first sign-in plays the test character, starting at `start` or where the
    // character would.
    pub fn join_as(&mut self, character: &str, start: Option<Place>) -> Player {
        let account = self.next_account();
        self.server.hand_over(&account, character, start);
        self.enter_world(&account, SCREEN)
    }

    // A player new to the game, on a character of their own.
    pub fn join_fresh(&mut self) -> Player {
        let account = self.next_account();
        self.enter_world(&account, SCREEN)
    }

    // A desktop client started without a sign-in, on the screen that asks for one.
    pub fn start_signed_out(&mut self) -> Player {
        let account = self.next_account();
        self.start_client(&account, None, SCREEN)
    }

    // A content deploy: rows of the content change and the server restarts on them, as a deploy does,
    // running `while_down` in between. Clients reconnect, sync the changed files and rejoin. Returns
    // how many files changed.
    pub fn deploy(&mut self, edits: &[ContentEdit], while_down: impl FnOnce()) -> usize {
        let deployed = self.folder.join("deployed");
        if !deployed.exists() {
            copy_folder(&self.content, &deployed);
        }
        for (file, edit) in edits {
            let path = deployed.join(file);
            let text = std::fs::read_to_string(&path).expect("a content file to deploy");
            let mut row: Value = serde_json::from_str(&text).expect("a content row");
            edit(&mut row);
            let text = serde_json::to_string_pretty(&row).expect("a content row");
            std::fs::write(&path, text + "\n").expect("a deployed content file");
        }
        self.server.restart_on(&deployed, while_down);
        edits.len()
    }

    fn next_account(&mut self) -> String {
        self.joined += 1;
        format!("player{}", self.joined)
    }

    fn enter_world(&mut self, account: &str, screen: (u32, u32)) -> Player {
        let token = launch::fake_token(account);
        let mut player = self.start_client(account, Some(token), screen);
        player.wait_within("the player to stand in the world", JOINING, |snapshot| {
            in_world(snapshot).then_some(())
        });
        player
    }

    fn start_client(&mut self, account: &str, token: Option<String>, screen: (u32, u32)) -> Player {
        let desktop = Desktop::start(
            &self.server,
            &self.client_binary,
            token,
            &self.folder.join(account),
            self.recorded,
            screen,
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
    std::fs::create_dir_all(to).expect("a folder to copy into");
    for entry in std::fs::read_dir(from).expect("a folder to copy") {
        let entry = entry.expect("a folder entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("an entry's type").is_dir() {
            copy_folder(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("a copied file");
        }
    }
}
