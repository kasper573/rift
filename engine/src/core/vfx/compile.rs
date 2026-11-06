//! The content's shaders compiled as the renderer compiles them, without a GPU: each composed with
//! its stage's own header, as the client composes it, against bevy's shader libraries and the
//! engine's, under the shader defs of every pass the stage draws in, and validated. What a shader's
//! file alone can't show shows here: a header importing what no library holds, or a pass whose
//! defs leave the shader reading what isn't there.

use std::time::{Duration, Instant};

use bevy::app::PluginsState;
use bevy::pbr::MATERIAL_BIND_GROUP_INDEX;
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::settings::{RenderCreation, WgpuSettings};
use bevy::shader::{ShaderCache, ShaderCacheError, ShaderCacheSource, ShaderDefVal};
use bevy::window::ExitCondition;

use super::render::{VfxMaterialsPlugin, compiled_shader};
use super::shader::{Stage, VfxShaderDef, VfxShaderId};
use crate::core::content::Content;
use crate::core::render::Look;
use crate::core::render::world::WorldScenePlugin;

/// A shader that doesn't compile in a pass it draws in, and what the compiler says.
#[derive(Clone, Debug)]
pub struct UncompiledShader {
    pub shader: VfxShaderId,
    pub pass: String,
    pub message: String,
}

/// Every shader of `content` that doesn't compile in a pass it draws in, as the renderer compiles
/// it for a game that looks as `look` says.
pub fn uncompiled(content: &Content, look: Look) -> Vec<UncompiledShader> {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .set(RenderPlugin {
                render_creation: RenderCreation::Automatic(Box::new(WgpuSettings {
                    backends: None,
                    ..default()
                })),
                ..default()
            })
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::log::LogPlugin>(),
    )
    .add_plugins((WorldScenePlugin, VfxMaterialsPlugin))
    .insert_resource(look)
    .insert_resource(content.clone());
    settle(&mut app);
    let mut cache: ShaderCache<(), ()> = ShaderCache::new((), validate);
    for (id, shader) in app.world().resource::<Assets<Shader>>().iter() {
        cache.set_shader(id, shader.clone());
    }
    let mut failed = Vec::new();
    for id in content.ids::<VfxShaderDef>() {
        let Some(handle) = compiled_shader(app.world(), id) else {
            continue;
        };
        for (pass, defs) in passes(id.get(content).stage) {
            let message = match cache.get(0, handle.id(), &defs) {
                Ok(_) => continue,
                Err(ShaderCacheError::ShaderImportNotYetAvailable) => {
                    "imports a module no shader library holds".to_owned()
                }
                Err(error) => error.to_string(),
            };
            failed.push(UncompiledShader {
                shader: id,
                pass: pass.to_owned(),
                message,
            });
        }
    }
    failed
}

/// Updates `app` until its shaders have loaded: the content's composed, and every library's,
/// which load as assets of their own a few frames on.
fn settle(app: &mut App) {
    let began = Instant::now();
    while app.plugins_state() == PluginsState::Adding && began.elapsed() < LOADING_AT_MOST {
        bevy::tasks::tick_global_task_pools_on_main_thread();
    }
    app.finish();
    app.cleanup();
    let mut steady = 0;
    let mut known = 0;
    while steady < STEADY_FRAMES && began.elapsed() < LOADING_AT_MOST {
        app.update();
        let count = app.world().resource::<Assets<Shader>>().len();
        steady = if count == known { steady + 1 } else { 0 };
        known = count;
        std::thread::sleep(Duration::from_millis(5));
    }
}

const STEADY_FRAMES: u32 = 30;
const LOADING_AT_MOST: Duration = Duration::from_secs(30);

/// The passes a shader of `stage` draws in, each with the defs its pipeline compiles it under: as
/// a lit mesh with positions, normals, uvs and colours, in the main pass, and in the prepass where
/// the stage writes depth and normals ahead of it, a prop also drawn from the arenas; a surface leaves the prepass to the material it
/// plays over, and a cover's sheet, blended over the ground's patches, draws in the main pass
/// alone.
fn passes(stage: Stage) -> Vec<(&'static str, Vec<ShaderDefVal>)> {
    let main: Vec<ShaderDefVal> = vec![
        ShaderDefVal::UInt(
            "MATERIAL_BIND_GROUP".into(),
            MATERIAL_BIND_GROUP_INDEX as u32,
        ),
        "MESH_PIPELINE".into(),
        "VERTEX_OUTPUT_INSTANCE_INDEX".into(),
        "VERTEX_POSITIONS".into(),
        "VERTEX_NORMALS".into(),
        "VERTEX_UVS".into(),
        "VERTEX_UVS_A".into(),
        "VERTEX_COLORS".into(),
        "VIEW_PROJECTION_PERSPECTIVE".into(),
        "AVAILABLE_STORAGE_BUFFER_BINDINGS__GE_3".into(),
    ];
    let with = |base: &[ShaderDefVal], more: &[&'static str]| -> Vec<ShaderDefVal> {
        base.iter()
            .cloned()
            .chain(more.iter().map(|def| ShaderDefVal::from(*def)))
            .collect()
    };
    let prepass = with(
        &main,
        &["PREPASS_PIPELINE", "DEPTH_PREPASS", "NORMAL_PREPASS"],
    );
    match stage {
        Stage::Cover => vec![
            ("copies", main.clone()),
            ("copies prepass", prepass.clone()),
            ("blades", with(&main, &["ROOTED"])),
            ("blades prepass", with(&prepass, &["ROOTED"])),
            ("sheet", with(&main, &["SHEET_PATCH", "VERTEX_UVS_B"])),
        ],
        Stage::Prop => vec![("main", main.clone()), ("prepass", prepass.clone())],
        Stage::Surface | Stage::Sky | Stage::Rain => vec![("main", main)],
        Stage::Ground | Stage::Billboard | Stage::Screen | Stage::Text => Vec::new(),
    }
}

/// Parses and validates a composed module as the GPU driver would.
fn validate(
    _: &(),
    source: ShaderCacheSource,
    _: &bevy::shader::ValidateShader,
) -> Result<(), ShaderCacheError> {
    let ShaderCacheSource::Wgsl(source) = source else {
        return Ok(());
    };
    let module = naga::front::wgsl::parse_str(&source)
        .map_err(|error| ShaderCacheError::CreateShaderModule(error.emit_to_string(&source)))?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|error| ShaderCacheError::CreateShaderModule(error.emit_to_string(&source)))?;
    Ok(())
}
