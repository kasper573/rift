//! Light a vfx lays over a thing's own lit surface: a tint, a rim round its silhouette and a glow,
//! over its body and over what it holds apart. Every world material reads them from one buffer by
//! the slot its mesh's tag names, so a thing lit apart from the rest keeps sharing its materials
//! with them. The tag also tells how much of a fading thing has gone.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::MeshTag;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;
use bevy::render::storage::ShaderBuffer;

use super::afloat::WaterLid;
use super::opacity::Opacity;

/// What a vfx lays over a thing this frame: over its body, and over what it holds.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct SurfaceShine {
    pub body: SurfaceLook,
    pub held: SurfaceLook,
}

/// One look laid over a surface, each colour in linear light with its strength in alpha.
#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct SurfaceLook {
    /// The colour the surface takes, by its own lightness.
    pub tint: Vec4,
    /// Light round the silhouette, brightest where the surface turns away from the eye.
    pub rim: Vec4,
    /// Light added evenly over the surface.
    pub glow: Vec4,
    /// A sphere the look is seen within, fading out past it: xyz its centre, w its radius, 0 for
    /// everywhere.
    pub mask: Vec4,
}

/// A piece of what a thing holds, which a look laid over what the thing holds covers rather than
/// one laid over its body.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct HeldPiece;

/// The systems that tag every mesh with its fade and its look: what lays a look this frame does so
/// before them.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MeshesTagged;

/// The buffer every world material reads the looks from, by slot; slot 0 is no look at all.
pub const SHINE_BUFFER: Handle<ShaderBuffer> = Handle::Uuid(
    bevy::asset::uuid::Uuid::from_u128(0x5e1f_ace0_0000_4000_8000_0000_0000_0109),
    std::marker::PhantomData,
);

/// How many of a tag's low bits tell how much of a mesh has faded; the bits over them name its
/// look's slot.
const FADE_BITS: u32 = 8;

pub(super) fn install(app: &mut App) {
    app.init_asset::<ShaderBuffer>();
    let mut buffers = app.world_mut().resource_mut::<Assets<ShaderBuffer>>();
    let none = ShaderBuffer::new(looks_words(&[]), RenderAssetUsages::default());
    buffers
        .insert(SHINE_BUFFER.id(), none)
        .expect("a uuid asset id never goes stale");
}

/// Tags every mesh under a fading or shining thing with how much of it to drop and the slot of the
/// look it wears, those its scene spawns among them meanwhile, and lays the looks in the buffer.
/// A mesh under a thing that stopped shining loses its look. A lid over the water keeps its tag
/// for the water's level.
#[allow(clippy::too_many_arguments)]
pub(super) fn tag(
    fading: Query<(Entity, Ref<Opacity>)>,
    shining: Query<(Entity, &SurfaceShine)>,
    mut stopped: RemovedComponents<SurfaceShine>,
    opacities: Query<&Opacity>,
    holders: Query<&ChildOf>,
    children: Query<&Children>,
    meshes: Query<Option<&MeshTag>, (With<Mesh3d>, Without<WaterLid>)>,
    held: Query<(), With<HeldPiece>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut laid: Local<Vec<SurfaceLook>>,
    mut commands: Commands,
) {
    let mut roots: HashMap<Entity, (u32, Option<(u32, u32)>)> = HashMap::default();
    for (entity, opacity) in &fading {
        if opacity.is_changed() || opacity.0 < 1.0 {
            roots.insert(entity, (dropped(opacity.0), None));
        }
    }
    for entity in stopped.read() {
        let shown = Opacity::inherited(entity, &opacities, &holders);
        roots.entry(entity).or_insert((dropped(shown), None));
    }
    let mut looks = Vec::new();
    for (entity, shine) in &shining {
        let slot = looks.len() as u32 + 1;
        looks.extend([shine.body, shine.held]);
        let shown = Opacity::inherited(entity, &opacities, &holders);
        roots.entry(entity).or_insert((dropped(shown), None)).1 = Some((slot, slot + 1));
    }
    if *laid != looks {
        if let Some(mut buffer) = buffers.get_mut(&SHINE_BUFFER) {
            *buffer = ShaderBuffer::new(looks_words(&looks), RenderAssetUsages::default());
        }
        *laid = looks;
    }
    for (root, (dropped, slots)) in roots {
        let mut pending = vec![(root, false)];
        while let Some((part, in_held)) = pending.pop() {
            let in_held = in_held || held.contains(part);
            if let Ok(tagged) = meshes.get(part) {
                let slot = slots.map_or(0, |(body, held)| if in_held { held } else { body });
                let wanted = dropped | slot << FADE_BITS;
                if tagged.map_or(0, |tag| tag.value) != wanted {
                    commands.entity(part).try_insert(MeshTag::new(wanted));
                }
            }
            if let Ok(under) = children.get(part) {
                pending.extend(under.iter().map(|child| (child, in_held)));
            }
        }
    }
}

/// How many of 255 pixels a mesh drops that shows `opacity` of itself.
fn dropped(opacity: f32) -> u32 {
    ((1.0 - opacity.clamp(0.0, 1.0)) * 255.0).round() as u32
}

/// The looks as the buffer holds them, after the empty one at slot 0.
fn looks_words(looks: &[SurfaceLook]) -> Vec<f32> {
    std::iter::once(SurfaceLook::default())
        .chain(looks.iter().copied())
        .flat_map(|look| {
            [look.tint, look.rim, look.glow, look.mask]
                .into_iter()
                .flat_map(|word| word.to_array())
        })
        .collect()
}
