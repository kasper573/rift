use bevy::prelude::*;
use engine::core::render::world::material::{
    Glass, MaterialLook, SurfaceFinish, casts_shadow, world_material,
};
use engine::core::vfx::uniform::ParamsUniform;

#[test]
fn a_models_glass_keeps_its_own_finish_and_lets_light_through_where_it_is_seen_through() {
    let finish = SurfaceFinish {
        roughness: 0.8,
        reflectance: 0.5,
        metal_roughness: 0.45,
        metallic: 0.7,
    };
    let pane = StandardMaterial {
        specular_transmission: 1.0,
        perceptual_roughness: 0.1,
        ior: 1.5,
        ..default()
    };

    let clear = world_material(pane.clone(), &finish, Glass::Clear).base;
    assert_eq!(
        (clear.specular_transmission, clear.perceptual_roughness),
        (1.0, 0.1),
        "clear glass lets light through, as smooth as its model made it"
    );

    let solid = world_material(pane, &finish, Glass::Solid).base;
    assert_eq!(
        (solid.specular_transmission, solid.perceptual_roughness),
        (0.0, 0.1),
        "solid glass lets none through, and stays as smooth"
    );

    let wood = world_material(StandardMaterial::default(), &finish, Glass::Clear).base;
    assert_eq!(
        (wood.specular_transmission, wood.perceptual_roughness),
        (0.0, finish.roughness),
        "what is no glass takes the world's finish"
    );
}

#[test]
fn glass_and_a_material_set_to_cast_no_shadow_let_light_through() {
    let look = |shadow| MaterialLook {
        material: "cage".to_owned(),
        color: None,
        normal: None,
        wear: None,
        shader: None,
        params: ParamsUniform::default(),
        translucency: 0.0,
        shadow,
    };
    assert!(casts_shadow(None, false), "a model's own material casts");
    assert!(!casts_shadow(None, true), "a pane of its glass doesn't");
    assert!(casts_shadow(Some(&look(true)), false), "a look casts");
    assert!(
        !casts_shadow(Some(&look(false)), false),
        "unless its material is set to cast none"
    );
    assert!(
        casts_shadow(Some(&look(true)), true),
        "and a look drawn over glass casts as its material says"
    );
}
