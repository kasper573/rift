use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use engine::core::content::{FilesystemFolder, FolderReader};
use engine::core::ground::PaintMap;
use engine::core::math::Rect;
use engine::core::units::Meters;
use engine::systems::area::scene::SceneNode;
use engine::systems::area::{AreaDef, AreaModule};
use engine::systems::registry;
use serde_json::{Value, json};
use testkit::{Game, Map, ROCK, TURF, path};

/// A land rises over its relief exactly; its biomes each cover a share of it, the cold one on the
/// heights as the height chills the climate; each grows its props and roams its monsters only
/// where it lies; and running it again replaces what it laid out, keeping what a designer added.
#[test]
fn a_land_lays_out_its_biomes_by_climate_and_keeps_what_a_designer_added() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    Game::new()
        .area(
            "start",
            &Map::open(16, 16),
            json!([{ "type": "safe_zone", "rect": { "x": 1.0, "y": 1.0, "w": 6.0, "h": 6.0 } }]),
        )
        .hero("start", 4.0, 4.0)
        .prop("pine", json!({ "type": "circle", "radius": 0.5 }))
        .npc("wolf", 10.0, json!([]))
        .write_to(folder.path());
    let relief = 10.0;
    let biome = |name: &str, temperature: f32, terrain: &str, props: Value, monsters: Value| {
        json!({
            "name": name,
            "temperature": temperature,
            "moisture": 0.5,
            "ground": [{ "terrain": path("terrain", terrain) }],
            "props": props,
            "monsters": monsters,
        })
    };
    let land = json!({
        "name": "Wilds",
        "width": 512,
        "height": 512,
        "seed": 3,
        "relief": relief,
        "shape": { "wavelength": 128.0, "warp": 0.5, "mountains": 0.5, "roughness": 0.2 },
        "erosion": { "drops_per_square_meter": 0.05 },
        "climate": { "wavelength": 256.0, "height_chill": 1.0, "runoff_wetness": 0.3, "blend": 0.05 },
        "biomes": [
            biome("Fells", 0.0, ROCK, json!([]), json!([])),
            biome(
                "Woods",
                1.0,
                TURF,
                json!([{ "props": [path("prop", "pine")], "per_square_meter": 0.005, "clumping": 0.5 }]),
                json!([{ "npc": path("npc", "wolf"), "per_square_kilometer": 100.0 }]),
            ),
        ],
    });

    landscape(folder.path(), &land);
    let area = read(folder.path());
    assert_eq!(area.name, "Wilds");

    let (columns, rows) = area.heights.samples();
    let heights: Vec<f32> = (0..rows)
        .flat_map(|row| (0..columns).map(move |column| (column, row)))
        .map(|(column, row)| area.heights.sample(column, row).0)
        .collect();
    let lowest = heights.iter().copied().fold(f32::INFINITY, f32::min);
    let highest = heights.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    assert!((lowest - 0.0).abs() < 1e-3 && (highest - relief).abs() < 1e-3);

    let painted = |terrain: u8| -> Vec<f32> {
        let (columns, rows) = area.paint.pixels();
        (0..rows)
            .flat_map(|row| (0..columns).map(move |column| (column, row)))
            .filter(|&(column, row)| area.paint.index(column, row) == terrain)
            .map(|(column, row)| area.heights.height_at(PaintMap::pixel_pos(column, row)).0)
            .collect()
    };
    let (rock, turf) = (painted(0), painted(1));
    let share = rock.len() as f32 / (rock.len() + turf.len()) as f32;
    assert!(
        (0.25..=0.75).contains(&share),
        "the fells cover {share} of the land"
    );
    let mean = |heights: &[f32]| heights.iter().sum::<f32>() / heights.len() as f32;
    assert!(
        mean(&rock) > mean(&turf),
        "the fells lie higher: {} m against {} m",
        mean(&rock),
        mean(&turf)
    );

    let laid = laid_out(&area.scene);
    let (scatters, populations): (Vec<_>, Vec<_>) = modules(&laid)
        .into_iter()
        .partition(|module| matches!(module, AreaModule::Scatter(_)));
    assert!(!scatters.is_empty() && !populations.is_empty());
    for scatter in &scatters {
        let AreaModule::Scatter(scatter) = scatter else {
            unreachable!()
        };
        assert!(
            share_painted(&area, &scatter.region, 1) > 0.0,
            "pines grow only where the woods reach"
        );
    }
    for population in &populations {
        let AreaModule::Population(population) = population else {
            panic!("the land laid out a module of another kind")
        };
        let roams = population.roams.expect("a population roams a tile");
        assert!(area.size.bounds().contains_rect(&roams));
        assert!(
            share_painted(&area, &roams, 1) > 0.0,
            "wolves roam only where the woods reach"
        );
    }

    landscape(folder.path(), &land);
    let again = read(folder.path());
    let folders = again
        .scene
        .iter()
        .filter(|node| matches!(node, SceneNode::Folder(_)))
        .count();
    assert_eq!(folders, 1, "a second run replaces what the first laid out");
    assert!(
        modules(&again.scene)
            .iter()
            .any(|module| matches!(module, AreaModule::SafeZone(_))),
        "what a designer added stays"
    );
}

fn landscape(game: &Path, land: &Value) {
    let recipes = tempfile::tempdir().expect("a folder for the recipe");
    let recipe = recipes.path().join("land.json");
    std::fs::write(&recipe, land.to_string()).expect("the land's recipe writes");
    let laid = Command::new(env!("CARGO_BIN_EXE_rift"))
        .args(["landscape"])
        .arg(game)
        .arg(path("area", "start"))
        .arg(&recipe)
        .output()
        .expect("rift runs");
    assert!(
        laid.status.success(),
        "{}",
        String::from_utf8_lossy(&laid.stderr)
    );
}

fn read(game: &Path) -> AreaDef {
    FolderReader::new(registry(), Arc::new(FilesystemFolder(game.to_path_buf())))
        .read(&path("area", "start"))
        .map_err(|errors| format!("{} problem(s)", errors.len()))
        .expect("the land reads as the editor writes it")
}

/// The share of `region` painted `terrain`.
fn share_painted(area: &AreaDef, region: &Rect<Meters>, terrain: u8) -> f32 {
    let (columns, rows) = area.paint.pixels();
    let within: Vec<u8> = (0..rows)
        .flat_map(|row| (0..columns).map(move |column| (column, row)))
        .filter(|&(column, row)| region.contains(PaintMap::pixel_pos(column, row)))
        .map(|(column, row)| area.paint.index(column, row))
        .collect();
    within.iter().filter(|&&painted| painted == terrain).count() as f32 / within.len() as f32
}

/// The modules of the one folder the land laid out.
fn laid_out(scene: &[SceneNode]) -> Vec<SceneNode> {
    scene
        .iter()
        .filter(|node| matches!(node, SceneNode::Folder(_)))
        .cloned()
        .collect()
}

fn modules(nodes: &[SceneNode]) -> Vec<AreaModule> {
    nodes
        .iter()
        .flat_map(|node| match node {
            SceneNode::Folder(folder) => modules(&folder.children),
            SceneNode::Module(module) => vec![module.module.clone()],
        })
        .collect()
}
