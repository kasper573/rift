mod art;
mod exports;

use ui::egui;

use editor::import::formats;
use editor::import::records::{ImportMapping, ImportRecord};
use editor::import::tileset;
use editor::views::area::grid::BundlePalette;
use editor::views::tileset::SheetRect;
use engine::core::content::FolderReader;
use engine::systems::area::grid::{self, PaletteRow};
use engine::systems::area::tileset::{TilesetDef, Walk};
use image::RgbaImage;
use testkit::path;

use self::art::{TILE, fill};

#[test]
fn a_tileset_grid_is_cut_whole_and_a_wider_one_keeps_tile_data_and_palettes() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let (reader, row) = beach();
    let old = art::tileset();
    let spaced = exports::spaced_grid(dir.path(), "beach", &old, 16, 2, 1, 1);
    let export = formats::decode(&formats::sort_out(&[spaced]).unwrap()).unwrap();
    let settings = tileset::TileGridSettings {
        tile: 16,
        margin: 2,
        spacing: 1,
        extruded: 1,
    };
    let cut = tileset::cut_grid(export.sheet.as_ref().unwrap(), settings).expect("whole tiles");
    assert_eq!((cut.columns, cut.rows), (art::COLUMNS, art::ROWS));
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
    let island = art::island_palette();
    let palettes = vec![BundlePalette {
        bundle: path("area", "island"),
        rows: palette(&island),
    }];
    let plan =
        tileset::tile_plan(&old, &row, &wide, &path("tileset", "beach"), &palettes).expect("plans");
    assert_eq!(
        (plan.old_columns, plan.new_columns),
        (art::COLUMNS, art::COLUMNS + 4)
    );
    assert!(plan.gone.is_empty());
    assert_eq!(plan.new_tiles, 4);
    assert_eq!(
        plan.matches[&art::PAVED],
        (art::PAVED / art::COLUMNS) * (art::COLUMNS + 4) + art::PAVED % art::COLUMNS,
        "found by pixels at the same column and row"
    );
    assert_eq!(plan.ready(), Ok(()));
    let (new_row, rewritten) =
        tileset::apply_tile_plan(&plan, &row, &path("tileset", "beach"), &palettes).unwrap();
    assert_eq!(
        row.tiles.len() - plan.blank_data,
        new_row.tiles.len(),
        "transparent tiles carry nothing"
    );
    for (old_index, data) in &row.tiles {
        let Some(target) = plan.matches.get(old_index) else {
            assert!(
                formats::cell(&old, (TILE, TILE), *old_index)
                    .is_some_and(|t| formats::is_blank(&t))
            );
            continue;
        };
        let moved = &new_row.tiles[target];
        assert_eq!(moved.walk, data.walk);
        assert_eq!(moved.surface, data.surface);
        assert_eq!(moved.animation.len(), data.animation.len());
        for (before, after) in data.animation.iter().zip(&moved.animation) {
            assert_eq!(after.tile, plan.matches[&before.tile]);
            assert_eq!(after.ms, before.ms);
        }
    }
    let after = &rewritten
        .iter()
        .find(|palette| palette.bundle == path("area", "island"))
        .expect("island rewritten")
        .rows;
    let before = palette(&island);
    assert_eq!(before.len(), after.len());
    for (old, new) in before.iter().zip(after) {
        assert_eq!(plan.matches[&old.tile], new.tile);
        assert_eq!(old.flip, new.flip);
    }

    let game = dir.path().join("game");
    beach_game().write_to(&game);
    let beach = game.join(path("tileset", "beach"));
    wide.save(beach.join("image.png")).unwrap();
    let written = reader
        .write(&path("tileset", "beach"), &new_row)
        .expect("the tileset writes");
    for (file, bytes) in written.files {
        std::fs::write(game.join(file), bytes).unwrap();
    }
    for palette in &rewritten {
        std::fs::write(
            game.join(format!("{}/palette.csv", palette.bundle)),
            grid::write_palette(&palette.rows),
        )
        .unwrap();
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
fn importing_onto_an_open_tileset_lands_on_a_click_and_the_tileset_shows_on() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let mut editor = crate::support::open(beach_game(), &dir);
    let beach = path("tileset", "beach");
    let old = art::tileset();
    let mut wide = RgbaImage::new(old.width() + 64, old.height());
    image::GenericImage::copy_from(&mut wide, &old, 0, 0).unwrap();
    fill(&mut wide, old.width(), 0, 64, 16, [200, 90, 120, 255]);
    let delivered = dir.path().join("beach_wide.png");
    wide.save(&delivered).unwrap();
    editor.open_tab(editor::tabs::EditorTab::Tileset { key: beach.clone() });
    editor.place_tabs();
    editor::import::dropped(&mut editor, vec![delivered]);
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !editor.views.tileset.reimports.contains_key(&beach) {
        assert!(std::time::Instant::now() < until, "the import reads");
        editor::import::poll(&mut editor);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let mut frames = crate::support::Frames::new(egui::vec2(1600.0, 1000.0));
    let mut show = |editor: &mut editor::app::Editor, events| {
        frames.run(events, |ui| {
            editor::views::tileset::show(editor, ui, &beach)
        })
    };
    let confirm = show(&mut editor, Vec::new()).at("Import, as one step");

    for events in crate::support::click(confirm) {
        show(&mut editor, events);
    }
    let drawn = show(&mut editor, Vec::new());

    let image = crate::support::files(&mut editor)
        .get(&format!("{beach}/image.png"))
        .map(|bytes| image::load_from_memory(bytes).expect("a png").to_rgba8());
    assert_eq!(image.map(|image| image.width()), Some(wide.width()));
    assert!(drawn.rect("Import, as one step").is_none(), "{drawn:?}");
}

#[test]
fn tiles_selected_on_a_sheet_take_the_brush_in_one_step() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let mut editor = crate::support::open(beach_game(), &dir);
    let beach = path("tileset", "beach");
    editor.open_tab(editor::tabs::EditorTab::Tileset { key: beach.clone() });
    editor.place_tabs();
    let mut frames = crate::support::Frames::new(egui::vec2(1600.0, 1000.0));
    let mut show = |editor: &mut editor::app::Editor, events| {
        frames.run(events, |ui| {
            editor::views::tileset::show(editor, ui, &beach)
        })
    };
    show(&mut editor, Vec::new());
    let selection = SheetRect {
        x: 2,
        y: 3,
        w: 4,
        h: 2,
    };
    let tiles: Vec<u32> = selection.tiles(art::COLUMNS).collect();
    editor.views.tileset.view_mut(&beach).selection = Some(selection);
    editor::views::tileset::select_tile(&mut editor, &beach, tiles[0]);
    let walks = |editor: &mut editor::app::Editor| -> Vec<Option<Walk>> {
        let row = crate::support::settled(editor)
            .read::<TilesetDef>(&beach)
            .expect("the tileset reads");
        tiles.iter().map(|tile| row.walk(*tile)).collect()
    };
    let before = walks(&mut editor);
    let paint = show(&mut editor, Vec::new()).at("Paint walk: blocks on these tiles");

    for events in crate::support::click(paint) {
        show(&mut editor, events);
    }

    assert_eq!(walks(&mut editor), vec![Some(Walk::Blocks); tiles.len()]);
    editor.undo();
    assert_eq!(walks(&mut editor), before);
}

