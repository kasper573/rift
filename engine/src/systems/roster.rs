//! Who stands where as a tick begins, and the scans of who is near, for what asks of actors around a
//! spot: scripts, vision, skills.

use bevy_ecs::prelude::*;
use bevy_ecs::query::Has;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::Content;
use crate::core::math::Pos;
use crate::core::script::{
    Compiler, Eval, Expr, FlowCx, QueryDef, ScriptFault, Value, ValueType, Vocabulary,
};
use crate::core::spatial::SpatialIndex;
use crate::core::text::LineText;
use crate::core::tick::{Cached, Planner};
use crate::core::units::{MeterPos, Meters};
use crate::systems::area::{self, Area, AreaId, AreaTag};
use crate::systems::combat::Attackers;
use crate::systems::movement::{self, Position};
use crate::systems::npc::{Npc, NpcId, Pack, Packs};
use crate::systems::player::{ArrivalHold, PlayedBy};
use crate::systems::requirement::Describers;
use crate::systems::stat::{self, Stats};
use crate::systems::visibility::{self, Presence};

/// Who stood where as the tick began. Every script in a tick counts the same actors, whatever order
/// the scripts run in. Each is filed by the cell of the ground it stands in, so a query near a spot
/// passes over only those standing in the cells around it.
#[derive(Resource)]
pub struct TickRoster {
    /// The characters, then the npcs.
    listed: Vec<Listed>,
    characters: usize,
    /// Where the npcs are listed before they join `listed`, kept for its capacity.
    npcs: Vec<Listed>,
    /// The characters' spots and the npcs' apart, so a scan for one kind reaching none ends at
    /// once.
    character_spots: SpatialIndex,
    npc_spots: SpatialIndex,
}

impl Default for TickRoster {
    fn default() -> TickRoster {
        TickRoster {
            listed: Vec::new(),
            characters: 0,
            npcs: Vec::new(),
            character_spots: SpatialIndex::new(SPOT_CELL),
            npc_spots: SpatialIndex::new(SPOT_CELL),
        }
    }
}

const SPOT_CELL: Meters = Meters(16.0);

#[derive(Clone, Copy)]
pub struct Listed {
    pub entity: Entity,
    pub at: Pos<Meters>,
    pub area: AreaId,
    pub npc: Option<NpcId>,
    pub character: bool,
    pub dead: bool,
    pub held: bool,
    pub presence: bool,
}

impl TickRoster {
    pub fn iter(&self) -> impl Iterator<Item = &Listed> {
        self.listed.iter()
    }

    /// The actors `among` that may stand within `within` of `from`, in no particular order: every
    /// one that does, and some a little farther.
    pub fn near(
        &self,
        among: Among,
        from: Pos<Meters>,
        within: Meters,
    ) -> impl Iterator<Item = &Listed> {
        let characters = matches!(
            among,
            Among::Characters | Among::Anyone | Among::Attackers | Among::PackAttackers
        );
        let npcs = among != Among::Characters;
        characters
            .then(|| self.character_spots.near(from, within))
            .into_iter()
            .flatten()
            .chain(
                npcs.then(|| {
                    self.npc_spots
                        .near(from, within)
                        .map(|index| index + self.characters)
                })
                .into_iter()
                .flatten(),
            )
            .map(|index| &self.listed[index])
    }
}

/// Lists who stands where as the tick begins, for every script run this tick.
#[derive(Default)]
pub struct ListRoster {
    query: Cached<Listing, Actors>,
    roster: TickRoster,
}

/// Players and npcs: who the roster lists.
type Actors = Or<(With<PlayedBy>, With<Npc>)>;

/// What the roster lists of each actor.
type Listing = (
    Entity,
    &'static Position,
    &'static AreaTag,
    Option<&'static Npc>,
    Option<&'static Stats>,
    Has<PlayedBy>,
    Has<ArrivalHold>,
    Has<Presence>,
);

impl Planner for ListRoster {
    fn prepare(&mut self, world: &mut World) {
        self.query.ready(world);
    }

