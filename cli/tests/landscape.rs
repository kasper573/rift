use std::process::Command;
use std::sync::Arc;

use engine::core::content::{FilesystemFolder, FolderReader};
use engine::core::math::Rect;
use engine::core::units::Meters;
use engine::systems::area::scene::SceneNode;
use engine::systems::area::{AreaDef, AreaModule};
use engine::systems::registry;
use serde_json::json;
use testkit::{Game, Map, ROCK, TURF, path};

/// A landscape rises over its relief exactly, lays each terrain over as much ground as the next
/// from the lowest ground up, strews a prop only where its terrain lies, and has npcs roam it.
#[test]
fn a_landscape_rises_over_its_relief_painted_in_bands_strewn_and_roamed_where_asked() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    Game::new()
        .terrains()
        .area("start", &Map::open(16, 16), json!([]))
        .hero("start", 4.0, 4.0)
        .prop("pine", json!({ "type": "circle", "radius": 0.5 }))
        .npc("wolf", 10.0, json!([]))
        .write_to(folder.path());
    let relief = 10.0;
    let wilds = "wilds.area";
    let laid = Command::new(env!("CARGO_BIN_EXE_rift"))
        .args(["landscape"])
        .arg(folder.path())
        .args([
            wilds,
            "--name",
            "Wilds",
            "--width",
            "256",
            "--height",
            "128",
            "--seed",
            "3",
            "--relief",
            &relief.to_string(),
            "--wavelength",
            "32",
            "--terrain",
            &path("terrain", TURF),
            "--terrain",
            &path("terrain", ROCK),
            "--scatter",
            &format!("0:{}:0.05:0.5-1.5", path("prop", "pine")),
            "--population",
            &format!("{}:1000", path("npc", "wolf")),
        ])
        .output()
        .expect("rift runs");
    assert!(
        laid.status.success(),
        "{}",
        String::from_utf8_lossy(&laid.stderr)
    );

    let reader = FolderReader::new(
        registry(),
        Arc::new(FilesystemFolder(folder.path().to_path_buf())),
    );
    let area: AreaDef = reader
        .read(wilds)
        .map_err(|errors| format!("{} problem(s)", errors.len()))
        .expect("the wilds read as the editor writes them");
    let area = &area;
    assert_eq!(area.name, "Wilds");

    let (columns, rows) = area.heights.samples();
    let heights: Vec<f32> = (0..rows)
        .flat_map(|row| (0..columns).map(move |column| (column, row)))
        .map(|(column, row)| area.heights.sample(column, row).0)
        .collect();
    let lowest = heights.iter().copied().fold(f32::INFINITY, f32::min);
    let highest = heights.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    assert!((lowest - 0.0).abs() < 1e-3 && (highest - relief).abs() < 1e-3);

    let (columns, rows) = area.paint.pixels();
    let pixels: Vec<(u32, u32)> = (0..rows)
        .flat_map(|row| (0..columns).map(move |column| (column, row)))
        .collect();
    let rock = pixels
        .iter()
        .filter(|&&(column, row)| area.paint.index(column, row) == 1)
        .count();
    let share = rock as f32 / pixels.len() as f32;
    assert!(
        (0.45..=0.55).contains(&share),
        "rock over {share} of the ground"
    );
    let (_, highest_turf) = heights_where(area, &pixels, 0);
    let (lowest_rock, _) = heights_where(area, &pixels, 1);
    assert!(
        highest_turf <= lowest_rock + 1e-3,
        "the rock lies above the turf: turf up to {highest_turf}, rock from {lowest_rock}"
    );

    let (scatters, populations): (Vec<Rect<Meters>>, Vec<Rect<Meters>>) = {
        let mut scatters = Vec::new();
        let mut populations = Vec::new();
        for module in modules(&area.scene) {
            match module {
                AreaModule::Scatter(scatter) => scatters.push(scatter.region),
                AreaModule::Population(population) => {
                    populations.push(population.roams.expect("a population roams a block"));
                }
                _ => panic!("the landscape laid out a module of another kind"),
            }
        }
        (scatters, populations)
    };
    assert!(!scatters.is_empty() && !populations.is_empty());
    let bounds = area.size.bounds();
    for region in scatters.iter().chain(&populations) {
        assert!(
            bounds.contains_rect(region),
            "{region:?} lies within the land"
        );
    }
    for region in &scatters {
        let within: Vec<&(u32, u32)> = pixels
            .iter()
            .filter(|&&(column, row)| {
                region.contains(engine::core::ground::Paint::pixel_pos(column, row))
            })
            .collect();
        let turf = within
            .iter()
            .filter(|&&&(column, row)| area.paint.index(column, row) == 0)
            .count();
        assert!(turf * 2 > within.len(), "a scatter lies mostly on turf");
    }
    let strewn = scatters.iter().map(|region| region.area()).sum::<f32>();
    assert!(
        strewn < bounds.area() * 0.75,
        "the rock stays unstrewn: {strewn} of {} m²",
        bounds.area()
    );
}

/// The lowest and highest ground painted `terrain`.
fn heights_where(area: &AreaDef, pixels: &[(u32, u32)], terrain: u8) -> (f32, f32) {
    pixels
        .iter()
        .filter(|&&(column, row)| area.paint.index(column, row) == terrain)
        .map(|&(column, row)| {
            area.heights
                .height_at(engine::core::ground::Paint::pixel_pos(column, row))
                .0
        })
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), height| {
            (low.min(height), high.max(height))
        })
}

fn modules(nodes: &[SceneNode]) -> Vec<&AreaModule> {
    nodes
        .iter()
        .flat_map(|node| match node {
            SceneNode::Folder(folder) => modules(&folder.children),
            SceneNode::Module(module) => vec![&module.module],
        })
        .collect()
}
