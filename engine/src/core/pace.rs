//! How work that would hold a frame is paced. A client's main thread is also its window's, and in
//! a browser the only thread there is, so whatever long work runs on it (loading the game's
//! content, decoding a sound, baking a paint, making a rig's clips) takes a slice of every frame
//! and breathes until the next. A server, a tool or a test runs the same work whole, in parallel
//! where it can.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bevy::platform::time::Instant;
use bevy_ecs::prelude::Resource;
use futures_timer::Delay;
use rayon::prelude::*;

/// How long a breath waits before looking for the next frame again.
const FRAME_WAIT: Duration = Duration::from_millis(1);

/// The pace paced work runs at: whole, or a slice of every frame.
#[derive(Resource, Clone)]
pub struct Pace(Arc<Paced>);

enum Paced {
    Whole,
    Framed(Frames),
}

struct Frames {
    frame: AtomicU64,
    slice: Mutex<Slice>,
}

/// A frame's slice of time for paced work, counted from the moment the work first asks for it,
/// as the frame's own work before that is not the slice's to spend.
struct Slice {
    started: Option<Instant>,
    budget: Duration,
}

impl Default for Pace {
    fn default() -> Pace {
        Pace::whole()
    }
}

impl Pace {
    /// Work runs whole, in parallel where it can: the pace of a server, a tool and a test.
    pub fn whole() -> Pace {
        Pace(Arc::new(Paced::Whole))
    }

    /// Work takes a slice of every frame, each begun with `frame_begins`: the pace of a client,
    /// whose frames must keep coming.
    pub fn framed() -> Pace {
        Pace(Arc::new(Paced::Framed(Frames {
            frame: AtomicU64::new(0),
            slice: Mutex::new(Slice {
                started: None,
                budget: Duration::ZERO,
            }),
        })))
    }

    /// Whether work is paced by frames at all. Work whose cost falls on the frames after it (a
    /// load it starts) can't be measured by a slice, so it holds itself to a few starts a frame
    /// only where frames are paced.
    pub fn is_framed(&self) -> bool {
        matches!(&*self.0, Paced::Framed(_))
    }

    /// Begins a frame that gives paced work `budget` of its time.
    pub fn frame_begins(&self, budget: Duration) {
        let Paced::Framed(frames) = &*self.0 else {
            return;
        };
        frames.frame.fetch_add(1, Ordering::Relaxed);
        *frames
            .slice
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Slice {
            started: None,
            budget,
        };
    }

    /// Whether this frame's slice is used up, so a loop stops until the next frame.
    pub fn spent(&self) -> bool {
        let Paced::Framed(frames) = &*self.0 else {
            return false;
        };
        let mut slice = frames
            .slice
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let started = *slice.started.get_or_insert_with(Instant::now);
        started.elapsed() > slice.budget
    }

    /// Lets the frame go once its slice is used up, and comes back in the next.
    pub async fn breathe(&self) {
        let Paced::Framed(frames) = &*self.0 else {
            return;
        };
        if !self.spent() {
            return;
        }
        let frame = frames.frame.load(Ordering::Relaxed);
        while frames.frame.load(Ordering::Relaxed) == frame {
            Delay::new(FRAME_WAIT).await;
        }
    }

    /// `work` on every item: across every thread at once where work runs whole, one after another
    /// with breaths between them where it is paced.
    pub async fn each<T, R>(&self, items: Vec<T>, work: impl Fn(T) -> R + Send + Sync) -> Vec<R>
    where
        T: Send,
        R: Send,
    {
        match &*self.0 {
            Paced::Whole => items.into_par_iter().map(work).collect(),
            Paced::Framed(_) => {
                let mut done = Vec::with_capacity(items.len());
                for item in items {
                    done.push(work(item));
                    self.breathe().await;
                }
                done
            }
        }
    }
}
