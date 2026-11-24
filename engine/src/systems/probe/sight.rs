//! What a client knows of the game: the probe's report of everything its session holds, which
//! takes no screen to tell. A client with no window tells it as the game client does.

use bevy::ecs::entity::MapEntities;
use bevy::prelude::*;
use bevy_terminal::LineKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::ServerNames;
use crate::core::content::Content;
use crate::core::geometry::Capsule;
use crate::core::interpolate::Jumps;
use crate::core::math::{Pos, PosSchema, Rect, RectSchema};
use crate::core::script::Scripts;
use crate::core::time;
use crate::core::units::{Meters, Percent, Radians, Seconds};
use crate::systems::actor::held::{self, Held};
use crate::systems::actor::skeleton::SkeletonId;
use crate::systems::actor::stance::{HandIndex, StanceId};
use crate::systems::actor::{
    self, Action, Actor, ActorModel, Appearance, Hitbox, Motion, Playing, Pose,
};
use crate::systems::area::{self, AreaId, AreaObject, AreaTag, TimeOfDayName};
use crate::systems::area_instance::entrance::ShownInstanceEntrance;
use crate::systems::area_instance::{self, ShownAreaInstance, ShownInstanceCall};
use crate::systems::attention::{self, Attention, AttentionId, StatusBadges};
use crate::systems::chat::ChatChannel;
use crate::systems::chat::log::ChatLog;
use crate::systems::combat::Attitude;
use crate::systems::crafting::{self, RecipeId};
use crate::systems::dialogue::{ChoiceChip, Conversation, Speaker};
use crate::systems::equipment::{self, Belongings, EquipmentPlace};
use crate::systems::friend::{self, ShownFriends};
use crate::systems::history::{HistoryBook, HistoryMark, HistoryRecord, HistoryTopic};
use crate::systems::identity::Lingering;
use crate::systems::item::rarity::ItemRarityId;
use crate::systems::item::{self, DroppedItem, ItemId, ItemStack, Reservation};
use crate::systems::mail::{self, InboxSender, LetterId, LetterTexts};
use crate::systems::market::{self, ShownMarket};
use crate::systems::messages;
use crate::systems::mount::{self, MountId, Riding, ShownMount};
use crate::systems::movement::Position;
use crate::systems::npc::{Npc, NpcId};
use crate::systems::party::{self, ShownParty};
use crate::systems::player::{Away, PlayedBy, commands_locked, session};
use crate::systems::prompt::{self, PromptKind, PromptView};
use crate::systems::quest::{Parties, QuestId, QuestLog, QuestResult};
use crate::systems::reach::Engaged;
use crate::systems::resource::{self, Left, Resource, ResourceId, Working};
use crate::systems::shop::{self, ShopId};
use crate::systems::skill::{self, Rank, SkillId};
use crate::systems::social_request::SocialRequestId;
use crate::systems::society::AccountNumber;
use crate::systems::society::view::{CharacterAccountNumber, SocietyView};
use crate::systems::stat::{self, Vital};
use crate::systems::status::{self, StatusCategory, StatusEffectId};
use crate::systems::trade::{self, ShownTrade};
use crate::systems::window::WindowStep;

/// What a client knows of the game around its character, every id as the server knows it.
pub fn sight(world: &mut World) -> ProbeSight {
    let mut sight = sight_as_held(world);
    sight.map_entities(&mut ServerNames(
        world.resource::<bevy_replication::ServerEntities>(),
    ));
    sight
}

