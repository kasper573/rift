//! The one way items reach a player: what fits goes into their bag, the rest arrives as a message
//! from the game, and the player is told. A delivery is never refused for a full bag.

use crate::core::script::owner_name;
use crate::core::script::owner_site;
use crate::systems::item::effects::PaysItems;
use bevy_ecs::prelude::*;

use crate::core::content::{Content, ContentProblem, Loading, SETTINGS_FILE, SavedList};
use crate::core::script::{OwnerRow, Scripts};
use crate::core::text::LineText;
use crate::systems::account::AccountId;
use crate::systems::area::AreaId;
use crate::systems::history::HistoryTopic;
use crate::systems::item::{Inventory, ItemStack};
use crate::systems::messages::{self, messages};
use crate::systems::notification::{self, Notification};
use crate::systems::player::{self, CharacterAccount};
use crate::systems::quest::{QuestDef, QuestId};
use crate::systems::society::{self, Settling};
use crate::systems::{GameSettings, WorldArea, mail};

messages! {
    /// What the game says of items it delivers to players.
    pub struct DeliverMessages {
        /// What the notification of a delivery says of `{items}` that had no room in the player's
        /// bag and were sent to their mail.
        overflow[items: Line],
        /// The title of the message the game sends a player whose bag had no room for what `{source}`
        /// gave them: an npc, an item, an area or whatever else the script giving it is of.
        given_items_title(source: Text),
        /// The text of the message the game sends a player whose bag had no room for what `{source}`
        /// gave them.
        given_items_text,
    }
}

/// What delivered items to a player, which words the message of what had no room uses.
#[derive(Clone, Debug, PartialEq)]
pub enum DeliveryReason {
    InstanceCleared(AreaId),
    QuestCompleted(QuestId),
    /// A trade with the player called `with`.
    Traded {
        with: String,
    },
    /// A purchase on the market.
    MarketBought,
    /// A seller's listing on the market, bought.
    MarketSold,
    /// The goods of a listing on the market, back as it ended unbought.
    MarketReturned,
    /// A script's `items` effect, named by the file it runs in, where it runs in one.
    Given(Option<OwnerRow>),
    /// Items a saved bag held that it had no room for when its player came back.
    KeptAside,
}

/// How a delivery went: what went into the bag, and what was sent as a message.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Delivered {
    pub kept: Vec<ItemStack>,
    pub sent: Vec<ItemStack>,
}

/// What a notification says of `sent`, the stacks of a delivery that went to mail.
pub fn overflow_line(content: &Content, sent: &[ItemStack]) -> LineText {
    let messages = messages::of(content);
    messages
        .deliver
        .overflow(ItemStack::listed(&messages.interface, sent))
}

/// Delivers `stacks` to `character`, who stands in this world: what fits into their bag, the rest
/// asked of society as a message. A character whose player is away from them gets everything by
/// message. Tells the player what went to mail.
pub fn to_character(
    world: &mut World,
    character: Entity,
    stacks: &[ItemStack],
    reason: DeliveryReason,
) -> Delivered {
    let content = world.resource::<Content>().clone();
    let delivered = fit(world, character, &content, stacks);
    if delivered.sent.is_empty() {
        return delivered;
    }
    if let Some(account) = world.get::<CharacterAccount>(character) {
        let overflow = Overflow {
            to: account.0.clone(),
            said: reason.said(
                &content,
                world.get_resource::<WorldArea>().map(|area| area.0),
            ),
            stacks: delivered.sent.clone().into(),
        };
        society::send_overflow(world, overflow);
    }
    let told = Notification::feed(HistoryTopic::Item, overflow_line(&content, &delivered.sent));
    notification::notify(world, character, None, told);
    delivered
}

/// Delivers `stacks` to `account`, wherever they are: into the bag when they are at their
/// character, the rest, or everything when they are not, as a message at once. Tells the player
/// what went to mail, when they play.
pub fn to_account(
    settling: &mut Settling,
    content: &Content,
    account: &AccountId,
    stacks: &[ItemStack],
    reason: DeliveryReason,
) -> Delivered {
    let (delivered, here) = match settling.reach(account) {
        Some((world, character)) => (
            fit(world, character, content, stacks),
            world.get_resource::<WorldArea>().map(|area| area.0),
        ),
        None => (
            Delivered {
                kept: Vec::new(),
                sent: stacks.to_vec(),
            },
            None,
        ),
    };
    if delivered.sent.is_empty() {
        return delivered;
    }
    let overflow = Overflow {
        to: account.clone(),
        said: reason.said(content, here),
        stacks: delivered.sent.clone().into(),
    };
    overflow.settle(settling);
    let told = Notification::feed(HistoryTopic::Item, overflow_line(content, &delivered.sent));
    settling.tell(account, told);
    delivered
}

