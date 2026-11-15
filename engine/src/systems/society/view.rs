//! What a player sees of society: their own standing in it, told to their client alone.

use bevy_app::App;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{AccountNumber, ServerSociety, Society};
use crate::core::time::UnixMillis;
use crate::systems::GameSettings;
use crate::systems::account::AccountId;
use crate::systems::area::AreaId;
use crate::systems::player::CharacterAccount;
use crate::systems::social_request::{
    AcceptsRequestsFrom, SocialRequest, SocialRequestId, SocialRequestKind,
};

pub fn register(app: &mut App) {
    use bevy_replication::AppReplicationExt;
    app.replicate_private::<SocietyView>();
}

/// What society holds of a character's account, as its player sees it.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SocietyView {
    /// The account, as other players know it.
    pub account: AccountNumber,
    /// Who the account takes requests from.
    pub accepts: AcceptsRequestsFrom,
    /// Everyone the account keeps away, by name.
    pub blocked: Vec<SocietyMember>,
    /// The requests the account asked or is asked, the oldest first.
    pub requests: Vec<RequestCard>,
    /// The account's friends: those playing first, by name, then the rest by when they were
    /// last seen, the latest first.
    pub friends: Vec<FriendRow>,
}

/// A friend, as the player whose friend they are sees them.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FriendRow {
    pub player: SocietyMember,
    /// The area they play in, while they are at their character.
    pub online_in: Option<AreaId>,
    pub last_seen: UnixMillis,
}

/// Another player, as one player's client knows them.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SocietyMember {
    pub account: AccountNumber,
    pub name: String,
}

/// A request as one of its two players sees it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RequestCard {
    pub id: SocialRequestId,
    pub kind: SocialRequestKind,
    /// Whether the player it is shown to is the one who asked.
    pub mine: bool,
    /// The other player of the two.
    pub other: SocietyMember,
    /// What it says to the player it is shown to, in the game's words.
    pub says: String,
    /// Until when it shows on the HUD.
    pub shown_until: UnixMillis,
    /// Until when it waits for its answer.
    pub until: UnixMillis,
}

/// Shows every character that came into the world what society holds of its account.
pub fn greet(world: &mut World, arrived: &mut QueryState<Entity, Added<CharacterAccount>>) {
    let arrived: Vec<Entity> = arrived.iter(world).collect();
    if arrived.is_empty() {
        return;
    }
    let society = world.resource::<ServerSociety>().clone();
    let society = society.read();
    for character in arrived {
        show(world, character, &society);
    }
}

/// Shows the player of `character` what `society` holds of their account, as it now stands.
pub(crate) fn show(world: &mut World, character: Entity, society: &Society) {
    let Some(account) = world.get::<CharacterAccount>(character) else {
        return;
    };
    let Some(view) = SocietyView::of(society, &account.0, world.resource::<GameSettings>()) else {
        return;
    };
    if world.get::<SocietyView>(character) != Some(&view) {
        world.entity_mut(character).insert(view);
    }
}

impl SocietyView {
    /// What `society` holds of `account`, for an account it knows.
    fn of(society: &Society, account: &AccountId, settings: &GameSettings) -> Option<SocietyView> {
        let member = |account: &AccountId| society.member(account);
        let mut blocked: Vec<SocietyMember> = society
            .blocks()
            .blocked_by(account)
            .filter_map(member)
            .collect();
        blocked.sort_by(|a, b| (&a.name, a.account).cmp(&(&b.name, b.account)));
        let card = |request: &SocialRequest| {
            let mine = request.asker == *account;
            if request.declined && !mine {
                return None;
            }
            let other = member(if mine { &request.asked } else { &request.asker })?;
            let words = (society.source(request.kind)?.words)(&settings.words);
            let says = if mine { words.asked } else { words.asks }
                .replace("{name}", &other.name)
                .replace("{about}", request.about.as_deref().unwrap_or_default());
            Some(RequestCard {
                id: request.id,
                kind: request.kind,
                mine,
                other,
                says,
                shown_until: request.shown_until,
                until: request.until,
            })
        };
        let mut friends: Vec<FriendRow> = society
            .friends()
            .of_account(account)
            .filter_map(|friend| {
                Some(FriendRow {
                    player: member(friend)?,
                    online_in: society.playing_in(friend),
                    last_seen: society.accounts().get(friend)?.last_seen,
                })
            })
            .collect();
        friends.sort_by(|a, b| {
            let online = b.online_in.is_some().cmp(&a.online_in.is_some());
            let seen = match a.online_in {
                Some(_) => a.player.name.cmp(&b.player.name),
                None => b.last_seen.cmp(&a.last_seen),
            };
            online
                .then(seen)
                .then(a.player.account.cmp(&b.player.account))
        });
        Some(SocietyView {
            account: society.accounts().number(account)?,
            accepts: society.accepts(account, &settings.requests),
            blocked,
            requests: society
                .requests()
                .of_account(account)
                .filter_map(card)
                .collect(),
            friends,
        })
    }
}
