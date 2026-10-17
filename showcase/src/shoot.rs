use std::cell::RefCell;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

use driver::{ContentEdit, Pace, Player, Playtest, PlaytestSetup};
use engine::systems::script::Place;

use crate::chapters::Chapter;
use crate::film::{Edge, Film};

// Every take of a chapter plays out alike, wherever and however often it's shot.
const SEED: u64 = 1931;

// Where a chapter is played and filmed: a playtest of its own, everyone in it at a viewer's pace.
pub struct Shoot {
    playtest: Playtest,
    title: &'static str,
    summary: &'static str,
    out: PathBuf,
}

pub struct Binaries {
    pub server: PathBuf,
    pub client: PathBuf,
}

impl Shoot {
    pub fn start(
        chapter: &Chapter,
        out: PathBuf,
        binaries: &Binaries,
        content: &Path,
        scratch: PathBuf,
    ) -> Shoot {
        Shoot {
            playtest: Playtest::start(PlaytestSetup {
                server_binary: binaries.server.clone(),
                client_binary: binaries.client.clone(),
                content: content.to_path_buf(),
                folder: scratch,
                pace: Pace::Viewer,
                seed: Some(SEED),
                recorded: true,
            }),
            title: chapter.title,
            summary: chapter.summary,
            out,
        }
    }

    pub fn join_as(&mut self, character: &str, start: Option<Place>) -> Player {
        self.playtest.join_as(character, start)
    }

    pub fn join_fresh(&mut self) -> Player {
        self.playtest.join_fresh()
    }

    pub fn start_signed_out(&mut self) -> Player {
        self.playtest.start_signed_out()
    }

    pub fn deploy(&mut self, edits: &[ContentEdit], while_down: impl FnOnce()) -> usize {
        self.playtest.deploy(edits, while_down)
    }

    // Puts the player's screen on camera: the chapter opens on a card with its title and summary.
    pub fn roll(&mut self, mut player: Player) -> Star {
        let film = Rc::new(RefCell::new(Film::roll(self.title, self.summary)));
        let watching = film.clone();
        player.observe(move |snapshot| watching.borrow_mut().watch(snapshot));
        let mut star = Star {
            player,
            film,
            out: self.out.clone(),
        };
        star.hold();
        star
    }
}

// The player on camera. The chapter plays as fast as the game lets it; only a caption holds it,
// until a viewer could have read it.
pub struct Star {
    player: Player,
    film: Rc<RefCell<Film>>,
    out: PathBuf,
}

impl Star {
    // Each caption starts a beat of the chapter: it goes up at once and the chapter plays on beneath
    // it, but the next one waits until this one could be read.
    pub fn caption(&mut self, text: &str) {
        self.hold();
        self.film.borrow_mut().caption(text, Edge::Bottom);
    }

    // Clear of whatever the game draws along the bottom edge.
    pub fn caption_top(&mut self, text: &str) {
        self.hold();
        self.film.borrow_mut().caption(text, Edge::Top);
    }

    // Lets the viewer finish reading the caption that's up, while what it describes stays on screen.
    pub fn hold(&mut self) {
        let until = self.film.borrow().read_until();
        self.player.linger_until(until);
    }

    // A recording can't show which channel a track plays on, or how loud, so this panel does.
    pub fn show_mixer(&mut self) {
        self.film.borrow_mut().show_mixer();
    }

    pub fn hide_mixer(&mut self) {
        self.film.borrow_mut().hide_mixer();
    }

    // The other player's screen shows in a corner of this one's while `during` plays.
    pub fn inset(
        &mut self,
        mut other: Player,
        label: &str,
        during: impl FnOnce(&mut Star, &mut Player),
    ) {
        let from = SystemTime::now();
        during(self, &mut other);
        if let Some(footage) = other.stop() {
            self.film.borrow_mut().inset(footage, label, from);
        }
    }
}

impl Deref for Star {
    type Target = Player;

    fn deref(&self) -> &Player {
        &self.player
    }
}

impl DerefMut for Star {
    fn deref_mut(&mut self) -> &mut Player {
        &mut self.player
    }
}

// The film is cut once the last caption has been read. A failed chapter's film is cut as it stands,
// and left with the rest of what the chapter left behind.
impl Drop for Star {
    fn drop(&mut self) {
        let failed = std::thread::panicking();
        if !failed {
            self.hold();
        }
        let scratch = self.player.folder().to_path_buf();
        let out = if failed {
            scratch.join("film.mp4")
        } else {
            self.out.clone()
        };
        if let Some(footage) = self.player.stop() {
            self.film.borrow_mut().render(&footage, &scratch, &out);
            footage.discard();
        }
    }
}