/// What a client knows of the game around its character: the session's part of what the game
/// client's probe reports, all a client with no window tells.
#[derive(MapEntities, Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeSight {
    pub area: Option<AreaId>,
    #[entities]
    pub me: Option<ProbeBody>,
    #[entities]
    pub actors: Vec<ProbeBody>,
    #[entities]
    pub objects: Vec<ProbeObject>,
    #[entities]
    pub items: Vec<ProbeGroundItem>,
    #[entities]
    pub resources: Vec<ProbeResource>,
    pub warps: Vec<ProbeWarp>,
    /// Every slot of the bag in its order, `None` for an empty one.
    pub bag: Vec<Option<ProbeStack>>,
    pub quests: ProbeQuests,
    pub statuses: Vec<ProbeStatus>,
    pub basic_attack: Option<SkillId>,
    pub skills: Option<ProbeSkills>,
    /// The conversation the character is in.
    #[entities]
    pub conversation: Option<ProbeConversation>,
    /// The requests between the character and other players that still wait for an answer.
    pub requests: Vec<ProbeRequest>,
    /// The character's friends, the requests of friendship in and out, the players kept away,
    /// and what the latest search by name found.
    pub friends: ShownFriends,
    /// The party the character is in, and the invitations that wait for their answer.
    pub party: ShownParty,
    /// The letters of the inbox, the newest first.
    pub letters: Vec<ProbeLetter>,
    /// The lines heard on the channels the player is in, the oldest first, as many as the game
    /// keeps.
    pub chat_log: Vec<ProbeChatLine>,
    /// The character's history, the oldest record first, as many as the game keeps.
    pub history: Vec<ProbeRecord>,
    /// The mount the character rides, the seat they hold and who sits where.
    pub mount: Option<ShownMount>,
    /// The instance the character is inside.
    pub instance: Option<ShownAreaInstance>,
    /// The entrance of an instance the character stands before.
    pub instance_entrance: Option<ShownInstanceEntrance>,
    /// The call to an instance the character leads or was called to.
    pub instance_call: Option<ShownInstanceCall>,
    /// The trade the character is in.
    pub trade: Option<ShownTrade>,
    /// The market, while it is open to the character.
    pub market: Option<ShownMarket>,
    /// The shop, while it is open to the character.
    pub shop: Option<ProbeShop>,
    /// The prompt before the character.
    pub prompt: Option<ProbePrompt>,
    /// What the prompt before the character crafts, when it crafts.
    pub crafting: Option<ProbeCrafting>,
    /// The time of day that holds, in a game that names its times and once the client has been
    /// told the hour.
    pub time_of_day: Option<TimeOfDayName>,
    /// The spell of weather under way over the area, by its name, once the client has been told
    /// the weather's course.
    pub weather: Option<String>,
}

#[derive(MapEntities, Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeBody {
    #[entities]
    #[schemars(with = "u64")]
    pub id: Entity,
    pub name: String,
    pub appearance: Appearance,
    pub flies: bool,
    pub player: bool,
    /// The number other players name its account by, when it is a player's character.
    pub account: Option<AccountNumber>,
    /// Whether it is a character whose player is gone, standing in the world without them.
    pub away: bool,
    #[schemars(with = "PosSchema")]
    pub at: Pos<Meters>,
    pub capsule: Capsule,
    /// The way it faces.
    pub facing: Radians,
    pub action: Action,
    /// The pose it holds while it idles.
    pub pose: Pose,
    /// The statuses it carries.
    pub statuses: Vec<StatusEffectId>,
    /// The actor it is casting at, a blow or a bolt alike.
    #[entities]
    #[schemars(with = "Option<u64>")]
    pub engaged: Option<Entity>,
    /// Whether it stands still rather than walks.
    pub resting: bool,
    /// The mount it rides.
    pub rides: Option<MountId>,
    /// How many times it was put where it stands rather than walked there: a warp, a respawn.
    #[schemars(with = "u32")]
    pub jumps: Jumps,
    pub health: f32,
    pub max_health: f32,
    pub mana: f32,
    pub max_mana: f32,
    pub npc: Option<NpcId>,
    pub friendly: bool,
    pub locked: bool,
    pub marks: Vec<AttentionId>,
    pub badges: Vec<AttentionId>,
    /// The marks its plate shows, most pressing first: the first drawn large over its name, the
    /// rest as pips after it.
    pub plate_marks: Vec<AttentionId>,
    pub skeleton: SkeletonId,
    /// The stance its hands or a script put it in.
    pub stance: Option<StanceId>,
    /// The motion it was told to play, if any.
    pub motion: Option<Motion>,
    /// Each of its hands, in their order.
    pub hands: Vec<ProbeHand>,
    /// What each hand of its body holds, as the server says.
    pub wearing: Vec<ProbeWorn>,
    /// The item in its tool slot.
    pub tool: Option<ItemId>,
    /// The work it has under way at a resource.
    #[entities]
    pub gathering: Option<ProbeGathering>,
}

