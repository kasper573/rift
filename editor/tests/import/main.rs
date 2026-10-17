mod exports;

use std::collections::BTreeMap;
use std::path::PathBuf;

use editor::import::formats::{self, ExportFormat};
use editor::import::model::{self, CuePlacement};
use editor::import::records::ImportRecord;
use editor::import::tileset;
use engine::systems::area::grid::{self, PaletteRow};
use image::RgbaImage;
use serde_json::Value;

const CONTENT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../example_game");

#[test]
fn an_aseprite_sheet_imports_to_the_expected_row_and_image() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let row = row("model", "farmer_girl");
    let sheet = asset("models/farmer-girl.png");
    let strips = strips_of(&row, &sheet);
    let files = exports::aseprite(dir.path(), "farmer-girl", &strips, true);
    let dropped = formats::sort_out(&files).expect("a sheet and its json");
    let export = formats::decode(&dropped).expect("decodes");
    assert_eq!(export.kind, ExportFormat::SheetJson);
    assert_eq!(export.frame, (48, 64));
    assert!(export.trimmed);
    assert_eq!(export.strips.len(), 40);
    assert!(export.timed());
    let plan = plan_for(&row, &sheet, &export, None);
    assert!(plan.missing.is_empty());
    assert_eq!(plan.ready(), Ok(()));
    assert_eq!(plan.cues_kept(), plan.cues_total);
    let built = model::build(&plan, &export, Some(&row), "farmer_girl").expect("builds");
    assert_eq!(built.image, "models/farmer-girl.png");
    let image = formats::decode_image_bytes(&built.png).expect("a png");
    assert_eq!(image.dimensions(), (8 * 48, 40 * 64));
    for (name, frames) in &strips {
        let (action, direction) = name.split_once('_').unwrap();
        let after = built.row["sprites"]["actions"][action][direction]
            .as_array()
            .unwrap();
        assert_eq!(after.len(), frames.len(), "{name}");
        for (frame, (pixels, ms)) in after.iter().zip(frames) {
            assert_eq!(frame["ms"].as_u64().unwrap() as u32, *ms, "{name}");
            let index = frame["frame"].as_u64().unwrap() as u32;
            let cut = image::imageops::crop_imm(&image, (index % 8) * 48, (index / 8) * 64, 48, 64)
                .to_image();
            assert_eq!(
                formats::pixel_hash(&cut),
                formats::pixel_hash(pixels),
                "{name} pixels"
            );
        }
    }
    assert_eq!(built.row["hitbox"], row["hitbox"]);
    assert_eq!(built.row["modules"], row["modules"]);
    let apex: Vec<bool> = built.row["sprites"]["actions"]["attack"]["s"]
        .as_array()
        .unwrap()
        .iter()
        .map(|frame| frame["apex"].as_bool().unwrap_or(false))
        .collect();
    assert_eq!(
        apex,
        vec![false, false, false, false, false, true, false, false]
    );
}

#[test]
fn a_texturepacker_sheet_with_rotated_frames_imports_alike() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let row = row("model", "farmer_girl");
    let sheet = asset("models/farmer-girl.png");
    let strips = strips_of(&row, &sheet);
    let files = exports::texturepacker(dir.path(), "farmer-girl", &strips);
    let export = formats::decode(&formats::sort_out(&files).unwrap()).expect("decodes");
    assert_eq!(export.strips.len(), 40);
    assert!(!export.timed(), "TexturePacker carries no durations");
    for (name, frames) in &strips {
        let strip = export.strip(name).expect(name);
        for (frame, (pixels, _)) in strip.frames.iter().zip(frames) {
            assert_eq!(frame.hash, formats::pixel_hash(pixels), "{name} unrotated");
        }
    }
    let (mapping, notes) = model::default_mapping(&export);
    assert!(mapping.values().all(Option::is_some));
    assert!(
        notes
            .values()
            .all(|note| *note == model::MappingNote::ByName)
    );
    let asked: BTreeMap<String, u32> = export.strips.iter().map(|s| (s.tag.clone(), 90)).collect();
    let plan = model::plan(Some(&row), Some(&sheet), &export, &mapping, &asked, None);
    let strip = plan.strips.iter().find(|s| s.name == "walk_s").unwrap();
    assert!(strip.lengths.iter().all(|ms| *ms == 90));
}

