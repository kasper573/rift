use std::cell::RefCell;
use std::collections::HashSet;
use std::time::Duration;

use bevy::tasks::futures_lite::future::block_on;
use engine::core::content::store::ContentStore;
use engine::core::content::sync::{
    ContentSource, ServedContent, SyncError, SyncPolicy, SyncStatus, SyncedContent, sync,
};
use engine::core::content::version;
use engine::core::platform::{FetchFailure, LocalFuture};
use engine::core::time::{Backoff, Seconds};
use testkit::{Game, wav};

use crate::store::MemoryStore;
use crate::support::game;

const FILES: &str = "/content/files/";

/// The assets are read from the store as they are asked for, so one the store loses after the
/// sync reads as missing, and reads again once it is back.
#[test]
fn a_first_visit_fetches_everything_keeping_only_it_and_a_later_visit_only_the_manifest() {
    let server = FakeServer::new(served(sounding()));
    let store = MemoryStore::default();
    let stale = "0".repeat(64);
    store.put(&stale, b"left over from another game");

    let synced = run(&server, &store).expect("the first visit syncs");

    let every_file = expected_hashes(&server.served.borrow(), &synced);
    let requested: HashSet<String> = server.requested_hashes().into_iter().collect();
    assert_eq!(requested, every_file);
    assert_eq!(synced.assets.len(), synced.index.files.len());
    assert!(!synced.rows.is_empty());
    assert!(!synced.memory_only);
    let kept: HashSet<String> = store.hashes().into_iter().collect();
    assert_eq!(kept, every_file);

    server.forget_requests();
    let again = run(&server, &store).expect("the next visit syncs");
    assert_eq!(server.requests.borrow().as_slice(), ["/content/manifest"]);
    assert!(again.manifest.same_content(&synced.manifest));
    assert_eq!(again.assets.len(), synced.assets.len());

    let file = &again.index.files[0];
    let bytes = again.assets.read(&file.path).expect("the asset reads");
    store.drop_entry(&file.hash);
    assert!(
        again.assets.read(&file.path).is_none(),
        "lost from the store"
    );
    store.put(&file.hash, &bytes);
    assert_eq!(again.assets.read(&file.path).as_deref(), Some(&bytes[..]));
}

#[test]
fn a_player_fetches_only_the_media_content_names() {
    let server = FakeServer::new(served(
        sounding()
            .file("downloads/Unused Clip.glb", testkit::glb(&[]))
            .file("downloads/manual.pdf", "a manual"),
    ));
    let synced = run(&server, &MemoryStore::default()).expect("it syncs");

    let fetched: HashSet<&str> = synced
        .index
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    assert!(fetched.contains(slash_clip().as_str()), "{fetched:?}");
    assert!(
        fetched.iter().all(|path| !path.starts_with("downloads/")),
        "nothing names the downloads: {fetched:?}"
    );
}

#[test]
fn an_interrupted_sync_fetches_exactly_the_files_the_store_lacks() {
    let server = FakeServer::new(served(sounding()));
    let complete = MemoryStore::default();
    run(&server, &complete).expect("a full sync");
    let every_file = complete.hashes();
    let (kept, lacked) = every_file.split_at(every_file.len() / 2);
    let partial = MemoryStore::default();
    for hash in kept {
        let bytes = block_on(complete.read(hash)).expect("a kept file");
        partial.put(hash, &bytes);
    }
    server.forget_requests();

    run(&server, &partial).expect("the sync resumes");

    let requested: HashSet<String> = server.requested_hashes().into_iter().collect();
    let lacked: HashSet<String> = lacked.iter().cloned().collect();
    assert_eq!(requested, lacked);
    assert_eq!(partial.hashes(), every_file);
}

