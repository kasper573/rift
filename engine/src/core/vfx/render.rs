//! Draws vfx on the client: each instance's layers on the stage its shader runs on. A surface
//! shader runs in the material of the thing it plays on, a ground shader on a patch of the terrain
//! under it, a billboard shader and a sprite on a quad facing the camera at it, a screen or text
//! shader over the presented frame, and a widget site draws through the interface.

use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use ui::egui;

use bevy::asset::uuid::Uuid;
use bevy::camera::visibility::RenderLayers;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{
    ExtendedMaterial, Material, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline,
    MaterialPipeline, MaterialPipelineKey, MaterialPlugin,
};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::{Shader, ShaderRef};
use bevy::sprite_render::{
    AlphaMode2d, Material2d, Material2dKey, Material2dPipeline, Material2dPlugin,
};
use bevy::window::PrimaryWindow;
use bevy_egui::EguiPrimaryContextPass;
use ui::egui::Color32;
use ui::egui::load::SizedTexture;

use super::shader::PARAMS_BYTES;
use super::{
    BodyShape, CompiledVfxShaders, ParamKind, ParamValue, SiteKind, Stage, VfxDef, VfxInstance,
    VfxLayer, VfxParams, VfxShaderDef, VfxShaderId, VfxSite, sprite_cell,
};
use crate::core::content::Content;
use crate::core::geometry::{PlacedShape, Radians, Shape};
use crate::core::gui::{self, Band, WidgetRects};
use crate::core::math::{Pos, Rgba, Size};
use crate::core::render::present::{self, PRESENT_LAYER, PresentScale};
use crate::core::render::transition::WorldViewSystems;
use crate::core::render::world::billboard::Billboard;
use crate::core::render::world::decal::LIFT;
use crate::core::render::world::terrain;
use crate::core::render::world::toon::ToonMaterial;
use crate::core::render::{CameraRig, GroundHeights, ground_height, world_point};
use crate::core::time::{Seconds, server_now};
use crate::core::units::Meters;

pub struct VfxRenderPlugin;

impl Plugin for VfxRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(Material2dPlugin::<VfxMaterial>::default())
            .add_plugins(MaterialPlugin::<VfxWorldMaterial>::default())
            .add_plugins(MaterialPlugin::<SurfaceMaterial>::default())
            .add_plugins(MaterialPlugin::<super::cover::CoverMaterial>::default())
            .init_resource::<VfxShaders>()
            .init_resource::<VfxAssets>()
            .add_observer(unswap_removed)
            .add_systems(
                Update,
                (
                    load_shaders.run_if(resource_exists_and_changed::<Content>),
                    super::cover::blow,
                ),
            )
            .add_systems(
                PostUpdate,
                sync.in_set(WorldViewSystems)
                    .before(bevy::transform::TransformSystems::Propagate),
            )
            .add_systems(EguiPrimaryContextPass, draw_overlays.in_set(Band::World));
    }
}

/// A shader the client draws on a site of its own accord, for as long as the component stays: an
/// aim preview, a highlight. Its time runs from `since`, an uptime.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ShaderOverlay {
    pub shader: VfxShaderId,
    pub site: VfxSite,
    pub params: VfxParams,
    pub since: Seconds,
}

/// How big a thing a vfx plays on is: what a billboard stands over and a patch of ground covers.
/// Whoever draws a thing says, as it stands at its entity's transform.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct SiteExtent {
    /// How wide and deep the thing is on the ground, in metres.
    pub footprint: Vec2,
    /// How tall it stands, in metres.
    pub height: f32,
}

/// The margin around every world site that a shader layer may draw on, so spent matter (smoke,
/// sparks, mist) can drift clear of the body and dissolve; the prelude fades the margin's outer
/// edge, and the screen is drawn as is.
pub const MARGIN: Meters = Meters(2.0);

/// How many points of a path site a shader can read; a longer path keeps its first ones.
pub const MAX_PATH_POINTS: usize = 16;

/// How many of a sprite's or a site's own pixels make a metre of the world.
pub const TEXELS_PER_METER: f32 = 16.0;

/// A site measured for drawing: where it is in the world or on the screen and how big, in its own
/// pixels, with `px` of them to a sprite pixel; its facing and the heading the vfx travels; the
/// body and path a shader reads through the prelude; and the image a shader works on, when the
/// site has one.
#[derive(Clone)]
pub struct SiteFrame {
    pub kind: SiteKind,
    pub place: SitePlace,
    pub size: Vec2,
    pub px: f32,
    pub facing: Radians,
    pub heading: Radians,
    pub body: SiteBody,
    pub path: SitePath,
    pub texture: Option<SiteTexture>,
    /// Which way the camera looks, so a board facing it shows headings as the screen has them.
    pub camera_yaw: Radians,
}

/// Where a site stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SitePlace {
    /// On the ground at a point of the world, `footprint` metres across and `height` tall, the
    /// surface of `body` when it is a thing with a mesh.
    World {
        at: Vec3,
        footprint: Vec2,
        height: f32,
        body: Option<Entity>,
    },
    /// On the presented frame, `center` window pixels from its middle with y up, `z` above it.
    Screen { center: Vec2, z: f32 },
}

/// The image a shader samples through the prelude's `sample`: the texel rect of the site within
/// it and the size of one of its texels.
#[derive(Clone)]
pub struct SiteTexture {
    pub image: Handle<Image>,
    pub rect: Rect,
    pub pixel: Vec2,
}

/// How a site's body lies on the quad drawn over it: its shape and measures in metres, and where
/// the site's point sits in site uv.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SiteBody {
    pub shape: BodyShape,
    pub origin: Vec2,
    pub extent: Vec2,
}

/// The points of a path site in site uv, two to a word, as the prelude's `path_point` reads them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SitePath {
    pub count: u32,
    pub points: [Vec4; MAX_PATH_POINTS / 2],
}

