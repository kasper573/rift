//! A game whose spawn area is laid out again and again, side by side, into one larger area: its
//! ground, and everything its scene holds with it, so a larger area carries what as many areas
//! would. A tile's residents are npcs of their own, copies of the first tile's, and its named
//! objects are named apart, as an area holds each of those once, and its test characters start
//! on it, copies of the first tile's.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use engine::core::content::{Content, ContentFile, WrittenFiles, content_path, name_of, parent_of};
use engine::core::ground::{GroundSize, Heightfield, Paint, SAMPLES_PER_METER};
use engine::core::math::Pos;
use engine::core::script::VarValue;
use engine::core::units::Meters;
use engine::systems::GameSettings;
use engine::systems::area::scene::{SceneFolder, SceneNode};
use engine::systems::area::{AreaDef, AreaModule};
use engine::systems::npc::{NpcDef, NpcId};
use engine::systems::player::snapshot::{CharacterSnapshot, CharacterStart, start_of};
use engine::systems::script::Place;

use crate::folder::{reader_of, write};

/// Sets the cell size of the game written at `into`, whose content is `content`.
pub fn with_cell_size(into: &Path, content: &Content, size: Meters) -> Result<(), String> {
    let mut settings = content.built::<GameSettings>().clone();
    settings.spatial_partition_cell_size = size;
    let files = reader_of(into).write_settings(&settings);
    write(
        into,
        files.map(|files| WrittenFiles {
            files,
            removed: Vec::new(),
        }),
    )
}

/// Writes into `into` the game at `game` with its spawn area laid out `side` by `side` times.
pub fn tiled(game: &Path, content: &Content, side: u32, into: &Path) -> Result<(), String> {
    copy_folder(game, into)?;
    let spawn = content.built::<GameSettings>().initial_spawn;
    let area_path = spawn.area.path(content).to_owned();
    let tile_size = spawn.area.get(content).size;
    let tiles: Vec<(u32, u32)> = (0..side)
        .flat_map(|row| (0..side).map(move |column| (column, row)))
        .collect();
    let residents = residents_of(
        content
            .built::<GameSettings>()
            .initial_spawn
            .area
            .get(content),
        content,
    );
    let reader = reader_of(into);
    let mut copies = BTreeMap::new();
    for &tile in tiles.iter().skip(1) {
        for npc in &residents {
            let copy = content_path(
                parent_of(npc),
                &format!("{}-{}-{}", name_of(npc), tile.0, tile.1),
                NpcDef::TABLE,
                NpcDef::SHAPE,
            );
            let def: NpcDef = reader
                .read(npc)
                .map_err(|errors| format!("{npc} doesn't read: {} problem(s)", errors.len()))?;
            write(into, reader.write(&copy, &def))?;
            copies.insert((tile, npc.clone()), copy);
        }
    }
    let characters: Vec<&str> = content
        .table::<CharacterSnapshot>()
        .ids()
        .filter(|&id| start_of(content, id.get(content)).area == spawn.area)
        .map(|id| id.path(content))
        .collect();
    for &tile in tiles.iter().skip(1) {
        for character in &characters {
            let copy = content_path(
                parent_of(character),
                &format!("{}-{}-{}", name_of(character), tile.0, tile.1),
                CharacterSnapshot::TABLE,
                CharacterSnapshot::SHAPE,
            );
            let mut snapshot: CharacterSnapshot = reader.read(character).map_err(|errors| {
                format!("{character} doesn't read: {} problem(s)", errors.len())
            })?;
            let start = start_of(content, &snapshot);
            snapshot.start = CharacterStart::At(Place {
                area: start.area,
                at: Pos::new(
                    start.at.x + (tile.0 * tile_size.width) as f32,
                    start.at.y + (tile.1 * tile_size.height) as f32,
                ),
            });
            write(into, reader.write(&copy, &snapshot))?;
        }
    }

    let reader = reader_of(into);
    let npc_of =
        |path: &str| reader.from_json::<NpcId>(&serde_json::Value::String(path.to_owned()));
    let mut resident_copies = BTreeMap::new();
    for ((tile, npc), copy) in &copies {
        resident_copies.insert((*tile, npc_of(npc)?), npc_of(copy)?);
    }
    let area: AreaDef = reader
        .read(&area_path)
        .map_err(|errors| format!("{area_path} doesn't read: {} problem(s)", errors.len()))?;
    let (width, height) = (area.size.width, area.size.height);
    let size = GroundSize {
        width: width * side,
        height: height * side,
    };
    let (columns, rows) = (width * SAMPLES_PER_METER, height * SAMPLES_PER_METER);
    let mut tiled = area.clone();
    tiled.size = size;
    tiled.heights = Heightfield::from_fn(size, |column, row| {
        area.heights.sample(column % columns, row % rows)
    });
    tiled.paint = Paint::from_fn(size, |column, row| {
        area.paint.index(column % columns, row % rows)
    });
    tiled.scene = Vec::with_capacity(tiles.len());
    for &tile in &tiles {
        let mut nodes = area.scene.clone();
        if tile != (0, 0) {
            let mut problem = None;
            for_each_module(&mut nodes, &mut |module| {
                module.shift((tile.0 * width) as f32, (tile.1 * height) as f32);
                if let Err(error) = apart(module, tile, &copies, &resident_copies) {
                    problem.get_or_insert(error);
                }
            });
            if let Some(problem) = problem {
                return Err(problem);
            }
        }
        tiled.scene.push(SceneNode::Folder(SceneFolder {
            label: format!("Tile {}, {}", tile.0, tile.1),
            hidden: false,
            locked: false,
            children: nodes,
        }));
    }
    write(into, reader.write(&area_path, &tiled))
}