    fn plan(&mut self, world: &World) {
        let TickRoster {
            listed,
            characters,
            npcs,
            character_spots,
            npc_spots,
        } = &mut self.roster;
        listed.clear();
        npcs.clear();
        for (entity, at, tag, npc, stats, character, held, presence) in
            self.query.query(world).iter()
        {
            let entry = Listed {
                entity,
                at: at.pos,
                area: tag.area,
                npc: npc.map(|npc| npc.def),
                character,
                dead: stats.is_some_and(stat::dead),
                held,
                presence,
            };
            if character {
                listed.push(entry);
            } else {
                npcs.push(entry);
            }
        }
        *characters = listed.len();
        rayon::join(
            || npc_spots.refile_spots(npcs.iter().map(|entry| entry.at)),
            || character_spots.refile_spots(listed.iter().map(|entry| entry.at)),
        );
        listed.append(npcs);
    }

    fn apply(&mut self, world: &mut World) {
        let mut held = world.get_resource_or_init::<TickRoster>();
        std::mem::swap(&mut self.roster, &mut *held);
    }
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Among {
    Characters,
    Npcs,
    Attackers,
    PackAttackers,
    Anyone,
}

pub fn nearest(
    world: &World,
    cx: &FlowCx,
    among: Among,
    npc: Option<NpcId>,
    within: Meters,
) -> Option<Entity> {
    around(world, cx, among, npc, within)
        .min_by(|(a, da), (b, db)| da.0.total_cmp(&db.0).then(a.cmp(b)))
        .map(|(entity, _)| entity)
}

pub fn around<'w>(
    world: &'w World,
    cx: &FlowCx,
    among: Among,
    npc: Option<NpcId>,
    within: Meters,
) -> impl Iterator<Item = (Entity, Meters)> + 'w {
    scanned(world, Scan::of(world, cx, among), among, npc, within)
}

/// Everyone of `among` within `within` of where `scan` looks from.
fn scanned<'w>(
    world: &'w World,
    scan: Option<Scan<'w>>,
    among: Among,
    npc: Option<NpcId>,
    within: Meters,
) -> impl Iterator<Item = (Entity, Meters)> + 'w {
    let candidates = scan
        .as_ref()
        .map(|scan| (scan.roster, scan.from))
        .into_iter()
        .flat_map(move |(roster, from)| roster.near(among, from, within));
    candidates.filter_map(move |listed| {
        let scan = scan.as_ref()?;
        let fits = Some(listed.entity) != scan.owner
            && listed.area == scan.area
            && !listed.dead
            && !listed.held
            && match among {
                Among::Characters => listed.character,
                Among::Npcs => listed.npc.is_some(),
                Among::Anyone => true,
                Among::Attackers | Among::PackAttackers => scan.attackers.contains(&listed.entity),
            }
            && npc.is_none_or(|npc| listed.npc == Some(npc));
        if !fits {
            return None;
        }
        let distance = scan.from.distance(listed.at);
        if distance > within {
            return None;
        }
        let seen = match scan.owner {
            Some(owner) => {
                !(scan.owner_presence || listed.presence)
                    || visibility::coexist(world, owner, listed.entity)
            }
            None => !listed.presence,
        };
        let exposed = scan.shelter.is_none_or(|area| !area.safe(listed.at));
        (seen && exposed).then_some((listed.entity, distance))
    })
}

struct Scan<'w> {
    roster: &'w TickRoster,
    /// Who looks, when someone does rather than a point of the world.
    owner: Option<Entity>,
    from: Pos<Meters>,
    area: AreaId,
    shelter: Option<&'w Area>,
    attackers: Vec<Entity>,
    owner_presence: bool,
}