/// Sends `stacks` to `account` as a message from the game without trying their bag, for stacks
/// that are only known as saved.
pub fn by_mail(
    settling: &mut Settling,
    content: &Content,
    account: &AccountId,
    stacks: SavedList<ItemStack>,
    reason: DeliveryReason,
) {
    let overflow = Overflow {
        to: account.clone(),
        said: reason.said(content, None),
        stacks,
    };
    overflow.settle(settling);
}

/// What a delivery had no room for, on its way to its player's messages.
#[derive(Clone, Debug, PartialEq)]
pub struct Overflow {
    to: AccountId,
    /// The message's title and text.
    said: (String, String),
    stacks: SavedList<ItemStack>,
}

impl Overflow {
    /// Writes it as a message from the game, whether or not its player still plays.
    pub(crate) fn settle(self, settling: &mut Settling) {
        mail::from_system(
            settling.society,
            settling.unwritten,
            settling.now,
            &self.to,
            (&self.said.0, &self.said.1),
            self.stacks,
        );
    }
}

/// A game whose content delivers items has mail, to send what a bag has no room for.
pub fn check(loading: &mut Loading, content: &Content, settings: &GameSettings) {
    if settings.mail.is_some() {
        return;
    }
    let unsent = |found: &str| {
        ContentProblem::new(
            "delivers items without mail",
            found,
            "mail in the game's settings, to send what a bag has no room for",
        )
    };
    if settings.market.is_some() {
        loading.json_error(
            SETTINGS_FILE,
            "/market",
            unsent("a market, which delivers what is bought and sold"),
        );
    }
    if settings.trade.is_some() {
        loading.json_error(
            SETTINGS_FILE,
            "/trade",
            unsent("trading, which delivers what a trade gives"),
        );
    }
    let scripts = content.built::<Scripts>();
    let paying: Vec<OwnerRow> = scripts
        .owners()
        .map(|(row, _)| *row)
        .filter(|row| {
            scripts
                .may_do(*row)
                .reaching::<PaysItems>()
                .next()
                .is_some()
        })
        .collect();
    for row in paying {
        let site = owner_site(content, row);
        let problem = unsent("a script that gives items");
        loading.json_error(&site.file, &site.pointer, problem);
    }
    let rewarding: Vec<QuestId> = loading
        .rows::<QuestDef>(content)
        .into_iter()
        .filter(|(_, def)| def.rewards().next().is_some() || !def.pick_one().is_empty())
        .map(|(quest, _)| quest)
        .collect();
    for quest in rewarding {
        let problem = unsent("a quest whose rewards hold items");
        loading.row_error(quest, "/modules", problem);
    }
}

impl DeliveryReason {
    /// The title and text of the message of what had no room, in the game's words: a delivery
    /// from a script with no name of its own is named by the area it was delivered in.
    fn said(self, content: &Content, here: Option<AreaId>) -> (String, String) {
        let messages = messages::of(content);
        let marketed = |title: &str| (title.to_owned(), messages.market.message().to_owned());
        match self {
            DeliveryReason::InstanceCleared(area) => (
                messages.instance.rewards_title(&area.get(content).name),
                messages.instance.rewards_text().to_owned(),
            ),
            DeliveryReason::QuestCompleted(quest) => (
                messages.quest.rewards_title(&quest.get(content).title),
                messages.quest.rewards_text().to_owned(),
            ),
            DeliveryReason::Traded { with } => (
                messages.trade.overflow_title(&with),
                messages.trade.overflow_text().to_owned(),
            ),
            DeliveryReason::MarketBought => marketed(messages.market.bought_title()),
            DeliveryReason::MarketSold => marketed(messages.market.sold_title()),
            DeliveryReason::MarketReturned => marketed(messages.market.returned_title()),
            DeliveryReason::Given(row) => {
                let source = row
                    .and_then(|row| source_name(content, row))
                    .or_else(|| here.map(|area| area.get(content).name.clone()))
                    .unwrap_or_default();
                (
                    messages.deliver.given_items_title(source),
                    messages.deliver.given_items_text().to_owned(),
                )
            }
            DeliveryReason::KeptAside => (
                messages.mail.did_not_fit_title().to_owned(),
                messages.mail.did_not_fit_text().to_owned(),
            ),
        }
    }
}

/// What went into the bag of `character` of `stacks`, and what did not: everything does not, for
/// a character whose player is away from them.
fn fit(world: &mut World, character: Entity, content: &Content, stacks: &[ItemStack]) -> Delivered {
    let attended = player::attended(world, character);
    match world.get_mut::<Inventory>(character) {
        Some(mut bag) if attended => bag.give_what_fits(content, stacks),
        _ => Delivered {
            kept: Vec::new(),
            sent: stacks.to_vec(),
        },
    }
}

/// What the file a script runs in is called, where it has a name of its own.
fn source_name(content: &Content, row: OwnerRow) -> Option<String> {
    owner_name(content, row)
}
