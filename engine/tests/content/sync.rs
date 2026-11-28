use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bevy::asset::AssetPlugin;
use bevy::gltf::{Gltf, GltfMaterial, GltfPlugin};
use bevy::prelude::*;
use bevy::tasks::futures_lite::future::block_on;
use bevy::world_serialization::WorldSerializationPlugin;
use engine::core::assets::loader::PngLoader;
use engine::core::assets::{self, SyncedAssetReader};
use engine::core::content::FileHash;
use engine::core::content::store::ContentStore;
use engine::core::content::sync::{
    ContentSource, FILES, MANIFEST, ServedContent, SyncError, SyncPolicy, SyncStatus,
    SyncedContent, is_content_of, sync,
};
use engine::core::content::version;
use engine::core::os::FetchFailure;
use engine::core::time::Backoff;
use engine::core::units::Seconds;
use engine::systems::messages::{self, Messages};
use futures_util::future::BoxFuture;
use serde_json::json;
use testkit::{Game, Map, glb_naming_image, png, wav};

use crate::store::TestStore;
use crate::support::game;

/// The assets are read from the store as they are asked for, so one the store loses after the
/// sync reads as missing, and reads again once it is back.
#[test]
fn a_first_visit_fetches_everything_content_names_keeping_only_it_and_a_later_visit_only_the_manifest()
 {
    let server = FakeServer::new(served(
        sounding()
            .file("downloads/Unused Clip.glb", testkit::glb(&[]))
            .file("downloads/manual.pdf", "a manual"),
    ));
    let store = TestStore::default();
    let stale = "0".repeat(64);
    store.put(&stale, b"left over from another game");

    let synced = run(&server, &store).expect("the first visit syncs");

    let every_file = expected_hashes(&server.served.lock().unwrap(), &synced);
    let requested: HashSet<String> = server.requested_hashes().into_iter().collect();
    assert_eq!(requested, every_file);
    assert_eq!(synced.assets.len(), synced.index.files.len());
    let fetched: HashSet<&str> = synced
        .index
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    assert!(fetched.contains(slash_clip().as_str()), "{fetched:?}");
    assert!(
        fetched.iter().all(|path| !path.starts_with("downloads/")),
        "media nothing names is never fetched: {fetched:?}"
    );
    assert!(!synced.rows.is_empty());
    assert!(!synced.memory_only);
    let kept: HashSet<String> = store.hashes().into_iter().collect();
    assert_eq!(kept, every_file);

    server.forget_requests();
    let again = run(&server, &store).expect("the next visit syncs");
    assert_eq!(server.requests.lock().unwrap().as_slice(), [MANIFEST]);
    assert!(again.manifest.same_content(&synced.manifest));
    assert_eq!(again.assets.len(), synced.assets.len());

    let file = &again.index.files[0];
    let bytes = again.assets.read(&file.path).expect("the asset reads");
    store.drop_entry(&file.held.hash);
    assert!(
        again.assets.read(&file.path).is_none(),
        "lost from the store"
    );
    store.put(&file.held.hash, &bytes);
    assert_eq!(again.assets.read(&file.path).as_deref(), Some(&bytes[..]));
}

