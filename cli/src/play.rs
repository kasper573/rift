//! Plays characters on a running game server with no window, through the engine's headless
//! client: every line of stdin tells a client what to do, and stdout says how it goes.

use std::collections::BTreeMap;
use std::io::BufRead;
use std::path::Path;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Instant;

use engine::core::content::{Content, ContentScope, FilesystemFolder, LoadPurpose, with_content};
use engine::systems::TICK_HZ;
use engine::systems::player::headless::{self, HeadlessClient, PlayAsk, PlayLine};

use crate::folder;

/// Plays until stdin closes, which leaves the game as players closing their clients do.
pub fn run(game: &Path, http_url: &str) -> Result<(), String> {
    let content =
        folder::load(game, "play", LoadPurpose::Play).map_err(|report| report.to_string())?;
    if !headless::runs(http_url, &content, &FilesystemFolder(game.to_path_buf()))? {
        return Err(format!(
            "{http_url} runs another game than {}",
            game.display()
        ));
    }
    let asked = stdin_lines();
    let mut cast = Cast {
        http_url,
        content,
        players: BTreeMap::new(),
    };
    loop {
        let tick = Instant::now();
        loop {
            match asked.try_recv() {
                Ok(line) => {
                    if let Err(why) = cast.heed(&line) {
                        cast.say(&PlayLine::Refused { line, why });
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }
        cast.update();
        if let Some(rest) = TICK_HZ.period().checked_sub(tick.elapsed()) {
            std::thread::sleep(rest);
        }
    }
}

/// The clients of one run, each under the name whoever runs them gave it.
struct Cast<'a> {
    http_url: &'a str,
    content: Content,
    players: BTreeMap<String, Playing>,
}

struct Playing {
    client: HeadlessClient,
    in_world: bool,
}

impl Cast<'_> {
    fn heed(&mut self, line: &str) -> Result<(), String> {
        let ask: PlayAsk = serde_json::from_str(line).map_err(|error| error.to_string())?;
        match ask {
            PlayAsk::Join { player, token } => {
                if self.players.contains_key(&player) {
                    return Err(format!("{player} already plays"));
                }
                let client = HeadlessClient::connect(self.http_url, &token)?.knowing(&self.content);
                let in_world = false;
                self.players.insert(player, Playing { client, in_world });
            }
            PlayAsk::Request { player, request } => self
                .playing(&player)?
                .client
                .request(request)
                .map_err(|refusal| refusal.to_string())?,
            PlayAsk::Look { player } => {
                let sight = self.playing(&player)?.client.sight();
                let sight = Box::new(sight.ok_or("the client does not know the game")?);
                self.say(&PlayLine::Sight { player, sight });
            }
            PlayAsk::Leave { player } => {
                self.players
                    .remove(&player)
                    .ok_or_else(|| format!("nobody plays as {player}"))?;
            }
        }
        Ok(())
    }

    fn update(&mut self) {
        let mut said = Vec::new();
        self.players.retain(|player, playing| {
            playing.client.update();
            let player = player.clone();
            if !playing.client.is_connected() && !playing.client.is_connecting() {
                said.push(PlayLine::Dropped { player });
                return false;
            }
            if playing.client.in_world() != playing.in_world {
                playing.in_world = playing.client.in_world();
                said.push(if playing.in_world {
                    PlayLine::InWorld { player }
                } else {
                    PlayLine::OutOfWorld { player }
                });
            }
            true
        });
        for line in &said {
            self.say(line);
        }
    }

    fn playing(&mut self, player: &str) -> Result<&mut Playing, String> {
        self.players
            .get_mut(player)
            .ok_or_else(|| format!("nobody plays as {player}"))
    }

    fn say(&self, line: &PlayLine) {
        let (line, _) = with_content(ContentScope::of(&self.content), || {
            serde_json::to_string(line).expect("a serializable line")
        });
        println!("{line}");
    }
}

fn stdin_lines() -> Receiver<String> {
    let (lines, taken) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { return };
            if !line.trim().is_empty() && lines.send(line).is_err() {
                return;
            }
        }
    });
    taken
}
