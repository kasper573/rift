use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Condvar, Mutex};

use crate::core::content::Content;

// The driver asks with a line and waits for its answer: a snapshot of a frame after it, or what the
// game says, answered on the socket's own thread once the game is loaded.
pub(super) struct ProbeSocket {
    asked: Receiver<FrameQuestion>,
    answers: Answers,
    game: Arc<(Mutex<Option<Content>>, Condvar)>,
}

/// A question only a frame answers.
pub(super) enum FrameQuestion {
    Snapshot,
    Picture(PathBuf),
}

/// Where answers go, from whichever thread has one. A driver that hung up wants no more of them.
#[derive(Clone)]
pub(super) struct Answers(Arc<Mutex<TcpStream>>);

impl Answers {
    pub(super) fn give(&self, answer: &str) {
        let mut stream = self.0.lock().expect("the probe's answers");
        let _ = writeln!(stream, "{answer}");
    }
}

impl ProbeSocket {
    pub(super) fn connect(driver: SocketAddr) -> ProbeSocket {
        let stream = TcpStream::connect(driver)
            .unwrap_or_else(|error| panic!("the probe's driver at {driver}: {error}"));
        let questions = stream
            .try_clone()
            .expect("a second handle on the probe socket");
        let answers = Answers(Arc::new(Mutex::new(stream)));
        let game: Arc<(Mutex<Option<Content>>, Condvar)> = Arc::default();
        let (ask, asked) = channel();
        let (thread_answers, thread_game) = (answers.clone(), game.clone());
        std::thread::spawn(move || {
            for line in BufReader::new(questions).lines().map_while(Result::ok) {
                if let Some(question) = super::frame_question(&line) {
                    if ask.send(question).is_err() {
                        return;
                    }
                    continue;
                }
                let content = loaded(&thread_game);
                let question = super::read_question(&content, &line)
                    .unwrap_or_else(|error| panic!("the probe's driver asked {line}: {error}"));
                let answer = super::answer(&content, &question).expect("a question about the game");
                thread_answers.give(&answer);
            }
        });
        ProbeSocket {
            asked,
            answers,
            game,
        }
    }
}

fn loaded(game: &(Mutex<Option<Content>>, Condvar)) -> Content {
    let (content, known) = game;
    let content = known
        .wait_while(content.lock().expect("the probe's game"), |content| {
            content.is_none()
        })
        .expect("the probe's game");
    content.clone().expect("a loaded game")
}

impl ProbeSocket {
    /// The frame questions asked since the last frame.
    pub(super) fn asked(&self) -> Vec<FrameQuestion> {
        self.asked.try_iter().collect()
    }

    pub(super) fn answers(&self) -> Answers {
        self.answers.clone()
    }

    pub(super) fn know(&self, content: &Content) {
        let (game, known) = &*self.game;
        *game.lock().expect("the probe's game") = Some(content.clone());
        known.notify_all();
    }
}
