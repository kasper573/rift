use std::sync::Mutex;
use std::sync::mpsc::{Receiver, TryRecvError};

// A `Receiver` is `!Sync`, and everything the editor holds lives in one bevy resource, so each
// channel end is wrapped once here instead of at every field.
pub struct ChannelInbox<T>(Mutex<Receiver<T>>);

impl<T> ChannelInbox<T> {
    pub fn new(receiver: Receiver<T>) -> ChannelInbox<T> {
        ChannelInbox(Mutex::new(receiver))
    }

    pub fn drain(&self) -> Vec<T> {
        self.0
            .lock()
            .map(|receiver| receiver.try_iter().collect())
            .unwrap_or_default()
    }

    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        match self.0.lock() {
            Ok(receiver) => receiver.try_recv(),
            Err(_) => Err(TryRecvError::Disconnected),
        }
    }

    pub fn recv(&self) -> Option<T> {
        self.0.lock().ok().and_then(|receiver| receiver.recv().ok())
    }
}
