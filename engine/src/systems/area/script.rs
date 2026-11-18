//! How areas, their objects and their placed blueprints own scripts, and what scripts may do with
//! places.

use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::query;
use super::scene::scene_file;
use super::{AreaDef, AreaId, AreaModule, AreaObject};
use crate::core::content::{Content, ContentProblem, Loading, SchemaMark};
use crate::core::ground::TileAt;
use crate::core::math::Pos;
use crate::core::script::{
    Bind, EventDef, EventKind, Instances, Owner, OwnerRow, PlaceDef, ReachDef, RunnerKind,
    RunnerSpec, ScriptSite, Scripts, Vocabulary, owner_label, owner_site,
};
use crate::core::units::Meters;
use crate::systems::WorldArea;
use crate::systems::area_instance::EntersInstance;
use crate::systems::npc::{NPC_RUNNER, Npc};

pub const AREA_RUNNER: RunnerKind = RunnerKind::new("area");
pub const OBJECT_RUNNER: RunnerKind = RunnerKind::new("object");
pub const BLUEPRINT_RUNNER: RunnerKind = RunnerKind::new("blueprint");

/// The owners something is placed as: they start as the ground they stand on is reached.
pub const PLACED_RUNNERS: &[RunnerKind] = &[
    OBJECT_RUNNER,
    super::spawnable::SPAWNABLE_RUNNER,
    NPC_RUNNER,
    AREA_RUNNER,
    BLUEPRINT_RUNNER,
];

/// The start of a world, or of what is placed in it: raised once, as the owner begins.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Started {}

impl EventDef for Started {
    fn owners(&self) -> &'static [RunnerKind] {
        PLACED_RUNNERS
    }

    fn has_subject(&self) -> bool {
        false
    }
}

/// A place in the game: a point of an area.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(from = "PlaceRow", into = "PlaceRow")]
#[schemars(inline)]
pub struct Place {
    pub area: AreaId,
    pub at: Pos<Meters>,
}

#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(inline, transform = SchemaMark::Place.with(true))]
struct PlaceRow {
    area: AreaId,
    x: f32,
    y: f32,
}

impl From<PlaceRow> for Place {
    fn from(row: PlaceRow) -> Place {
        Place {
            area: row.area,
            at: Pos::new(row.x, row.y),
        }
    }
}

impl From<Place> for PlaceRow {
    fn from(place: Place) -> PlaceRow {
        PlaceRow {
            area: place.area,
            x: place.at.x,
            y: place.at.y,
        }
    }
}

impl PlaceDef for Place {}

/// A script that may send someone to a place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Warps(pub Place);

impl ReachDef for Warps {}

impl OwnerRow {
    pub fn of_area(area: AreaId) -> OwnerRow {
        OwnerRow {
            kind: AREA_RUNNER,
            row: area.raw(),
            object: None,
        }
    }

    pub fn of_object(area: AreaId, index: u32) -> OwnerRow {
        OwnerRow {
            kind: OBJECT_RUNNER,
            row: area.raw(),
            object: Some(index),
        }
    }

    pub fn of_placement(area: AreaId, ordinal: u32) -> OwnerRow {
        OwnerRow {
            kind: BLUEPRINT_RUNNER,
            row: area.raw(),
            object: Some(ordinal),
        }
    }
}

/// Registers the owners of the area's scripts, and what scripts say of places.
pub fn scripting(vocabulary: &mut Vocabulary) {
    query::scripting(vocabulary);
    vocabulary
        .runner(AREA_RUNNER, RunnerSpec::at(area_site, area_name))
        .runner(OBJECT_RUNNER, RunnerSpec::at(area_site, area_name))
        .runner(BLUEPRINT_RUNNER, RunnerSpec::at(blueprint_site, area_name))
        .event::<Started>("started")
        .place::<Place>()
        .binder(bind);
    super::object::scripting(vocabulary);
    super::warp::scripting(vocabulary);
    super::transition::scripting(vocabulary);
    super::spawnable::scripting(vocabulary);
    super::sky::script(vocabulary);
    super::vfx::scripting(vocabulary);
}

/// The owner of the scripts of the area this world serves.
pub fn area_owner(world: &World) -> Owner {
    Owner::Row(AREA_RUNNER, world.resource::<WorldArea>().0.raw())
}

/// The owner a script's subject is, when it is an npc or an object.
pub fn owner_of(world: &World, entity: Entity) -> Option<Owner> {
    if world.get::<Npc>(entity).is_some() {
        return Some(Owner::Entity(NPC_RUNNER, entity));
    }
    if world.get::<AreaObject>(entity).is_some() {
        return Some(Owner::Entity(OBJECT_RUNNER, entity));
    }
    None
}

/// Starts the scripts of the area this world serves.
pub fn spawn_area(world: &mut World) {
    let scripts = world.resource::<Scripts>().clone();
    let area = world.resource::<WorldArea>().0;
    let owner = Owner::Row(AREA_RUNNER, area.raw());
    let Some(procedures) = scripts.0.get(&OwnerRow::of_area(area)) else {
        return;
    };
    let mut instances = world.resource_mut::<Instances>();
    instances.spawn_owner(owner, procedures.clone());
    instances.raise(owner, EventKind::of::<Started>(), None, Vec::new(), |_| {
        true
    });
}

