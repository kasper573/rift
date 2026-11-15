use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::core::store::{Record, RecordId};
use crate::core::time::UnixMillis;
use crate::systems::account::AccountId;

/// What the server keeps of an account that has signed in: the name it plays under, and when it
/// first and last signed in. Nothing of the person behind it: that stays with whoever signs them
/// in.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Account {
    pub id: AccountId,
    pub name: String,
    pub first_seen: UnixMillis,
    pub last_seen: UnixMillis,
}

impl Record for Account {
    const KIND: &'static str = "account";

    fn id(&self) -> RecordId {
        RecordId(self.id.0.clone())
    }
}

/// Every account that has signed in to the server.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Accounts(BTreeMap<AccountId, Account>);

impl Accounts {
    pub fn of(accounts: impl IntoIterator<Item = Account>) -> Accounts {
        Accounts(
            accounts
                .into_iter()
                .map(|account| (account.id.clone(), account))
                .collect(),
        )
    }

    pub fn get(&self, id: &AccountId) -> Option<&Account> {
        self.0.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Account> {
        self.0.values()
    }

    /// Notes that `id` signed in as `name` at `now`: the account as it then is.
    pub fn signed_in(&mut self, id: &AccountId, name: &str, now: UnixMillis) -> &Account {
        let account = self.0.entry(id.clone()).or_insert_with(|| Account {
            id: id.clone(),
            name: name.to_owned(),
            first_seen: now,
            last_seen: now,
        });
        account.name = name.to_owned();
        account.last_seen = now;
        account
    }
}
