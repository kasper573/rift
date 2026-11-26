//! Draws vfx on the client, every layer in three dimensions: shapes, coats and patches of ground
//! drawn with their effect shaders, models, particles, lights, the looks laid over a thing's own
//! surface and the shake of the camera. Text the client letters is drawn with the text stage's
//! shaders over the presented frame.

mod effect;
mod particles;
mod play;
mod scene;
mod shapes;
mod site;
mod surface;

pub use effect::EffectMaterial;
pub use scene::{DrawnElsewhere, SiteExtent, VfxHand};
pub use site::{
    Run, SiteBody, SiteFrame, SitePlace, SiteTexture, TEXELS_PER_METER, TextMaterial, hide,
    present_shader, screen_piece,
};

use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::time::Duration;

use bevy::asset::uuid::Uuid;
use bevy::pbr::MaterialPlugin;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::shader::Shader;
use bevy::sprite_render::Material2dPlugin;
use bevy_hanabi::HanabiPlugin;

use super::{CompiledVfxShaders, VfxShaderId, standing};
use crate::core::content::Content;
use crate::core::render::sample::FrameWork;
use crate::core::render::transition::WorldViewSystems;
use crate::core::render::world::BodiesPosed;
use crate::core::render::world::shine::MeshesTagged;

/// The materials every shader stage drawn by the engine is drawn with, and the content's shaders
/// compiled for them: what any world that draws content needs, the client's and the editor's
/// alike.
pub struct VfxMaterialsPlugin;

impl Plugin for VfxMaterialsPlugin {
    fn build(&self, app: &mut App) {
        bevy::shader::load_shader_library!(app, "effect_frame.wesl");
        bevy::asset::embedded_asset!(app, "effect_vertex.wesl");
        app.add_plugins(Material2dPlugin::<TextMaterial>::default())
            .add_plugins(MaterialPlugin::<EffectMaterial>::default())
            .init_resource::<VfxShaders>()
            .init_resource::<FrameWork>()
            .add_systems(
                Update,
                load_shaders.run_if(resource_exists_and_changed::<Content>),
            );
        // The world's shaders sway on the frame's time from bevy's globals: wrapping it, as
        // bevy does every hour by default, would jump every swaying thing at once.
        if let Some(mut time) = app.world_mut().get_resource_mut::<Time<Virtual>>() {
            time.set_wrap_period(Duration::MAX);
        }
    }
}

/// The client's vfx: every instance drawn on its site as the game plays it.
pub struct VfxRenderPlugin;

impl Plugin for VfxRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((VfxMaterialsPlugin, HanabiPlugin))
            .init_resource::<play::ParticleAssets>()
            .init_resource::<play::Shining>()
            .add_observer(play::forget)
            .add_systems(
                Update,
                play::recompile.run_if(resource_exists_and_changed::<Content>),
            )
            .add_systems(
                PostUpdate,
                (standing::play, play::play)
                    .chain()
                    .in_set(WorldViewSystems)
                    .after(BodiesPosed)
                    .before(MeshesTagged)
                    .before(bevy::transform::TransformSystems::Propagate),
            );
    }
}

/// The shader a row compiles to for the content now loaded, once the client has built it.
pub fn compiled_shader(world: &World, shader: VfxShaderId) -> Option<Handle<Shader>> {
    world
        .get_resource::<VfxShaders>()?
        .by_id
        .get(&shader)
        .cloned()
}

// A shader asset is keyed by its path and its text, so a load of the content keeps the pipelines
// compiled for every shader whose text it didn't change, and compiles anew the ones it did.
#[derive(Resource, Default)]
struct VfxShaders {
    by_id: HashMap<VfxShaderId, Handle<Shader>>,
}

fn load_shaders(
    content: Res<Content>,
    mut shaders: ResMut<VfxShaders>,
    mut assets: ResMut<Assets<Shader>>,
) {
    let sources = content.built::<CompiledVfxShaders>();
    let by_id: HashMap<VfxShaderId, Handle<Shader>> = sources
        .0
        .iter()
        .map(|(id, source)| {
            let wesl = id.get(&content).stage.composes_wesl();
            let mut text = std::collections::hash_map::DefaultHasher::new();
            source.hash(&mut text);
            let name = format!(
                "vfx_shader/{}-{:016x}.{}",
                id.path(&content),
                text.finish(),
                if wesl { "wesl" } else { "wgsl" }
            );
            let kept = named(&name);
            let shader = if assets.contains(kept.id()) {
                kept
            } else if wesl {
                compiled(
                    &mut assets,
                    Shader::from_wesl(source.clone(), name.clone()),
                    &name,
                )
            } else {
                compiled(
                    &mut assets,
                    Shader::from_wgsl(source.clone(), name.clone()),
                    &name,
                )
            };
            (*id, shader)
        })
        .collect();
    for gone in shaders.by_id.values() {
        if !by_id.values().any(|kept| kept == gone) {
            assets.remove(gone.id());
        }
    }
    shaders.by_id = by_id;
}

/// Holds a compiled shader under the uuid of its name.
fn compiled(assets: &mut Assets<Shader>, shader: Shader, name: &str) -> Handle<Shader> {
    let handle = named(name);
    assets
        .insert(handle.id(), shader)
        .expect("a uuid asset id never goes stale");
    handle
}

/// The handle of the shader asset named `name`.
fn named(name: &str) -> Handle<Shader> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut hasher);
    let uuid = Uuid::from_u128(u128::from(hasher.finish()) << 64 | u128::from(name.len() as u64));
    Handle::Uuid(uuid, PhantomData)
}

/// Paints the material `handle` as `painted`, unless `same` finds it painted so already: a
/// material written to is prepared anew for the GPU, so one that holds still is left alone.
fn repaint<M: Asset>(world: &mut World, handle: &Handle<M>, painted: M, same: fn(&M, &M) -> bool) {
    let mut materials = world.resource_mut::<Assets<M>>();
    if materials.get(handle).is_some_and(|was| same(was, &painted)) {
        return;
    }
    let Some(mut material) = materials.get_mut(handle) else {
        return;
    };
    *material = painted;
    drop(material);
    world.resource_mut::<FrameWork>().vfx_writes += 1;
}