#[test]
fn a_gone_tile_still_painted_blocks_until_replaced() {
    let (_, row) = beach();
    let old = art::tileset();
    let mut changed = old.clone();
    fill(
        &mut changed,
        (art::PAVED % art::COLUMNS) * TILE,
        (art::PAVED / art::COLUMNS) * TILE,
        TILE,
        TILE,
        [1, 2, 3, 255],
    );
    let beach = path("tileset", "beach");
    let palettes = vec![BundlePalette {
        bundle: path("area", "island"),
        rows: palette(&format!(
            "tileset,tile,flip\n{beach},{},\n{beach},{},\n",
            art::PAVED,
            art::PAVED + 1
        )),
    }];
    let mut plan = tileset::tile_plan(&old, &row, &changed, &beach, &palettes).expect("plans");
    assert_eq!(plan.gone, vec![art::PAVED]);
    assert!(plan.gone_used.contains_key(&art::PAVED));
    assert!(plan.ready().is_err());
    plan.replacements.insert(art::PAVED, Some(art::PAVED + 1));
    assert_eq!(plan.ready(), Ok(()));
    let (_, rewritten) = tileset::apply_tile_plan(&plan, &row, &beach, &palettes).unwrap();
    assert_eq!(rewritten[0].rows[0].tile, art::PAVED + 1);
}