/// Where a shader layer is in its run: how far through its duration, and the seconds since its
/// instance started.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Run {
    pub progress: f32,
    pub time: Seconds,
}

/// A shader drawn on a quad over the presented frame.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
#[bind_group_data(VfxShaderKey)]
pub struct VfxMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub image: Handle<Image>,
    #[uniform(2)]
    pub site: SiteUniform,
    #[uniform(3)]
    pub params: ParamsUniform,
    pub shader: Handle<Shader>,
}

/// A shader drawn on a quad in the world: a patch of ground or a board facing the camera, unlit,
/// blended over what is behind it.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
#[bind_group_data(VfxShaderKey)]
pub struct VfxWorldMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub image: Handle<Image>,
    #[uniform(2)]
    pub site: SiteUniform,
    #[uniform(3)]
    pub params: ParamsUniform,
    pub shader: Handle<Shader>,
    pub depth_bias: f32,
}

/// A shader run in the material of the thing it plays on, over the thing's own toon shading.
pub type SurfaceMaterial = ExtendedMaterial<StandardMaterial, SurfaceVfx>;

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug, Default)]
#[bind_group_data(VfxShaderKey)]
pub struct SurfaceVfx {
    #[uniform(100)]
    pub site: SiteUniform,
    #[uniform(101)]
    pub params: ParamsUniform,
    pub shader: Handle<Shader>,
}

/// The site as the prelude's `site` reads it, built by `SiteFrame::uniform`.
#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct SiteUniform {
    rect: Vec4,
    size: Vec2,
    pixel: Vec2,
    kind: u32,
    facing: f32,
    heading: f32,
    progress: f32,
    time: f32,
    overscan: Vec2,
    shape: u32,
    point_count: u32,
    image: u32,
    origin: Vec2,
    extent: Vec2,
    metres: Vec2,
    points: [Vec4; MAX_PATH_POINTS / 2],
}

impl SiteUniform {
    pub(super) fn set_time(&mut self, time: f32) {
        self.time = time;
    }
}

/// A shader's parameters laid out as its `Params` struct, built by `ParamsUniform::pack`.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct ParamsUniform {
    words: [Vec4; PARAMS_BYTES / 16],
}

/// A piece that draws over the presented frame, hidden until something is presented on it: for
/// text a client letters itself.
pub fn screen_piece(world: &mut World) -> Entity {
    piece_for(world, Stage::Text, None)
}

