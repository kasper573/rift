use std::collections::BTreeMap;
use std::sync::Arc;

use crate::core::content::ContentScope;
use crate::core::store::{Record, RecordChange, RecordId, RecordRow};

/// Every record changed since it was last written, by kind and id: the record as it now is, or
/// that it is gone. Until a checkpoint has written a record, this is where its newest copy is.
#[derive(Default)]
pub struct Unwritten {
    changes: BTreeMap<RecordKey, Pending>,
}

/// A record's kind and its id within the kind.
pub type RecordKey = (&'static str, RecordId);

/// What a checkpoint is to write of a record.
#[derive(Clone)]
pub enum Pending {
    Put(Arc<dyn Writable>),
    Removed,
}

/// A record as a checkpoint takes it: written out off the tick, by whoever writes it.
pub trait Writable: Send + Sync {
    fn row(&self, scope: &ContentScope) -> RecordRow;
}

impl<R: Record + Send + Sync> Writable for R {
    fn row(&self, scope: &ContentScope) -> RecordRow {
        RecordRow::of(self, scope)
    }
}

impl Unwritten {
    /// Has the next checkpoint write `record` as it is now.
    pub fn put<R: Record + Send + Sync + 'static>(&mut self, record: R) {
        self.put_shared(Arc::new(record));
    }

    pub fn put_shared<R: Record + Send + Sync + 'static>(&mut self, record: Arc<R>) {
        self.changes
            .insert((R::KIND, record.id()), Pending::Put(record));
    }

    /// Has the next checkpoint remove the record of `R` under `id`.
    pub fn remove<R: Record>(&mut self, id: RecordId) {
        self.changes.insert((R::KIND, id), Pending::Removed);
    }

    pub fn len(&self) -> usize {
        self.changes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    pub fn holds(&self, key: &RecordKey) -> bool {
        self.changes.contains_key(key)
    }

    /// Everything unwritten, for a checkpoint to write.
    pub(super) fn pending(&self) -> Vec<(RecordKey, Pending)> {
        self.changes
            .iter()
            .map(|(key, pending)| (key.clone(), pending.clone()))
            .collect()
    }

    /// Forgets each of `written` that has not changed since it was handed out.
    pub(super) fn forget(&mut self, written: &[(RecordKey, Pending)]) {
        for (key, was) in written {
            let unchanged = match (self.changes.get(key), was) {
                (Some(Pending::Put(now)), Pending::Put(was)) => Arc::ptr_eq(now, was),
                (Some(Pending::Removed), Pending::Removed) => true,
                _ => false,
            };
            if unchanged {
                self.changes.remove(key);
            }
        }
    }
}

impl Pending {
    pub(super) fn change(&self, key: &RecordKey, scope: &ContentScope) -> RecordChange {
        match self {
            Pending::Put(record) => RecordChange::Put(record.row(scope)),
            Pending::Removed => RecordChange::Remove {
                kind: key.0,
                id: key.1.clone(),
            },
        }
    }
}
