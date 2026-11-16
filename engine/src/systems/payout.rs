//! The one way the game hands a player items it owes them: what fits goes into their bag, the
//! rest arrives as a message from the game, and a payout is never refused for a full bag.

use bevy_ecs::prelude::*;

use crate::core::content::{Content, ContentProblem, Loading, SETTINGS_FILE};
use crate::systems::account::AccountId;
use crate::systems::area::AreaId;
use crate::systems::item::{Inventory, ItemId, ItemStack};
use crate::systems::player::{self, CharacterAccount};
use crate::systems::quest::{QuestDef, QuestId};
use crate::systems::script::{self, EffectReach, OwnerRow, RiftScripts, RunnerKind};
use crate::systems::society::{self, Settling};
use crate::systems::text::LineText;
use crate::systems::{GameSettings, WorldArea, inbox};

/// What paid a player, which words the message of what had no room uses.
#[derive(Clone, Debug, PartialEq)]
pub enum PayoutReason {
    InstanceCleared(AreaId),
    QuestCompleted(QuestId),
    /// A trade with the player called `with`.
    Traded {
        with: String,
    },
    AuctionBought(ItemId),
    AuctionSold(ItemId),
    AuctionExpired(ItemId),
    AuctionCancelled(ItemId),
    /// A script's `items` effect, named by the file it runs in, where it runs in one.
    Given(Option<OwnerRow>),
}

/// How a payout went: what went into the bag, and what was sent as a message.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Paid {
    pub kept: Vec<ItemStack>,
    pub sent: Vec<ItemStack>,
}

impl Paid {
    /// What the payout's own notification adds when something went to messages, in the game's
    /// words.
    pub fn overflow(&self, content: &Content) -> Option<LineText> {
        if self.sent.is_empty() {
            return None;
        }
        let words = &content.built::<GameSettings>().words;
        let items = LineText::listed(self.sent.iter().map(|stack| stack.named(content)).collect());
        Some(LineText::plain(words.payout_overflow.as_str()).filling("{items}", items))
    }
}

/// Pays `character`, who stands in this world: what fits into their bag, the rest asked of
/// society as a message. A character whose player is away from them gets everything by message.
/// Never refused.
pub fn pay(
    world: &mut World,
    character: Entity,
    stacks: &[ItemStack],
    reason: PayoutReason,
) -> Paid {
    let content = world.resource::<Content>().clone();
    let attended = player::attended(world, character);
    let paid = match world.get_mut::<Inventory>(character) {
        Some(mut bag) if attended => give(&mut bag, &content, stacks),
        _ => Paid {
            kept: Vec::new(),
            sent: stacks.to_vec(),
        },
    };
    if paid.sent.is_empty() {
        return paid;
    }
    if let Some(account) = world.get::<CharacterAccount>(character) {
        let said = reason.said(
            &content,
            world.get_resource::<WorldArea>().map(|area| area.0),
        );
        let delivery = Delivery {
            to: account.0.clone(),
            said,
            stacks: paid.sent.clone(),
        };
        society::deliver(world, delivery);
    }
    paid
}

/// Pays `account`, wherever they are: into the bag when they are at their character, the rest,
/// or everything when they are not, as a message at once.
pub fn pay_account(
    settling: &mut Settling,
    content: &Content,
    account: &AccountId,
    stacks: &[ItemStack],
    reason: PayoutReason,
) -> Paid {
    let (paid, here) = match settling.reach(account) {
        Some((world, character)) => {
            let here = world.get_resource::<WorldArea>().map(|area| area.0);
            let attended = player::attended(world, character);
            let paid = match world.get_mut::<Inventory>(character) {
                Some(mut bag) if attended => give(&mut bag, content, stacks),
                _ => Paid {
                    kept: Vec::new(),
                    sent: stacks.to_vec(),
                },
            };
            (paid, here)
        }
        None => {
            let paid = Paid {
                kept: Vec::new(),
                sent: stacks.to_vec(),
            };
            (paid, None)
        }
    };
    if !paid.sent.is_empty() {
        let delivery = Delivery {
            to: account.clone(),
            said: reason.said(content, here),
            stacks: paid.sent.clone(),
        };
        delivery.settle(settling);
    }
    paid
}

/// What a payout in a world had no room for, on its way to its player's messages.
#[derive(Clone, Debug, PartialEq)]
pub struct Delivery {
    to: AccountId,
    /// The message's title and text.
    said: (String, String),
    stacks: Vec<ItemStack>,
}

impl Delivery {
    /// Writes the delivery as a message from the game, whether or not its player still plays.
    pub(crate) fn settle(self, settling: &mut Settling) {
        inbox::from_game(
            settling.society,
            settling.unwritten,
            settling.now,
            &self.to,
            (&self.said.0, &self.said.1),
            self.stacks,
        );
    }
}

