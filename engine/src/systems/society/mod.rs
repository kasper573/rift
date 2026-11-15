//! Everything of the server and of no one world: who has signed in, and what players share.
//! There is one of it, changed only where the server's worlds confer.

pub mod account;

pub use account::{Account, Accounts};

use crate::core::content::ContentScope;
use crate::core::store::{self, Record, RecordStore, RecordStoreProblem};
use crate::core::time::UnixMillis;
use crate::systems::account::AccountIdentity;
use crate::systems::save::Unwritten;

#[derive(Default)]
pub struct Society {
    accounts: Accounts,
}

impl Society {
    /// Society as `store` holds it: every record of the kinds it keeps whole in memory. Refused
    /// when the store cannot be read, or holds a record this build does not read whole.
    pub async fn read(
        store: &RecordStore,
        scope: &ContentScope,
    ) -> Result<Society, RecordStoreProblem> {
        Ok(Society {
            accounts: Accounts::of(all::<Account>(store, scope).await?),
        })
    }

    pub fn accounts(&self) -> &Accounts {
        &self.accounts
    }

    /// Notes that the account of `identity` signed in at `now`.
    pub fn signed_in(
        &mut self,
        identity: &AccountIdentity,
        now: UnixMillis,
        unwritten: &mut Unwritten,
    ) {
        let account = self.accounts.signed_in(&identity.id, &identity.name, now);
        unwritten.put(account.clone());
    }
}

async fn all<R: Record>(
    store: &RecordStore,
    scope: &ContentScope,
) -> Result<Vec<R>, RecordStoreProblem> {
    store
        .all(R::KIND)
        .await?
        .iter()
        .map(|body| store::read(body, scope))
        .collect()
}