#[test]
fn a_grid_asks_the_frame_size_and_maps_rows_by_order() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let row = row("model", "farmer_girl");
    let sheet = asset("models/farmer-girl.png");
    let strips = strips_of(&row, &sheet);
    let file = exports::grid(dir.path(), "farmer-girl", &strips, (48, 64));
    let mut export = formats::decode(&formats::sort_out(&[file]).unwrap()).expect("decodes");
    assert_eq!(export.kind, ExportFormat::Grid);
    assert!(export.strips.is_empty());
    assert!(export.cut_grid(50, 64).is_err());
    export.cut_grid(48, 64).expect("cuts");
    assert_eq!(export.strips.len(), 40);
    let (mapping, notes) = model::default_mapping(&export);
    assert_eq!(mapping["row 1"].as_deref(), Some("idle_s"));
    assert_eq!(mapping["row 40"].as_deref(), Some("death_w"));
    assert!(
        notes
            .values()
            .all(|note| *note == model::MappingNote::ByOrder)
    );
}

#[test]
fn a_numbered_sequence_imports_and_a_gap_is_refused() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let row = row("model", "farmer_girl");
    let sheet = asset("models/farmer-girl.png");
    let strips = strips_of(&row, &sheet);
    let files = exports::sequence(dir.path(), &strips[8..11]);
    let export = formats::decode(&formats::sort_out(&files).unwrap()).expect("decodes");
    assert_eq!(export.kind, ExportFormat::Sequence);
    assert_eq!(export.strips.len(), 3);
    assert_eq!(export.strip("walk_s").unwrap().frames.len(), 8);
    let gap: Vec<PathBuf> = files
        .into_iter()
        .filter(|f| !f.ends_with("walk_sw_4.png"))
        .collect();
    let refused = formats::decode(&formats::sort_out(&gap).unwrap()).unwrap_err();
    assert!(refused.contains("walk_sw skips"), "{refused}");
}

#[test]
fn a_gif_and_an_apng_carry_their_lengths() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let water = asset("tilesets/beach-tileset.png");
    let frames: Vec<RgbaImage> = (0..4)
        .map(|k| image::imageops::crop_imm(&water, (1 + 9 * k) * 16, 16, 112, 112).to_image())
        .collect();
    let gif = exports::gif(dir.path(), "beach-water", &frames, 25);
    let export = formats::decode(&formats::sort_out(&[gif]).unwrap()).expect("decodes");
    assert_eq!(export.kind, ExportFormat::Gif);
    assert_eq!(export.frame, (112, 112));
    let strip = &export.strips[0];
    assert_eq!(strip.tag, "beach-water");
    assert_eq!(
        strip.frames.iter().map(|f| f.ms).collect::<Vec<_>>(),
        vec![Some(250); 4]
    );
    let apng = exports::apng(dir.path(), "beach-water", &frames, (1, 3));
    let export = formats::decode(&formats::sort_out(&[apng]).unwrap()).expect("decodes");
    assert_eq!(export.kind, ExportFormat::Apng);
    assert_eq!(export.strips[0].frames[0].ms, Some(333), "half up");
    for (frame, pixels) in export.strips[0].frames.iter().zip(&frames) {
        assert_eq!(frame.hash, formats::pixel_hash(pixels));
    }
}