/// Draws a compiled shader over the site through its piece, with the site as the prelude reads it
/// and the parameters as the shader's `Params` struct holds them.
pub fn present_shader(
    world: &mut World,
    piece: Entity,
    frame: &SiteFrame,
    shader: Handle<Shader>,
    params: ParamsUniform,
    run: Run,
) {
    let Some(stage) = world.get::<VfxPiece>(piece).map(|piece| piece.stage) else {
        return;
    };
    let site = frame.uniform(run, stage);
    let image = frame
        .texture
        .as_ref()
        .map(|texture| texture.image.clone())
        .unwrap_or_else(|| world.resource::<VfxAssets>().blank.clone());
    match (stage, frame.place) {
        (Stage::Screen | Stage::Text, SitePlace::Screen { center, z }) => {
            if let Some(handle) = world
                .get::<MeshMaterial2d<VfxMaterial>>(piece)
                .map(|material| material.0.clone())
                && let Some(mut material) =
                    world.resource_mut::<Assets<VfxMaterial>>().get_mut(&handle)
            {
                material.image = image;
                material.site = site;
                material.params = params;
                material.shader = shader;
            }
            place(
                world,
                piece,
                Transform {
                    translation: center.extend(z),
                    scale: (frame.size * frame.overscan(stage)).extend(1.0),
                    ..default()
                },
            );
        }
        (Stage::Ground, SitePlace::World { at, footprint, .. }) => {
            paint_world(world, piece, image, site, params, shader);
            let size = footprint * frame.overscan(stage);
            lay_patch(world, piece, at, size);
            place(world, piece, Transform::default());
        }
        (
            Stage::Billboard,
            SitePlace::World {
                at,
                footprint,
                height,
                ..
            },
        ) => {
            paint_world(world, piece, image, site, params, shader);
            let drawn = frame.metres(stage) * frame.overscan(stage);
            let _ = footprint;
            place(
                world,
                piece,
                Transform {
                    translation: at + Vec3::Y * (height / 2.0),
                    scale: drawn.extend(1.0),
                    ..default()
                },
            );
        }
        (
            Stage::Surface,
            SitePlace::World {
                body: Some(body), ..
            },
        ) => {
            swap(world, piece, body, |surface| {
                surface.site = site;
                surface.params = params;
                surface.shader = shader;
            });
        }
        _ => hide(world, piece),
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

pub fn hide(world: &mut World, piece: Entity) {
    if let Some(mut visibility) = world.get_mut::<Visibility>(piece) {
        visibility.set_if_neq(Visibility::Hidden);
    }
    if world.get::<Swapped>(piece).is_some() {
        unswap(world, piece);
    }
}

impl SiteFrame {
    /// The site of a client's world as it stands this frame, headed the instance's way, or
    /// nothing while it cannot be drawn: a thing not yet placed, a screen not yet presented, or a
    /// widget, which draws through the interface alone.
    pub fn of(world: &World, site: &VfxSite, heading: Radians) -> Option<SiteFrame> {
        let camera_yaw = world
            .get_resource::<CameraRig>()
            .map_or(Radians::default(), |rig| rig.yaw());
        let frame = match site {
            VfxSite::Actor(entity) => SiteFrame::thing(world, *entity, SiteKind::Actor)?,
            VfxSite::Object(entity) => SiteFrame::thing(world, *entity, SiteKind::Object)?,
            VfxSite::Area(footprint) => SiteFrame::area(world, footprint),
            VfxSite::Path(points) => SiteFrame::path(world, points),
            VfxSite::Screen => {
                let window = world
                    .iter_entities()
                    .find(|entity| entity.contains::<PrimaryWindow>())
                    .and_then(|entity| entity.get::<Window>())?;
                let size = Vec2::new(window.resolution.width(), window.resolution.height());
                let texture = SiteTexture {
                    image: present::world_texture(world)?,
                    rect: Rect::from_corners(Vec2::ZERO, Vec2::ONE),
                    pixel: Vec2::ONE / size,
                };
                let px = present::SCALE
                    * world
                        .get_resource::<PresentScale>()
                        .map_or(1.0, |scale| scale.0.max(0.01));
                SiteFrame::screen(size, Some(texture), px)
            }
            VfxSite::Widget(_) => return None,
        };
        Some(SiteFrame {
            camera_yaw,
            ..frame.headed(heading)
        })
    }

    /// The frame with a vfx travelling `heading`, given in world terms as an instance carries it.
    pub fn headed(self, heading: Radians) -> SiteFrame {
        SiteFrame {
            heading: Radians(-heading.0),
            ..self
        }
    }

    /// The site a thing is: where it stands and how big it is, by its transform and extent.
    fn thing(world: &World, entity: Entity, kind: SiteKind) -> Option<SiteFrame> {
        let at = world.get::<Transform>(entity)?.translation;
        let extent = world.get::<SiteExtent>(entity)?;
        Some(SiteFrame {
            kind,
            place: SitePlace::World {
                at,
                footprint: extent.footprint,
                height: extent.height,
                body: Some(entity),
            },
            size: extent.footprint * TEXELS_PER_METER,
            px: 1.0,
            facing: Radians::default(),
            heading: Radians::default(),
            body: SiteBody::image(),
            path: SitePath::default(),
            texture: None,
            camera_yaw: Radians::default(),
        })
    }

    /// The patch an area site is drawn on, around its footprint's outline, with the footprint on
    /// it.
    fn area(world: &World, footprint: &PlacedShape) -> SiteFrame {
        let (center, size) = bounds(&footprint.outline());
        let size = size.max(Vec2::ONE);
        let min = center - size / 2.0;
        let origin = Vec2::new(
            (footprint.at.x - min.x) / size.x,
            (footprint.at.y - min.y) / size.y,
        );
        let (shape, extent) = match footprint.shape {
            Shape::Circle { radius } => (BodyShape::Circle, Vec2::new(radius.0, 0.0)),
            Shape::Rect { width, height } => (BodyShape::Rect, Vec2::new(width.0, height.0)),
            Shape::Cone { angle, length } => {
                (BodyShape::Cone, Vec2::new(angle.radians().0, length.0))
            }
            Shape::Line { length, width } => (BodyShape::Line, Vec2::new(length.0, width.0)),
        };
        let at = Pos::new(center.x, center.y);
        SiteFrame {
            kind: SiteKind::Area,
            place: SitePlace::World {
                at: world_point(at, ground_height(world, at)),
                footprint: size,
                height: size.y,
                body: None,
            },
            size: size * TEXELS_PER_METER,
            px: 1.0,
            facing: Radians(-footprint.facing.0),
            heading: Radians(-footprint.facing.0),
            body: SiteBody {
                shape,
                origin,
                extent,
            },
            path: SitePath::default(),
            texture: None,
            camera_yaw: Radians::default(),
        }
    }

    /// The patch a path site is drawn on, half a metre clear of its points on every side, facing
    /// from its first point to its last.
    fn path(world: &World, points: &[Pos<Meters>]) -> SiteFrame {
        let (center, size) = bounds(points);
        let size = size + Vec2::ONE;
        let min = center - size / 2.0;
        let uv = |at: Pos<Meters>| Vec2::new((at.x - min.x) / size.x, (at.y - min.y) / size.y);
        let mut path = SitePath::default();
        for (index, point) in points.iter().take(MAX_PATH_POINTS).enumerate() {
            let at = uv(*point);
            let word = &mut path.points[index / 2];
            if index % 2 == 0 {
                word.x = at.x;
                word.y = at.y;
            } else {
                word.z = at.x;
                word.w = at.y;
            }
            path.count = index as u32 + 1;
        }
        let facing = match (points.first(), points.last()) {
            (Some(first), Some(last)) if first != last => {
                let heading = *last - *first;
                -heading.y.atan2(heading.x)
            }
            _ => 0.0,
        };
        let at = Pos::new(center.x, center.y);
        SiteFrame {
            kind: SiteKind::Path,
            place: SitePlace::World {
                at: world_point(at, ground_height(world, at)),
                footprint: size,
                height: size.y,
                body: None,
            },
            size: size * TEXELS_PER_METER,
            px: 1.0,
            facing: Radians(facing),
            heading: Radians(facing),
            body: SiteBody {
                shape: BodyShape::None,
                origin: points.first().map_or(Vec2::splat(0.5), |first| uv(*first)),
                extent: Vec2::ZERO,
            },
            path,
            texture: None,
            camera_yaw: Radians::default(),
        }
    }

    /// A screen `size` pixels across, `px` of them to a world pixel, drawn over `texture` when
    /// there is a frame to work on.
    fn screen(size: Vec2, texture: Option<SiteTexture>, px: f32) -> SiteFrame {
        SiteFrame {
            kind: SiteKind::Screen,
            place: SitePlace::Screen {
                center: Vec2::ZERO,
                z: SCREEN_Z,
            },
            size,
            px,
            facing: Radians::default(),
            heading: Radians::default(),
            body: SiteBody::image(),
            path: SitePath::default(),
            texture,
            camera_yaw: Radians::default(),
        }
    }

    /// A line of text drawn on the screen at `center`, in window pixels from the window's middle
    /// with y up, over `texture`, the text's coverage as an image's alpha.
    pub fn text(center: Vec2, size: Vec2, texture: SiteTexture) -> SiteFrame {
        SiteFrame {
            kind: SiteKind::Text,
            place: SitePlace::Screen { center, z: TEXT_Z },
            size,
            px: 1.0,
            facing: Radians::default(),
            heading: Radians::default(),
            body: SiteBody::image(),
            path: SitePath::default(),
            texture: Some(texture),
            camera_yaw: Radians::default(),
        }
    }

    /// How many metres the site spans on `stage`: a patch of ground or a cover covers its
    /// footprint, a board stands as wide as the thing and as tall, the screen and text are their
    /// pixels over the pixel scale.
    pub fn metres(&self, stage: Stage) -> Vec2 {
        match (stage, self.place) {
            (Stage::Ground | Stage::Cover, SitePlace::World { footprint, .. }) => footprint,
            (
                Stage::Billboard | Stage::Surface,
                SitePlace::World {
                    footprint, height, ..
                },
            ) => Vec2::new(footprint.max_element(), height.max(0.1)),
            _ => self.size / self.px / TEXELS_PER_METER,
        }
    }

    /// How many times its own size the site is drawn on `stage`: the screen, text and a surface as
    /// they are, a quad in the world with its margin.
    pub fn overscan(&self, stage: Stage) -> Vec2 {
        match stage {
            Stage::Screen | Stage::Text | Stage::Surface | Stage::Cover => Vec2::ONE,
            Stage::Ground | Stage::Billboard => quad_overscan(self.metres(stage)),
        }
    }

    pub fn uniform(&self, run: Run, stage: Stage) -> SiteUniform {
        self.uniform_with(
            run,
            self.metres(stage),
            self.overscan(stage),
            self.texture.as_ref(),
            stage == Stage::Surface || self.texture.is_some(),
            stage == Stage::Billboard,
        )
    }

    fn uniform_with(
        &self,
        run: Run,
        metres: Vec2,
        overscan: Vec2,
        texture: Option<&SiteTexture>,
        image: bool,
        faces_camera: bool,
    ) -> SiteUniform {
        let (rect, pixel) = texture.map_or(
            (Rect::from_corners(Vec2::ZERO, Vec2::ONE), Vec2::ONE),
            |texture| (texture.rect, texture.pixel),
        );
        let turn = if faces_camera { self.camera_yaw.0 } else { 0.0 };
        SiteUniform {
            rect: Vec4::new(rect.min.x, rect.min.y, rect.max.x, rect.max.y),
            size: metres * TEXELS_PER_METER,
            pixel,
            kind: self.kind as u32,
            facing: self.facing.0 - turn,
            heading: self.heading.0 - turn,
            progress: run.progress,
            time: run.time.0,
            overscan,
            shape: self.body.shape as u32,
            point_count: self.path.count,
            image: u32::from(image),
            origin: self.body.origin,
            extent: self.body.extent,
            metres,
            points: self.path.points,
        }
    }
}

impl SiteBody {
    /// A site that is no area: its body is the box it is drawn in.
    pub fn image() -> SiteBody {
        SiteBody {
            shape: BodyShape::None,
            origin: Vec2::splat(0.5),
            extent: Vec2::ZERO,
        }
    }
}

impl Default for SitePath {
    fn default() -> SitePath {
        SitePath {
            count: 0,
            points: [Vec4::ZERO; MAX_PATH_POINTS / 2],
        }
    }
}

impl ParamsUniform {
    /// Each parameter the shader declares from `value`, or its default when that gives nothing or
    /// a value of another kind, laid out as the shader's `Params` struct.
    pub fn pack(
        shader: &VfxShaderDef,
        value: impl Fn(&str) -> Option<ParamValue>,
    ) -> ParamsUniform {
        let mut floats = [0.0f32; PARAMS_BYTES / 4];
        for slot in shader.layout() {
            let value = value(&slot.name)
                .filter(|value| value.kind() == slot.kind)
                .unwrap_or(slot.default);
            let at = slot.offset / 4;
            match (slot.kind, value) {
                (ParamKind::Number, ParamValue::Number(number)) => floats[at] = number,
                (ParamKind::Bool, ParamValue::Bool(flag)) => floats[at] = f32::from(u8::from(flag)),
                (ParamKind::Color, ParamValue::Color(color)) => {
                    let linear = color.color().to_linear();
                    floats[at..at + 4].copy_from_slice(&[
                        linear.red,
                        linear.green,
                        linear.blue,
                        linear.alpha,
                    ]);
                }
                _ => {}
            }
        }
        let mut words = [Vec4::ZERO; PARAMS_BYTES / 16];
        let (chunks, _) = floats.as_chunks::<4>();
        for (word, chunk) in words.iter_mut().zip(chunks) {
            *word = Vec4::from_array(*chunk);
        }
        ParamsUniform { words }
    }
}

impl Default for ParamsUniform {
    fn default() -> ParamsUniform {
        ParamsUniform {
            words: [Vec4::ZERO; PARAMS_BYTES / 16],
        }
    }
}

/// Which compiled shader a material draws with, as its pipeline is specialized by. Every compiled
/// shader is held under a uuid, so the key copies and the pipeline finds the shader by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VfxShaderKey(Uuid);

impl VfxShaderKey {
    pub(super) fn of(shader: &Handle<Shader>) -> VfxShaderKey {
        VfxShaderKey(match shader.id() {
            AssetId::Uuid { uuid } => uuid,
            AssetId::Index { .. } => Uuid::nil(),
        })
    }

    pub(super) fn handle(self) -> Handle<Shader> {
        Handle::Uuid(self.0, PhantomData)
    }
}

impl From<&VfxMaterial> for VfxShaderKey {
    fn from(material: &VfxMaterial) -> VfxShaderKey {
        VfxShaderKey::of(&material.shader)
    }
}

impl From<&VfxWorldMaterial> for VfxShaderKey {
    fn from(material: &VfxWorldMaterial) -> VfxShaderKey {
        VfxShaderKey::of(&material.shader)
    }
}

impl From<&SurfaceVfx> for VfxShaderKey {
    fn from(material: &SurfaceVfx) -> VfxShaderKey {
        VfxShaderKey::of(&material.shader)
    }
}

/// Holds a compiled shader under the uuid of its name, which no other load's shader shares.
fn compiled(assets: &mut Assets<Shader>, shader: Shader, name: &str) -> Handle<Shader> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut hasher);
    let uuid = Uuid::from_u128(u128::from(hasher.finish()) << 64 | u128::from(name.len() as u64));
    assets
        .insert(AssetId::Uuid { uuid }, shader)
        .expect("a uuid asset id never goes stale");
    Handle::Uuid(uuid, PhantomData)
}

