//! Reading an inbox from the store as its account comes to play, off the tick.

use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

use super::Letter;
use crate::core::content::ContentScope;
use crate::core::store::{self, Record, RecordOwner, RecordStore, RecordStoreProblem};
use crate::systems::account::AccountId;

/// Where inboxes are read from: a store, on the runtime that reads it, or nowhere, for a server
/// that saves nothing.
#[derive(Default)]
pub struct InboxReads(Option<ReadLink>);

struct ReadLink {
    store: RecordStore,
    runtime: tokio::runtime::Handle,
    scope: ContentScope,
    results: Sender<InboxRead>,
    read: Mutex<Receiver<InboxRead>>,
}

/// What the store holds of the inbox of `account`, or why it could not be read.
pub(super) struct InboxRead {
    pub account: AccountId,
    pub letters: Result<Vec<Letter>, RecordStoreProblem>,
}

impl InboxReads {
    /// Reads from `store` on `runtime`, each letter under the game `scope` is of: a stack that
    /// names an item the game no longer has is set aside in its letter.
    pub fn from(
        store: RecordStore,
        runtime: tokio::runtime::Handle,
        scope: ContentScope,
    ) -> InboxReads {
        let (results, read) = channel();
        InboxReads(Some(ReadLink {
            store,
            runtime,
            scope: scope.setting_aside(),
            results,
            read: Mutex::new(read),
        }))
    }

    /// Begins reading the inbox of `account`: false where there is no store to read.
    pub(super) fn begin(&self, account: &AccountId) -> bool {
        let Some(link) = &self.0 else {
            return false;
        };
        let (store, scope, results) =
            (link.store.clone(), link.scope.clone(), link.results.clone());
        let account = account.clone();
        link.runtime.spawn(async move {
            let owner = RecordOwner(account.0.clone());
            let letters = store.owned(Letter::KIND, &owner).await.and_then(|bodies| {
                bodies
                    .iter()
                    .map(|body| store::read::<Letter>(body, &scope))
                    .collect()
            });
            let _ = results.send(InboxRead { account, letters });
        });
        true
    }

    /// The inboxes read since this was last asked.
    pub(super) fn finished(&self) -> Vec<InboxRead> {
        match &self.0 {
            Some(link) => link
                .read
                .lock()
                .expect("inbox reads lock")
                .try_iter()
                .collect(),
            None => Vec::new(),
        }
    }
}
