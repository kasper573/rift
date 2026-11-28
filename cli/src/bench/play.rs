//! The bench's players play, so a tick carries the load of a live server rather than of idle
//! characters: they join as the content's test characters and play through the game's own
//! requests, each in the role its place in its world's roster gives it.

use bevy_ecs::prelude::*;
use engine::core::chunk::ChunkIndex;
use engine::core::content::Content;
use engine::core::math::{Pos, Size};
use engine::core::units::{MeterPos, Meters};
use engine::systems::area::AreaId;
use engine::systems::area::Place;
use engine::systems::dialogue::{Conversation, ConversationRequest};
use engine::systems::from_client;
use engine::systems::interact::{self, InteractRequest};
use engine::systems::item::{Inventory, ItemDef, ItemKind, UseItemRequest};
use engine::systems::movement::{MoveRequest, MoveSeq, Position};
use engine::systems::npc::Npc;
use engine::systems::player::snapshot::{CharacterSnapshot, JoinAs, JoinOrigin, start_of};
use engine::systems::player::{ClientId, Players};
use engine::systems::skill::{
    Affects, CastAim, Hotbar, HotbarEntry, SkillDef, Targeting, UseSkillRequest,
};
use engine::systems::visibility;

/// Ticks between one fighter's casts: two seconds at the server's rate.
pub const CAST_EVERY: usize = 60;
/// Ticks between one fighter's drinks from its bag: ten seconds.
const DRINK_EVERY: usize = 300;
/// Ticks between one walker's clicks ahead: a second and a half.
const WALK_EVERY: usize = 45;
/// How far ahead a walker clicks.
const STRIDE: Meters = Meters(10.0);
/// How far from the area's edge a walker keeps.
const EDGE: Meters = Meters(2.0);
/// Ticks between one talker's words: a second, a choice read and picked.
const TALK_EVERY: usize = 30;

/// What a player does, by its place in its world's roster: as on a live server, most fight or
/// walk about, and some talk with whoever lives there.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Casts the skills on its bar at the nearest foe, and drinks what its bag holds to drink.
    Fighter,
    /// Clicks ahead across the area, a stride at a time, never standing long.
    Walker,
    /// Talks with the nearest one who talks, and picks its way through the conversation.
    Talker,
}

impl Role {
    pub const ALL: [Role; 3] = [Role::Fighter, Role::Walker, Role::Talker];

    /// Two in five fight, two walk, one talks.
    pub fn of(index: usize) -> Role {
        match index % 5 {
            0 | 1 => Role::Fighter,
            2 | 3 => Role::Walker,
            _ => Role::Talker,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Role::Fighter => "fighters",
            Role::Walker => "walkers",
            Role::Talker => "talkers",
        }
    }
}

pub fn characters_in(content: &Content, zone: AreaId) -> Vec<(CharacterSnapshot, Place)> {
    content
        .table::<CharacterSnapshot>()
        .ids()
        .map(|id| id.get(content))
        .map(|snapshot| (snapshot.clone(), start_of(content, snapshot)))
        .filter(|(_, start)| start.area == zone)
        .collect()
}

pub fn join_as(world: &mut World, conn: Entity, character: &(CharacterSnapshot, Place)) {
    let (snapshot, start) = character.clone();
    world.entity_mut(conn).insert(JoinAs {
        snapshot,
        start,
        origin: JoinOrigin::Staged,
    });
}

/// Every player whose turn it is this tick plays it, in its role, on ground of `size`. The
/// world's players are numbered from `first` among every world's, and each plays its own rounds
/// by its number, so no two worlds play out alike.
pub fn act(
    world: &mut World,
    content: &Content,
    roster: &[Entity],
    first: usize,
    tick: usize,
    size: Size<Meters>,
) {
    for (index, &conn) in roster.iter().enumerate() {
        let Some(player) = player_of(world, conn) else {
            continue;
        };
        let turn = |every: usize| (tick + index).is_multiple_of(every);
        let round = |every: usize| tick / every + first + index;
        match Role::of(index) {
            Role::Fighter => {
                if turn(CAST_EVERY) {
                    cast(world, content, roster, index, player, round(CAST_EVERY));
                }
                if turn(DRINK_EVERY) {
                    drink(world, content, conn, player);
                }
            }
            Role::Walker if turn(WALK_EVERY) => {
                walk(world, conn, player, round(WALK_EVERY), size);
            }
            Role::Talker if turn(TALK_EVERY) => talk(world, conn, player, round(TALK_EVERY)),
            Role::Walker | Role::Talker => {}
        }
    }
}

