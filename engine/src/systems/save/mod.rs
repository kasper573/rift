//! Saving the game: every character and every record of the server as they stood at one
//! instant, written whole or not at all.

pub mod load;
pub mod notice;
pub mod unwritten;

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use bevy_ecs::change_detection::Tick;
use bevy_ecs::prelude::*;
pub use notice::Returned;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
pub use unwritten::{Pending, RecordKey, Unwritten, Writable};

use crate::core::content::{Content, ContentScope};
use crate::core::store::{
    Record, RecordChange, RecordId, RecordOwner, RecordRow, RecordStore, RecordStoreProblem,
};
use crate::core::time::UnixMillis;
use crate::core::units::Seconds;
use crate::systems::account::AccountId;
pub use crate::systems::player::snapshot::{GameChange, SavedCharacter};
use crate::systems::player::{self, Departure, Players};
use crate::systems::status::Statuses;

impl Record for SavedCharacter {
    const KIND: &'static str = "character";

    fn id(&self) -> RecordId {
        RecordId(self.account.0.clone())
    }

    fn owner(&self) -> Option<RecordOwner> {
        Some(RecordOwner(self.account.0.clone()))
    }
}

/// How saving stands, as every world of the server knows it.
#[derive(Resource, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ServerSaving {
    /// Since when nothing is saved, once enough saves in a row have failed that players are
    /// told: the last save that worked.
    pub failing_since: Option<UnixMillis>,
}

/// How far a world's characters are saved: its change tick as the last checkpoint that was
/// written took them, and as the one being written did.
#[derive(Resource, Default)]
struct SavedThrough {
    written: Option<Tick>,
    taken: Option<(CheckpointId, Tick)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CheckpointId(u64);

/// What saving has under way: when the next checkpoint is due, what is not yet written, and how
/// the last ones went.
pub struct Checkpoints {
    link: Option<StoreLink>,
    every: Seconds,
    due: Option<UnixMillis>,
    pub unwritten: Unwritten,
    departed: DepartedCharacters,
    keeps_departed: Duration,
    writing: Option<Writing>,
    taken: u64,
    failures: u32,
    /// When saving began: what nothing saved since is counted from.
    began: Option<UnixMillis>,
    last_saved: Option<UnixMillis>,
    reports: Vec<CheckpointReport>,
}

/// How one checkpoint went, for whoever keeps the server's numbers.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckpointReport {
    pub outcome: Result<usize, RecordStoreProblem>,
    pub took: Duration,
}

/// Where checkpoints are written, and on what.
struct StoreLink {
    store: RecordStore,
    runtime: tokio::runtime::Handle,
    scope: ContentScope,
    results: Sender<Written>,
    written: Receiver<Written>,
}

struct Writing {
    taken_at: UnixMillis,
    changes: Vec<(RecordKey, Pending)>,
}

struct Written {
    id: CheckpointId,
    report: CheckpointReport,
}

/// The characters that left their worlds and memory still holds: each as it left, newer than
/// anything the store has of it, or the same.
#[derive(Clone, Default)]
pub struct DepartedCharacters(Arc<Mutex<HashMap<AccountId, Departed>>>);

impl DepartedCharacters {
    pub fn of(&self, account: &AccountId) -> Option<Arc<SavedCharacter>> {
        self.held()
            .get(account)
            .map(|departed| departed.saved.clone())
    }

    fn held(&self) -> MutexGuard<'_, HashMap<AccountId, Departed>> {
        self.0.lock().expect("departed characters lock")
    }
}

/// A character that left its world, kept until a checkpoint has written it and no sign-in that
/// read the store before it left can still connect.
struct Departed {
    saved: Arc<SavedCharacter>,
    left: Instant,
}

/// How many checkpoints fail in a row before players are told the game is not being saved.
const TOLD_AFTER: u32 = 3;

impl Checkpoints {
    /// Saving nothing: what a server with no store keeps of it.
    pub fn none() -> Checkpoints {
        Checkpoints {
            link: None,
            every: Seconds(0.0),
            due: None,
            unwritten: Unwritten::default(),
            departed: DepartedCharacters::default(),
            keeps_departed: Duration::ZERO,
            writing: None,
            taken: 0,
            failures: 0,
            began: None,
            last_saved: None,
            reports: Vec::new(),
        }
    }