#[test]
fn a_bad_hash_is_fetched_again() {
    let server = FakeServer::new(served(sounding()));
    let store = MemoryStore::default();
    let first = run(&server, &store).expect("a full sync");
    let file = first
        .index
        .files
        .iter()
        .find(|file| file.path == slash_clip())
        .expect("the sound is indexed")
        .clone();

    server.corrupt_once.borrow_mut().insert(file.hash.clone());
    let fresh = MemoryStore::default();
    server.forget_requests();
    let synced = run(&server, &fresh).expect("a corrupt download is retried");
    let fetched_twice = server
        .requested_hashes()
        .iter()
        .filter(|hash| **hash == file.hash)
        .count();
    assert_eq!(fetched_twice, 2);
    assert_eq!(
        blake3::hash(&synced.assets.read(&file.path).expect("the asset reads"))
            .to_hex()
            .as_str(),
        file.hash
    );

    store.put(&file.hash, b"rotten on the device");
    server.forget_requests();
    let synced = run(&server, &store).expect("a corrupt stored file is refetched");
    assert_eq!(server.requested_hashes(), std::slice::from_ref(&file.hash));
    assert_eq!(
        blake3::hash(&synced.assets.read(&file.path).expect("the asset reads"))
            .to_hex()
            .as_str(),
        file.hash
    );
    let repaired = block_on(store.read(&file.hash)).expect("stored again");
    assert_eq!(blake3::hash(&repaired).to_hex().as_str(), file.hash);
}

#[test]
fn a_file_gone_from_the_server_restarts_from_the_manifest() {
    let v1 = served(sounding());
    let v2 = served(sounding().part("sfx", "slash", "clip.wav", longer_slash()));
    let changed = v1.manifest.index.hash.clone();
    let server = FakeServer::new(v1);
    *server.deploy_on_404.borrow_mut() = Some((changed, v2));
    let store = MemoryStore::default();

    let synced = run(&server, &store).expect("the sync restarts and finishes");

    assert_eq!(server.manifests_requested(), 2);
    assert_eq!(synced.manifest, server.served.borrow().manifest);
    assert_eq!(synced.assets.len(), synced.index.files.len());
    assert!(server.deploy_on_404.borrow().is_none());
}

#[test]
fn a_deploy_fetches_only_what_changed_and_drops_what_it_replaced() {
    let after = sounding()
        .edit("class", "hero", |row| row["name"] = "Wanderer".into())
        .part("sfx", "slash", "clip.wav", longer_slash());
    let server = FakeServer::new(served(sounding()));
    let store = MemoryStore::default();
    let v1 = run(&server, &store).expect("the first visit syncs");
    let old_sound = v1
        .index
        .files
        .iter()
        .find(|file| file.path == slash_clip())
        .expect("the sound is indexed")
        .clone();

    *server.served.borrow_mut() = served(after);
    server.forget_requests();
    let v2 = run(&server, &store).expect("the deploy syncs");

    let new_sound = v2
        .index
        .files
        .iter()
        .find(|file| file.path == old_sound.path)
        .expect("the sound is still indexed")
        .clone();
    let requested: HashSet<String> = server.requested_hashes().into_iter().collect();
    let expected: HashSet<String> = [
        v2.manifest.index.hash.clone(),
        v2.manifest.rows.hash.clone(),
        new_sound.hash.clone(),
    ]
    .into_iter()
    .collect();
    assert_eq!(requested, expected);
    assert_eq!(
        v2.changed_assets(&v1),
        std::slice::from_ref(&old_sound.path)
    );
    assert!(!v2.manifest.same_content(&v1.manifest));
    let kept: HashSet<String> = store.hashes().into_iter().collect();
    assert!(!kept.contains(&old_sound.hash));
    assert!(!kept.contains(&v1.manifest.rows.hash));
    assert_eq!(kept, expected_hashes(&server.served.borrow(), &v2));
}

#[test]
fn a_store_that_refuses_writes_keeps_the_visit_in_memory() {
    let server = FakeServer::new(served(sounding()));
    let store = MemoryStore::refusing_writes();

    let synced = run(&server, &store).expect("the visit still syncs");

    assert!(synced.memory_only);
    assert_eq!(synced.assets.len(), synced.index.files.len());
    assert!(store.hashes().is_empty());
    assert!(
        synced
            .index
            .files
            .iter()
            .all(|file| synced.assets.read(&file.path).is_some()),
        "every asset reads from memory"
    );
}