/// A hand of a body, as its skeleton declares it.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeHand {
    /// What its skeleton calls it.
    pub name: String,
    /// What the player calls it, as the equipment panel heads it.
    pub label: String,
}

/// An item a hand of a body holds.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeWorn {
    pub hand: HandIndex,
    pub item: ItemId,
}

#[derive(MapEntities, Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeObject {
    #[entities]
    #[schemars(with = "u64")]
    pub id: Entity,
    pub name: Option<String>,
    pub title: Option<String>,
    #[schemars(with = "PosSchema")]
    pub at: Pos<Meters>,
    #[schemars(with = "RectSchema")]
    pub hitbox: Rect<Meters>,
    pub marks: Vec<AttentionId>,
    /// The statuses it carries.
    pub statuses: Vec<StatusEffectId>,
}

#[derive(MapEntities, Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeGroundItem {
    #[entities]
    #[schemars(with = "u64")]
    pub id: Entity,
    pub item: ItemId,
    pub rarity: ItemRarityId,
    pub count: u32,
    #[schemars(with = "PosSchema")]
    pub at: Pos<Meters>,
    /// Whether this player may take it.
    pub mine: bool,
}

/// A resource standing in the world.
#[derive(MapEntities, Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeResource {
    #[entities]
    #[schemars(with = "u64")]
    pub id: Entity,
    pub resource: ResourceId,
    pub name: String,
    #[schemars(with = "PosSchema")]
    pub at: Pos<Meters>,
    pub capsule: Capsule,
    /// How many extractions it still holds.
    pub left: u32,
    /// How many may work it at once.
    pub room: u32,
    /// Who works it.
    #[entities]
    #[schemars(with = "Vec<u64>")]
    pub workers: Vec<Entity>,
}

/// The work a character has under way at a resource.
#[derive(MapEntities, Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeGathering {
    #[entities]
    #[schemars(with = "u64")]
    pub resource: Entity,
    /// The tool it works with, none by hand.
    pub tool: Option<ItemId>,
    /// How far through the extraction under way it is, from none to all of it.
    pub progress: f32,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeWarp {
    pub name: String,
    pub to: AreaId,
    #[schemars(with = "PosSchema")]
    pub at: Pos<Meters>,
    #[schemars(with = "RectSchema")]
    pub rect: Rect<Meters>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug)]
pub struct ProbeStack {
    pub item: ItemId,
    pub count: u32,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default)]
pub struct ProbeQuests {
    pub active: Vec<ProbeQuest>,
    pub finished: Vec<ProbeFinishedQuest>,
    pub tracked: Vec<QuestId>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeQuest {
    pub quest: QuestId,
    pub ready: bool,
    pub left: Option<Seconds>,
    /// Each objective's [have, need].
    pub progress: Vec<[u32; 2]>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug)]
pub struct ProbeFinishedQuest {
    pub quest: QuestId,
    pub result: QuestResult,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ProbeStatus {
    pub effect: StatusEffectId,
    pub category: StatusCategory,
    pub left: Option<Seconds>,
}

/// The character's class, the level it stands at and the skills it has learned.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ProbeSkills {
    pub class: String,
    pub level: u32,
    pub to_spend: u32,
    pub learned: Vec<ProbeRank>,
    /// Whether the character is offered to unlearn its skills.
    pub reset_offered: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ProbeRank {
    pub skill: String,
    pub rank: Rank,
}

/// A conversation as its player holds it: what has been said in it, and what they may answer.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, MapEntities)]
pub struct ProbeConversation {
    /// The step of the conversation, which an answer to it names.
    pub step: WindowStep,
    pub node: String,
    #[entities]
    #[schemars(with = "Option<u64>")]
    pub with: Option<Entity>,
    pub lines: Vec<ProbeSaid>,
    pub choices: Vec<ProbeAnswer>,
}

/// A line said in a conversation, and who says it: none for a narrator.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeSaid {
    pub by: Option<String>,
    pub text: String,
}

