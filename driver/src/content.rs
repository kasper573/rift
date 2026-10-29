use std::ops::RangeInclusive;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use engine::core::content::{
    Content, ContentFile, ContentId, ContentScope, FilesystemFolder, with_content,
};
use engine::core::math::{Offset, Pos, Rect};
use engine::core::units::{MeterPos, Meters};
use engine::systems::actor::model::ModelId;
use engine::systems::actor::stance::HandIndex;
use engine::systems::area::{self, Area, AreaDef, AreaId, AreaModule, PropDef};
use engine::systems::input::map::{InputAction, InputMap};
use engine::systems::item::{ItemId, Wear};
use engine::systems::movement::NavMesh;
use engine::systems::npc::NpcId;
use engine::systems::probe::ProbeSnapshot;
use engine::systems::script::Place;
use engine::systems::{GameSettings, load_content};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::seen::clickable_point;

/// The content the driver reads names and places from: the one the server it drives plays.
pub fn content() -> &'static Content {
    CONTENT
        .get()
        .expect("a session loads the content before anything reads it")
}

pub(crate) fn load(folder: &Path) {
    CONTENT.get_or_init(|| {
        load_content(
            Arc::new(FilesystemFolder(folder.to_path_buf())),
            "the driver did not start",
        )
        .unwrap_or_else(|report| panic!("{report}"))
    });
}

static CONTENT: OnceLock<Content> = OnceLock::new();

pub fn input_name(action: InputAction) -> String {
    input_names(action)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("{action:?} has no binding"))
}

pub fn input_names(action: InputAction) -> Vec<String> {
    InputMap::new(content())
        .bindings(action)
        .iter()
        .map(|binding| binding.name())
        .collect()
}

/// The one file of `Row`'s type named `name`, wherever in the game folder it is.
pub fn id<Row: ContentFile>(name: &str) -> ContentId<Row> {
    let content = content();
    let mut named = content.ids::<Row>().filter(|id| id.name(content) == name);
    match (named.next(), named.next()) {
        (Some(id), None) => id,
        (None, _) => panic!("no {} named {name}", Row::TABLE),
        (Some(_), Some(_)) => panic!("more than one {} named {name}", Row::TABLE),
    }
}

/// The path of the one file of the type `table` named `name`.
pub fn path_named(table: &str, name: &str) -> &'static str {
    let content = content();
    let files = content
        .table_named(table)
        .unwrap_or_else(|| panic!("no content type {table}"));
    let mut named = files
        .paths()
        .iter()
        .filter(|path| engine::core::content::name_of(path) == name);
    match (named.next(), named.next()) {
        (Some(path), None) => path,
        (None, _) => panic!("no {table} named {name}"),
        (Some(_), Some(_)) => panic!("more than one {table} named {name}"),
    }
}

/// Content ids travel by path, so JSON is read and written within the content's scope.
pub fn to_json(value: &impl Serialize) -> String {
    let (json, _) = with_content(ContentScope::of(content()), || {
        serde_json::to_string(value).expect("serializable")
    });
    json
}

pub fn from_json<T: DeserializeOwned>(json: &str) -> T {
    let (value, _) = with_content(ContentScope::of(content()), || serde_json::from_str(json));
    value.unwrap_or_else(|error| panic!("unreadable {}: {error}", std::any::type_name::<T>()))
}

pub fn area_named(name: &str) -> &'static Area {
    area::load(content(), id::<AreaDef>(name))
}

/// Whether the item is held in a hand, putting its wearer in the stance named `stance`.
pub fn held_in_stance(item: ItemId, stance: &str) -> bool {
    let content = content();
    matches!(
        item.get(content).wear(),
        Some(Wear::Held { stance: held, .. }) if held.name(content) == stance
    )
}

/// What a body of `model` calls its `hand`, as the equipment window names it.
pub fn hand_name(model: ModelId, hand: HandIndex) -> String {
    let content = content();
    let skeleton = model.get(content).skeleton.get(content);
    let name = &skeleton
        .hands
        .get(usize::from(hand.0))
        .unwrap_or_else(|| panic!("a body of {} has no hand {}", model.name(content), hand.0))
        .name;
    let mut letters = name.chars();
    letters
        .next()
        .map(|first| first.to_uppercase().chain(letters).collect())
        .unwrap_or_default()
}

/// Where the townsperson lives: standing there puts a player at their feet.
pub fn resident_home(npc: &str) -> Place {
    area::residence(content(), id(npc))
        .unwrap_or_else(|| panic!("nobody named {npc:?} lives anywhere"))
}

/// The middle of the ground where the monster roams: its own corner of the area, or all of it.
pub fn hunting_ground(npc: &str) -> Place {
    roaming(npc)
        .next()
        .map(|(area, roams)| Place {
            area,
            at: roams
                .unwrap_or_else(|| Rect::from_size(area::load(content(), area).size))
                .center(),
        })
        .unwrap_or_else(|| panic!("no {npc} roams anywhere"))
}

/// The middle of the tightest ground a pack of the monster keeps to: where several of them stand
/// close enough together for one cast to reach them all.
pub fn pack_ground(npc: &str) -> Place {
    roaming(npc)
        .filter_map(|(area, roams)| Some((area, roams?)))
        .min_by(|(_, a), (_, b)| {
            (a.size.width * a.size.height).total_cmp(&(b.size.width * b.size.height))
        })
        .map(|(area, roams)| Place {
            area,
            at: roams.center(),
        })
        .unwrap_or_else(|| panic!("no {npc} keeps to a ground of its own anywhere"))
}

