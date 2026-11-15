//! What a server saves: records in one table, held by whatever its address names.

mod address;

use std::fmt;

pub use address::RecordStoreAddress;
use serde::Serialize;
use serde::de::DeserializeOwned;
use sqlx::any::{AnyPoolOptions, AnyRow};
use sqlx::{AnyPool, Row};

use crate::core::content::{ContentScope, with_content};

/// Something the server saves: one type, written as its JSON under a kind and an id.
pub trait Record: Serialize + DeserializeOwned {
    const KIND: &'static str;

    fn id(&self) -> RecordId;

    /// The account whose sign-in reads it, for the kinds read then.
    fn owner(&self) -> Option<RecordOwner> {
        None
    }
}

/// A record's key within its kind.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RecordId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RecordOwner(pub String);

/// A record as the table holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordRow {
    pub kind: &'static str,
    pub id: RecordId,
    pub owner: Option<RecordOwner>,
    pub body: String,
}

impl RecordRow {
    /// `record` as it is written, the files it names by their paths in `scope`.
    pub fn of<R: Record>(record: &R, scope: &ContentScope) -> RecordRow {
        let (body, _) = with_content(scope.clone(), || serde_json::to_string(record));
        RecordRow {
            kind: R::KIND,
            id: record.id(),
            owner: record.owner(),
            body: body.unwrap_or_else(|error| panic!("a {} record is written: {error}", R::KIND)),
        }
    }
}

/// The record a `body` read from the store holds, the files it names found in `scope`. Refused
/// when the body is not one this build can read whole.
pub fn read<R: Record>(body: &str, scope: &ContentScope) -> Result<R, RecordStoreProblem> {
    let (record, _) = with_content(scope.clone(), || serde_json::from_str::<R>(body));
    record.map_err(|error| RecordStoreProblem(format!("a {} record: {error}", R::KIND)))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordChange {
    Put(RecordRow),
    Remove { kind: &'static str, id: RecordId },
}

/// One table of records. The address picks what holds it, and nothing above the store differs.
#[derive(Clone)]
pub struct RecordStore {
    pool: AnyPool,
}

impl RecordStore {
    /// Connects, and creates the table in a database that has none.
    pub async fn open(address: &RecordStoreAddress) -> Result<RecordStore, RecordStoreProblem> {
        sqlx::any::install_default_drivers();
        let pool = AnyPoolOptions::new()
            .max_connections(CONNECTIONS)
            .connect(address.as_str())
            .await?;
        sqlx::query(CREATE_TABLE).execute(&pool).await?;
        sqlx::query(CREATE_OWNER_INDEX).execute(&pool).await?;
        Ok(RecordStore { pool })
    }

    /// Every change, in one transaction: all of them are written, or none.
    pub async fn write(&self, changes: Vec<RecordChange>) -> Result<(), RecordStoreProblem> {
        let mut transaction = self.pool.begin().await?;
        for change in changes {
            match change {
                RecordChange::Put(row) => {
                    sqlx::query(PUT)
                        .bind(row.kind)
                        .bind(row.id.0)
                        .bind(row.owner.map(|owner| owner.0))
                        .bind(row.body)
                        .execute(&mut *transaction)
                        .await?;
                }
                RecordChange::Remove { kind, id } => {
                    sqlx::query(REMOVE)
                        .bind(kind)
                        .bind(id.0)
                        .execute(&mut *transaction)
                        .await?;
                }
            }
        }
        transaction.commit().await?;
        Ok(())
    }

    /// The body of the record of `kind` under `id`.
    pub async fn one(
        &self,
        kind: &'static str,
        id: &RecordId,
    ) -> Result<Option<String>, RecordStoreProblem> {
        let row = sqlx::query(ONE)
            .bind(kind)
            .bind(id.0.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(body).transpose()
    }

    /// The bodies of every record of `kind` that `owner` owns.
    pub async fn owned(
        &self,
        kind: &'static str,
        owner: &RecordOwner,
    ) -> Result<Vec<String>, RecordStoreProblem> {
        let rows = sqlx::query(OWNED)
            .bind(kind)
            .bind(owner.0.as_str())
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(body).collect()
    }

    /// The bodies of every record of `kind`.
    pub async fn all(&self, kind: &'static str) -> Result<Vec<String>, RecordStoreProblem> {
        let rows = sqlx::query(ALL).bind(kind).fetch_all(&self.pool).await?;
        rows.iter().map(body).collect()
    }
}

/// Why the store could not do what it was asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordStoreProblem(pub String);

impl fmt::Display for RecordStoreProblem {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<sqlx::Error> for RecordStoreProblem {
    fn from(error: sqlx::Error) -> RecordStoreProblem {
        RecordStoreProblem(error.to_string())
    }
}

fn body(row: &AnyRow) -> Result<String, RecordStoreProblem> {
    Ok(row.try_get::<String, _>(0)?)
}

/// The store's task writes one checkpoint at a time, and the rest read for sign-ins.
const CONNECTIONS: u32 = 4;

const CREATE_TABLE: &str = "CREATE TABLE IF NOT EXISTS records (\
    kind TEXT NOT NULL, \
    id TEXT NOT NULL, \
    owner TEXT, \
    body TEXT NOT NULL, \
    PRIMARY KEY (kind, id))";
const CREATE_OWNER_INDEX: &str =
    "CREATE INDEX IF NOT EXISTS records_owner ON records (kind, owner)";
const PUT: &str = "INSERT INTO records (kind, id, owner, body) VALUES ($1, $2, $3, $4) \
    ON CONFLICT (kind, id) DO UPDATE SET owner = excluded.owner, body = excluded.body";
const REMOVE: &str = "DELETE FROM records WHERE kind = $1 AND id = $2";
const ONE: &str = "SELECT body FROM records WHERE kind = $1 AND id = $2";
const OWNED: &str = "SELECT body FROM records WHERE kind = $1 AND owner = $2 ORDER BY id";
const ALL: &str = "SELECT body FROM records WHERE kind = $1 ORDER BY id";
