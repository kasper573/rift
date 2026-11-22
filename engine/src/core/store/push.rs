//! Bringing what a store holds to the shapes this build reads, without migrating it: every record
//! is read as its type, and whatever of it doesn't read is dropped, the least that can be: the
//! field or the list entry that doesn't read, and the whole record only when what it can't be
//! without doesn't read. What still reads is kept as it is, and what was dropped is gone.

use serde_json::Value;
use serde_path_to_error::{Path, Segment};

use super::{Record, RecordChange, RecordId, RecordRow, RecordStore, RecordStoreProblem};
use crate::core::content::{ContentScope, with_content};

/// A kind of record as a push reads it: as the server reads it, in the scope it reads it in.
pub struct PushedKind {
    pub kind: &'static str,
    scope: ContentScope,
    push: fn(&str, &ContentScope) -> PushedRecord,
}

impl PushedKind {
    /// Records of `R`, read in `scope` as the server reads them.
    pub fn of<R: Record>(scope: ContentScope) -> PushedKind {
        PushedKind {
            kind: R::KIND,
            scope,
            push: push_record::<R>,
        }
    }
}

/// What a push makes of one record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PushedRecord {
    /// It reads whole, and stays as it is.
    Whole,
    /// It reads once what is listed is dropped, and is written back without it.
    Kept {
        row: RecordRow,
        dropped: Vec<Dropped>,
    },
    /// What it can't be without doesn't read, and the record goes.
    Gone(Dropped),
}

/// Something of a record a push drops: where in the record it was, and why it doesn't read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dropped {
    pub at: String,
    pub why: String,
}

/// One record a push changes: its kind and id, and what the push makes of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushedChange {
    pub kind: &'static str,
    pub id: RecordId,
    pub pushed: PushedRecord,
}

/// What pushing `kinds` makes of every record of them `store` holds: the records it changes, and
/// how many it left as they were.
pub async fn plan(
    store: &RecordStore,
    kinds: &[PushedKind],
) -> Result<(Vec<PushedChange>, usize), RecordStoreProblem> {
    let mut changes = Vec::new();
    let mut whole = 0;
    for kind in kinds {
        for (id, body) in store.rows(kind.kind).await? {
            match (kind.push)(&body, &kind.scope) {
                PushedRecord::Whole => whole += 1,
                pushed => changes.push(PushedChange {
                    kind: kind.kind,
                    id,
                    pushed,
                }),
            }
        }
    }
    Ok((changes, whole))
}

/// Writes what `changes` make of their records, in one transaction.
pub async fn apply(
    store: &RecordStore,
    changes: &[PushedChange],
) -> Result<(), RecordStoreProblem> {
    let writes = changes
        .iter()
        .filter_map(|change| match &change.pushed {
            PushedRecord::Whole => None,
            PushedRecord::Kept { row, .. } => Some(RecordChange::Put(row.clone())),
            PushedRecord::Gone(_) => Some(RecordChange::Remove {
                kind: change.kind,
                id: change.id.clone(),
            }),
        })
        .collect();
    store.write(writes).await
}

/// What a push makes of the record `body` holds, read as `R` in `scope`: each time it doesn't
/// read, what the failure points at is dropped, until it reads or the record itself has to go.
fn push_record<R: Record>(body: &str, scope: &ContentScope) -> PushedRecord {
    let mut held: Value = match serde_json::from_str(body) {
        Ok(held) => held,
        Err(error) => {
            return PushedRecord::Gone(Dropped {
                at: String::new(),
                why: format!("not JSON: {error}"),
            });
        }
    };
    let mut dropped = Vec::new();
    loop {
        let (read, _) = with_content(scope.clone(), || {
            serde_path_to_error::deserialize::<_, R>(&held)
        });
        let error = match read {
            Ok(_) if dropped.is_empty() => return PushedRecord::Whole,
            Ok(record) => {
                return PushedRecord::Kept {
                    row: RecordRow::of(&record, scope),
                    dropped,
                };
            }
            Err(error) => error,
        };
        let failed = Dropped {
            at: error.path().to_string(),
            why: error.inner().to_string(),
        };
        if !remove(&mut held, error.path()) {
            return PushedRecord::Gone(failed);
        }
        dropped.push(failed);
    }
}

/// Removes what `path` points at from `held`, or the nearest of what holds it that can go on its
/// own: a field of an object or an entry of a list. Fails when only the whole of `held` can go.
fn remove(held: &mut Value, path: &Path) -> bool {
    let segments: Vec<&Segment> = path
        .iter()
        .take_while(|segment| matches!(segment, Segment::Seq { .. } | Segment::Map { .. }))
        .collect();
    let Some((last, holders)) = segments.split_last() else {
        return false;
    };
    let mut holder = held;
    for segment in holders {
        let inner = match segment {
            Segment::Seq { index } => holder.get_mut(*index),
            Segment::Map { key } => holder.get_mut(key.as_str()),
            Segment::Enum { .. } | Segment::Unknown => None,
        };
        let Some(inner) = inner else {
            return false;
        };
        holder = inner;
    }
    match (last, holder) {
        (Segment::Seq { index }, Value::Array(entries)) if *index < entries.len() => {
            entries.remove(*index);
            true
        }
        (Segment::Map { key }, Value::Object(fields)) => fields.remove(key).is_some(),
        _ => false,
    }
}