#[test]
fn an_animation_lands_on_a_block_reusing_frames_found_by_pixels() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let (_, row) = beach();
    let water = art::tileset();
    let frames = art::water(&water);
    let gif = exports::gif(dir.path(), "beach-water", &frames, 25);
    let export = formats::decode(&formats::sort_out(&[gif]).unwrap()).unwrap();
    let plan = tileset::anim_plan(&water, &row, &export, true, (1, 1)).expect("plans");
    assert_eq!(
        plan.animated() as u32,
        art::WATER * art::WATER - art::STILL_WATER
    );
    assert_eq!(plan.still as u32, art::STILL_WATER);
    assert!(
        plan.new_images.is_empty(),
        "every frame is on the tileset already"
    );
    assert!(plan.fits.is_some());
    let (png, new_row) = tileset::apply_anim_plan(&plan, &water, &row, &export).unwrap();
    assert!(png.is_none(), "nothing appended, nothing redrawn");
    let first = art::COLUMNS + 1;
    let animation = &new_row.tiles[&first].animation;
    let tiles: Vec<u32> = animation.iter().map(|frame| frame.tile).collect();
    assert_eq!(tiles, (0..4).map(|k| first + 9 * k).collect::<Vec<_>>());
    assert!(animation.iter().all(|frame| frame.ms == 250));
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
    fill(&mut fresh[1], 0, 0, TILE, TILE, [9, 9, 9, 255]);
    let gif = exports::gif(dir.path(), "beach-water-new", &fresh, 25);
    let export = formats::decode(&formats::sort_out(&[gif]).unwrap()).unwrap();
    let plan = tileset::anim_plan(&water, &row, &export, true, (1, 1)).unwrap();
    assert_eq!(plan.new_images.len(), 1);
    let (png, new_row) = tileset::apply_anim_plan(&plan, &water, &row, &export).unwrap();
    let grown = formats::decode_image_bytes(&png.expect("appended")).unwrap();
    assert_eq!(
        grown.height(),
        water.height() + TILE,
        "new frames go after the last tile"
    );
    assert_eq!(
        new_row.tiles[&first].animation[1].tile,
        art::COLUMNS * art::ROWS
    );
}

#[test]
fn import_records_written_by_earlier_editors_still_read() {
    let written = serde_json::json!([
        {
            "export": "/art/beach.png",
            "files": ["/art/beach.png"],
            "table": "tileset",
            "key": "beach",
            "mapping": { "grid": { "tile": 16, "margin": 2, "spacing": 1, "extruded": 1 } },
            "size": 4,
            "hash": "2233",
            "strips": {},
            "part": ""
        },
        {
            "export": "/art/water.gif",
            "files": ["/art/water.gif"],
            "table": "tileset",
            "key": "beach",
            "mapping": { "animation": { "column": 3, "row": 4 } },
            "size": 4,
            "hash": "4455",
            "strips": {},
            "part": "animation 3,4"
        }
    ]);
    let records: Vec<ImportRecord> = serde_json::from_value(written).expect("reads");
    assert_eq!(
        records[0].mapping,
        ImportMapping::Grid {
            grid: tileset::TileGridSettings {
                tile: 16,
                margin: 2,
                spacing: 1,
                extruded: 1,
            }
        }
    );
    assert_eq!(
        records.iter().map(ImportRecord::block).collect::<Vec<_>>(),
        vec![None, Some(tileset::TileBlock { column: 3, row: 4 })]
    );
    let rewritten: Vec<ImportRecord> =
        serde_json::from_value(serde_json::to_value(&records).unwrap()).expect("reads back");
    assert!(
        rewritten
            .iter()
            .zip(&records)
            .all(|(again, record)| again.mapping == record.mapping && again.files == record.files)
    );
}

/// The beach tileset of `beach_game`, read as the game reads it, and the reader it was read with.
fn beach() -> (FolderReader, TilesetDef) {
    let reader = FolderReader::new(
        engine::systems::registry(),
        std::sync::Arc::new(beach_game().folder()),
    );
    let tileset = reader
        .read::<TilesetDef>(&path("tileset", "beach"))
        .expect("the beach tileset reads");
    (reader, tileset)
}

/// A whole game whose island is painted with the beach tileset.
fn beach_game() -> testkit::Game {
    let mut tileset = Vec::new();
    art::tileset()
        .write_to(
            &mut std::io::Cursor::new(&mut tileset),
            image::ImageFormat::Png,
        )
        .unwrap();
    testkit::Game::new()
        .area("island", &testkit::Map::open(5, 5), serde_json::json!([]))
        .hero("island", 2.0, 2.0)
        .engine_rows()
        .sfx("step")
        .row(
            "surface",
            "sand",
            serde_json::json!({ "name": "Sand", "footstep": path("sfx", "step") }),
        )
        .row("tileset", "beach", art::tileset_row())
        .part("tileset", "beach", "image.png", tileset)
        .part("area", "island", "palette.csv", art::island_palette())
        .whole()
}

fn palette(text: &str) -> Vec<PaletteRow> {
    let file = grid::GridFile {
        path: "palette.csv",
        text,
    };
    grid::read_palette(file, &mut Vec::new()).expect("a palette")
}