#[test]
fn importing_again_keeps_the_cues_that_still_fit_and_asks_for_the_rest() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let row = row("model", "farmer_girl");
    let sheet = asset("models/farmer-girl.png");
    let mut strips = strips_of(&row, &sheet);
    let run_e = strips.iter_mut().find(|(name, _)| name == "run_e").unwrap();
    run_e.1.remove(5);
    run_e.1.remove(4);
    let attack_s = strips
        .iter_mut()
        .find(|(name, _)| name == "attack_s")
        .unwrap();
    attack_s.1.remove(2);
    attack_s.1.remove(1);
    let files = exports::aseprite(dir.path(), "farmer-girl", &strips, true);
    let export = formats::decode(&formats::sort_out(&files).unwrap()).expect("decodes");
    let plan = plan_for(&row, &sheet, &export, None);
    let run = plan.strips.iter().find(|s| s.name == "run_e").unwrap();
    assert_eq!(run.before.len(), 8);
    assert_eq!(run.after_count(), 6);
    let steps: Vec<(usize, Option<usize>)> =
        run.cues.iter().map(|cue| (cue.from, cue.to)).collect();
    assert_eq!(
        steps,
        vec![(0, Some(0)), (4, None)],
        "the first step follows its frame, the fifth has nothing to follow"
    );
    assert_eq!(run.needs_place(), 1);
    let attack = plan.strips.iter().find(|s| s.name == "attack_s").unwrap();
    assert!(
        attack
            .cues
            .iter()
            .all(|cue| cue.from == 5 && cue.to == Some(3) && cue.by_pixels)
    );
    assert_eq!(plan.ready(), Err("one cue needs a place".to_owned()));
    let mut plan = plan;
    plan.strip_mut("run_e").unwrap().cues[1].choice = Some(CuePlacement::Frame(3));
    assert_eq!(plan.ready(), Ok(()));
    let built = model::build(&plan, &export, Some(&row), "farmer_girl").expect("builds");
    let after = built.row["sprites"]["actions"]["run"]["e"]
        .as_array()
        .unwrap();
    assert_eq!(after.len(), 6);
    assert_eq!(after[0]["step"], Value::Bool(true));
    assert_eq!(after[3]["step"], Value::Bool(true));
    let apex: Vec<bool> = built.row["sprites"]["actions"]["attack"]["s"]
        .as_array()
        .unwrap()
        .iter()
        .map(|frame| frame["apex"].as_bool().unwrap_or(false))
        .collect();
    assert_eq!(apex, vec![false, false, false, true, false, false]);
    assert_eq!(
        built.row["sprites"]["actions"]["attack"]["s"][3]["sound"],
        Value::String("slash03".to_owned())
    );
}

#[test]
fn importing_again_asks_before_replacing_lengths_edited_here() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let mut row = row("model", "farmer_girl");
    let sheet = asset("models/farmer-girl.png");
    let strips = strips_of(&row, &sheet);
    let files = exports::aseprite(dir.path(), "farmer-girl", &strips, false);
    let export = formats::decode(&formats::sort_out(&files).unwrap()).expect("decodes");
    let (mapping, _) = model::default_mapping(&export);
    let plan = model::plan(
        Some(&row),
        Some(&sheet),
        &export,
        &mapping,
        &BTreeMap::new(),
        None,
    );
    let built = model::build(&plan, &export, Some(&row), "farmer_girl").unwrap();
    let record = model::record(&export, "farmer_girl", &mapping, built.lengths.clone());
    assert_eq!(
        record.strips["attack_s"],
        vec![120, 110, 110, 100, 60, 70, 110, 120]
    );
    row["sprites"]["actions"]["attack"]["s"][4]["ms"] = Value::from(90);
    row["sprites"]["actions"]["attack"]["s"][5]["ms"] = Value::from(80);
    let plan = model::plan(
        Some(&row),
        Some(&sheet),
        &export,
        &mapping,
        &BTreeMap::new(),
        Some(&record),
    );
    assert!(!plan.decided());
    assert_eq!(plan.retimed(), 1);
    let attack = plan.strips.iter().find(|s| s.name == "attack_s").unwrap();
    let retimed = attack.retimed.as_ref().expect("asks which lengths win");
    assert_eq!(retimed.here[4], 90);
    assert_eq!(retimed.export[4], 60);
    assert!(retimed.keep_here);
    assert_eq!(attack.final_lengths()[4], 90);
    let untouched = plan.strips.iter().find(|s| s.name == "walk_s").unwrap();
    assert!(untouched.retimed.is_none());
    let same = model::plan(
        Some(&row),
        Some(&sheet),
        &export,
        &mapping,
        &BTreeMap::new(),
        None,
    );
    assert_eq!(
        same.retimed(),
        0,
        "without a record the export's lengths simply win"
    );
}