/// A choice a conversation offers, in the order it offers them.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeAnswer {
    pub label: String,
    /// The items choosing it gives.
    pub gets: Vec<ItemId>,
    pub locked: bool,
}

/// A request between the player and another that waits for its answer.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeRequest {
    pub id: SocialRequestId,
    /// The other player of the two.
    pub with: String,
    /// Whether the player is the one who asked.
    pub mine: bool,
    /// What it says to the player, in the game's words.
    pub says: String,
}

/// A letter of the inbox.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ProbeLetter {
    pub id: LetterId,
    pub title: String,
    /// Who wrote it, none for the game itself.
    pub from: Option<String>,
    pub read: bool,
    /// Each stack attached, as its count and its item's name.
    pub attached: Vec<String>,
    /// Its text, once the letter was opened and the server has sent it.
    pub text: Option<String>,
}

/// A line heard in chat.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ProbeChatLine {
    /// The channel it was said on, none for what the game tells the player alone.
    pub channel: Option<ChatChannel>,
    /// Who said it, none for what the game tells.
    pub by: Option<String>,
    pub text: String,
    /// Whether it tells that something the player asked for failed.
    pub failed: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeRecord {
    pub topic: HistoryTopic,
    pub by: Option<String>,
    pub text: String,
    pub mark: Option<HistoryMark>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeShop {
    pub shop: ShopId,
    pub wares: Vec<ProbeWare>,
    pub buyback: Vec<ProbeStack>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeWare {
    pub item: ItemId,
    pub count: u32,
    pub left: Option<u32>,
    pub refusal: Option<String>,
}

/// The prompt before the character: the step an answer to it names, its words and how many
/// stacks it takes.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbePrompt {
    pub step: WindowStep,
    pub title: String,
    pub text: Option<String>,
    pub slots: u32,
    /// Whether it crafts, rather than takes what is placed for the script that asked.
    pub crafts: bool,
}

/// What the prompt before the character crafts: every recipe it was opened with, and how far
/// the craft under way is.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeCrafting {
    pub recipes: Vec<ProbeRecipe>,
    /// How far through the craft under way it is, from none to all of it.
    pub progress: Option<f32>,
}

/// A recipe of a crafting prompt.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeRecipe {
    /// Its name within its book.
    pub id: RecipeId,
    /// Everything one craft gives.
    pub makes: Vec<ProbeStack>,
    pub from: Vec<ProbeStack>,
    /// How long one craft takes at a crafting speed of 100%.
    pub takes: Seconds,
    /// How often one craft comes off: one that fails spends its materials and makes nothing.
    pub chance: Percent,
    /// Whether the character's bag covers every material.
    pub can_make: bool,
    /// Whether only a crafter who masters it makes it.
    pub mastery: bool,
    /// Whether it requires a mastery the character lacks.
    pub locked: bool,
}

pub(super) fn sight_as_held(world: &mut World) -> ProbeSight {
    let me = session::me(world);
    let area = me
        .and_then(|seen| world.get::<AreaTag>(seen))
        .map(|tag| tag.area);
    ProbeSight {
        area,
        me: me.and_then(|me| body(world, me)),
        actors: actors(world, me, area),
        objects: objects(world, area),
        items: items(world, area),
        resources: resources(world, area),
        warps: warps(world, area),
        bag: bag(world),
        quests: me.and_then(|seen| quests(world, seen)).unwrap_or_default(),
        statuses: me.map(|me| statuses(world, me)).unwrap_or_default(),
        basic_attack: me
            .and_then(|me| world.get::<skill::BasicAttack>(me))
            .and_then(|slot| slot.0),
        skills: me.and_then(|me| skills(world, me)),
        conversation: me.and_then(|me| conversation(world, me)),
        requests: requests(world, me),
        friends: friend::shown(world),
        party: party::shown(world),
        letters: letters(world),
        chat_log: chat_log(world),
        history: history(world),
        mount: mount::shown(world),
        instance: area_instance::shown(world),
        instance_entrance: area_instance::entrance::shown(world),
        instance_call: area_instance::shown_call(world),
        trade: trade::shown(world),
        market: market::shown(world),
        shop: shop(world),
        prompt: prompt(world),
        crafting: crafting(world),
        time_of_day: time_of_day(world),
        weather: area.and_then(|area| weather(world, area)),
    }
}