#[test]
fn another_build_is_refused_before_anything_downloads() {
    let server = FakeServer::new(served(sounding()));
    let store = MemoryStore::default();

    let refused = block_on(sync(
        &server,
        &store,
        "0badf00d",
        quickly(),
        SyncStatus::default(),
    ));

    assert_eq!(
        refused.err(),
        Some(SyncError::OtherBuild {
            server: version::build().to_owned()
        })
    );
    assert!(server.requested_hashes().is_empty());
    assert!(store.hashes().is_empty());
}

/// A small game with a sound to fetch.
fn sounding() -> Game {
    game().sfx("slash")
}

fn slash_clip() -> String {
    format!("{}/clip.wav", testkit::path("sfx", "slash"))
}

fn longer_slash() -> Vec<u8> {
    let mut slash = wav();
    slash.extend([0; 16]);
    slash
}

fn served(game: Game) -> ServedContent {
    let folder = game.folder();
    let content = game.load();
    ServedContent::new(&content, &folder, version::build(), "Rift").expect("the folder is served")
}

fn quickly() -> SyncPolicy {
    SyncPolicy {
        timeout: Duration::from_secs(5),
        backoff: Backoff {
            first: Seconds(0.001),
            longest: Seconds(0.002),
        },
        ..SyncPolicy::default()
    }
}

fn run(server: &FakeServer, store: &dyn ContentStore) -> Result<SyncedContent, SyncError> {
    block_on(sync(
        server,
        store,
        version::build(),
        quickly(),
        SyncStatus::default(),
    ))
}

fn expected_hashes(served: &ServedContent, index: &SyncedContent) -> HashSet<String> {
    index
        .index
        .files
        .iter()
        .map(|file| file.hash.clone())
        .chain([
            served.manifest.index.hash.clone(),
            served.manifest.rows.hash.clone(),
        ])
        .collect()
}

struct FakeServer {
    served: RefCell<ServedContent>,
    requests: RefCell<Vec<String>>,
    corrupt_once: RefCell<HashSet<String>>,
    deploy_on_404: RefCell<Option<(String, ServedContent)>>,
}

impl FakeServer {
    fn new(served: ServedContent) -> FakeServer {
        FakeServer {
            served: RefCell::new(served),
            requests: RefCell::new(Vec::new()),
            corrupt_once: RefCell::new(HashSet::new()),
            deploy_on_404: RefCell::new(None),
        }
    }

    fn requested_hashes(&self) -> Vec<String> {
        self.requests
            .borrow()
            .iter()
            .filter_map(|path| path.strip_prefix(FILES))
            .map(str::to_owned)
            .collect()
    }

    fn manifests_requested(&self) -> usize {
        self.requests
            .borrow()
            .iter()
            .filter(|path| *path == "/content/manifest")
            .count()
    }

    fn forget_requests(&self) {
        self.requests.borrow_mut().clear();
    }
}

impl ContentSource for FakeServer {
    fn get(&self, path: &str) -> LocalFuture<Result<Vec<u8>, FetchFailure>> {
        self.requests.borrow_mut().push(path.to_owned());
        let result = if path == "/content/manifest" {
            Ok(serde_json::to_vec(&self.served.borrow().manifest).expect("a manifest"))
        } else if let Some(hash) = path.strip_prefix(FILES) {
            let gone = self
                .deploy_on_404
                .borrow()
                .as_ref()
                .is_some_and(|(missing, _)| missing == hash);
            if gone {
                let (_, next) = self.deploy_on_404.borrow_mut().take().expect("a deploy");
                *self.served.borrow_mut() = next;
                Err(FetchFailure::Status {
                    status: 404,
                    body: String::new(),
                })
            } else if self.corrupt_once.borrow_mut().remove(hash) {
                Ok(b"not the file".to_vec())
            } else {
                self.served
                    .borrow()
                    .file(hash)
                    .map(|bytes| bytes.to_vec())
                    .ok_or(FetchFailure::Status {
                        status: 404,
                        body: String::new(),
                    })
            }
        } else {
            Err(FetchFailure::Status {
                status: 400,
                body: String::new(),
            })
        };
        Box::pin(std::future::ready(result))
    }
}