/// A game whose content pays out items has messages, to send what a bag has no room for.
pub fn check(loading: &mut Loading, content: &Content, settings: &GameSettings) {
    if settings.messages.is_some() {
        return;
    }
    let unsent = |found: &str| {
        ContentProblem::new(
            "pays items without messages",
            found,
            "messages in the game's settings, to send what a bag has no room for",
        )
    };
    if settings.auctions.is_some() {
        loading.json_error(
            SETTINGS_FILE,
            "/auctions",
            unsent("an auction house, which pays what is bought and sold"),
        );
    }
    if settings.trade.is_some() {
        loading.json_error(
            SETTINGS_FILE,
            "/trade",
            unsent("trading, which pays what a trade gives"),
        );
    }
    let scripts = content.built::<RiftScripts>();
    let paying: Vec<OwnerRow> = scripts
        .owners()
        .map(|(row, _)| *row)
        .filter(|row| {
            scripts
                .may_do(*row)
                .reaches
                .iter()
                .any(|reach| matches!(reach, EffectReach::PaysItems))
        })
        .collect();
    for row in paying {
        let site = script::owner_site(content, row);
        let problem = unsent("a script that pays out items");
        loading.json_error(&site.file, &site.pointer, problem);
    }
    let rewarding: Vec<QuestId> = loading
        .rows::<QuestDef>(content)
        .into_iter()
        .filter(|(_, def)| !def.rewards().is_empty() || !def.pick_one().is_empty())
        .map(|(quest, _)| quest)
        .collect();
    for quest in rewarding {
        let problem = unsent("a quest whose rewards hold items");
        loading.row_error(quest, "/modules", problem);
    }
}

impl PayoutReason {
    /// The title and text of the message of what had no room, in the game's words: a payout
    /// from a script with no name of its own is named by the area it was paid in.
    fn said(self, content: &Content, here: Option<AreaId>) -> (String, String) {
        let words = &content.built::<GameSettings>().words;
        let filled = |title: &str, text: &str, hole: &str, with: &str| {
            (title.replace(hole, with), text.replace(hole, with))
        };
        let auctioned = |title: &str, item: ItemId| {
            let named = title
                .replace("{item}", &item.get(content).name)
                .replace("{price}", "");
            (named.trim().to_owned(), words.auction_message.clone())
        };
        match self {
            PayoutReason::InstanceCleared(area) => filled(
                &words.instance_rewards_title,
                &words.instance_rewards_text,
                "{area}",
                &area.get(content).name,
            ),
            PayoutReason::QuestCompleted(quest) => filled(
                &words.quest_rewards_title,
                &words.quest_rewards_text,
                "{quest}",
                &quest.get(content).title,
            ),
            PayoutReason::Traded { with } => filled(
                &words.trade_overflow_title,
                &words.trade_overflow_text,
                "{name}",
                &with,
            ),
            PayoutReason::AuctionBought(item) => auctioned(&words.auction_bought, item),
            PayoutReason::AuctionSold(item) => auctioned(&words.auction_sold, item),
            PayoutReason::AuctionExpired(item) => auctioned(&words.auction_expired, item),
            PayoutReason::AuctionCancelled(item) => auctioned(&words.auction_cancelled, item),
            PayoutReason::Given(row) => {
                let source = row
                    .and_then(|row| source_name(content, row))
                    .or_else(|| here.map(|area| area.get(content).name.clone()))
                    .unwrap_or_default();
                filled(
                    &words.given_items_title,
                    &words.given_items_text,
                    "{source}",
                    &source,
                )
            }
        }
    }
}

/// Gives each of `stacks` that fits, tried on its own and in order, so one that finds no room
/// never holds back the others.
fn give(bag: &mut Inventory, content: &Content, stacks: &[ItemStack]) -> Paid {
    let mut paid = Paid::default();
    for &stack in stacks {
        match bag.exchange(content, &[], &[stack]) {
            Ok(()) => paid.kept.push(stack),
            Err(_) => paid.sent.push(stack),
        }
    }
    paid
}

/// What the file a script runs in is called, where it has a name of its own.
fn source_name(content: &Content, row: OwnerRow) -> Option<String> {
    use crate::systems::area::AreaDef;
    use crate::systems::area::spawnable::SpawnableDef;
    use crate::systems::item::ItemDef;
    use crate::systems::mount::MountDef;
    use crate::systems::npc::NpcDef;
    use crate::systems::resource::ResourceDef;
    use crate::systems::shop::ShopDef;
    use crate::systems::skill::SkillDef;
    use crate::systems::status::StatusEffectDef;
    let name = match row.kind {
        RunnerKind::Area | RunnerKind::Object | RunnerKind::Blueprint => {
            &row.id::<AreaDef>().get(content).name
        }
        RunnerKind::Npc => &row.id::<NpcDef>().get(content).name,
        RunnerKind::Item => &row.id::<ItemDef>().get(content).name,
        RunnerKind::Quest => &row.id::<QuestDef>().get(content).title,
        RunnerKind::Shop => &row.id::<ShopDef>().get(content).title,
        RunnerKind::Resource => &row.id::<ResourceDef>().get(content).name,
        RunnerKind::Skill => &row.id::<SkillDef>().get(content).name,
        RunnerKind::StatusEffect => &row.id::<StatusEffectDef>().get(content).name,
        RunnerKind::Mount => &row.id::<MountDef>().get(content).name,
        RunnerKind::Spawnable => &row.id::<SpawnableDef>().get(content).title,
        RunnerKind::RecipeBook | RunnerKind::Command | RunnerKind::Any => return None,
    };
    Some(name.clone())
}