pub(super) fn stack(stack: &ItemStack) -> ProbeStack {
    ProbeStack {
        item: stack.item,
        count: stack.count,
    }
}

fn body(world: &World, entity: Entity) -> Option<ProbeBody> {
    let viewer = session::me(world);
    let marks = viewer
        .and_then(|viewer| world.get::<Attention>(viewer))
        .map(|attention| attention.of(entity).to_vec())
        .unwrap_or_default();
    let plate_marks = attention::plate_marks(world, viewer, entity);
    let locked = commands_locked(world, entity);
    let entity = world.get_entity(entity).ok()?;
    let actor = entity.get::<Actor>()?;
    let position = entity.get::<Position>()?;
    let appearance = entity.get::<Appearance>()?.clone();
    let built = ActorModel::get(world.resource::<Content>(), appearance.model);
    Some(ProbeBody {
        id: entity.id(),
        engaged: entity.get::<Engaged>().map(|engaged| engaged.0),
        name: entity
            .get::<actor::Name>()
            .map_or_else(String::new, |name| name.name.clone()),
        flies: built.airborne(),
        player: entity.contains::<PlayedBy>(),
        account: entity
            .get::<CharacterAccountNumber>()
            .map(|account| account.0),
        away: entity.contains::<Away>(),
        at: position.pos,
        capsule: entity.get::<Hitbox>()?.0,
        facing: actor.facing,
        action: actor.action,
        pose: actor.pose,
        statuses: carried(world, entity.id()),
        resting: !actor.action.moves(),
        rides: entity.get::<Riding>().map(|riding| riding.mount),
        jumps: position.jumps,
        health: stat::current(world, entity.id(), Vital::Health),
        max_health: stat::most(world, entity.id(), Vital::Health),
        mana: stat::current(world, entity.id(), Vital::Mana),
        max_mana: stat::most(world, entity.id(), Vital::Mana),
        npc: entity.get::<Npc>().map(|npc| npc.def),
        friendly: entity.get::<Attitude>() == Some(&Attitude::Friendly),
        locked,
        marks,
        plate_marks,
        badges: entity
            .get::<StatusBadges>()
            .map(|badges| badges.0.clone())
            .unwrap_or_default(),
        skeleton: built.skeleton_id(),
        stance: held::stance_of(world, entity.id()),
        motion: entity
            .get::<Playing>()
            .filter(|playing| playing.motion.shows_over(actor.action))
            .map(|playing| playing.motion.clone()),
        hands: built
            .hands()
            .iter()
            .map(|hand| ProbeHand {
                name: hand.name.clone(),
                label: hand.label().to_owned(),
            })
            .collect(),
        wearing: entity
            .get::<Held>()
            .map(|wearing| {
                wearing
                    .hands
                    .iter()
                    .map(|(&hand, &item)| ProbeWorn { hand, item })
                    .collect()
            })
            .unwrap_or_default(),
        tool: equipment::in_slot(world, entity.id(), &EquipmentPlace::Tool),
        gathering: entity.get::<Working>().map(|working| ProbeGathering {
            resource: working.resource,
            tool: working.tool,
            progress: resource::progress(world, working).unwrap_or(0.0),
        }),
        appearance,
    })
}

fn actors(world: &mut World, me: Option<Entity>, area: Option<AreaId>) -> Vec<ProbeBody> {
    let others: Vec<Entity> = world
        .query_filtered::<(Entity, &AreaTag), With<Actor>>()
        .iter(world)
        .filter(|(entity, tag)| Some(*entity) != me && area.is_none_or(|area| tag.area == area))
        .map(|(entity, _)| entity)
        .collect();
    others
        .into_iter()
        .filter_map(|entity| body(world, entity))
        .collect()
}

