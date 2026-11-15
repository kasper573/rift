use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::store::{Record, RecordId};
use crate::core::time::UnixMillis;
use crate::systems::account::AccountId;
use crate::systems::social_request::AcceptsRequestsFrom;

/// What the server keeps of an account that has signed in: the name it plays under, when it
/// first and last signed in, and who it takes requests from. Nothing of the person behind it:
/// that stays with whoever signs them in.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Account {
    pub id: AccountId,
    pub name: String,
    pub first_seen: UnixMillis,
    pub last_seen: UnixMillis,
    /// Who the account takes requests from, once its player has chosen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepts_requests_from: Option<AcceptsRequestsFrom>,
}

impl Record for Account {
    const KIND: &'static str = "account";

    fn id(&self) -> RecordId {
        RecordId(self.id.0.clone())
    }
}

/// What players name an account by to the server: a number the server deals each account for as
/// long as it runs. The id an account signs in with is its sign-in's to know, and never told to
/// another player.
#[derive(
    JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub struct AccountNumber(pub u32);

/// Every account that has signed in to the server.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Accounts {
    known: BTreeMap<AccountId, (AccountNumber, Account)>,
    numbered: Vec<AccountId>,
}

impl Accounts {
    /// The accounts, numbered in the order they first signed in.
    pub fn of(accounts: impl IntoIterator<Item = Account>) -> Accounts {
        let mut accounts: Vec<Account> = accounts.into_iter().collect();
        accounts.sort_by(|a, b| (a.first_seen, &a.id).cmp(&(b.first_seen, &b.id)));
        let mut all = Accounts::default();
        for account in accounts {
            all.add(account);
        }
        all
    }

    pub fn get(&self, id: &AccountId) -> Option<&Account> {
        self.known.get(id).map(|(_, account)| account)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Account> {
        self.known.values().map(|(_, account)| account)
    }

    pub fn number(&self, id: &AccountId) -> Option<AccountNumber> {
        self.known.get(id).map(|(number, _)| *number)
    }

    /// The account players name by `number`.
    pub fn numbered(&self, number: AccountNumber) -> Option<&AccountId> {
        self.numbered.get((number.0 as usize).checked_sub(1)?)
    }

    /// Notes that `id` signed in at `now`: the account as it then is. One new to the server goes
    /// by `name` until its character plays under another.
    pub fn signed_in(&mut self, id: &AccountId, name: &str, now: UnixMillis) -> &Account {
        if !self.known.contains_key(id) {
            self.add(Account {
                id: id.clone(),
                name: name.to_owned(),
                first_seen: now,
                last_seen: now,
                accepts_requests_from: None,
            });
        }
        self.change(id, |account| account.last_seen = now)
            .expect("the account was just made known")
    }

    /// Changes the account under `id`, when there is one: the account as it then is.
    pub fn change(
        &mut self,
        id: &AccountId,
        change: impl FnOnce(&mut Account),
    ) -> Option<&Account> {
        let (_, account) = self.known.get_mut(id)?;
        change(account);
        Some(account)
    }

    fn add(&mut self, account: Account) {
        self.numbered.push(account.id.clone());
        let number = AccountNumber(self.numbered.len() as u32);
        self.known.insert(account.id.clone(), (number, account));
    }
}