#[test]
fn a_tileset_grid_is_cut_whole_and_a_wider_one_keeps_tile_data_and_palettes() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let row = row("tileset", "beach_tileset");
    let old = asset("tilesets/beach-tileset.png");
    let spaced = exports::spaced_grid(dir.path(), "beach-tileset", &old, 16, 2, 1, 1);
    let export = formats::decode(&formats::sort_out(&[spaced]).unwrap()).unwrap();
    let settings = tileset::TileGridSettings {
        tile: 16,
        margin: 2,
        spacing: 1,
        extruded: 1,
    };
    let cut = tileset::cut_grid(export.sheet.as_ref().unwrap(), settings).expect("whole tiles");
    assert_eq!((cut.columns, cut.rows), (36, 26));
    assert_eq!(
        formats::pixel_hash(&cut.image),
        formats::pixel_hash(&old),
        "margins, spacing and extrusion dropped"
    );
    assert!(
        tileset::cut_grid(
            export.sheet.as_ref().unwrap(),
            tileset::TileGridSettings {
                tile: 16,
                ..settings
            }
        )
        .is_ok()
    );
    assert!(
        tileset::cut_grid(
            export.sheet.as_ref().unwrap(),
            tileset::TileGridSettings {
                tile: 17,
                ..settings
            }
        )
        .is_err()
    );

    let mut wide = RgbaImage::new(old.width() + 64, old.height());
    image::GenericImage::copy_from(&mut wide, &old, 0, 0).unwrap();
    fill(&mut wide, old.width(), 0, 64, 16, [200, 90, 120, 255]);
    let island =
        std::fs::read_to_string(format!("{CONTENT}/tables/area/island/palette.csv")).unwrap();
    let palettes = vec![("island".to_owned(), palette(&island))];
    let plan = tileset::tile_plan(&old, &row, &wide, "beach_tileset", &palettes);
    assert_eq!((plan.old_columns, plan.new_columns), (36, 40));
    assert!(plan.gone.is_empty());
    assert_eq!(plan.new_tiles, 4);
    assert_eq!(
        plan.matches[&450],
        12 * 40 + 18,
        "found by pixels at the same column and row"
    );
    assert_eq!(plan.ready(), Ok(()));
    let (new_row, rewritten) =
        tileset::apply_tile_plan(&plan, &row, "beach_tileset", &palettes).unwrap();
    let old_tiles = row["tiles"].as_object().unwrap();
    let new_tiles = new_row["tiles"].as_object().unwrap();
    assert_eq!(
        old_tiles.len() - plan.blank_data,
        new_tiles.len(),
        "transparent tiles carry nothing"
    );
    for (index, data) in old_tiles {
        let old_index: u32 = index.parse().unwrap();
        let Some(target) = plan.matches.get(&old_index) else {
            assert!(
                tileset::tile_image(&old, 16, old_index).is_some_and(|t| formats::is_blank(&t))
            );
            continue;
        };
        let moved = &new_tiles[&target.to_string()];
        assert_eq!(moved["walk"], data["walk"]);
        assert_eq!(moved["surface"], data["surface"]);
        if let Some(frames) = data["animation"].as_array() {
            let after = moved["animation"].as_array().unwrap();
            assert_eq!(after.len(), frames.len());
            for (before, after) in frames.iter().zip(after) {
                let tile = before["tile"].as_u64().unwrap() as u32;
                assert_eq!(after["tile"].as_u64().unwrap() as u32, plan.matches[&tile]);
                assert_eq!(after["ms"], before["ms"]);
            }
        }
    }
    let (_, text) = rewritten
        .iter()
        .find(|(area, _)| area == "island")
        .expect("island rewritten");
    let before = palette(&island);
    let after = palette(text);
    assert_eq!(before.len(), after.len());
    for (old, new) in before.iter().zip(&after) {
        assert_eq!(plan.matches[&old.tile], new.tile);
        assert_eq!(old.flip, new.flip);
    }

    let game = dir.path().join("game");
    editor::templates::new_game_from(std::path::Path::new(CONTENT), "Beach", &game).unwrap();
    wide.save(game.join("assets/tilesets/beach-tileset.png"))
        .unwrap();
    std::fs::write(
        game.join("tables/tileset/beach_tileset.json"),
        serde_json::to_string_pretty(&new_row).unwrap(),
    )
    .unwrap();
    for (area, text) in &rewritten {
        std::fs::write(game.join(format!("tables/area/{area}/palette.csv")), text).unwrap();
    }
    let loaded = engine::systems::load_content(
        std::sync::Arc::new(engine::core::content::FilesystemFolder(game)),
        "cargo test failed",
    );
    if let Err(report) = &loaded {
        panic!("the folder loads after the re-import:\n{report}");
    }
}