/// A place `by` metres east of a spot and a metre down from its row, clear of whoever stands there.
pub fn east_of(spot: Place, by: f32) -> Place {
    Place {
        area: spot.area,
        at: spot.at + Offset::new(by, 1.0),
    }
}

/// A marked spot: the one object of the name in the game, or where the townsperson of the name
/// lives.
pub fn marker_at(name: &str) -> Place {
    let mut marked = content()
        .ids::<AreaDef>()
        .filter_map(|area| placed_object(area, name).map(|at| Place { area, at }));
    match (marked.next(), marked.next()) {
        (Some(place), None) => place,
        (None, _) => resident_home(name),
        (Some(_), Some(_)) => panic!("more than one object named {name}"),
    }
}

/// The walkable ground nearest `spot` that its terrain lays a picture of its own over.
pub fn pictured_ground(spot: Place) -> Pos<Meters> {
    let content = content();
    let area = area::load(content, spot.area);
    let pictured = |at: Pos<Meters>| {
        area.terrains
            .get(usize::from(area.paint.at(at)))
            .is_some_and(|terrain| terrain.get(content).picture.is_some())
    };
    ground_near(spot, Meters(0.0)..=Meters(f32::INFINITY))
        .into_iter()
        .find(|at| pictured(*at))
        .unwrap_or_else(|| panic!("no ground with a picture of its own near {spot:?}"))
}

/// Where the props of `spot`'s area that `which` picks stand, nearest `spot` first.
pub fn props_near(spot: Place, which: impl Fn(&PropDef) -> bool) -> Vec<Pos<Meters>> {
    let content = content();
    let mut props: Vec<Pos<Meters>> = area::load(content, spot.area)
        .props
        .iter()
        .filter(|placement| which(placement.prop.get(content)))
        .map(|placement| placement.at)
        .collect();
    props.sort_by(|a, b| a.distance_to(spot.at).total_cmp(&b.distance_to(spot.at)));
    props
}

/// Where the lamps of the spot's area stand, nearest the spot first.
pub fn lamps_near(spot: Place) -> Vec<Pos<Meters>> {
    let mut lamps: Vec<Pos<Meters>> = area::load(content(), spot.area)
        .lights
        .iter()
        .map(|light| light.at)
        .collect();
    lamps.sort_by(|a, b| a.distance_to(spot.at).total_cmp(&b.distance_to(spot.at)));
    lamps
}

/// Where the game starts a new player.
pub fn initial_spawn() -> Place {
    content().built::<GameSettings>().initial_spawn
}

/// Ground in view beside the player, farthest from anyone first, so nobody strolls under the
/// pointer.
pub fn open_ground_beside(snapshot: &ProbeSnapshot) -> Vec<Pos<Meters>> {
    let me = snapshot.me.as_ref().expect("the player").at;
    let room = |ground: &Pos<Meters>| {
        snapshot
            .actors
            .iter()
            .map(|actor| actor.at.distance_to(*ground))
            .fold(f32::INFINITY, f32::min)
    };
    let mut ground: Vec<Pos<Meters>> = ground_near(
        Place {
            area: snapshot.area.expect("an area"),
            at: me,
        },
        Meters(2.0)..=Meters(4.0),
    )
    .into_iter()
    .filter(|ground| clickable_point(snapshot, *ground).is_some())
    .collect();
    ground.sort_by(|a, b| room(b).total_cmp(&room(a)));
    ground
}

/// Nearest first, and only what can be walked to from the spot.
pub fn ground_near(spot: Place, within: RangeInclusive<Meters>) -> Vec<Pos<Meters>> {
    let at = spot.at;
    let mesh = &area::load(content(), spot.area).mesh;
    let from = mesh.nearest(at).expect("walkable ground near");
    let mut ground: Vec<Pos<Meters>> = lattice(mesh)
        .filter(|ground| mesh.connects(from, *ground) && within.contains(&ground.distance(at)))
        .collect();
    ground.sort_by(|a, b| a.distance_to(at).total_cmp(&b.distance_to(at)));
    ground
}

pub(crate) fn placed_object(area: AreaId, object: &str) -> Option<Pos<Meters>> {
    area::objects(area::placed(content(), area))
        .find(|(_, def)| def.name.as_deref() == Some(object))
        .map(|(_, def)| def.shape.center())
}

/// The walkable points of the mesh on whole metres, the candidates a chapter picks ground from.
pub fn lattice(mesh: &NavMesh) -> impl Iterator<Item = Pos<Meters>> + '_ {
    let bounds = mesh.bounds();
    let (min, max) = (bounds.min(), bounds.max());
    let xs = (min.x.ceil() as i32)..=(max.x.floor() as i32);
    let ys = (min.y.ceil() as i32)..=(max.y.floor() as i32);
    ys.flat_map(move |y| xs.clone().map(move |x| Pos::new(x as f32, y as f32)))
        .filter(move |point| mesh.walkable(*point))
}

fn roaming(npc: &str) -> impl Iterator<Item = (AreaId, Option<Rect<Meters>>)> {
    let npc: NpcId = id(npc);
    content().ids::<AreaDef>().flat_map(move |area| {
        area::modules(content(), area).filter_map(move |module| match module {
            AreaModule::Population(population) if population.npc == npc => {
                Some((area, population.roams))
            }
            _ => None,
        })
    })
}
