//! What floats is drawn over the sheet it floats on. A sheet of water lies level, so the inside
//! of a hull that rides lower than the surface would show the water standing in it. Every mesh
//! under something afloat is therefore drawn a second time once the sheets are down, wherever
//! it is what the eye meets first: a boat shows its own boards, and whoever sits in it their
//! own legs.

use bevy::asset::AssetEvent;
use bevy::light::NotShadowCaster;
use bevy::mesh::skinning::SkinnedMesh;
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;

use super::material::WorldMaterial;
use super::opacity::Opacity;

/// Marks a body that floats: it and all that hangs under it is drawn over the sheets, and the
/// water it rides bears it, so its limbs raise no rings where they dip under the surface.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Afloat;

/// A mesh drawn again over the sheets, held by the mesh it repeats, in the material made from
/// the one that mesh had when it was made.
#[derive(Component)]
pub(super) struct OverSheets {
    of: AssetId<WorldMaterial>,
}

/// The material that draws over the sheets for each material a floating mesh is drawn with.
#[derive(Resource, Default)]
pub(super) struct OverSheetMaterials(HashMap<AssetId<WorldMaterial>, Handle<WorldMaterial>>);

type Floating<'a> = (
    &'a Mesh3d,
    &'a MeshMaterial3d<WorldMaterial>,
    Option<Ref<'a, SkinnedMesh>>,
);

/// Keeps a second mesh over the sheets for every mesh under something afloat that shows whole,
/// in step with the mesh it repeats, and takes away those whose mesh floats no longer.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_over_sheets(
    afloat: Query<Entity, With<Afloat>>,
    children: Query<&Children>,
    holders: Query<&ChildOf>,
    opacities: Query<&Opacity>,
    meshes: Query<Floating, Without<OverSheets>>,
    repeats: Query<(Entity, &OverSheets, &ChildOf)>,
    mut events: MessageReader<AssetEvent<WorldMaterial>>,
    mut materials: ResMut<Assets<WorldMaterial>>,
    mut over: ResMut<OverSheetMaterials>,
    mut commands: Commands,
) {
    let stale: HashSet<AssetId<WorldMaterial>> = events
        .read()
        .filter_map(|event| match *event {
            AssetEvent::Modified { id } | AssetEvent::Removed { id } => Some(id),
            _ => None,
        })
        .filter(|id| over.0.remove(id).is_some())
        .collect();
    let floating: HashSet<Entity> = afloat
        .iter()
        .filter(|body| Opacity::inherited(*body, &opacities, &holders) >= 1.0)
        .flat_map(|body| std::iter::once(body).chain(children.iter_descendants(body)))
        .filter(|part| meshes.contains(*part))
        .collect();
    let mut repeated: HashSet<Entity> = HashSet::new();
    for (repeat, drawn, held) in &repeats {
        let mesh = held.parent();
        let current = meshes
            .get(mesh)
            .ok()
            .filter(|_| floating.contains(&mesh))
            .filter(|(_, material, _)| material.id() == drawn.of && !stale.contains(&drawn.of));
        match current {
            Some((_, _, skin)) => {
                repeated.insert(mesh);
                if let Some(skin) = skin.filter(|skin| skin.is_changed()) {
                    commands
                        .entity(repeat)
                        .try_insert(SkinnedMesh::clone(&skin));
                }
            }
            None => commands.entity(repeat).try_despawn(),
        }
    }
    for mesh in floating.difference(&repeated) {
        let Ok((drawn, material, skin)) = meshes.get(*mesh) else {
            continue;
        };
        let of = material.id();
        let Some(over) = over_sheets(&mut materials, &mut over, of) else {
            continue;
        };
        let mut repeat = commands.spawn((
            ChildOf(*mesh),
            OverSheets { of },
            drawn.clone(),
            MeshMaterial3d(over),
            Transform::IDENTITY,
            NotShadowCaster,
        ));
        if let Some(skin) = skin {
            repeat.insert(SkinnedMesh::clone(&skin));
        }
    }
}

/// The material drawing over the sheets what the material `of` draws: the same, blended, so it
/// is drawn once the sheets are and leaves the depth as the mesh it repeats wrote it.
fn over_sheets(
    materials: &mut Assets<WorldMaterial>,
    over: &mut OverSheetMaterials,
    of: AssetId<WorldMaterial>,
) -> Option<Handle<WorldMaterial>> {
    if let Some(made) = over.0.get(&of) {
        return Some(made.clone());
    }
    let mut material = materials.get(of)?.clone();
    material.base.alpha_mode = AlphaMode::Blend;
    material.base.depth_bias = OVER_ITS_MESH;
    let made = materials.add(material);
    over.0.insert(of, made.clone());
    Some(made)
}

/// How much nearer than the mesh it repeats a mesh over the sheets is drawn, in steps of the
/// depth: enough that it never loses to the depth that mesh wrote.
const OVER_ITS_MESH: f32 = 8.0;
