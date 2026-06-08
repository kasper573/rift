use world::features::sfx::{SfxDef, SfxPitch, SfxVolume};

#[test]
fn fixed_resolves_to_its_level_ignoring_the_roll() {
    assert_eq!(SfxVolume::Fixed(0.5).resolve(0.0), 0.5);
    assert_eq!(SfxVolume::Fixed(0.5).resolve(1.0), 0.5);
}

#[test]
fn random_maps_the_roll_across_its_range() {
    let v = SfxVolume::Random(0.8, 1.0);
    assert!((v.resolve(0.0) - 0.8).abs() < 1e-6);
    assert!((v.resolve(1.0) - 1.0).abs() < 1e-6);
    assert!((v.resolve(0.5) - 0.9).abs() < 1e-6);
}

#[test]
fn a_number_parses_as_fixed_and_an_array_as_random() {
    assert_eq!(
        serde_json::from_str::<SfxVolume>("0.7").unwrap(),
        SfxVolume::Fixed(0.7)
    );
    assert_eq!(
        serde_json::from_str::<SfxVolume>("[0.8, 1.0]").unwrap(),
        SfxVolume::Random(0.8, 1.0)
    );
}

#[test]
fn an_absent_volume_defaults_to_fixed_one() {
    let def: SfxDef = serde_json::from_str(r#"{ "id": "x", "src": "y.wav" }"#).unwrap();
    assert_eq!(def.volume, SfxVolume::Fixed(1.0));
}

#[test]
fn pitch_random_maps_the_roll_across_its_range() {
    let p = SfxPitch::Random(0.9, 1.2);
    assert!((p.resolve(0.0) - 0.9).abs() < 1e-6);
    assert!((p.resolve(1.0) - 1.2).abs() < 1e-6);
}

#[test]
fn a_pitch_number_parses_as_fixed_and_an_array_as_random() {
    assert_eq!(
        serde_json::from_str::<SfxPitch>("1.5").unwrap(),
        SfxPitch::Fixed(1.5)
    );
    assert_eq!(
        serde_json::from_str::<SfxPitch>("[0.9, 1.2]").unwrap(),
        SfxPitch::Random(0.9, 1.2)
    );
}

#[test]
fn an_absent_pitch_defaults_to_fixed_one() {
    let def: SfxDef = serde_json::from_str(r#"{ "id": "x", "src": "y.wav" }"#).unwrap();
    assert_eq!(def.pitch, SfxPitch::Fixed(1.0));
}
