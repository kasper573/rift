use std::time::Duration;

use bevy_ecs::prelude::*;
use engine::core::content::Content;
use engine::core::store::{Record, RecordId, RecordStore, RecordStoreAddress};
use engine::core::time::WallClock;
use engine::core::units::Seconds;
use engine::systems::player;
use engine::systems::save::{CheckpointReport, Checkpoints, SavedCharacter};
use engine::systems::worlds::{self, AboveWorlds};
use testkit::Sim;

/// How long a test's server plays on between two checkpoints.
pub const EVERY: Seconds = Seconds(30.0);

/// A store in a file of its own, and the runtime checkpoints are written on.
pub struct Saves {
    pub runtime: tokio::runtime::Runtime,
    pub store: RecordStore,
    address: RecordStoreAddress,
    _folder: tempfile::TempDir,
}

impl Saves {
    pub fn in_a_file() -> Saves {
        let runtime = tokio::runtime::Runtime::new().expect("a runtime");
        let folder = tempfile::tempdir().expect("a folder for the store");
        let address = RecordStoreAddress::file(&folder.path().join("records.sqlite"));
        let store = runtime
            .block_on(RecordStore::open(&address))
            .expect("a file opens");
        Saves {
            runtime,
            store,
            address,
            _folder: folder,
        }
    }

    /// What a server that saves the game `content` holds to this store keeps above its worlds.
    pub fn above(&self, content: &Content) -> AboveWorlds {
        AboveWorlds {
            checkpoints: Checkpoints::to(
                self.store.clone(),
                self.runtime.handle().clone(),
                content,
                EVERY,
                Duration::ZERO,
            ),
            ..AboveWorlds::unsaved()
        }
    }

    pub fn character(&self, account: &str) -> Option<String> {
        let id = RecordId(account.to_owned());
        self.runtime
            .block_on(self.store.one(SavedCharacter::KIND, &id))
            .expect("the store is read")
    }

    pub fn all(&self, kind: &'static str) -> Vec<String> {
        self.runtime
            .block_on(self.store.all(kind))
            .expect("the store is read")
    }

    /// Has the file refuse every character put in it, or take them again.
    pub fn refuse_characters(&self, refuse: bool) {
        let rule = if refuse {
            "CREATE TRIGGER refuse BEFORE INSERT ON records WHEN NEW.kind = 'character' \
             BEGIN SELECT RAISE(ABORT, 'refused'); END"
        } else {
            "DROP TRIGGER refuse"
        };
        self.runtime.block_on(async {
            let pool = sqlx::AnyPool::connect(self.address.as_str())
                .await
                .expect("the file opens again");
            sqlx::query(rule).execute(&pool).await.expect("a rule");
        });
    }
}

/// Has the worlds of `sims` confer, as a server's do between ticks: who left them since.
pub fn confer(sims: &mut [&mut Sim], above: &mut AboveWorlds) -> Vec<player::Departure> {
    let now = sims[0].world.resource::<WallClock>().now;
    let mut views: Vec<Option<&mut World>> =
        sims.iter_mut().map(|sim| Some(&mut sim.world)).collect();
    worlds::confer(&mut views, above, now)
}

/// The next checkpoint of worlds that have conferred before, an interval on: how it went, once
/// the store has answered.
pub fn checkpoint(sims: &mut [&mut Sim], above: &mut AboveWorlds) -> CheckpointReport {
    for sim in sims.iter_mut() {
        sim.later(EVERY.0);
    }
    for _ in 0..5000 {
        confer(sims, above);
        if let Some(report) = above.checkpoints.reports().pop() {
            return report;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("the store never answered the checkpoint");
}