impl<'w> Scan<'w> {
    fn of(world: &'w World, cx: &FlowCx, among: Among) -> Option<Scan<'w>> {
        let owner = cx.owner?;
        let from = movement::position(world, owner)?;
        let area = world.get::<AreaTag>(owner)?.area;
        let roster = world.get_resource::<TickRoster>()?;
        let shelter =
            area::wild(world, owner).then(|| area::load(world.resource::<Content>(), area));
        let attackers: Vec<Entity> = match among {
            Among::Attackers => world
                .get::<Attackers>(owner)
                .map(|attackers| attackers.ids.clone())
                .unwrap_or_default(),
            Among::PackAttackers => {
                let packs = world.resource::<Packs>();
                let members = match world.get::<Pack>(owner) {
                    Some(pack) => packs.members(*pack),
                    None => std::slice::from_ref(&owner),
                };
                members
                    .iter()
                    .filter(|&&member| {
                        world.get::<AreaTag>(member).map(|tag| tag.area) == Some(area)
                    })
                    .flat_map(|&member| {
                        world
                            .get::<Attackers>(member)
                            .map(|attackers| attackers.ids.clone())
                            .unwrap_or_default()
                    })
                    .collect()
            }
            _ => Vec::new(),
        };
        Some(Scan {
            roster,
            owner: Some(owner),
            from,
            area,
            shelter,
            attackers,
            owner_presence: world.get::<Presence>(owner).is_some(),
        })
    }

    /// A look from the point `from` of this world, which nobody attacks and nothing shelters.
    fn at(world: &'w World, from: Pos<Meters>, among: Among) -> Option<Scan<'w>> {
        if matches!(among, Among::Attackers | Among::PackAttackers) {
            return None;
        }
        Some(Scan {
            roster: world.get_resource::<TickRoster>()?,
            owner: None,
            from,
            area: world.get_resource::<crate::systems::WorldArea>()?.0,
            shelter: None,
            attackers: Vec::new(),
            owner_presence: false,
        })
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Nearest {
    pub among: Among,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npc: Option<NpcId>,
    #[schemars(extend("default" = 6.0))]
    pub within: Box<Expr>,
}

impl QueryDef for Nearest {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Actor)
    }

    fn reads_subject(&self) -> bool {
        false
    }

    fn is_shared(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        let Nearest { among, npc, within } = self;
        Ok({
            let (among, npc) = (*among, *npc);
            let within = cx.expr(within, ValueType::Number)?;
            Box::new(move |world, cx| {
                let within = Meters(within(world, cx).as_num() as f32);
                Value::Actor(nearest(world, cx, among, npc, within))
            })
        })
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Count {
    pub among: Among,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npc: Option<NpcId>,
    #[schemars(extend("default" = 6.0))]
    pub within: Box<Expr>,
}

impl QueryDef for Count {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Integer)
    }

    fn reads_subject(&self) -> bool {
        false
    }

    fn is_shared(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        let Count { among, npc, within } = self;
        Ok({
            let (among, npc) = (*among, *npc);
            let within = cx.expr(within, ValueType::Number)?;
            Box::new(move |world, cx| {
                let within = Meters(within(world, cx).as_num() as f32);
                Value::Int(around(world, cx, among, npc, within).count() as i64)
            })
        })
    }
}

/// Everyone of `among` within `within` of the point `from`, or of the owner without one.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Around {
    pub among: Among,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npc: Option<NpcId>,
    #[schemars(extend("default" = 6.0))]
    pub within: Box<Expr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<Box<Expr>>,
}

impl QueryDef for Around {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Actors)
    }

    fn reads_subject(&self) -> bool {
        false
    }

    fn is_shared(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        let Around {
            among,
            npc,
            within,
            from,
        } = self;
        Ok({
            let (among, npc) = (*among, *npc);
            let within = cx.expr(within, ValueType::Number)?;
            let from = cx.opt_expr(from.as_deref(), ValueType::Point)?;
            Box::new(move |world, cx| {
                let within = Meters(within(world, cx).as_num() as f32);
                let scan = match &from {
                    Some(from) => match from(world, cx) {
                        Value::Point(Some(at)) => Scan::at(world, at, among),
                        _ => None,
                    },
                    None => Scan::of(world, cx, among),
                };
                Value::Actors(
                    scanned(world, scan, among, npc, within)
                        .map(|(actor, _)| actor)
                        .collect(),
                )
            })
        })
    }
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary
        .query::<Nearest>("nearest")
        .query::<Count>("count")
        .query::<Around>("around");
    vocabulary
        .extension::<Describers>()
        .alone::<Nearest>(|_, d| LineText::plain(d.words().nearest()))
        .alone::<Count>(|_, d| LineText::plain(d.words().count()))
        .alone::<Around>(|_, d| LineText::plain(d.words().around()));
}