#[test]
fn a_store_lacking_files_or_holding_rotten_ones_is_made_whole_by_fetching_exactly_those() {
    {
        let server = FakeServer::new(served(sounding()));
        let complete = TestStore::default();
        run(&server, &complete).expect("a full sync");
        let every_file = complete.hashes();
        let (kept, lacked) = every_file.split_at(every_file.len() / 2);
        let partial = TestStore::default();
        for hash in kept {
            let bytes = complete.read(hash).expect("a kept file");
            partial.put(hash, &bytes);
        }
        server.forget_requests();

        run(&server, &partial).expect("the sync resumes");

        let requested: HashSet<String> = server.requested_hashes().into_iter().collect();
        let lacked: HashSet<String> = lacked.iter().cloned().collect();
        assert_eq!(requested, lacked);
        assert_eq!(partial.hashes(), every_file);
    }
    {
        let server = FakeServer::new(served(sounding()));
        let store = TestStore::default();
        let first = run(&server, &store).expect("a full sync");
        let file = first
            .index
            .files
            .iter()
            .find(|file| file.path == slash_clip())
            .expect("the sound is indexed")
            .clone();

        server
            .corrupt_once
            .lock()
            .unwrap()
            .insert(file.held.hash.clone());
        let fresh = TestStore::default();
        server.forget_requests();
        let synced = run(&server, &fresh).expect("a corrupt download is retried");
        let fetched_twice = server
            .requested_hashes()
            .iter()
            .filter(|hash| **hash == file.held.hash)
            .count();
        assert_eq!(fetched_twice, 2);
        assert_eq!(
            blake3::hash(&synced.assets.read(&file.path).expect("the asset reads"))
                .to_hex()
                .as_str(),
            file.held.hash
        );

        store.put(&file.held.hash, b"rotten on the device");
        server.forget_requests();
        let synced = run(&server, &store).expect("a corrupt stored file is refetched");
        assert_eq!(
            server.requested_hashes(),
            std::slice::from_ref(&file.held.hash)
        );
        assert_eq!(
            blake3::hash(&synced.assets.read(&file.path).expect("the asset reads"))
                .to_hex()
                .as_str(),
            file.held.hash
        );
        let repaired = store.read(&file.held.hash).expect("stored again");
        assert_eq!(blake3::hash(&repaired).to_hex().as_str(), file.held.hash);
    }
}

#[test]
fn a_file_gone_from_the_server_restarts_from_the_manifest() {
    let v1 = served(sounding());
    let v2 = served(sounding().part("sfx", "slash", "clip.wav", longer_slash()));
    let changed = v1.manifest.index.hash.clone();
    let server = FakeServer::new(v1);
    *server.deploy_on_404.lock().unwrap() = Some((changed, v2));
    let store = TestStore::default();

    let synced = run(&server, &store).expect("the sync restarts and finishes");

    assert_eq!(server.manifests_requested(), 2);
    assert_eq!(synced.manifest, server.served.lock().unwrap().manifest);
    assert_eq!(synced.assets.len(), synced.index.files.len());
    assert!(server.deploy_on_404.lock().unwrap().is_none());
}

#[test]
fn a_deploy_fetches_only_what_changed_and_drops_what_it_replaced() {
    let after = sounding()
        .edit("class", "hero", |row| row["name"] = "Wanderer".into())
        .part("sfx", "slash", "clip.wav", longer_slash());
    let server = FakeServer::new(served(sounding()));
    let store = TestStore::default();
    let v1 = run(&server, &store).expect("the first visit syncs");
    let old_sound = v1
        .index
        .files
        .iter()
        .find(|file| file.path == slash_clip())
        .expect("the sound is indexed")
        .clone();

    *server.served.lock().unwrap() = served(after);
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
        new_sound.held.hash.clone(),
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
    assert!(!kept.contains(&old_sound.held.hash));
    assert!(!kept.contains(&v1.manifest.rows.hash));
    assert_eq!(kept, expected_hashes(&server.served.lock().unwrap(), &v2));
}

