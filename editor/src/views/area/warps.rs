use super::stamp::object_names;
use super::{AreaSnapshot, nodes};
use crate::app::Editor;
use crate::project::IdPaths;
use crate::rows;
use crate::tabs::file_title;
use engine::core::math::Pos;
use engine::core::units::{MeterPos, Meters};
use engine::systems::area::object::{ObjectDef, ObjectShape};
use engine::systems::area::scene::{ModuleNode, SceneNode};
use engine::systems::area::{self, AreaDef, AreaModule};
use engine::systems::script::{Place, VarValue};

/// Every place the object's scripts lead to, by the path of its area and its ground.
pub(crate) fn warp_destinations(
    object: &ObjectDef,
    paths: &impl IdPaths,
) -> Vec<(String, Pos<Meters>)> {
    object
        .scripts
        .iter()
        .flat_map(|script| script.values.values())
        .filter_map(|value| match value {
            VarValue::Place(place) => Some((paths.path_of(place.area)?.to_owned(), place.at)),
            _ => None,
        })
        .collect()
}

/// Points every place the object's scripts name at `to`, so a copy of a warp leads there instead.
fn redirect_places(object: &mut ObjectDef, to: Place) {
    for value in object
        .scripts
        .iter_mut()
        .flat_map(|script| script.values.values_mut())
    {
        if let VarValue::Place(place) = value {
            *place = to;
        }
    }
}

/// Adds an object to the area the warp object at `node` leads to, on the nearest walkable cell
/// beside its arrival, with the object's scripts pointed back here, so the player can come back.
pub(crate) fn return_door(editor: &mut Editor, frame: &AreaSnapshot, node: &str) {
    let area_key = &frame.bundle;
    let Some(AreaModule::Object(object)) = frame.module(node) else {
        return;
    };
    let Some((target_area, target_at)) = warp_destinations(object, frame.reader.as_ref())
        .into_iter()
        .next()
    else {
        editor.toast("The object has no warp to return from");
        return;
    };
    let Some(content) = editor.project.as_ref().and_then(|p| p.content.clone()) else {
        return;
    };
    let Some(here_id) = content.by_path::<AreaDef>(area_key) else {
        return;
    };
    let Some(there_id) = content.by_path::<AreaDef>(&target_area) else {
        return;
    };
    let here = area::load(&content, here_id);
    let there = area::load(&content, there_id);
    let door_bounds = object.shape.bounds();
    let back_to = here
        .mesh()
        .nearest_where(door_bounds.center(), here.bodies.spacing(), |at| {
            !door_bounds.contains(at)
        });
    let Some(back_to) = back_to else {
        editor.toast("No walkable cell outside the door to come back to");
        return;
    };
    let Some(beside_arrival) =
        there
            .mesh()
            .nearest_where(target_at, there.bodies.spacing(), |at| {
                at.distance(target_at) >= there.bodies.spacing()
            })
    else {
        editor.toast("The destination has no walkable cell beside the arrival for a door back");
        return;
    };
    let mut names = object_names(area::placed(&content, there_id));
    let Some(here_in_files) = frame.reader.id_of::<AreaDef>(area_key) else {
        return;
    };
    let name = rows::free_name(
        &format!("{}_back", object.name.as_deref().unwrap_or("door")),
        &names,
    );
    names.insert(name.clone());
    let mut door = ObjectDef {
        name: Some(name),
        title: None,
        shape: ObjectShape::Point(beside_arrival),
        look: None,
        shown: None,
        scripts: object.scripts.clone(),
    };
    redirect_places(
        &mut door,
        Place {
            area: here_in_files,
            at: back_to,
        },
    );
    let door = SceneNode::Module(ModuleNode::new(AreaModule::Object(door)));
    let applied = nodes::change_scene(editor, &target_area, |scene| {
        nodes::SceneTree::new(scene).append("", vec![door]);
        Some(format!(
            "Return door from {} to {}",
            file_title(&target_area),
            file_title(area_key)
        ))
    });
    if applied {
        editor.toast(format!(
            "Added a door back in {target_area} at {:.1}, {:.1}",
            beside_arrival.x, beside_arrival.y
        ));
    }
}