impl Material2d for VfxMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Default
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    fn specialize(
        _pipeline: &Material2dPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(fragment) = descriptor
            .fragment
            .as_mut()
            .filter(|fragment| !fragment.shader_defs.contains(&PREPASS.into()))
        {
            fragment.shader = key.bind_group_data.handle();
        }
        Ok(())
    }
}

impl Material for VfxWorldMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Default
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn depth_bias(&self) -> f32 {
        self.depth_bias
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader = key.bind_group_data.handle();
        }
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

impl MaterialExtension for SurfaceVfx {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Default
    }

    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(fragment) = descriptor
            .fragment
            .as_mut()
            .filter(|fragment| !fragment.shader_defs.contains(&PREPASS.into()))
        {
            fragment.shader = key.bind_group_data.handle();
        }
        Ok(())
    }
}

/// The shader def bevy sets on its prepass pipelines, where a surface shader has no lighting to
/// paint over and so stays out of.
const PREPASS: &str = "PREPASS_PIPELINE";
const WIDGET_PULSE: f32 = 0.6;
/// How far above the presented frame a screen shader and a line of text are drawn.
const SCREEN_Z: f32 = 1.0;
const TEXT_Z: f32 = 3.0;
/// How far a patch of ground draws over the terrain, beyond the decals.
const GROUND_BIAS: f32 = 2.0;
/// The shader a sprite layer draws its sheet through: the frame as it is.
const SPRITE_SOURCE: &str = "fn vfx(uv: vec2<f32>) -> vec4<f32> { return sample(uv); }\n";

