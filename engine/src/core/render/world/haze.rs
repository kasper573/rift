//! Haze that lies in boxes standing on the ground, drawn over everything the camera sees once the
//! world is drawn: each pixel hazes by how far the way from the eye to what it shows runs through
//! each box, the sky behind none of them.

use bevy::core_pipeline::core_3d::main_transparent_pass_3d;
use bevy::core_pipeline::{Core3d, Core3dSystems, FullscreenShader};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::extract_component::{
    ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin,
    UniformComponentPlugin,
};
use bevy::render::render_resource::binding_types::{texture_depth_2d, uniform_buffer};
use bevy::render::render_resource::{
    BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, BlendComponent,
    BlendState, CachedRenderPipelineId, ColorTargetState, ColorWrites, FragmentState,
    PipelineCache, RenderPassDescriptor, RenderPipelineDescriptor, ShaderStages, ShaderType,
    TextureFormat, TextureUsages,
};
use bevy::render::renderer::{RenderContext, ViewQuery};
use bevy::render::view::{
    ExtractedView, ViewDepthStencilTexture, ViewTarget, ViewUniform, ViewUniformOffset,
    ViewUniforms,
};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};

/// The most boxes of haze a camera looks through at once: the nearest of them.
pub const MAX_HAZE_BOXES: usize = 32;

/// The haze a camera sees the world through: `color`, lying in `boxes` (the first `count` of
/// them), with the ground `ground` high under the eye.
#[derive(Component, ExtractComponent, ShaderType, Clone, Copy, Debug, Default, PartialEq)]
#[extract_app(RenderApp)]
pub struct Haze {
    pub color: Vec3,
    pub ground: f32,
    pub boxes: [HazeBox; MAX_HAZE_BOXES],
    pub count: u32,
}

/// A box of haze standing on the ground over the rect from `low` to `high` (x east, y south),
/// `height` deep: what is seen through `clear` metres of it or less stays clear, and the haze
/// thickens evenly to `thickest` of the way to its colour through `thick` metres of it.
#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct HazeBox {
    pub low: Vec2,
    pub high: Vec2,
    pub height: f32,
    pub clear: f32,
    pub thick: f32,
    pub thickest: f32,
}

impl Haze {
    /// Haze in `color` lying in `boxes`, the nearest [`MAX_HAZE_BOXES`] of them taken, over
    /// ground `ground` high under the eye.
    pub fn new(color: Color, ground: f32, boxes: impl IntoIterator<Item = HazeBox>) -> Haze {
        let linear = color.to_linear();
        let mut haze = Haze {
            color: Vec3::new(linear.red, linear.green, linear.blue),
            ground,
            ..Haze::default()
        };
        for (slot, haze_box) in haze.boxes.iter_mut().zip(boxes) {
            *slot = haze_box;
            haze.count += 1;
        }
        haze
    }
}

pub struct HazePlugin;

impl Plugin for HazePlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "haze.wesl");
        app.add_plugins((
            ExtractComponentPlugin::<Haze>::default(),
            UniformComponentPlugin::<Haze>::default(),
        ))
        .add_systems(PostUpdate, let_depth_be_read);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(RenderStartup, start_pipeline)
            .add_systems(Render, queue_pipelines.in_set(RenderSystems::Queue))
            .add_systems(
                Core3d,
                draw_haze
                    .after(main_transparent_pass_3d)
                    .in_set(Core3dSystems::MainPass),
            );
    }
}

/// Every hazed camera's depth can be read once the world is drawn, as the haze reads it.
fn let_depth_be_read(mut cameras: Query<&mut Camera3d, (With<Haze>, Changed<Camera3d>)>) {
    for mut camera in &mut cameras {
        let usages = TextureUsages::from(camera.depth_texture_usages);
        if !usages.contains(TextureUsages::TEXTURE_BINDING) {
            camera.depth_texture_usages = (usages | TextureUsages::TEXTURE_BINDING).into();
        }
    }
}

#[derive(Resource)]
struct HazePipeline {
    layout: BindGroupLayoutDescriptor,
    shader: Handle<Shader>,
}

#[derive(Component, Clone, Copy)]
struct HazePipelineId(CachedRenderPipelineId);

type HazedCamera = (With<Haze>, With<ExtractedCamera>);

type HazedView = (
    &'static ExtractedCamera,
    &'static ViewTarget,
    &'static ViewUniformOffset,
    &'static DynamicUniformIndex<Haze>,
    &'static HazePipelineId,
    &'static ViewDepthStencilTexture,
);

fn start_pipeline(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(HazePipeline {
        layout: BindGroupLayoutDescriptor::new(
            "haze_bind_group_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    uniform_buffer::<ViewUniform>(true),
                    uniform_buffer::<Haze>(true),
                    texture_depth_2d(),
                ),
            ),
        ),
        shader: bevy::asset::load_embedded_asset!(assets.as_ref(), "haze.wesl"),
    });
}

fn queue_pipelines(
    mut commands: Commands,
    cache: Res<PipelineCache>,
    pipeline: Res<HazePipeline>,
    fullscreen: Res<FullscreenShader>,
    views: Query<(Entity, &ExtractedView), HazedCamera>,
    mut queued: Local<HashMap<TextureFormat, CachedRenderPipelineId>>,
) {
    for (entity, view) in &views {
        let id = *queued.entry(view.target_format).or_insert_with(|| {
            cache.queue_render_pipeline(RenderPipelineDescriptor {
                label: Some("haze_pipeline".into()),
                layout: vec![pipeline.layout.clone()],
                vertex: fullscreen.to_vertex_state(),
                fragment: Some(FragmentState {
                    shader: pipeline.shader.clone(),
                    targets: vec![Some(ColorTargetState {
                        format: view.target_format,
                        blend: Some(BlendState {
                            color: BlendComponent::OVER,
                            alpha: BlendComponent::OVER,
                        }),
                        write_mask: ColorWrites::ALL,
                    })],
                    ..default()
                }),
                ..default()
            })
        });
        commands.entity(entity).insert(HazePipelineId(id));
    }
}

fn draw_haze(
    view: ViewQuery<HazedView>,
    pipeline: Option<Res<HazePipeline>>,
    cache: Res<PipelineCache>,
    view_uniforms: Res<ViewUniforms>,
    hazes: Res<ComponentUniforms<Haze>>,
    mut ctx: RenderContext,
) {
    let (camera, target, view_offset, haze_index, pipeline_id, depth) = view.into_inner();
    let (Some(pipeline), Some(view_binding), Some(haze_binding)) = (
        pipeline,
        view_uniforms.uniforms.binding(),
        hazes.uniforms().binding(),
    ) else {
        return;
    };
    let Some(depth_view) = depth.attachment.depth_stencil_views().depth_only_view() else {
        return;
    };
    let Some(render_pipeline) = cache.get_render_pipeline(pipeline_id.0) else {
        return;
    };
    let bind_group = ctx.render_device().create_bind_group(
        "haze_bind_group",
        &cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((view_binding, haze_binding, depth_view)),
    );
    let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("haze"),
        color_attachments: &[Some(target.get_color_attachment())],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    if let Some(viewport) = camera.viewport.as_ref() {
        pass.set_camera_viewport(viewport);
    }
    pass.set_render_pipeline(render_pipeline);
    pass.set_bind_group(0, &bind_group, &[view_offset.offset, haze_index.index()]);
    pass.draw(0..3, 0..1);
}