#[test]
fn a_store_that_refuses_writes_keeps_the_visit_in_memory() {
    let server = FakeServer::new(served(sounding()));
    let store = TestStore::refusing_writes();

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
    let store = TestStore::default();

    let refused = block_on(sync(
        server.clone(),
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

/// The rows arrive ahead of the media, so a first visit already has the game's words to tell how
/// the media's download goes.
#[test]
fn a_first_visit_reads_the_games_words_before_its_media_downloads() {
    let server = FakeServer::new(served(sounding()));
    let status = SyncStatus::default();
    let heard: Arc<Mutex<Option<Messages>>> = Arc::default();
    *server.before_media.lock().unwrap() = Some(Box::new({
        let status = status.clone();
        let heard = heard.clone();
        move || {
            let rows = status.rows().expect("the rows arrived before the media");
            *heard.lock().unwrap() = messages::read_alone(rows);
        }
    }));

    block_on(sync(
        server.clone(),
        &TestStore::default(),
        version::build(),
        quickly(),
        status,
    ))
    .expect("it syncs");

    let content = sounding().load();
    assert_eq!(
        heard.lock().unwrap().as_ref(),
        Some(messages::of(&content)),
        "the words read early are the game's"
    );
}

/// A streamed part, as the tiles of an area's ground are, is left out of the sync and fetched as it
/// is first read, then kept in the store like the rest: the visit after keeps it without a fetch.
#[test]
fn a_streamed_part_is_fetched_as_it_is_first_read_and_kept() {
    let rows = [
        "........", "........", "........", "...#....", "........", "........", "........",
        "........",
    ];
    let server = FakeServer::new(served(sounding().area("cove", &Map::of(&rows), json!([]))));
    let store = TestStore::default();
    let synced = run(&server, &store).expect("it syncs");
    let indexed = |ending: &str| {
        synced
            .index
            .files
            .iter()
            .find(|file| file.path.ends_with(ending))
            .unwrap_or_else(|| panic!("{ending} is indexed"))
            .clone()
    };
    let (tile, overview) = (indexed("paint/0_0.png"), indexed("paint_overview.png"));
    assert!(tile.streamed && !overview.streamed);
    let requested = server.requested_hashes();
    assert!(requested.contains(&overview.held.hash) && !requested.contains(&tile.held.hash));
    assert!(!store.hashes().contains(&tile.held.hash));

    server.forget_requests();
    let bytes = synced.assets.read(&tile.path).expect("the tile reads");
    assert_eq!(FileHash::of(&bytes).hash, tile.held.hash);
    assert_eq!(
        server.requested_hashes(),
        std::slice::from_ref(&tile.held.hash)
    );
    assert!(store.hashes().contains(&tile.held.hash));
    server.forget_requests();
    assert!(synced.assets.read(&tile.path).is_some());
    assert!(server.requested_hashes().is_empty(), "kept once fetched");

    run(&server, &store).expect("the next visit syncs");
    assert!(store.hashes().contains(&tile.held.hash), "a visit keeps it");
}

#[test]
fn whoever_holds_a_game_folder_knows_whether_a_server_runs_that_game() {
    let manifest = served(sounding()).manifest;
    let holds = |game: Game| {
        let folder = game.folder();
        let registry = game.registry();
        let content = game.load();
        is_content_of(&manifest, registry, &content, &folder).expect("the folder reads")
    };

    assert!(holds(sounding()));
    assert!(
        !holds(sounding().edit("class", "hero", |row| row["name"] = "Wanderer".into())),
        "a row that differs is another game"
    );
    assert!(
        !holds(sounding().part("sfx", "slash", "clip.wav", longer_slash())),
        "media that differs is another game"
    );
}

/// Every prop bundle keeps its own copy of the images its model names, and models naming copies
/// of the same bytes draw them from one texture: the copies load once, and an image of other
/// bytes loads apart.
#[test]
fn models_naming_copies_of_one_image_draw_it_from_one_texture() {
    let bark = png(4, 4);
    let prop = |game: Game, name: &str, image: &str, bytes: &[u8]| {
        game.prop(name, json!({ "type": "none" }))
            .part("prop", name, "model.glb", glb_naming_image(image))
            .part("prop", name, image, bytes.to_vec())
    };
    let game = prop(game(), "oak", "textures/bark.png", &bark);
    let game = prop(game, "elm", "textures/elm-bark.png", &bark);
    let game = prop(game, "ash", "textures/bark.png", &png(8, 8));
    let store = TestStore::default();
    let synced = run(&FakeServer::new(served(game)), &store).expect("it syncs");

    let mut app = App::new();
    let files = SyncedAssetReader::default();
    files.replace(synced.assets);
    files.register(&mut app);
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        WorldSerializationPlugin,
        bevy::mesh::MeshPlugin,
        GltfPlugin::default(),
    ))
    .init_asset::<Image>()
    .register_asset_loader(PngLoader);
    app.finish();
    app.cleanup();
    let model = |name: &str| format!("{}/model.glb", testkit::path("prop", name));
    let assets = app.world().resource::<AssetServer>();
    let models: Vec<Handle<Gltf>> = ["oak", "elm", "ash"]
        .into_iter()
        .map(|name| assets::model(assets, model(name)))
        .collect();
    testkit::until(&mut app, |app| {
        let assets = app.world().resource::<AssetServer>();
        models.iter().all(|model| {
            let state = assets.recursive_dependency_load_state(model);
            assert!(!state.is_failed(), "a model fails to load: {state:?}");
            state.is_loaded()
        })
    });
    let world = app.world();
    let textures: Vec<AssetId<Image>> = models
        .iter()
        .map(|model| {
            let model = world
                .resource::<Assets<Gltf>>()
                .get(model)
                .expect("a loaded model");
            let material = world
                .resource::<Assets<GltfMaterial>>()
                .get(&model.materials[0])
                .expect("the model's material");
            material
                .base_color_texture
                .as_ref()
                .expect("a coloured material")
                .id()
        })
        .collect();
    assert_eq!(
        textures[0], textures[1],
        "copies of one image are one texture"
    );
    assert_ne!(
        textures[0], textures[2],
        "an image of other bytes is a texture of its own"
    );
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
    let registry = game.registry();
    let content = game.load();
    ServedContent::new(registry, &content, &folder, version::build(), "Rift")
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

fn run(server: &Arc<FakeServer>, store: &ContentStore) -> Result<SyncedContent, SyncError> {
    block_on(sync(
        server.clone(),
        store,
        version::build(),
        quickly(),
        SyncStatus::default(),
    ))
}

/// Every file a visit fetches and keeps: all the index names but the streamed parts, which it
/// fetches only as they are read.
fn expected_hashes(served: &ServedContent, index: &SyncedContent) -> HashSet<String> {
    index
        .index
        .files
        .iter()
        .filter(|file| !file.streamed)
        .map(|file| file.held.hash.clone())
        .chain([
            served.manifest.index.hash.clone(),
            served.manifest.rows.hash.clone(),
        ])
        .collect()
}

struct FakeServer {
    served: Mutex<ServedContent>,
    requests: Mutex<Vec<String>>,
    corrupt_once: Mutex<HashSet<String>>,
    deploy_on_404: Mutex<Option<(String, ServedContent)>>,
    before_media: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl FakeServer {
    fn new(served: ServedContent) -> Arc<FakeServer> {
        Arc::new(FakeServer {
            served: Mutex::new(served),
            requests: Mutex::new(Vec::new()),
            corrupt_once: Mutex::new(HashSet::new()),
            deploy_on_404: Mutex::new(None),
            before_media: Mutex::new(None),
        })
    }

    fn requested_hashes(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter_map(|path| path.strip_prefix(FILES))
            .map(str::to_owned)
            .collect()
    }

    fn manifests_requested(&self) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|path| *path == MANIFEST)
            .count()
    }

    fn forget_requests(&self) {
        self.requests.lock().unwrap().clear();
    }
}

impl ContentSource for FakeServer {
    fn get(&self, path: &str) -> BoxFuture<'static, Result<Vec<u8>, FetchFailure>> {
        self.requests.lock().unwrap().push(path.to_owned());
        let media = path.strip_prefix(FILES).is_some_and(|hash| {
            let manifest = &self.served.lock().unwrap().manifest;
            hash != manifest.index.hash && hash != manifest.rows.hash
        });
        if media && let Some(before) = self.before_media.lock().unwrap().take() {
            before();
        }
        let result = if path == MANIFEST {
            Ok(serde_json::to_vec(&self.served.lock().unwrap().manifest).expect("a manifest"))
        } else if let Some(hash) = path.strip_prefix(FILES) {
            let gone = self
                .deploy_on_404
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|(missing, _)| missing == hash);
            if gone {
                let (_, next) = self.deploy_on_404.lock().unwrap().take().expect("a deploy");
                *self.served.lock().unwrap() = next;
                Err(FetchFailure::Status {
                    status: 404,
                    body: String::new(),
                })
            } else if self.corrupt_once.lock().unwrap().remove(hash) {
                Ok(b"not the file".to_vec())
            } else {
                self.served
                    .lock()
                    .unwrap()
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
