use std::fs;
use std::path::Path;
use std::sync::Arc;

use engine::core::content::gltf;
use engine::core::content::{ContentFolder, FileHash, FilesystemFolder, FolderReader};
use engine::core::units::Meters;
use engine::systems::area::prop::{Footprint, ModelHash, PropDef};
use engine::systems::{player, registry};

/// Shapes the footprint of each of `props`, named by its path in the game folder, round all of
/// its model that whoever stands `tall` would meet: the game's player, unless a height is given.
pub fn run(game: &Path, tall: Option<Meters>, props: &[String]) {
    let files = Arc::new(FilesystemFolder(game.to_path_buf()));
    let reader = FolderReader::new(registry(), files.clone());
    let tall = tall.unwrap_or_else(|| {
        player::stature_in_folder(&reader)
            .unwrap_or_else(|errors| panic!("the game's character does not read: {errors:?}"))
    });
    for prop in props {
        let row: PropDef = reader
            .read(prop)
            .unwrap_or_else(|errors| panic!("{prop} does not read as a prop: {errors:?}"));
        let model = files
            .read(&row.model.path())
            .unwrap_or_else(|error| panic!("{prop}'s model: {error}"));
        let triangles =
            gltf::triangles(&model).unwrap_or_else(|error| panic!("{prop}'s model: {error}"));
        let fitted_to = ModelHash(FileHash::of(&model).hash);
        let Some(footprint) = Footprint::fitted(&triangles, tall, fitted_to) else {
            eprintln!("{prop}: its model has no ground to stand on");
            std::process::exit(1);
        };
        let fitted = PropDef { footprint, ..row };
        let (file, bytes) = reader
            .write_fields(prop, &fitted)
            .unwrap_or_else(|error| panic!("{prop}: {error}"));
        fs::write(game.join(&file), bytes).expect("a prop of the game written");
        println!("{file}: fitted");
    }
}