/// What sets `module` of `tile` apart from the first tile's: its own resident, its own name.
fn apart(
    module: &mut AreaModule,
    tile: (u32, u32),
    copies: &BTreeMap<((u32, u32), String), String>,
    resident_copies: &BTreeMap<((u32, u32), NpcId), NpcId>,
) -> Result<(), String> {
    match module {
        AreaModule::Resident(resident) => {
            resident.npc = *resident_copies
                .get(&(tile, resident.npc))
                .ok_or("a resident of the spawn area has no copy")?;
        }
        AreaModule::Object(object) => {
            if let Some(name) = &mut object.name {
                *name = format!("{name}-{}-{}", tile.0, tile.1);
            }
        }
        AreaModule::Blueprint(placement) => {
            for value in placement.values.values_mut() {
                if let VarValue::Str(path) = value
                    && let Some(copy) = copies.get(&(tile, path.clone()))
                {
                    *path = copy.clone();
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// The npcs `area` holds as residents, placed by its scene or by a blueprint it places.
fn residents_of(area: &AreaDef, content: &Content) -> Vec<String> {
    let mut residents = Vec::new();
    let mut nodes = area.scene.clone();
    for_each_module(&mut nodes, &mut |module| match module {
        AreaModule::Resident(resident) => residents.push(resident.npc.path(content).to_owned()),
        AreaModule::Blueprint(placement) => {
            for value in placement.values.values() {
                if let VarValue::Str(path) = value
                    && content.table::<NpcDef>().id_of(path).is_some()
                {
                    residents.push(path.clone());
                }
            }
        }
        _ => {}
    });
    residents.sort();
    residents.dedup();
    residents
}

fn for_each_module(nodes: &mut [SceneNode], visit: &mut impl FnMut(&mut AreaModule)) {
    for node in nodes {
        match node {
            SceneNode::Folder(folder) => for_each_module(&mut folder.children, visit),
            SceneNode::Module(module) => visit(&mut module.module),
        }
    }
}

fn copy_folder(from: &Path, to: &Path) -> Result<(), String> {
    for entry in walkdir::WalkDir::new(from).follow_links(true) {
        let entry = entry.map_err(|error| error.to_string())?;
        let target = to.join(
            entry
                .path()
                .strip_prefix(from)
                .expect("a walked path lies in the walked folder"),
        );
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target).map_err(|error| error.to_string())?;
        } else {
            fs::copy(entry.path(), &target)
                .map_err(|error| format!("{}: {error}", entry.path().display()))?;
        }
    }
    Ok(())
}