/// The quad and the blank image every piece starts with.
#[derive(Resource)]
struct VfxAssets {
    quad: Handle<Mesh>,
    blank: Handle<Image>,
}

// A shader asset is keyed by its path, so each load of the content gets paths of its own and the
// pipelines compiled for the last load are not reused for new text.
#[derive(Resource)]
struct VfxShaders {
    by_id: HashMap<VfxShaderId, Handle<Shader>>,
    sprite: Handle<Shader>,
    generation: u32,
}

/// The client's side of an instance or an overlay: one render piece per layer, children of the
/// entity so they go when it goes.
#[derive(Component, Clone)]
struct VfxView {
    pieces: Vec<Entity>,
}

#[derive(Component)]
struct VfxPiece {
    stage: Stage,
}

/// Where a ground piece's patch was last laid, so it is laid again only when the site moves.
#[derive(Component, Default)]
struct Patched(Option<(Vec3, Vec2)>);

/// The meshes a surface piece has taken over, each with the material it wore, so they get it back
/// when the piece stops drawing.
#[derive(Component, Clone, Default)]
struct Swapped {
    body: Option<Entity>,
    meshes: Vec<(Entity, Handle<ToonMaterial>)>,
    material: Option<Handle<SurfaceMaterial>>,
}

impl FromWorld for VfxAssets {
    fn from_world(world: &mut World) -> VfxAssets {
        let quad = world
            .resource_mut::<Assets<Mesh>>()
            .add(Rectangle::new(1.0, 1.0));
        let blank = world.resource_mut::<Assets<Image>>().add(Image::new_fill(
            bevy::render::render_resource::Extent3d::default(),
            bevy::render::render_resource::TextureDimension::D2,
            &[0, 0, 0, 0],
            bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
            bevy::asset::RenderAssetUsages::default(),
        ));
        VfxAssets { quad, blank }
    }
}

impl FromWorld for VfxShaders {
    fn from_world(world: &mut World) -> VfxShaders {
        let sprite = VfxShaderDef {
            name: "sprite".to_owned(),
            stage: Stage::Billboard,
            params: Vec::new(),
        };
        let composed = sprite.compose(SPRITE_SOURCE);
        let name = "vfx_shader/sprite.wgsl";
        VfxShaders {
            by_id: HashMap::default(),
            sprite: compiled(
                &mut world.resource_mut::<Assets<Shader>>(),
                Shader::from_wgsl(composed, name),
                name,
            ),
            generation: 0,
        }
    }
}

fn load_shaders(
    content: Res<Content>,
    mut shaders: ResMut<VfxShaders>,
    mut assets: ResMut<Assets<Shader>>,
) {
    shaders.generation += 1;
    let generation = shaders.generation;
    let sources = content.built::<CompiledVfxShaders>();
    shaders.by_id = sources
        .0
        .iter()
        .map(|(id, source)| {
            let (shader, name) = if id.get(&content).composes_wesl() {
                let name = format!("vfx_shader/{}-{generation}.wesl", id.path(&content));
                (Shader::from_wesl(source.clone(), name.clone()), name)
            } else {
                let name = format!("vfx_shader/{}-{generation}.wgsl", id.path(&content));
                (Shader::from_wgsl(source.clone(), name.clone()), name)
            };
            (*id, compiled(&mut assets, shader, &name))
        })
        .collect();
}

