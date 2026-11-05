use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use serde::de::DeserializeOwned;

/// The driver asks with a line and waits for its answer: a frame after it answers a question of
/// `Q`, and any other line is answered on the socket's own thread.
pub struct ProbeSocket<Q> {
    asked: Receiver<Q>,
    answers: Answers,
}

/// Where answers go, from whichever thread has one. A driver that hung up wants no more of them.
#[derive(Clone)]
pub struct Answers(Arc<Mutex<TcpStream>>);

impl Answers {
    pub fn give(&self, answer: &str) {
        let mut stream = self.0.lock().expect("the probe's answers");
        let _ = writeln!(stream, "{answer}");
    }
}

impl<Q: DeserializeOwned + Send + 'static> ProbeSocket<Q> {
    /// Connects to the driver at `driver`, answering every line that isn't a question of `Q` with
    /// what `otherwise` says of it.
    pub fn connect(
        driver: SocketAddr,
        mut otherwise: impl FnMut(&str) -> String + Send + 'static,
    ) -> ProbeSocket<Q> {
        let stream = TcpStream::connect(driver)
            .unwrap_or_else(|error| panic!("the probe's driver at {driver}: {error}"));
        let questions = stream
            .try_clone()
            .expect("a second handle on the probe socket");
        let answers = Answers(Arc::new(Mutex::new(stream)));
        let (ask, asked) = channel();
        let thread_answers = answers.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(questions).lines().map_while(Result::ok) {
                if let Ok(question) = serde_json::from_str::<Q>(&line) {
                    if ask.send(question).is_err() {
                        return;
                    }
                    continue;
                }
                thread_answers.give(&otherwise(&line));
            }
        });
        ProbeSocket { asked, answers }
    }

    /// The frame questions asked since the last frame.
    pub fn asked(&self) -> Vec<Q> {
        self.asked.try_iter().collect()
    }

    pub fn answers(&self) -> Answers {
        self.answers.clone()
    }
}
