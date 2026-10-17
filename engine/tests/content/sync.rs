use std::cell::RefCell;
use std::collections::HashSet;
use std::time::Duration;

use bevy::tasks::futures_lite::future::block_on;
use engine::core::content::FilesystemFolder;
use engine::core::content::store::ContentStore;
use engine::core::content::sync::{
    ContentSource, ServedContent, SyncError, SyncPolicy, SyncStatus, SyncedContent, sync,
};
use engine::core::content::version;
use engine::core::platform::{FetchFailure, LocalFuture};
use engine::core::time::{Backoff, Seconds};

use crate::store::MemoryStore;
use crate::support::{TempFolder, load};

const FILES: &str = "/content/files/";

#[test]
fn a_first_visit_fetches_everything_and_the_next_day_only_the_manifest() {
    let folder = TempFolder::of_template();
    let server = FakeServer::new(served(&folder));
    let store = MemoryStore::default();

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
    assert!(again.same_as(&synced));
    assert_eq!(again.assets.len(), synced.assets.len());

    server.forget_requests();
    let evicted = MemoryStore::default();
    run(&server, &evicted).expect("an evicted store is rebuilt");
    let requested: HashSet<String> = server.requested_hashes().into_iter().collect();
    assert_eq!(requested, every_file);
}

#[test]
fn an_interrupted_sync_fetches_exactly_the_files_the_store_lacks() {
    let folder = TempFolder::of_template();
    let server = FakeServer::new(served(&folder));
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
    let folder = TempFolder::of_template();
    let server = FakeServer::new(served(&folder));
    let store = MemoryStore::default();
    let first = run(&server, &store).expect("a full sync");
    let file = first.index.files[0].clone();

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
        blake3::hash(&synced.assets[&file.path]).to_hex().as_str(),
        file.hash
    );

    store.put(&file.hash, b"rotten on the device");
    server.forget_requests();
    let synced = run(&server, &store).expect("a corrupt stored file is refetched");
    assert_eq!(server.requested_hashes(), std::slice::from_ref(&file.hash));
    assert_eq!(
        blake3::hash(&synced.assets[&file.path]).to_hex().as_str(),
        file.hash
    );
    let repaired = block_on(store.read(&file.hash)).expect("stored again");
    assert_eq!(blake3::hash(&repaired).to_hex().as_str(), file.hash);
}

#[test]
fn a_file_gone_from_the_server_restarts_from_the_manifest() {
    let before = TempFolder::of_template();
    let after = TempFolder::of_template();
    after.append("assets/audio/combat/slash03.wav", &[0; 16]);
    let v1 = served(&before);
    let v2 = served(&after);
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
    let before = TempFolder::of_template();
    let after = TempFolder::of_template();
    after.edit("tables/class/adventurer.json", |row| {
        row["name"] = serde_json::Value::String("Wanderer".to_owned());
    });
    after.append("assets/audio/combat/slash03.wav", &[0; 16]);
    let server = FakeServer::new(served(&before));
    let store = MemoryStore::default();
    let v1 = run(&server, &store).expect("the first visit syncs");
    let old_sound = v1
        .index
        .files
        .iter()
        .find(|file| file.path == "assets/audio/combat/slash03.wav")
        .expect("the sound is indexed")
        .clone();

    *server.served.borrow_mut() = served(&after);
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
    assert!(!v2.same_as(&v1));
    let kept: HashSet<String> = store.hashes().into_iter().collect();
    assert!(!kept.contains(&old_sound.hash));
    assert!(!kept.contains(&v1.manifest.rows.hash));
    assert_eq!(kept, expected_hashes(&server.served.borrow(), &v2));
}

#[test]
fn a_store_that_refuses_writes_keeps_the_visit_in_memory() {
    let folder = TempFolder::of_template();
    let server = FakeServer::new(served(&folder));
    let store = MemoryStore::refusing_writes();

    let synced = run(&server, &store).expect("the visit still syncs");

    assert!(synced.memory_only);
    assert_eq!(synced.assets.len(), synced.index.files.len());
    assert!(store.hashes().is_empty());
}

#[test]
fn stale_entries_leave_the_store_after_a_full_sync() {
    let folder = TempFolder::of_template();
    let server = FakeServer::new(served(&folder));
    let store = MemoryStore::default();
    let stale = "0".repeat(64);
    store.put(&stale, b"left over from another game");

    let synced = run(&server, &store).expect("the sync finishes");

    assert!(!store.hashes().contains(&stale));
    let kept: HashSet<String> = store.hashes().into_iter().collect();
    assert_eq!(kept, expected_hashes(&server.served.borrow(), &synced));
}

#[test]
fn another_build_is_refused_before_anything_downloads() {
    let folder = TempFolder::of_template();
    let server = FakeServer::new(served(&folder));
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

fn served(folder: &TempFolder) -> ServedContent {
    let content = load(folder.path()).unwrap_or_else(|report| panic!("{report}"));
    ServedContent::new(
        &content,
        &FilesystemFolder(folder.path().to_path_buf()),
        version::build(),
        "Rift",
    )
    .expect("the folder is served")
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