fn sync(world: &mut World) {
    let Some(content) = world.get_resource::<Content>().cloned() else {
        return;
    };
    let Some(now) = server_now(world) else {
        return;
    };
    let instances: Vec<(Entity, VfxInstance)> = world
        .query::<(Entity, &VfxInstance)>()
        .iter(world)
        .map(|(entity, instance)| (entity, instance.clone()))
        .collect();
    for (entity, instance) in instances {
        let def = instance.vfx.get(&content);
        let pieces = match world.get::<VfxView>(entity) {
            Some(view) => view.pieces.clone(),
            None => attach(world, entity, def, &content),
        };
        let frame = SiteFrame::of(world, &instance.site, instance.heading);
        let elapsed = now.since(instance.started);
        for (layer, &piece) in def.layers.iter().zip(&pieces) {
            let Some((frame, time)) = frame.as_ref().zip(layer.at(elapsed)) else {
                hide(world, piece);
                continue;
            };
            match layer {
                VfxLayer::Sprite { .. } => {
                    present_sprite(world, piece, frame, layer, time.local);
                }
                VfxLayer::Shader {
                    shader: id, params, ..
                } => {
                    let Some(compiled) = compiled_shader(world, *id) else {
                        hide(world, piece);
                        continue;
                    };
                    let packed = ParamsUniform::pack(id.get(&content), |name| {
                        params.get(name)?.resolve(&instance.params)
                    });
                    let run = Run {
                        progress: time.progress,
                        time: elapsed,
                    };
                    present_shader(world, piece, frame, compiled, packed, run);
                }
            }
        }
    }
    let uptime = Seconds(world.resource::<Time>().elapsed_secs());
    let overlays: Vec<(Entity, ShaderOverlay)> = world
        .query::<(Entity, &ShaderOverlay)>()
        .iter(world)
        .map(|(entity, overlay)| (entity, overlay.clone()))
        .collect();
    for (entity, overlay) in overlays {
        let stage = overlay.shader.get(&content).stage;
        let piece = match world.get::<VfxView>(entity) {
            Some(view) => view.pieces.first().copied(),
            None => {
                let piece = piece_for(world, stage, None);
                attach_pieces(world, entity, vec![piece]);
                Some(piece)
            }
        };
        let Some(piece) = piece else {
            continue;
        };
        let Some((frame, compiled)) = SiteFrame::of(world, &overlay.site, Radians::default())
            .zip(compiled_shader(world, overlay.shader))
        else {
            hide(world, piece);
            continue;
        };
        let packed = ParamsUniform::pack(overlay.shader.get(&content), |name| {
            overlay.params.get(name)
        });
        let run = Run {
            progress: 0.0,
            time: uptime - overlay.since,
        };
        present_shader(world, piece, &frame, compiled, packed, run);
    }
}

/// A piece per layer, each on the stage its layer plays on, a sprite layer's with its sheet.
fn attach(world: &mut World, entity: Entity, def: &VfxDef, content: &Content) -> Vec<Entity> {
    let assets = world.resource::<AssetServer>().clone();
    let pieces: Vec<Entity> = def
        .layers
        .iter()
        .map(|layer| {
            let sheet = match layer {
                VfxLayer::Sprite { sheet, .. } => Some(assets.load(sheet.path())),
                VfxLayer::Shader { .. } => None,
            };
            piece_for(world, layer.stage(content), sheet)
        })
        .collect();
    attach_pieces(world, entity, pieces.clone());
    pieces
}

fn attach_pieces(world: &mut World, entity: Entity, pieces: Vec<Entity>) {
    for piece in &pieces {
        world.entity_mut(*piece).insert(ChildOf(entity));
    }
    world.entity_mut(entity).insert((
        VfxView { pieces },
        Transform::default(),
        Visibility::default(),
    ));
}

/// A piece drawing on `stage`, hidden until something is presented on it: a quad over the frame,
/// a patch of ground, a board facing the camera, or the hold on a thing's surface.
fn piece_for(world: &mut World, stage: Stage, sheet: Option<Handle<Image>>) -> Entity {
    let (quad, blank) = {
        let assets = world.resource::<VfxAssets>();
        (assets.quad.clone(), assets.blank.clone())
    };
    let image = sheet.unwrap_or(blank);
    let piece = world
        .spawn((VfxPiece { stage }, Transform::default(), Visibility::Hidden))
        .id();
    match stage {
        Stage::Cover => {}
        Stage::Screen | Stage::Text => {
            let material = world
                .resource_mut::<Assets<VfxMaterial>>()
                .add(VfxMaterial {
                    image,
                    site: SiteUniform::default(),
                    params: ParamsUniform::default(),
                    shader: Handle::default(),
                });
            world.entity_mut(piece).insert((
                Mesh2d(quad),
                MeshMaterial2d(material),
                RenderLayers::layer(PRESENT_LAYER),
            ));
        }
        Stage::Ground | Stage::Billboard => {
            let material = world
                .resource_mut::<Assets<VfxWorldMaterial>>()
                .add(VfxWorldMaterial {
                    image,
                    site: SiteUniform::default(),
                    params: ParamsUniform::default(),
                    shader: Handle::default(),
                    depth_bias: if stage == Stage::Ground {
                        GROUND_BIAS
                    } else {
                        0.0
                    },
                });
            let mut entity = world.entity_mut(piece);
            entity.insert((
                Mesh3d(quad),
                MeshMaterial3d(material),
                NotShadowCaster,
                NotShadowReceiver,
            ));
            if stage == Stage::Ground {
                entity.insert(Patched::default());
            } else {
                entity.insert(Billboard);
            }
        }
        Stage::Surface => {
            world.entity_mut(piece).insert(Swapped::default());
        }
    }
    piece
}