    /// Saving the game `content` holds to `store` every `every`, each checkpoint written on
    /// `runtime`. A character that left stays in memory for `keeps_departed` at the least: as
    /// long as a sign-in that read the store before it left may take to connect.
    pub fn to(
        store: RecordStore,
        runtime: tokio::runtime::Handle,
        content: &Content,
        every: Seconds,
        keeps_departed: Duration,
    ) -> Checkpoints {
        let (results, written) = channel();
        Checkpoints {
            link: Some(StoreLink {
                store,
                runtime,
                scope: ContentScope::of(content),
                results,
                written,
            }),
            every,
            keeps_departed,
            ..Checkpoints::none()
        }
    }

    /// The character of `account` as it left its world, while memory still holds it: newer than
    /// anything the store has of it, or the same.
    pub fn departed(&self, account: &AccountId) -> Option<Arc<SavedCharacter>> {
        self.departed.of(account)
    }

    /// The characters memory holds since they left, to be asked from any thread.
    pub fn memory(&self) -> DepartedCharacters {
        self.departed.clone()
    }

    /// How many records wait to be written: what a crash right now would cost.
    pub fn waiting(&self) -> usize {
        self.unwritten.len()
    }

    /// The last save that worked.
    pub fn last_saved(&self) -> Option<UnixMillis> {
        self.last_saved
    }

    /// How the checkpoints since this was last asked went.
    pub fn reports(&mut self) -> Vec<CheckpointReport> {
        std::mem::take(&mut self.reports)
    }

    /// Takes in the characters that left `views` since the worlds last conferred, and says whose
    /// clients they were.
    pub fn take_in(&mut self, views: &mut [Option<&mut World>]) -> Vec<Departure> {
        let mut gone = Vec::new();
        for world in views.iter_mut().flatten() {
            let Some(mut departures) = world.get_resource_mut::<player::Departures>() else {
                continue;
            };
            gone.append(&mut departures.0);
        }
        for departure in &gone {
            let Some(saved) = &departure.left else {
                continue;
            };
            let saved = Arc::new(saved.clone());
            if self.link.is_some() {
                self.unwritten.put_shared(saved.clone());
            }
            self.departed.held().insert(
                saved.account.clone(),
                Departed {
                    saved,
                    left: Instant::now(),
                },
            );
        }
        gone
    }

    /// Hears how the checkpoint being written went, once it is known.
    fn hear(&mut self, views: &mut [Option<&mut World>], wait: bool) {
        let Some(link) = &self.link else {
            return;
        };
        if self.writing.is_none() {
            return;
        }
        let heard = if wait {
            link.written.recv().ok()
        } else {
            link.written.try_recv().ok()
        };
        let Some(written) = heard else {
            return;
        };
        let Some(writing) = self.writing.take() else {
            return;
        };
        let worked = written.report.outcome.is_ok();
        for world in views.iter_mut().flatten() {
            let Some(mut through) = world.get_resource_mut::<SavedThrough>() else {
                continue;
            };
            if let Some((id, tick)) = through.taken.take()
                && id == written.id
                && worked
            {
                through.written = Some(tick);
            }
        }
        if worked {
            self.unwritten.forget(&writing.changes);
            self.failures = 0;
            self.last_saved = Some(writing.taken_at);
        } else {
            self.failures += 1;
        }
        self.reports.push(written.report);
        let keeps = self.keeps_departed;
        let unwritten = &self.unwritten;
        self.departed.held().retain(|_, departed| {
            departed.left.elapsed() < keeps
                || unwritten.holds(&(SavedCharacter::KIND, departed.saved.id()))
        });
    }

    /// Whether a checkpoint is due at `now`: the first one an interval after saving began.
    fn due(&mut self, now: UnixMillis) -> bool {
        self.began.get_or_insert(now);
        let due = *self.due.get_or_insert(now.after(self.every));
        now >= due
    }