#[test]
fn a_gone_tile_still_painted_blocks_until_replaced() {
    let row = row("tileset", "beach_tileset");
    let old = asset("tilesets/beach-tileset.png");
    let mut changed = old.clone();
    fill(
        &mut changed,
        (450 % 36) * 16,
        (450 / 36) * 16,
        16,
        16,
        [1, 2, 3, 255],
    );
    let palettes = vec![(
        "island".to_owned(),
        palette("tileset,tile,flip\nbeach_tileset,450,\nbeach_tileset,451,\n"),
    )];
    let mut plan = tileset::tile_plan(&old, &row, &changed, "beach_tileset", &palettes);
    assert_eq!(plan.gone, vec![450]);
    assert!(plan.gone_used.contains_key(&450));
    assert!(plan.ready().is_err());
    plan.replacements.insert(450, Some(451));
    assert_eq!(plan.ready(), Ok(()));
    let (_, rewritten) = tileset::apply_tile_plan(&plan, &row, "beach_tileset", &palettes).unwrap();
    assert_eq!(palette(&rewritten[0].1)[0].tile, 451);
}

#[test]
fn an_animation_lands_on_a_block_reusing_frames_found_by_pixels() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let row = row("tileset", "beach_tileset");
    let water = asset("tilesets/beach-tileset.png");
    let frames: Vec<RgbaImage> = (0..4)
        .map(|k| image::imageops::crop_imm(&water, (1 + 9 * k) * 16, 16, 112, 112).to_image())
        .collect();
    let gif = exports::gif(dir.path(), "beach-water", &frames, 25);
    let export = formats::decode(&formats::sort_out(&[gif]).unwrap()).unwrap();
    let plan = tileset::anim_plan(&water, &row, &export, true, (1, 1)).expect("plans");
    assert_eq!(plan.animated(), 33);
    assert_eq!(plan.still, 16);
    assert!(
        plan.new_images.is_empty(),
        "every frame is on the tileset already"
    );
    assert!(plan.fits.is_some());
    let (png, new_row) = tileset::apply_anim_plan(&plan, &water, &row, &export).unwrap();
    assert!(png.is_none(), "nothing appended, nothing redrawn");
    let animation = new_row["tiles"]["37"]["animation"].as_array().unwrap();
    let tiles: Vec<u64> = animation
        .iter()
        .map(|f| f["tile"].as_u64().unwrap())
        .collect();
    assert_eq!(tiles, vec![37, 46, 55, 64]);
    assert!(animation.iter().all(|f| f["ms"] == 250));
    let frozen = exports::gif(
        dir.path(),
        "beach-still",
        &[frames[0].clone(), frames[0].clone()],
        25,
    );
    let frozen = formats::decode(&formats::sort_out(&[frozen]).unwrap()).unwrap();
    let still = tileset::anim_plan(&water, &row, &frozen, true, (1, 1));
    assert!(still.is_err(), "a block that never changes is refused");
    let mut fresh = frames.clone();
    fill(&mut fresh[1], 0, 0, 16, 16, [9, 9, 9, 255]);
    let gif = exports::gif(dir.path(), "beach-water-new", &fresh, 25);
    let export = formats::decode(&formats::sort_out(&[gif]).unwrap()).unwrap();
    let plan = tileset::anim_plan(&water, &row, &export, true, (1, 1)).unwrap();
    assert_eq!(plan.new_images.len(), 1);
    let (png, new_row) = tileset::apply_anim_plan(&plan, &water, &row, &export).unwrap();
    let grown = formats::decode_image_bytes(&png.expect("appended")).unwrap();
    assert_eq!(
        grown.height(),
        water.height() + 16,
        "new frames go after the last tile"
    );
    assert_eq!(
        new_row["tiles"]["37"]["animation"][1]["tile"],
        Value::from(36 * 26)
    );
}