/// Casts the `round`th skill on `player`'s bar, aimed as its targeting asks.
fn cast(
    world: &mut World,
    content: &Content,
    roster: &[Entity],
    index: usize,
    player: Entity,
    round: usize,
) {
    let skills: Vec<_> = world
        .get::<Hotbar>(player)
        .map(|bar| {
            bar.0
                .iter()
                .filter_map(|entry| match entry {
                    HotbarEntry::Skill(skill) => Some(*skill),
                    HotbarEntry::Empty | HotbarEntry::Item(_) => None,
                })
                .collect()
        })
        .unwrap_or_default();
    if skills.is_empty() {
        return;
    }
    let skill = skills[round % skills.len()];
    let def: &SkillDef = skill.get(content);
    let Some(at) = world.get::<Position>(player).map(|position| position.pos) else {
        return;
    };
    let aim = match def.target {
        Targeting::Caster => CastAim::None,
        Targeting::Single {
            affects: Affects::Enemy,
            ..
        } => match nearest_npc(world, at, |_, _| true) {
            Some((npc, _)) => CastAim::Actor(npc),
            None => return,
        },
        Targeting::Single {
            affects: Affects::Ally,
            ..
        } => {
            let other = roster[(index + 1) % roster.len()];
            match player_of(world, other) {
                Some(ally) => CastAim::Actor(ally),
                None => return,
            }
        }
        Targeting::RangedArea {
            affects: Affects::Enemy,
            ..
        }
        | Targeting::MeleeArea {
            affects: Affects::Enemy,
            ..
        } => CastAim::Point(nearest_npc(world, at, |_, _| true).map_or(at, |(_, pos)| pos)),
        Targeting::RangedArea {
            affects: Affects::Ally,
            ..
        }
        | Targeting::MeleeArea {
            affects: Affects::Ally,
            ..
        }
        | Targeting::Spot { .. } => CastAim::Point(at),
    };
    let conn = roster[index];
    from_client(world, conn, UseSkillRequest { skill, aim });
}

/// Uses the first thing in `player`'s bag that is there to be used.
fn drink(world: &mut World, content: &Content, conn: Entity, player: Entity) {
    let usable = world.get::<Inventory>(player).and_then(|bag| {
        bag.stacks().find_map(|(slot, stack)| {
            let def: &ItemDef = stack.item.get(content);
            matches!(def.kind, ItemKind::Usable { .. }).then_some(slot)
        })
    });
    if let Some(slot) = usable {
        from_client(world, conn, UseItemRequest { slot: slot as u32 });
    }
}

/// Clicks a stride ahead of `player`, the way its `round` turns it, kept within ground of `size`.
fn walk(world: &mut World, conn: Entity, player: Entity, round: usize, size: Size<Meters>) {
    let Some(at) = world.get::<Position>(player).map(|position| position.pos) else {
        return;
    };
    let heading = round as f32 * GOLDEN_ANGLE;
    let to = Pos::new(
        (at.x + heading.cos() * STRIDE.0).clamp(EDGE.0, size.width - EDGE.0),
        (at.y + heading.sin() * STRIDE.0).clamp(EDGE.0, size.height - EDGE.0),
    );
    from_client(
        world,
        conn,
        MoveRequest {
            pos: to,
            seq: MoveSeq(round as u32),
        },
    );
}

/// A turn that never comes round to the same heading, so a walker's clicks fan out over the
/// ground rather than pace a line.
const GOLDEN_ANGLE: f32 = 2.399_963;

/// Picks the `round`th choice of the conversation `player` is in, or leaves one that offers none;
/// out of one, walks up to the nearest one who talks.
fn talk(world: &mut World, conn: Entity, player: Entity, round: usize) {
    if let Some(conversation) = world.get::<Conversation>(player) {
        let step = conversation.step;
        let request = match conversation.choices.len() {
            0 => ConversationRequest::Leave { step },
            choices => ConversationRequest::Pick {
                step,
                choice: (round % choices) as u32,
            },
        };
        from_client(world, conn, request);
        return;
    }
    let Some(at) = world.get::<Position>(player).map(|position| position.pos) else {
        return;
    };
    let talking = |world: &World, npc: Entity| interact::interaction_of(world, npc).is_some();
    if let Some((target, _)) = nearest_npc(world, at, talking) {
        from_client(world, conn, InteractRequest { target });
    }
}

pub fn player_of(world: &World, conn: Entity) -> Option<Entity> {
    let client = world.get::<ClientId>(conn)?;
    world.resource::<Players>().0.get(client).copied()
}

/// The nearest npc the player's client is shown that `fits`, as a player would pick one.
fn nearest_npc(
    world: &World,
    at: Pos<Meters>,
    fits: impl Fn(&World, Entity) -> bool,
) -> Option<(Entity, Pos<Meters>)> {
    world
        .resource::<ChunkIndex>()
        .around(at, visibility::sight(world))
        .filter(|&near| world.get::<Npc>(near).is_some() && fits(world, near))
        .filter_map(|near| Some((near, world.get::<Position>(near)?.pos)))
        .min_by(|a, b| a.1.distance(at).0.total_cmp(&b.1.distance(at).0))
}