    /// Snapshots every character in play that changed since it was last written and hands them,
    /// with everything unwritten, to the store.
    fn take(&mut self, views: &mut [Option<&mut World>], now: UnixMillis) {
        let Some(link) = &self.link else {
            return;
        };
        self.taken += 1;
        let id = CheckpointId(self.taken);
        let characters: Vec<SavedCharacter> = views
            .par_iter_mut()
            .filter_map(|world| world.as_deref_mut())
            .flat_map_iter(|world| snapshot_changed(world, id))
            .collect();
        let changes = self.unwritten.pending();
        let (store, scope, results) =
            (link.store.clone(), link.scope.clone(), link.results.clone());
        let handed = changes.clone();
        link.runtime.spawn(async move {
            let began = Instant::now();
            let outcome = write(&store, &scope, handed, characters).await;
            let _ = results.send(Written {
                id,
                report: CheckpointReport {
                    outcome,
                    took: began.elapsed(),
                },
            });
        });
        self.writing = Some(Writing {
            taken_at: now,
            changes,
        });
        self.due = Some(now.after(self.every));
    }

    /// What every world is told of how saving stands.
    fn tell(&mut self, views: &mut [Option<&mut World>]) {
        let telling = ServerSaving {
            failing_since: (self.failures >= TOLD_AFTER)
                .then(|| self.last_saved.or(self.began))
                .flatten(),
        };
        for world in views.iter_mut().flatten() {
            if world.get_resource::<ServerSaving>() != Some(&telling) {
                world.insert_resource(telling);
            }
        }
    }
}

/// In `worlds::confer`, once society has settled: takes in whoever left, hears how the last
/// checkpoint went and, when the interval is up and none is being written, takes the next.
pub fn checkpoint(
    views: &mut [Option<&mut World>],
    checkpoints: &mut Checkpoints,
    now: UnixMillis,
) -> Vec<Departure> {
    let gone = checkpoints.take_in(views);
    checkpoints.hear(views, false);
    if checkpoints.link.is_some() && checkpoints.writing.is_none() && checkpoints.due(now) {
        checkpoints.take(views, now);
    }
    checkpoints.tell(views);
    gone
}

/// Saves everything as it stands at `now` and waits until it is written: what a server does
/// before it stops. Says what was not written when the store refuses it.
pub fn save_all(
    views: &mut [Option<&mut World>],
    checkpoints: &mut Checkpoints,
    now: UnixMillis,
) -> Result<(), RecordStoreProblem> {
    if checkpoints.link.is_none() {
        return Ok(());
    }
    checkpoints.take_in(views);
    checkpoints.hear(views, true);
    checkpoints.take(views, now);
    checkpoints.hear(views, true);
    match checkpoints.reports.last() {
        Some(CheckpointReport {
            outcome: Err(problem),
            ..
        }) => Err(RecordStoreProblem(format!(
            "{} records are not written: {problem}",
            checkpoints.waiting()
        ))),
        _ => Ok(()),
    }
}

/// The characters of `world` that changed since a checkpoint last wrote them, as they stand.
/// One that carries an effect that runs out counts as changed for as long as it does: its
/// snapshot holds the time the effect has left, which is less each time.
fn snapshot_changed(world: &mut World, id: CheckpointId) -> Vec<SavedCharacter> {
    if !world.contains_resource::<Players>() {
        return Vec::new();
    }
    let since = world.get_resource_or_init::<SavedThrough>().written;
    let taken = world.increment_change_tick();
    world.resource_mut::<SavedThrough>().taken = Some((id, taken));
    let world = &*world;
    world
        .resource::<Players>()
        .0
        .values()
        .filter(|&&character| {
            since.is_none_or(|since| player::changed_since(world, character, since))
                || world
                    .get::<Statuses>(character)
                    .is_some_and(|statuses| statuses.0.iter().any(|held| held.until.is_some()))
        })
        .filter_map(|&character| SavedCharacter::of_live(world, character))
        .collect()
}

/// Writes one checkpoint: everything unwritten, then every character in play, which is newer
/// than a copy of it that left earlier. How many records it wrote.
async fn write(
    store: &RecordStore,
    scope: &ContentScope,
    changes: Vec<(RecordKey, Pending)>,
    characters: Vec<SavedCharacter>,
) -> Result<usize, RecordStoreProblem> {
    let rows: Vec<RecordChange> = changes
        .iter()
        .map(|(key, pending)| pending.change(key, scope))
        .chain(
            characters
                .iter()
                .map(|character| RecordChange::Put(RecordRow::of(character, scope))),
        )
        .collect();
    let count = rows.len();
    store.write(rows).await.map(|()| count)
}
