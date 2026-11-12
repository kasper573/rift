use bevy::prelude::*;
use engine::core::render::world::material::{Glass, SurfaceFinish, world_material};

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