fn row(table: &str, key: &str) -> Value {
    let text =
        std::fs::read_to_string(format!("{CONTENT}/tables/{table}/{key}.json")).expect("the row");
    serde_json::from_str(&text).expect("json")
}

fn asset(path: &str) -> RgbaImage {
    image::open(format!("{CONTENT}/assets/{path}"))
        .expect("the asset")
        .into_rgba8()
}

fn strips_of(row: &Value, sheet: &RgbaImage) -> Vec<(String, Vec<(RgbaImage, u32)>)> {
    let (fw, fh) = (
        row["sprites"]["frame_size"]["width"].as_u64().unwrap() as u32,
        row["sprites"]["frame_size"]["height"].as_u64().unwrap() as u32,
    );
    let columns = sheet.width() / fw;
    model::strip_names()
        .into_iter()
        .map(|name| {
            let (action, direction) = name.split_once('_').unwrap();
            let frames = row["sprites"]["actions"][action][direction]
                .as_array()
                .unwrap()
                .iter()
                .map(|frame| {
                    let index = frame["frame"].as_u64().unwrap() as u32;
                    let cut = image::imageops::crop_imm(
                        sheet,
                        (index % columns) * fw,
                        (index / columns) * fh,
                        fw,
                        fh,
                    )
                    .to_image();
                    (cut, frame["ms"].as_u64().unwrap() as u32)
                })
                .collect();
            (name, frames)
        })
        .collect()
}

fn palette(text: &str) -> Vec<PaletteRow> {
    let file = grid::GridFile {
        path: "palette.csv",
        text,
    };
    grid::read_palette(file, &mut Vec::new()).expect("a palette")
}

fn fill(image: &mut RgbaImage, x: u32, y: u32, w: u32, h: u32, color: [u8; 4]) {
    for dy in 0..h {
        for dx in 0..w {
            image.put_pixel(x + dx, y + dy, image::Rgba(color));
        }
    }
}

fn plan_for(
    row: &Value,
    sheet: &RgbaImage,
    export: &formats::AnimationExport,
    record: Option<&ImportRecord>,
) -> model::ModelImportPlan {
    let (mapping, _) = model::default_mapping(export);
    model::plan(
        Some(row),
        Some(sheet),
        export,
        &mapping,
        &BTreeMap::new(),
        record,
    )
}
