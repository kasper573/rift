use image::{Image, encode_png};
use tiled::{Prop, Tilesets, load_map};

#[test]
fn csv_tile_layer_loads() {
    let map = load_map(
        r#"{"width":2,"height":1,"tilewidth":16,"tileheight":16,
            "orientation":"orthogonal",
            "layers":[{"type":"tilelayer","name":"Ground","width":2,"height":1,
                       "encoding":"csv","data":[1,2]}],
            "tilesets":[{"firstgid":1,"source":"x.json"}]}"#,
    )
    .unwrap();
    assert_eq!((map.width.0, map.height.0), (2.0, 1.0));
    let ground = map.tile_layer("Ground").unwrap();
    assert_eq!((ground.at(0, 0), ground.at(1, 0)), (1, 2));
    assert_eq!(ground.at(-1, 0), 0, "outside the layer is empty");
    assert_eq!(ground.at(2, 0), 0, "outside the layer is empty");
    assert_eq!(
        ground.cells().collect::<Vec<_>>(),
        vec![(0, 0, 1), (1, 0, 2)]
    );
    assert_eq!(map.tilesets[0].first_gid, 1);
}

#[test]
fn base64_tile_layer_loads() {
    let map = load_map(
        r#"{"width":2,"height":1,"tilewidth":16,"tileheight":16,
            "orientation":"orthogonal",
            "layers":[{"type":"tilelayer","name":"Ground","width":2,"height":1,
                       "encoding":"base64","data":"AQAAAAIAAAA="}],
            "tilesets":[{"firstgid":1,"source":"x.json"}]}"#,
    )
    .unwrap();
    let cells: Vec<u32> = map
        .tile_layer("Ground")
        .unwrap()
        .cells()
        .map(|(_, _, raw)| raw)
        .collect();
    assert_eq!(cells, vec![1, 2]);
}

fn fixture() -> Tilesets {
    let map = load_map(
        r#"{"width":1,"height":1,"tilewidth":16,"tileheight":16,
            "orientation":"orthogonal","layers":[],
            "tilesets":[{"firstgid":1,"source":"ts.json"}]}"#,
    )
    .unwrap();
    let tileset: &'static [u8] = br#"{"columns":2,"tilecount":4,"tilewidth":16,"tileheight":16,
        "image":"ts.png","imagewidth":32,"imageheight":32,
        "tiles":[{"id":0,"animation":[{"tileid":0,"duration":100},{"tileid":3,"duration":100}],
                  "properties":[{"name":"Walkable","type":"bool","value":true}]}]}"#;
    let png: &'static [u8] = Box::leak(encode_png(&Image::new(32, 32)).into_boxed_slice());
    Tilesets::load(&map, |path| match path {
        "ts.json" => Some(tileset),
        "ts.png" => Some(png),
        _ => None,
    })
    .unwrap()
}

#[test]
fn tilesets_resolve_regions_animation_and_flips() {
    let tilesets = fixture();
    assert!(
        tilesets.resolve(0, 0.0).is_none(),
        "gid 0 is the empty cell"
    );

    let (image, first, flip) = tilesets.resolve(1, 0.05).unwrap();
    assert_eq!((image.width, image.height), (32, 32));
    assert_eq!(
        (first.pos.x.0, first.pos.y.0, first.size.x.0, first.size.y.0),
        (0.0, 0.0, 16.0, 16.0)
    );
    assert_eq!(flip, (false, false));

    let (_, second, _) = tilesets.resolve(1, 0.15).unwrap();
    assert_eq!(
        (second.pos.x.0, second.pos.y.0),
        (16.0, 16.0),
        "the animation advances to tile 3"
    );

    let (_, _, flipped) = tilesets.resolve(1 | tiled::FLIP_H, 0.0).unwrap();
    assert_eq!(flipped, (true, false));
}

#[test]
fn tilesets_expose_tile_properties() {
    let tilesets = fixture();
    assert_eq!(tilesets.property(1, "Walkable"), Some(&Prop::Bool(true)));
    assert_eq!(tilesets.property(2, "Walkable"), None);
    assert_eq!(tilesets.property(0, "Walkable"), None);
}