fn objects(world: &mut World, area: Option<AreaId>) -> Vec<ProbeObject> {
    let viewer = session::me(world);
    let attention = viewer
        .and_then(|viewer| world.get::<Attention>(viewer))
        .cloned();
    world
        .query::<(Entity, &AreaObject, &Position, &AreaTag)>()
        .iter(world)
        .filter(|(.., tag)| area.is_none_or(|area| tag.area == area))
        .filter(|(_, object, ..)| object.look.is_some())
        .map(|(entity, object, at, _)| ProbeObject {
            id: entity,
            name: object.name.clone(),
            title: object.title.clone(),
            at: at.pos,
            hitbox: object.shape.bounds(),
            marks: attention
                .as_ref()
                .map(|attention| attention.of(entity).to_vec())
                .unwrap_or_default(),
            statuses: carried(world, entity),
        })
        .collect()
}

/// The statuses a thing carries, in the order it took them.
fn carried(world: &World, thing: Entity) -> Vec<StatusEffectId> {
    world
        .get::<status::Statuses>(thing)
        .map(|statuses| statuses.0.iter().map(|held| held.effect).collect())
        .unwrap_or_default()
}

fn items(world: &mut World, area: Option<AreaId>) -> Vec<ProbeGroundItem> {
    let content = world.resource::<Content>().clone();
    let me = item::DropTaker::me(world);
    world
        .query::<(
            Entity,
            &DroppedItem,
            &Position,
            &AreaTag,
            Option<&Reservation>,
        )>()
        .iter(world)
        .filter(|(.., tag, _)| area.is_none_or(|area| tag.area == area))
        .map(|(entity, dropped, at, _, reservation)| ProbeGroundItem {
            id: entity,
            item: dropped.item,
            rarity: dropped.item.get(&content).rarity,
            count: dropped.count,
            at: at.pos,
            mine: item::takeable(reservation, &me),
        })
        .collect()
}

/// The resources that stand in `area`: one that ran out and is fading from view is not among them.
fn resources(world: &mut World, area: Option<AreaId>) -> Vec<ProbeResource> {
    let content = world.resource::<Content>().clone();
    let standing: Vec<(Entity, ResourceId, u32, Pos<Meters>, Capsule)> = world
        .query_filtered::<(Entity, &Resource, &Left, &Position, &Hitbox, &AreaTag), Without<Lingering>>()
        .iter(world)
        .filter(|(.., tag)| area.is_none_or(|area| tag.area == area))
        .map(|(entity, resource, left, at, hitbox, _)| {
            (entity, resource.def, left.0, at.pos, hitbox.0)
        })
        .collect();
    let working: Vec<(Entity, Entity)> = world
        .query::<(Entity, &Working)>()
        .iter(world)
        .map(|(worker, working)| (worker, working.resource))
        .collect();
    standing
        .into_iter()
        .map(|(entity, def, left, at, capsule)| ProbeResource {
            id: entity,
            resource: def,
            name: def.get(&content).name.clone(),
            at,
            capsule,
            left,
            room: def.get(&content).max_occupants,
            workers: working
                .iter()
                .filter(|(_, resource)| *resource == entity)
                .map(|(worker, _)| *worker)
                .collect(),
        })
        .collect()
}

fn warps(world: &World, area: Option<AreaId>) -> Vec<ProbeWarp> {
    let Some(area) = area else {
        return Vec::new();
    };
    let content = world.resource::<Content>();
    let scripts = world.resource::<Scripts>();
    let mut warps: Vec<ProbeWarp> = area::warp::warp_objects(content, scripts, area)
        .into_iter()
        .map(|object| ProbeWarp {
            name: object.name,
            to: object.to.area,
            at: object.shape.center(),
            rect: object.shape.bounds(),
        })
        .collect();
    warps.sort_by(|a, b| a.name.cmp(&b.name));
    warps
}

fn bag(world: &World) -> Vec<Option<ProbeStack>> {
    item::my_bag(world)
        .map(|inventory| {
            inventory
                .slots
                .iter()
                .map(|slot| slot.as_ref().map(stack))
                .collect()
        })
        .unwrap_or_default()
}

