//! Parses an [`ActorModel`] from its Tiled `.tsx` tileset: per-action/direction animation strips,
//! the sound/step/apex tile flags, and the sheet image and hitbox dimensions.

use std::collections::{HashMap, HashSet};

use tiled::{Frame, PropertyValue};

use super::{ActorModel, IDLE};
use crate::core::assets;
use crate::core::math::Size;
use crate::sfx::SfxId;

pub(super) fn load(name: &str) -> ActorModel {
    let tileset = tiled::Loader::with_reader(assets::tiled_reader)
        .load_tsx_tileset(format!("{}/{name}.tsx", assets::ACTORS))
        .unwrap_or_else(|error| panic!("actor model {name}: {error}"));
    let source = tileset
        .image
        .as_ref()
        .and_then(|image| image.source.file_name()?.to_str())
        .unwrap_or_else(|| panic!("actor model {name} declares no sheet image"));
    let sheet = assets::find(assets::ACTORS, source)
        .unwrap_or_else(|| panic!("actor model {name} has no sheet {source}"));

    let mut strips: HashMap<String, [Vec<Frame>; 8]> = HashMap::new();
    let mut sounds = HashMap::new();
    let mut steps = HashSet::new();
    let mut apexes = HashSet::new();
    for (id, tile) in tileset.tiles() {
        if let Some(PropertyValue::StringValue(sfx)) = tile.properties.get("sfx") {
            sounds.insert(id, SfxId(sfx.clone()));
        }
        if let Some(PropertyValue::BoolValue(true)) = tile.properties.get("step") {
            steps.insert(id);
        }
        if let Some(PropertyValue::BoolValue(true)) = tile.properties.get("apex") {
            apexes.insert(id);
        }
        if let Some(PropertyValue::StringValue(action)) = tile.properties.get("action") {
            let dir = match tile.properties.get("dir") {
                Some(PropertyValue::IntValue(dir)) if (0..8).contains(dir) => *dir as usize,
                _ => panic!("actor model {name}: '{action}' tile {id} needs a dir in 0..8"),
            };
            let strip = tile
                .animation
                .clone()
                .filter(|frames| !frames.is_empty())
                .unwrap_or_else(|| panic!("actor model {name}: '{action}' dir {dir} is empty"));
            strips.entry(action.clone()).or_default()[dir] = strip;
        }
    }
    for (action, dirs) in &strips {
        if dirs.iter().any(Vec::is_empty) {
            panic!("actor model {name}: action '{action}' is missing a direction strip");
        }
    }
    if !strips.contains_key(IDLE) {
        panic!("actor model {name} must declare an idle action");
    }

    let dimension = |key: &str| match tileset.properties.get(key) {
        Some(PropertyValue::FloatValue(value)) => *value,
        _ => panic!("actor model {name} needs a float '{key}' tileset property"),
    };
    ActorModel {
        name: name.to_owned(),
        sheet,
        frame: Size::new(tileset.tile_width as f32, tileset.tile_height as f32),
        columns: tileset.columns.max(1),
        hitbox: Size::new(dimension("hitbox_width"), dimension("hitbox_height")),
        strips,
        sounds,
        steps,
        apexes,
    }
}
