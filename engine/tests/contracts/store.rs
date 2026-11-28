use engine::core::content::{ContentScope, SavedList};
use engine::core::store::push::{self, PushedKind, PushedRecord};
use engine::core::store::{
    self, Record, RecordChange, RecordId, RecordOwner, RecordRow, RecordStore, RecordStoreAddress,
};
use serde::{Deserialize, Serialize};

/// Where a test run that brings a database server of its own has the store's cases run again.
const SERVER_ADDRESS: &str = "RIFT_TEST_RECORD_STORE";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
struct Letter {
    id: String,
    to: String,
    text: String,
}

impl Record for Letter {
    const KIND: &'static str = "test_letter";

    fn id(&self) -> RecordId {
        RecordId(self.id.clone())
    }

    fn owner(&self) -> Option<RecordOwner> {
        Some(RecordOwner(self.to.clone()))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Note {
    id: String,
    text: String,
}

impl Record for Note {
    const KIND: &'static str = "test_note";

    fn id(&self) -> RecordId {
        RecordId(self.id.clone())
    }
}

fn letter(id: &str, to: &str, text: &str) -> Letter {
    Letter {
        id: id.to_owned(),
        to: to.to_owned(),
        text: text.to_owned(),
    }
}

fn put<R: Record>(record: &R) -> RecordChange {
    RecordChange::Put(RecordRow::of(record, &ContentScope::new()))
}

fn remove<R: Record>(record: &R) -> RecordChange {
    RecordChange::Remove {
        kind: R::KIND,
        id: record.id(),
    }
}

fn read_one<R: Record>(body: Option<String>) -> Option<R> {
    read_all(body.into_iter().collect()).pop()
}

fn read_all<R: Record>(bodies: Vec<String>) -> Vec<R> {
    bodies
        .iter()
        .map(|body| store::read(body, &ContentScope::new()).expect("a record this build wrote"))
        .collect()
}

/// The store on the database server the run names. One server opens a store once, so the tests,
/// which all share the server's one table, open it one at a time.
async fn server() -> Option<RecordStore> {
    static OPENING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let address = std::env::var(SERVER_ADDRESS).ok()?;
    let _one_at_a_time = OPENING.lock().await;
    Some(
        RecordStore::open(&RecordStoreAddress::new(address))
            .await
            .expect("the database server opens"),
    )
}

/// Runs `case` on a store in a file of its own, and again on the database server the run names.
fn on_every_store<F, Fut>(case: F)
where
    F: Fn(RecordStore, &'static str) -> Fut,
    Fut: Future<Output = ()>,
{
    let runtime = tokio::runtime::Runtime::new().expect("a runtime");
    let folder = tempfile::tempdir().expect("a folder for the store");
    let file = RecordStoreAddress::file(&folder.path().join("records.sqlite"));
    runtime.block_on(async {
        case(
            RecordStore::open(&file).await.expect("a file opens"),
            "file",
        )
        .await;
        if let Some(server) = server().await {
            case(server, "server").await;
        }
    });
}

#[test]
fn records_put_are_read_back_until_removed_each_kind_keeping_its_own_ids() {
    on_every_store(async |store, on| {
        let first = letter("put-1", "put-wren", "Come to the lake.");
        let second = letter("put-2", "put-wren", "Bring a rod.");
        let other = letter("put-3", "put-hale", "It isn't \"here\".\nIt's there: 'ö'.");
        store
            .write(vec![put(&first), put(&second), put(&other)])
            .await
            .expect("a write");

        let owner = RecordOwner("put-wren".to_owned());
        let one = store.one(Letter::KIND, &other.id()).await.expect("a read");
        assert_eq!(read_one::<Letter>(one), Some(other.clone()), "{on}");
        let owned = store.owned(Letter::KIND, &owner).await.expect("a read");
        assert_eq!(
            read_all::<Letter>(owned),
            [first.clone(), second.clone()],
            "{on}"
        );

        let rewritten = letter("put-1", "put-hale", "Never mind.");
        store
            .write(vec![put(&rewritten), remove(&second)])
            .await
            .expect("a write");
        let owned = store.owned(Letter::KIND, &owner).await.expect("a read");
        assert!(
            owned.is_empty(),
            "{on}: a record put again has its new owner"
        );
        let gone = store.one(Letter::KIND, &second.id()).await.expect("a read");
        assert_eq!(gone, None, "{on}");
        let all: Vec<Letter> = read_all(store.all(Letter::KIND).await.expect("a read"));
        let kept: Vec<Letter> = all
            .into_iter()
            .filter(|letter| letter.id.starts_with("put-"))
            .collect();
        assert_eq!(kept, [rewritten.clone(), other.clone()], "{on}");

        store
            .write(vec![remove(&rewritten), remove(&other)])
            .await
            .expect("a write");

        let note = Note {
            id: "kinds-1".to_owned(),
            text: "A note.".to_owned(),
        };
        let same_id = letter("kinds-1", "kinds-wren", "A letter.");
        store
            .write(vec![put(&note), put(&same_id)])
            .await
            .expect("a write");
        store.write(vec![remove(&same_id)]).await.expect("a write");
        let kept = store.one(Note::KIND, &note.id()).await.expect("a read");
        assert_eq!(read_one::<Note>(kept), Some(note.clone()), "{on}");
        store.write(vec![remove(&note)]).await.expect("a write");
    });
}

#[test]
fn a_write_that_fails_leaves_nothing() {
    let runtime = tokio::runtime::Runtime::new().expect("a runtime");
    let folder = tempfile::tempdir().expect("a folder for the store");
    let file = RecordStoreAddress::file(&folder.path().join("records.sqlite"));
    runtime.block_on(async {
        let store = RecordStore::open(&file).await.expect("a file opens");
        // A file takes any text, so the test has it refuse one id.
        let pool = sqlx::AnyPool::connect(file.as_str())
            .await
            .expect("the file opens again");
        sqlx::query(
            "CREATE TRIGGER refuse BEFORE INSERT ON records WHEN NEW.id = 'whole-refused' \
             BEGIN SELECT RAISE(ABORT, 'refused'); END",
        )
        .execute(&pool)
        .await
        .expect("a trigger");
        fails_whole(store, "{}", "file").await;

        if let Some(server) = server().await {
            // No database server stores the byte that ends a string.
            fails_whole(server, "{\"text\":\"\0\"}", "server").await;
        }
    });
}

async fn fails_whole(store: RecordStore, refused_body: &str, on: &str) {
    let kept = letter("whole-kept", "whole-wren", "Written before.");
    store.write(vec![put(&kept)]).await.expect("a write");

    let lost = letter("whole-lost", "whole-wren", "Written with what fails.");
    let refused = RecordChange::Put(RecordRow {
        kind: Letter::KIND,
        id: RecordId("whole-refused".to_owned()),
        owner: None,
        body: refused_body.to_owned(),
    });
    let failed = store.write(vec![put(&lost), remove(&kept), refused]).await;
    assert!(failed.is_err(), "{on}: a change the store refuses");

    let lost = store.one(Letter::KIND, &lost.id()).await.expect("a read");
    assert_eq!(lost, None, "{on}: what was put with it is not there");
    let still = store.one(Letter::KIND, &kept.id()).await.expect("a read");
    assert_eq!(
        read_one::<Letter>(still),
        Some(kept.clone()),
        "{on}: what was removed with it is"
    );
    store.write(vec![remove(&kept)]).await.expect("a write");
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
struct Satchel {
    id: String,
    owner: String,
    #[serde(default)]
    items: SavedList<Pocket>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
struct Pocket {
    slot: u32,
    item: String,
}

impl Record for Satchel {
    const KIND: &'static str = "test_satchel";

    fn id(&self) -> RecordId {
        RecordId(self.id.clone())
    }
}

/// A push brings what a store holds to the shapes this build reads: a record that reads whole is
/// left alone; one an earlier build wrote loses only what doesn't read, a field this build has no
/// more and an entry of a list without what this build needs of it; and one without what it can't
/// be without goes. Nothing is written until the push is applied, and after it everything reads.
#[test]
fn a_push_keeps_what_still_reads_and_drops_the_rest() {
    on_every_store(async |store, on| {
        let raw = |id: &str, body: &str| {
            RecordChange::Put(RecordRow {
                kind: Satchel::KIND,
                id: RecordId(id.to_owned()),
                owner: None,
                body: body.to_owned(),
            })
        };
        store
            .write(vec![
                raw(
                    "push-whole",
                    r#"{"id":"push-whole","owner":"wren","items":[{"slot":0,"item":"rod"}]}"#,
                ),
                raw(
                    "push-older",
                    r#"{"id":"push-older","owner":"hale","max":20,"items":[{"item":"net"},{"slot":3,"item":"hook"}]}"#,
                ),
                raw("push-broken", r#"{"id":"push-broken","items":[]}"#),
            ])
            .await
            .expect("a write");
        let kinds = [PushedKind::of::<Satchel>(
            ContentScope::new().setting_aside(),
        )];

        let (changes, whole) = push::plan(&store, &kinds).await.expect("a plan");
        assert_eq!(whole, 1, "{on}: the satchel that reads whole is left alone");
        let older = changes
            .iter()
            .find(|change| change.id.0 == "push-older")
            .expect("the older satchel changes");
        let PushedRecord::Kept { dropped, .. } = &older.pushed else {
            panic!("{on}: the older satchel is kept: {:?}", older.pushed);
        };
        let at: Vec<&str> = dropped.iter().map(|dropped| dropped.at.as_str()).collect();
        assert_eq!(
            at,
            ["max", "items[0]"],
            "{on}: it loses the field and the entry that don't read, nothing more"
        );
        let broken = changes
            .iter()
            .find(|change| change.id.0 == "push-broken")
            .expect("the broken satchel changes");
        assert!(
            matches!(broken.pushed, PushedRecord::Gone(_)),
            "{on}: a satchel without its owner goes: {:?}",
            broken.pushed
        );
        let untouched = store
            .one(Satchel::KIND, &RecordId("push-older".to_owned()))
            .await
            .expect("a read");
        assert!(
            untouched.is_some_and(|body| body.contains("max")),
            "{on}: planning writes nothing"
        );

        push::apply(&store, &changes)
            .await
            .expect("the push is written");
        let kept = store
            .one(Satchel::KIND, &RecordId("push-older".to_owned()))
            .await
            .expect("a read");
        assert_eq!(
            read_one::<Satchel>(kept),
            Some(Satchel {
                id: "push-older".to_owned(),
                owner: "hale".to_owned(),
                items: SavedList::from(vec![Pocket {
                    slot: 3,
                    item: "hook".to_owned(),
                }]),
            }),
            "{on}"
        );
        let gone = store
            .one(Satchel::KIND, &RecordId("push-broken".to_owned()))
            .await
            .expect("a read");
        assert_eq!(gone, None, "{on}");
        let (again, _) = push::plan(&store, &kinds).await.expect("a plan");
        assert!(again.is_empty(), "{on}: after a push everything reads");

        store
            .write(
                ["push-whole", "push-older"]
                    .map(|id| RecordChange::Remove {
                        kind: Satchel::KIND,
                        id: RecordId(id.to_owned()),
                    })
                    .into(),
            )
            .await
            .expect("a write");
    });
}