fn quests(world: &World, seen: Entity) -> Option<ProbeQuests> {
    let content = world.resource::<Content>();
    let log = world.get::<QuestLog>(seen)?;
    let belongings = Belongings::by(world, seen);
    let now = time::LocalClock::of_this_machine().now;
    Some(ProbeQuests {
        active: log
            .active
            .iter()
            .map(|active| ProbeQuest {
                quest: active.quest,
                ready: active.ready(content, world.resource::<Parties>(), belongings),
                left: active.left(now),
                progress: active
                    .quest
                    .get(content)
                    .progress(content, world.resource::<Parties>(), active, belongings)
                    .iter()
                    .map(|progress| [progress.have, progress.need])
                    .collect(),
            })
            .collect(),
        finished: log
            .finished
            .iter()
            .map(|finished| ProbeFinishedQuest {
                quest: finished.quest,
                result: finished.result,
            })
            .collect(),
        tracked: log.tracked.clone(),
    })
}

fn statuses(world: &World, me: Entity) -> Vec<ProbeStatus> {
    let content = world.resource::<Content>();
    let now = time::server_now(world);
    world
        .get::<status::Statuses>(me)
        .map(|statuses| {
            statuses
                .0
                .iter()
                .map(|held| ProbeStatus {
                    effect: held.effect,
                    category: held.effect.get(content).category,
                    left: held.until.zip(now).map(|(until, now)| until.since(now)),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn skills(world: &World, me: Entity) -> Option<ProbeSkills> {
    let content = world.resource::<Content>();
    let class = crate::systems::class::of(world, me)?;
    Some(ProbeSkills {
        class: class.path(content).to_owned(),
        level: crate::systems::class::level(world, me),
        to_spend: crate::systems::class::points_to_spend(world, me).0,
        learned: world
            .get::<skill::Learned>(me)
            .map(|learned| learned.0.iter().map(rank).collect())
            .unwrap_or_default(),
        reset_offered: world.get::<skill::ResetPrompt>(me).is_some(),
    })
}

pub(super) fn rank((skill, rank): (&impl ToString, &Rank)) -> ProbeRank {
    ProbeRank {
        skill: skill.to_string(),
        rank: *rank,
    }
}

fn conversation(world: &World, me: Entity) -> Option<ProbeConversation> {
    let content = world.resource::<Content>();
    let talk = world.get::<Conversation>(me)?;
    let name = |of: Option<Entity>| Some(world.get::<actor::Name>(of?)?.name.clone());
    Some(ProbeConversation {
        step: talk.step,
        node: talk.node.clone(),
        with: talk.with,
        lines: talk
            .lines
            .iter()
            .map(|line| ProbeSaid {
                by: match line.by {
                    Speaker::Npc(npc) => Some(npc.get(content).name.to_owned()),
                    Speaker::Object => name(talk.with),
                    Speaker::Character => name(Some(me)),
                    Speaker::Narrator => None,
                },
                text: line.text.plain_words(content),
            })
            .collect(),
        choices: talk
            .choices
            .iter()
            .map(|choice| ProbeAnswer {
                label: choice.label.plain_words(content),
                gets: choice
                    .chips
                    .iter()
                    .filter_map(|chip| match chip {
                        ChoiceChip::Gets { item, .. } => Some(*item),
                        _ => None,
                    })
                    .collect(),
                locked: choice.locked,
            })
            .collect(),
    })
}

fn requests(world: &World, me: Option<Entity>) -> Vec<ProbeRequest> {
    me.and_then(|me| world.get::<SocietyView>(me))
        .map(|view| {
            view.requests
                .iter()
                .map(|card| ProbeRequest {
                    id: card.id,
                    with: card.other.name.clone(),
                    mine: card.mine,
                    says: card.says.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn letters(world: &World) -> Vec<ProbeLetter> {
    let Some(inbox) = mail::inbox(world) else {
        return Vec::new();
    };
    let content = world.resource::<Content>();
    let texts = world.resource::<LetterTexts>();
    inbox
        .rows
        .iter()
        .map(|row| ProbeLetter {
            id: row.id,
            title: row.title.clone(),
            from: match &row.from {
                InboxSender::System => None,
                InboxSender::Character(from) => Some(from.name.clone()),
            },
            read: row.read,
            attached: row
                .attached
                .iter()
                .map(|stack| stack.describe(content))
                .collect(),
            text: texts.of(row.id).map(str::to_owned),
        })
        .collect()
}

fn chat_log(world: &World) -> Vec<ProbeChatLine> {
    world
        .resource::<ChatLog>()
        .lines()
        .map(|line| ProbeChatLine {
            channel: ChatChannel::of(line.channel),
            by: match &line.kind {
                LineKind::Said(speaker) => Some(speaker.name.clone()),
                LineKind::Notice | LineKind::Failed => None,
            },
            text: line.text.clone(),
            failed: matches!(line.kind, LineKind::Failed),
        })
        .collect()
}

fn history(world: &World) -> Vec<ProbeRecord> {
    let content = world.resource::<Content>();
    world
        .resource::<HistoryBook>()
        .records()
        .iter()
        .map(|record| record_line(record, content))
        .collect()
}

fn record_line(record: &HistoryRecord, content: &Content) -> ProbeRecord {
    let text = record.entry.text.plain_words(content);
    ProbeRecord {
        topic: record.entry.topic,
        by: record.entry.by.clone(),
        text: record.entry.tally.map_or_else(
            || text.clone(),
            |tally| tally.line(&text, &messages::of(content).history),
        ),
        mark: record.entry.mark,
    }
}

fn shop(world: &World) -> Option<ProbeShop> {
    let content = world.resource::<Content>();
    let shown = shop::view(world)?;
    Some(ProbeShop {
        shop: shown.shop,
        wares: shown
            .shop
            .get(content)
            .sells()
            .zip(&shown.wares)
            .map(|(ware, view)| ProbeWare {
                item: ware.item,
                count: ware.count,
                left: view.left,
                refusal: view
                    .refusal
                    .as_ref()
                    .map(|refusal| refusal.line(content).plain_words(content)),
            })
            .collect(),
        buyback: shown
            .buyback
            .iter()
            .map(|buyback| ProbeStack {
                item: buyback.sale.item,
                count: buyback.sale.count,
            })
            .collect(),
    })
}

fn prompt(world: &World) -> Option<ProbePrompt> {
    let content = world.resource::<Content>();
    let view: &PromptView = prompt::view(world)?;
    Some(ProbePrompt {
        step: view.step,
        title: view.title.clone(),
        text: view.text.as_ref().map(|text| text.plain_words(content)),
        slots: view.slots,
        crafts: view.kind == PromptKind::Hosted,
    })
}

fn crafting(world: &World) -> Option<ProbeCrafting> {
    crafting::view(world)?;
    let stacks = |stacks: &[ItemStack]| stacks.iter().map(stack).collect();
    Some(ProbeCrafting {
        recipes: crafting::listed(world)
            .into_iter()
            .map(|listed| ProbeRecipe {
                id: listed.recipe.id.clone(),
                mastery: listed.recipe.mastery,
                locked: listed.locked,
                makes: stacks(&listed.recipe.makes),
                from: stacks(&listed.recipe.from),
                takes: listed.recipe.takes,
                chance: listed.recipe.chance,
                can_make: listed.can_make,
            })
            .collect(),
        progress: crafting::progress(world),
    })
}

fn time_of_day(world: &World) -> Option<TimeOfDayName> {
    let content = world.resource::<Content>();
    let cycle = content
        .built::<crate::systems::GameSettings>()
        .daylight
        .as_ref()?;
    let course = world.resource::<time::DayClock>().0?;
    let hour = cycle.hour(course, time::server_now(world)?);
    cycle.time_at(hour).map(|time| time.name.clone())
}

fn weather(world: &World, area: AreaId) -> Option<String> {
    let climate = &area::load(world.resource::<Content>(), area).climate;
    let spell = area::weather::spell_now(world, climate)?;
    climate.spells.get(spell).map(|spell| spell.name.clone())
}