/// Starts the scripts of the blueprints placed on the tile at `tile`: each placement owns its
/// scripts, and they start as the ground they stand on is first reached.
pub fn spawn_placed(world: &mut World, tile: TileAt) {
    let scripts = world.resource::<Scripts>().clone();
    let content = world.resource::<Content>().clone();
    let area = world.resource::<WorldArea>().0;
    let mut instances = world.resource_mut::<Instances>();
    for ordinal in super::load(&content, area).placements_in(tile) {
        let procedures = scripts.of(OwnerRow::of_placement(area, *ordinal));
        if procedures.is_empty() {
            continue;
        }
        let owner = Owner::Row(BLUEPRINT_RUNNER, *ordinal);
        instances.spawn_owner(owner, procedures.to_vec());
        instances.raise(owner, EventKind::of::<Started>(), None, Vec::new(), |_| {
            true
        });
    }
}

/// Checks that every place a script may send someone to is somewhere they can stand.
pub fn check_places(loading: &mut Loading) {
    if !loading.thorough() {
        return;
    }
    let content = loading.content();
    let scripts = content.built::<Scripts>();
    for reached in scripts.reaching::<Warps>() {
        report(loading, &content, &reached, "warps to", reached.reach.0);
    }
    for reached in scripts.reaching::<EntersInstance>() {
        report(
            loading,
            &content,
            &reached,
            "enters an instance at",
            reached.reach.to,
        );
        report(
            loading,
            &content,
            &reached,
            "returns to",
            reached.reach.return_to,
        );
    }
}

fn report<R>(
    loading: &mut Loading,
    content: &Content,
    reached: &crate::core::script::Reached<R>,
    does: &str,
    place: Place,
) {
    if let Err(problem) = super::check_place(content, place) {
        let site = owner_site(content, reached.row);
        loading.json_error(
            &site.file,
            &site.pointer,
            ContentProblem::new(
                problem.rule,
                format!("{} {does} {}", reached.label, problem.found),
                problem.expected,
            )
            .near(problem.nearest),
        );
    }
}

fn area_site(content: &Content, row: OwnerRow) -> ScriptSite {
    let area: AreaId = row.id();
    match row.object {
        Some(index) => {
            let (file, pointer) = super::placed(content, area)[index as usize].site(content, area);
            ScriptSite { file, pointer }
        }
        None => ScriptSite {
            file: scene_file(area.path(content)),
            pointer: String::new(),
        },
    }
}

fn area_name(content: &Content, row: OwnerRow) -> Option<String> {
    let area: AreaId = row.id();
    Some(area.get(content).name.clone())
}

fn blueprint_site(content: &Content, row: OwnerRow) -> ScriptSite {
    let area: AreaId = row.id();
    let placement = row
        .object
        .and_then(|ordinal| super::placements(content, area).get(ordinal as usize));
    ScriptSite {
        file: placement.map_or_else(String::new, |placement| {
            scene_file(placement.blueprint.path(content))
        }),
        pointer: String::new(),
    }
}

fn bind(loading: &mut Loading, content: &Content, bind: &mut Bind) {
    for (id, _) in loading.rows::<AreaDef>(content) {
        let key = id.path(content);
        let placed = super::placed(content, id);
        let scene = scene_file(key);
        let own = placed
            .iter()
            .filter_map(|placed| match &placed.module {
                AreaModule::Script(script) => Some((
                    ScriptSite {
                        file: scene.clone(),
                        pointer: placed.origin.clone(),
                    },
                    script,
                )),
                _ => None,
            })
            .collect();
        bind.bind(content, OwnerRow::of_area(id), owner_label(key, None), own);
        for (index, object) in super::objects(placed) {
            let name = object
                .name
                .as_deref()
                .or(object.title.as_deref())
                .unwrap_or("object");
            let (file, pointer) = placed[index as usize].site(content, id);
            let modules = object
                .scripts
                .iter()
                .enumerate()
                .map(|(at, module)| {
                    (
                        ScriptSite {
                            file: file.clone(),
                            pointer: format!("{pointer}/scripts/{at}"),
                        },
                        module,
                    )
                })
                .collect();
            bind.bind(
                content,
                OwnerRow::of_object(id, index),
                owner_label(key, Some(name)),
                modules,
            );
        }
        for (ordinal, placement) in super::placements(content, id).iter().enumerate() {
            let blueprint = placement.blueprint.path(content);
            let modules = placement
                .scripts
                .iter()
                .map(|(pointer, module)| {
                    (
                        ScriptSite {
                            file: scene_file(blueprint),
                            pointer: pointer.clone(),
                        },
                        module,
                    )
                })
                .collect();
            bind.bind(
                content,
                OwnerRow::of_placement(id, ordinal as u32),
                owner_label(blueprint, Some(&format!("{key} {ordinal}"))),
                modules,
            );
        }
    }
}
