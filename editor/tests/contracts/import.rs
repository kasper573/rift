use std::io::Write;
use std::path::Path;

use editor::import;
use engine::core::content::gltf;
use testkit::png;

use crate::support;

/// An FBX file in centimetres, Z up, of one joint a metre above its root.
const HIP_FBX: &str = r#"; FBX 7.4.0 project file
FBXHeaderExtension:  {
	FBXHeaderVersion: 1003
	FBXVersion: 7400
}
GlobalSettings:  {
	Version: 1000
	Properties70:  {
		P: "UpAxis", "int", "Integer", "",2
		P: "UpAxisSign", "int", "Integer", "",1
		P: "FrontAxis", "int", "Integer", "",1
		P: "FrontAxisSign", "int", "Integer", "",-1
		P: "CoordAxis", "int", "Integer", "",0
		P: "CoordAxisSign", "int", "Integer", "",1
		P: "UnitScaleFactor", "double", "Number", "",1
	}
}
Objects:  {
	Model: 1000, "Model::root", "Null" {
		Version: 232
	}
	Model: 1001, "Model::hip", "LimbNode" {
		Version: 232
		Properties70:  {
			P: "Lcl Translation", "Lcl Translation", "", "A",0,0,100
		}
	}
}
Connections:  {
	C: "OO",1000,0
	C: "OO",1001,1000
}
"#;

#[test]
fn importing_brings_folders_whole_unpacks_archives_and_converts_fbx() {
    let scratch = tempfile::tempdir().expect("scratch");
    let game = support::game().file("art/Pack/readme.txt", b"the game's own".to_vec());
    let mut editor = support::open(game, &scratch);
    let picked = tempfile::tempdir().expect("picked");
    let pack = picked.path().join("Pack");
    write(&pack.join("Textures/Wood Planks.png"), &png(4, 4));
    write(&pack.join("readme.txt"), b"the seller's");
    write(&pack.join("Hero.fbx"), HIP_FBX.as_bytes());
    write(&pack.join("hero.item.json"), b"{}");
    write(&pack.join(".DS_Store"), b"");
    let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    archive
        .start_file("deep/Rock.png", zip::write::SimpleFileOptions::default())
        .expect("an entry");
    archive.write_all(&png(2, 2)).expect("its bytes");
    let archive = archive.finish().expect("an archive").into_inner();
    write(&pack.join("Extras.zip"), &archive);

    let (made, failed) =
        import::import_now(&mut editor, "art", std::slice::from_ref(&pack)).expect("imported");
    assert!(
        failed.is_empty(),
        "{:?}",
        failed.iter().map(|f| &f.why).collect::<Vec<_>>()
    );
    let mut made = made;
    made.sort();
    assert_eq!(
        made,
        [
            "art/Pack/Extras/deep/Rock.png",
            "art/Pack/Hero.glb",
            "art/Pack/Textures/Wood Planks.png",
        ],
        "a file the game holds already, one named as content and a hidden one stay out"
    );
    let files = support::files(&mut editor);
    assert_eq!(
        files.get("art/Pack/readme.txt").map(|bytes| bytes.to_vec()),
        Some(b"the game's own".to_vec())
    );
    let hero = gltf::inspect(files.get("art/Pack/Hero.glb").expect("the converted model"))
        .expect("a glTF binary");
    let hip = hero.tree.named("hip").expect("the hip joint");
    let standing = hero.tree.worlds()[hip].w_axis;
    assert!(
        (standing.y - 1.0).abs() < 1e-4 && standing.z.abs() < 1e-4,
        "converted to metres with Y up: {standing:?}"
    );

    let project = support::settled(&mut editor);
    let (mut incoming, _) = import::review(
        &project.files,
        "art/Pack",
        import::gather(&[pack.join("readme.txt")], &|_| {}),
    );
    let readme = &mut incoming[0];
    assert!(
        readme.replaces && !readme.take,
        "a file already there is kept unless picked"
    );
    readme.take = true;
    import::import(&mut editor, incoming).expect("replaced");
    assert_eq!(
        support::files(&mut editor)
            .get("art/Pack/readme.txt")
            .map(|bytes| bytes.to_vec()),
        Some(b"the seller's".to_vec())
    );
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().expect("a folder")).expect("its folder");
    std::fs::write(path, bytes).expect("written");
}