/// Shows the frame a sprite layer is at on its board, placed on the site by its anchor, or hides
/// the board once a layer played once has nothing more to show or its sheet is not loaded.
fn present_sprite(
    world: &mut World,
    piece: Entity,
    frame: &SiteFrame,
    layer: &VfxLayer,
    local: Seconds,
) {
    let VfxLayer::Sprite {
        frame: cell,
        frames,
        fps,
        anchor,
        scale,
        play,
        ..
    } = layer
    else {
        return;
    };
    let SitePlace::World { at, height, .. } = frame.place else {
        hide(world, piece);
        return;
    };
    let Some(sheet) = world
        .get::<MeshMaterial3d<VfxWorldMaterial>>(piece)
        .and_then(|material| {
            world
                .resource::<Assets<VfxWorldMaterial>>()
                .get(&material.0)
        })
        .map(|material| material.image.clone())
    else {
        return;
    };
    let Some(sheet_size) = world
        .resource::<Assets<Image>>()
        .get(&sheet)
        .map(|image| Size::new(image.size_f32().x, image.size_f32().y))
    else {
        hide(world, piece);
        return;
    };
    let Some(shown) = sprite_cell(sheet_size, *cell, *frames, *fps, *play, local) else {
        hide(world, piece);
        return;
    };
    let whole = Vec2::new(sheet_size.width, sheet_size.height);
    let texture = SiteTexture {
        image: sheet.clone(),
        rect: Rect::from_corners(
            Vec2::new(shown.min().x, shown.min().y) / whole,
            Vec2::new(shown.max().x, shown.max().y) / whole,
        ),
        pixel: Vec2::ONE / whole,
    };
    let drawn = Vec2::new(shown.size.width, shown.size.height) * *scale / TEXELS_PER_METER;
    let run = Run {
        progress: 0.0,
        time: local,
    };
    let site = frame.uniform_with(run, drawn, Vec2::ONE, Some(&texture), true, true);
    let shader = world.resource::<VfxShaders>().sprite.clone();
    paint_world(world, piece, sheet, site, ParamsUniform::default(), shader);
    let middle = anchor.place_y(height / 2.0, height, drawn.y);
    place(
        world,
        piece,
        Transform {
            translation: at + Vec3::Y * middle,
            scale: drawn.extend(1.0),
            ..default()
        },
    );
}

fn paint_world(
    world: &mut World,
    piece: Entity,
    image: Handle<Image>,
    site: SiteUniform,
    params: ParamsUniform,
    shader: Handle<Shader>,
) {
    if let Some(handle) = world
        .get::<MeshMaterial3d<VfxWorldMaterial>>(piece)
        .map(|material| material.0.clone())
        && let Some(mut material) = world
            .resource_mut::<Assets<VfxWorldMaterial>>()
            .get_mut(&handle)
    {
        material.image = image;
        material.site = site;
        material.params = params;
        material.shader = shader;
    }
}

/// Lays a ground piece's patch over the terrain at `at`, `size` metres across, afresh when the
/// site moved.
fn lay_patch(world: &mut World, piece: Entity, at: Vec3, size: Vec2) {
    let laid = Some((at, size));
    if world
        .get::<Patched>(piece)
        .is_some_and(|patched| patched.0 == laid)
    {
        return;
    }
    let Some(heights) = world.get_resource::<GroundHeights>() else {
        return;
    };
    let patch = terrain::patch(&heights.heights, Vec2::new(at.x, at.z), size, 0.0, LIFT);
    let mesh = world.resource_mut::<Assets<Mesh>>().add(patch);
    let mut entity = world.entity_mut(piece);
    entity.insert((Mesh3d(mesh), Patched(laid)));
}

fn place(world: &mut World, piece: Entity, transform: Transform) {
    let mut entity = world.entity_mut(piece);
    if let Some(mut shown) = entity.get_mut::<Transform>() {
        *shown = transform;
    }
    if let Some(mut visibility) = entity.get_mut::<Visibility>() {
        visibility.set_if_neq(Visibility::Inherited);
    }
}

/// Runs the piece's surface shader in the materials of `body`'s meshes, taking over any still
/// wearing their toon material, and paints the shader's material.
fn swap(world: &mut World, piece: Entity, body: Entity, paint: impl FnOnce(&mut SurfaceVfx)) {
    let Some(mut swapped) = world.get::<Swapped>(piece).cloned() else {
        return;
    };
    if swapped.body != Some(body) {
        restore(world, &mut swapped);
        swapped.body = Some(body);
    }
    let wearing: Vec<(Entity, Handle<ToonMaterial>)> = world
        .get::<Children>(body)
        .into_iter()
        .flat_map(|children| descendants(world, children))
        .chain(std::iter::once(body))
        .filter_map(|mesh| {
            let toon = world.get::<MeshMaterial3d<ToonMaterial>>(mesh)?;
            Some((mesh, toon.0.clone()))
        })
        .collect();
    let base = wearing
        .first()
        .and_then(|(_, toon)| world.resource::<Assets<ToonMaterial>>().get(toon))
        .map(|toon| toon.base.clone());
    let material = match (&swapped.material, base) {
        (Some(material), _) => material.clone(),
        (None, Some(base)) => {
            let material = world
                .resource_mut::<Assets<SurfaceMaterial>>()
                .add(ExtendedMaterial {
                    base,
                    extension: SurfaceVfx::default(),
                });
            swapped.material = Some(material.clone());
            material
        }
        (None, None) => {
            world.entity_mut(piece).insert(swapped);
            return;
        }
    };
    for (mesh, toon) in wearing {
        world
            .entity_mut(mesh)
            .remove::<MeshMaterial3d<ToonMaterial>>()
            .insert(MeshMaterial3d(material.clone()));
        swapped.meshes.push((mesh, toon));
    }
    if let Some(mut surface) = world
        .resource_mut::<Assets<SurfaceMaterial>>()
        .get_mut(&material)
    {
        paint(&mut surface.extension);
    }
    world.entity_mut(piece).insert(swapped);
}

