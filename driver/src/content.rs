use std::ops::RangeInclusive;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use engine::core::content::{
    Content, ContentFile, ContentId, ContentScope, FilesystemFolder, with_content,
};
use engine::core::math::{Offset, Pos, Rect};
use engine::core::units::{MeterPos, Meters};
use engine::systems::area::{self, Area, AreaDef, AreaModule};
use engine::systems::input::map::{InputAction, InputMap};
use engine::systems::load_content;
use engine::systems::movement::NavMesh;
use engine::systems::probe::ProbeSnapshot;
use engine::systems::script::Place;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::seen::{area_name, clickable_point};

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

/// Where the townsperson lives: standing there puts a player at their feet.
pub fn resident_home(area: &str, npc: &str) -> Place {
    area::residence(content(), id(npc))
        .filter(|home| home.area == id(area))
        .unwrap_or_else(|| panic!("nobody named {npc:?} lives in {area}"))
}

/// The middle of the ground where the monster roams: its own corner of the area, or all of it.
pub fn hunting_ground(area: &str, npc: &str) -> Pos<Meters> {
    let npc = id(npc);
    let whole = Rect::from_size(area_named(area).size);
    area::modules(content(), id(area))
        .find_map(|module| match module {
            AreaModule::Population(population) if population.npc == npc => {
                Some(population.roams.unwrap_or(whole).center())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no {npc:?} roams {area}"))
}

/// The middle of the tightest ground a pack of the monster keeps to: where several of them stand
/// close enough together for one cast to reach them all.
pub fn pack_ground(area: &str, npc: &str) -> Pos<Meters> {
    let npc = id(npc);
    area::modules(content(), id(area))
        .filter_map(|module| match module {
            AreaModule::Population(population) if population.npc == npc => population.roams,
            _ => None,
        })
        .min_by(|a, b| (a.size.width * a.size.height).total_cmp(&(b.size.width * b.size.height)))
        .map(|roams| roams.center())
        .unwrap_or_else(|| panic!("no {npc:?} keeps to a ground in {area}"))
}

/// A place `by` metres east of a spot and a metre down from its row, clear of whoever stands there.
pub fn east_of(area: &str, spot: Pos<Meters>, by: f32) -> Place {
    Place {
        area: id(area),
        at: spot + Offset::new(by, 1.0),
    }
}

/// A marked spot: a named object, or where a townsperson lives.
pub fn marker_at(area: &str, name: &str) -> Pos<Meters> {
    placed_object(area, name).unwrap_or_else(|| resident_home(area, name).at)
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
        area_name(snapshot).expect("an area"),
        me,
        Meters(2.0)..=Meters(4.0),
    )
    .into_iter()
    .filter(|ground| clickable_point(snapshot, *ground).is_some())
    .collect();
    ground.sort_by(|a, b| room(b).total_cmp(&room(a)));
    ground
}

/// Nearest first, and only what can be walked to from the spot.
pub fn ground_near(
    area: &str,
    at: Pos<Meters>,
    within: RangeInclusive<Meters>,
) -> Vec<Pos<Meters>> {
    let mesh = &area_named(area).mesh;
    let from = mesh.nearest(at).expect("walkable ground near");
    let mut ground: Vec<Pos<Meters>> = lattice(mesh)
        .filter(|ground| mesh.connects(from, *ground) && within.contains(&ground.distance(at)))
        .collect();
    ground.sort_by(|a, b| a.distance_to(at).total_cmp(&b.distance_to(at)));
    ground
}

pub(crate) fn placed_object(area: &str, object: &str) -> Option<Pos<Meters>> {
    area::objects(area::placed(content(), id(area)))
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