fn descendants(world: &World, children: &Children) -> Vec<Entity> {
    let mut found = Vec::new();
    let mut pending: Vec<Entity> = children.iter().collect();
    while let Some(entity) = pending.pop() {
        found.push(entity);
        if let Some(children) = world.get::<Children>(entity) {
            pending.extend(children.iter());
        }
    }
    found
}

fn unswap(world: &mut World, piece: Entity) {
    let Some(mut swapped) = world.get::<Swapped>(piece).cloned() else {
        return;
    };
    restore(world, &mut swapped);
    world.entity_mut(piece).insert(swapped);
}

/// Gives every mesh a surface piece took over its toon material back.
fn restore(world: &mut World, swapped: &mut Swapped) {
    for (mesh, toon) in swapped.meshes.drain(..) {
        if let Ok(mut mesh) = world.get_entity_mut(mesh) {
            mesh.remove::<MeshMaterial3d<SurfaceMaterial>>()
                .insert(MeshMaterial3d(toon));
        }
    }
    swapped.body = None;
}

/// A surface piece that goes gives its meshes their materials back on the way out.
fn unswap_removed(gone: On<Remove<Swapped>>, swapped: Query<&Swapped>, mut commands: Commands) {
    let Ok(swapped) = swapped.get(gone.entity) else {
        return;
    };
    for (mesh, toon) in &swapped.meshes {
        commands
            .entity(*mesh)
            .remove::<MeshMaterial3d<SurfaceMaterial>>()
            .insert(MeshMaterial3d(toon.clone()));
    }
}

/// How many times its own size a site `metres` across is drawn, per axis, with `MARGIN` on every
/// side.
fn quad_overscan(metres: Vec2) -> Vec2 {
    let metres = metres.max(Vec2::splat(0.1));
    (metres + Vec2::splat(2.0 * MARGIN.0)) / metres
}

fn bounds(points: &[Pos<Meters>]) -> (Vec2, Vec2) {
    if points.is_empty() {
        return (Vec2::ZERO, Vec2::ZERO);
    }
    let min = points
        .iter()
        .fold(Vec2::MAX, |min, at| min.min(Vec2::new(at.x, at.y)));
    let max = points
        .iter()
        .fold(Vec2::MIN, |max, at| max.max(Vec2::new(at.x, at.y)));
    ((min + max) / 2.0, max - min)
}

/// The color a shader layer paints with, for drawing it where no shader runs: the first color
/// among what it is given, or the first the shader declares.
fn shader_color(shader: &VfxShaderDef, value: impl Fn(&str) -> Option<ParamValue>) -> Rgba {
    shader
        .params
        .iter()
        .find_map(|param| match value(&param.name).unwrap_or(param.default) {
            ParamValue::Color(color) => Some(color),
            ParamValue::Number(_) | ParamValue::Bool(_) => None,
        })
        .unwrap_or(Rgba::WHITE)
}

/// Every layer of a widget site draws through the interface rather than the world.
fn draw_overlays(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let Some(content) = world.get_resource::<Content>().cloned() else {
        return;
    };
    let Some(now) = server_now(world) else {
        return;
    };
    let instances: Vec<(Entity, VfxInstance)> = world
        .query::<(Entity, &VfxInstance)>()
        .iter(world)
        .map(|(entity, instance)| (entity, instance.clone()))
        .collect();
    for (entity, instance) in instances {
        let def = instance.vfx.get(&content);
        let elapsed = now.since(instance.started);
        let Some(rect) = widget_rect(world, &instance.site) else {
            continue;
        };
        for (index, layer) in def.layers.iter().enumerate() {
            let Some(time) = layer.at(elapsed) else {
                continue;
            };
            let area = gui::world_area(world, ("vfx", entity, index));
            match layer {
                VfxLayer::Sprite {
                    sheet,
                    frame,
                    frames,
                    fps,
                    scale,
                    play,
                    ..
                } => {
                    let Some(image) = gui::image_file(world, &sheet.path()) else {
                        continue;
                    };
                    let sheet = Size::new(image.size.x, image.size.y);
                    let Some(cell) = sprite_cell(sheet, *frame, *frames, *fps, *play, time.local)
                    else {
                        continue;
                    };
                    let uv = egui::Rect::from_min_max(
                        egui::pos2(cell.min().x / sheet.width, cell.min().y / sheet.height),
                        egui::pos2(cell.max().x / sheet.width, cell.max().y / sheet.height),
                    );
                    let drawn = egui::vec2(cell.size.width, cell.size.height) * *scale;
                    area.fixed_pos(rect.center() - drawn / 2.0)
                        .show(&gui, |ui| {
                            ui.add(
                                egui::Image::new(SizedTexture::new(image.texture, drawn)).uv(uv),
                            );
                        });
                }
                VfxLayer::Shader { shader, params, .. } => {
                    let color = shader_color(shader.get(&content), |name| {
                        params.get(name)?.resolve(&instance.params)
                    });
                    area.fixed_pos(rect.min).show(&gui, |ui| {
                        ui.painter().rect_filled(
                            rect,
                            4.0,
                            Color32::from(color)
                                .gamma_multiply(WIDGET_PULSE * (1.0 - time.progress)),
                        );
                    });
                }
            }
        }
    }
}

/// Where a widget site lies in the window; every other site draws in the world instead.
fn widget_rect(world: &World, site: &VfxSite) -> Option<egui::Rect> {
    match site {
        VfxSite::Widget(name) => world.resource::<WidgetRects>().0.get(&name.0).copied(),
        _ => None,
    }
}
