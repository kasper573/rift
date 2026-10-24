//! Draws vfx on the client: each instance's layers as sprites and shader quads in the world or
//! over the presented frame, or through the interface on a widget or the screen. The site frames,
//! pieces and presenters are public so the editor's stage draws a vfx exactly as the game does.

use std::collections::HashSet;
use ui::egui;

use bevy::camera::visibility::RenderLayers;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::{Shader, ShaderRef};
use bevy::sprite::Anchor;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey, Material2dPipeline};
use bevy::window::PrimaryWindow;
use bevy_egui::EguiPrimaryContextPass;
use ui::egui::Color32;
use ui::egui::load::SizedTexture;

use super::shader::PARAMS_BYTES;
use super::{
    BodyShape, CompiledVfxShaders, ParamKind, ParamValue, SiteKind, VfxDef, VfxInstance, VfxLayer,
    VfxParams, VfxShaderDef, VfxShaderId, VfxSite, sprite_cell,
};
use crate::core::assets::ImageRef;
use crate::core::content::Content;
use crate::core::geometry::{PlacedShape, Radians, Shape};
use crate::core::gui::{self, Band, WidgetRects};
use crate::core::math::{Pos, Rgba, Size};
use crate::core::render::present::{self, PRESENT_LAYER, PresentScale};
use crate::core::render::transition::WorldViewSystems;
use crate::core::render::{
    self as render, GroundDepth, RowDepth, RowDepths, TILE, ToScreen, atlas_rect,
};
use crate::core::time::{Seconds, server_now};
use crate::core::units::Meters;

pub struct VfxRenderPlugin;

impl Plugin for VfxRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(bevy::sprite_render::Material2dPlugin::<VfxMaterial>::default())
            .init_resource::<VfxShaders>()
            .init_resource::<VfxAssets>()
            .add_systems(
                Update,
                load_shaders.run_if(resource_exists_and_changed::<Content>),
            )
            .add_systems(
                PostUpdate,
                sync.before(render::order_rows).in_set(WorldViewSystems),
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

/// The margin around every world site that a shader layer may draw on, so spent matter (smoke,
/// sparks, mist) can drift clear of the body and dissolve; the prelude fades the margin's outer
/// edge, and the screen is drawn as is.
pub const MARGIN: Meters = Meters(2.0);

/// How many points of a path site a shader can read; a longer path keeps its first ones.
pub const MAX_PATH_POINTS: usize = 16;

/// A site measured for drawing: where it is and how big, in its own pixels, with `px` of them to
/// a sprite pixel; its facing and the heading the vfx travels, both in screen terms; the body and
/// path a shader reads through the prelude; the image a shader works on, when the site has one;
/// and, in the world, the sprite it is and the row depth it sits at.
#[derive(Clone)]
pub struct SiteFrame {
    pub kind: SiteKind,
    pub center: Vec2,
    pub size: Vec2,
    pub px: f32,
    pub facing: Radians,
    pub heading: Radians,
    pub body: SiteBody,
    pub path: SitePath,
    pub texture: Option<SiteTexture>,
    pub sprite: Option<Entity>,
    pub depth: Option<f32>,
}

/// The image a shader samples through the prelude's `sample`: the texel rect of the site within
/// it and the size of one of its texels.
#[derive(Clone)]
pub struct SiteTexture {
    pub image: Handle<Image>,
    pub rect: Rect,
    pub pixel: Vec2,
}

/// How a site's body lies on the quad drawn over it: its shape and measures in tiles, where the
/// site's point sits in site uv, and the quad's size in tiles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SiteBody {
    pub shape: BodyShape,
    pub origin: Vec2,
    pub extent: Vec2,
    pub tiles: Vec2,
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
    origin: Vec2,
    extent: Vec2,
    tiles: Vec2,
    points: [Vec4; MAX_PATH_POINTS / 2],
}

/// A shader's parameters laid out as its `Params` struct, built by `ParamsUniform::pack`.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct ParamsUniform {
    words: [Vec4; PARAMS_BYTES / 16],
}

/// The quad and the blank image every shader piece starts with.
#[derive(Resource)]
pub struct VfxAssets {
    quad: Handle<Mesh>,
    blank: Handle<Image>,
}

/// The quad a shader layer draws on, hidden until something is presented on it.
pub fn shader_piece(world: &mut World) -> Entity {
    let quad = world.resource::<VfxAssets>().quad.clone();
    let blank = world.resource::<VfxAssets>().blank.clone();
    let material = world
        .resource_mut::<Assets<VfxMaterial>>()
        .add(VfxMaterial {
            image: blank,
            site: SiteUniform::default(),
            params: ParamsUniform::default(),
            shader: Handle::default(),
        });
    world
        .spawn((
            Mesh2d(quad),
            MeshMaterial2d(material),
            Transform::default(),
            Visibility::Hidden,
            VfxPiece,
        ))
        .id()
}

/// The sprite a sprite layer shows its sheet through, hidden until a frame is presented on it.
pub fn sprite_piece(world: &mut World, sheet: Handle<Image>) -> Entity {
    world
        .spawn((
            Sprite {
                image: sheet,
                ..default()
            },
            Transform::default(),
            Visibility::Hidden,
            VfxPiece,
        ))
        .id()
}

/// Shows the frame a sprite layer is at on its piece, placed on the site by its anchor; false, and
/// the piece hidden, once a layer played once has nothing more to show or its sheet is not loaded.
pub fn present_sprite(
    world: &mut World,
    piece: Entity,
    frame: &SiteFrame,
    layer: &VfxLayer,
    local: Seconds,
) -> bool {
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
        return false;
    };
    let sheet = world
        .get::<Sprite>(piece)
        .and_then(|sprite| world.resource::<Assets<Image>>().get(&sprite.image))
        .map(|image| Size::new(image.size_f32().x, image.size_f32().y));
    let shown = sheet.and_then(|sheet| sprite_cell(sheet, *cell, *frames, *fps, *play, local));
    let Some(shown) = shown else {
        hide(world, piece);
        return false;
    };
    let drawn = Vec2::new(shown.size.width, shown.size.height) * *scale * frame.px;
    let at = Vec2::new(
        frame.center.x,
        anchor.place_y(frame.center.y, frame.size.y, drawn.y),
    );
    let mut entity = world.entity_mut(piece);
    if let Some(mut sprite) = entity.get_mut::<Sprite>() {
        sprite.rect = Some(atlas_rect(shown));
        sprite.custom_size = Some(drawn);
    }
    if let Some(mut transform) = entity.get_mut::<Transform>() {
        transform.translation.x = at.x;
        transform.translation.y = at.y;
    }
    if let Some(mut visibility) = entity.get_mut::<Visibility>() {
        visibility.set_if_neq(Visibility::Inherited);
    }
    true
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
    let blank = world.resource::<VfxAssets>().blank.clone();
    let Some(handle) = world
        .get::<MeshMaterial2d<VfxMaterial>>(piece)
        .map(|material| material.0.clone())
    else {
        return;
    };
    let image = frame
        .texture
        .as_ref()
        .map_or(blank, |texture| texture.image.clone());
    if let Some(mut material) = world.resource_mut::<Assets<VfxMaterial>>().get_mut(&handle) {
        material.image = image;
        material.site = frame.uniform(run);
        material.params = params;
        material.shader = shader;
    }
    let mut entity = world.entity_mut(piece);
    if let Some(mut transform) = entity.get_mut::<Transform>() {
        transform.translation.x = frame.center.x;
        transform.translation.y = frame.center.y;
        transform.scale = (frame.size * frame.overscan()).extend(1.0);
    }
    if let Some(mut visibility) = entity.get_mut::<Visibility>() {
        visibility.set_if_neq(Visibility::Inherited);
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

/// The layer the presented frame is drawn on, where a piece goes that sits over the whole world.
pub fn screen_layer() -> RenderLayers {
    RenderLayers::layer(PRESENT_LAYER)
}

pub fn hide(world: &mut World, piece: Entity) {
    if let Some(mut visibility) = world.get_mut::<Visibility>(piece) {
        visibility.set_if_neq(Visibility::Hidden);
    }
}

/// One vfx's layers as they stand at a moment, on the site they play on.
pub struct Presentation<'a> {
    pub layers: &'a [VfxLayer],
    pub pieces: &'a [Entity],
    pub frame: Option<&'a SiteFrame>,
    pub elapsed: Seconds,
    pub params: &'a VfxParams,
    pub content: &'a Content,
    /// The layer a screen piece is drawn on, where the frame it draws over is presented.
    pub screen_layer: &'a RenderLayers,
}

/// Shows a vfx's sprite and shader layers on their pieces as the game draws them: each layer at
/// its own time, placed on the frame and sorted into its depth, hidden before it starts and once
/// it is over. `shader` answers with a shader's compiled handle, or nothing while it has none. The
/// sprites a shader layer repaints join `overdrawn`, for `settle_overdrawn` to hide.
pub fn present_layers(
    world: &mut World,
    shown: &Presentation,
    shader: &dyn Fn(VfxShaderId) -> Option<Handle<Shader>>,
    overdrawn: &mut HashSet<Entity>,
) {
    for (layer, &piece) in shown.layers.iter().zip(shown.pieces) {
        let Some((frame, time)) = shown.frame.zip(layer.at(shown.elapsed)) else {
            hide(world, piece);
            continue;
        };
        match layer {
            VfxLayer::Sprite { .. } => {
                if present_sprite(world, piece, frame, layer, time.local) {
                    place_in_depth(world, piece, frame, 2.0, shown.screen_layer);
                }
            }
            VfxLayer::Shader {
                shader: id, params, ..
            } => {
                let Some(compiled) = shader(*id) else {
                    hide(world, piece);
                    continue;
                };
                let packed = ParamsUniform::pack(id.get(shown.content), |name| {
                    params.get(name)?.resolve(shown.params)
                });
                let run = Run {
                    progress: time.progress,
                    time: shown.elapsed,
                };
                present_shader(world, piece, frame, compiled, packed, run);
                place_in_depth(world, piece, frame, 1.0, shown.screen_layer);
                overdrawn.extend(frame.sprite);
            }
        }
    }
}

/// A piece per layer, each on the sheet `image_of` loads for it.
pub fn pieces_for(
    world: &mut World,
    layers: &[VfxLayer],
    image_of: &mut dyn FnMut(&mut World, &ImageRef) -> Handle<Image>,
) -> Vec<Entity> {
    layers
        .iter()
        .map(|layer| match layer {
            VfxLayer::Sprite { sheet, .. } => {
                let image = image_of(world, sheet);
                sprite_piece(world, image)
            }
            VfxLayer::Shader { .. } => shader_piece(world),
        })
        .collect()
}

/// The color a shader layer paints with, for drawing it where no shader runs: the first color
/// among what it is given, or the first the shader declares.
pub fn shader_color(shader: &VfxShaderDef, value: impl Fn(&str) -> Option<ParamValue>) -> Rgba {
    shader
        .params
        .iter()
        .find_map(|param| match value(&param.name).unwrap_or(param.default) {
            ParamValue::Color(color) => Some(color),
            ParamValue::Number(_) | ParamValue::Bool(_) => None,
        })
        .unwrap_or(Rgba::WHITE)
}

/// How many times its own size a site of `size` pixels is drawn, per axis, with `MARGIN` on every
/// side.
pub fn quad_overscan(size: Vec2) -> Vec2 {
    let size = size.max(Vec2::ONE);
    (size + Vec2::splat(2.0 * MARGIN.0 * TILE.0)) / size
}

impl SiteFrame {
    /// The site of a client's world as it stands this frame, headed the instance's way, or
    /// nothing while it cannot be drawn: a sprite not yet loaded, a screen not yet presented, or a
    /// widget, which draws through the interface alone.
    pub fn of(world: &World, site: &VfxSite, heading: Radians) -> Option<SiteFrame> {
        match site {
            VfxSite::Actor(entity) => SiteFrame::sprite(world, *entity, SiteKind::Actor)
                .map(|frame| frame.headed(heading)),
            VfxSite::Object(entity) => SiteFrame::sprite(world, *entity, SiteKind::Object)
                .map(|frame| frame.headed(heading)),
            VfxSite::Area(footprint) => Some(SiteFrame {
                depth: Some(world.resource::<GroundDepth>().0),
                ..SiteFrame::area(footprint)
            }),
            VfxSite::Path(points) => {
                let south = points.iter().map(|point| point.y).fold(f32::MIN, f32::max);
                Some(SiteFrame {
                    depth: Some(world.resource::<RowDepths>().at(Meters(south))),
                    ..SiteFrame::path(points)
                })
            }
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
                Some(SiteFrame::screen(size, Some(texture), px).headed(heading))
            }
            VfxSite::Widget(_) => None,
        }
    }

    /// The frame with a vfx travelling `heading`, given in world terms as an instance carries it.
    pub fn headed(self, heading: Radians) -> SiteFrame {
        SiteFrame {
            heading: Radians(-heading.0),
            ..self
        }
    }

    /// The site a sprite entity is: its drawn box, its image and, when it has one, its row depth.
    pub fn sprite(world: &World, entity: Entity, kind: SiteKind) -> Option<SiteFrame> {
        let sprite = world.get::<Sprite>(entity)?;
        let transform = world.get::<Transform>(entity)?;
        let anchor = world
            .get::<Anchor>(entity)
            .map_or(Vec2::ZERO, |anchor| anchor.0);
        let image = world.resource::<Assets<Image>>().get(&sprite.image)?;
        let sheet = image.size_f32();
        let rect = sprite
            .rect
            .unwrap_or_else(|| Rect::from_corners(Vec2::ZERO, sheet));
        let size = sprite.custom_size.unwrap_or_else(|| rect.size());
        Some(SiteFrame {
            kind,
            center: transform.translation.xy() - anchor * size,
            size,
            px: 1.0,
            facing: Radians::default(),
            heading: Radians::default(),
            body: SiteBody::image(size / TILE.0),
            path: SitePath::default(),
            texture: Some(SiteTexture {
                image: sprite.image.clone(),
                rect: Rect::from_corners(rect.min / sheet, rect.max / sheet),
                pixel: Vec2::ONE / sheet,
            }),
            sprite: Some(entity),
            depth: world.get::<RowDepth>(entity).map(|depth| depth.0),
        })
    }

    /// The quad an area site is drawn on, around its footprint's outline, with the footprint on it.
    pub fn area(footprint: &PlacedShape) -> SiteFrame {
        let (center, size) = bounds(&footprint.outline());
        let size = size.max(Vec2::ONE);
        let at = footprint.at.to_screen();
        let origin = Vec2::new(
            (at.x - (center.x - size.x / 2.0)) / size.x,
            ((center.y + size.y / 2.0) - at.y) / size.y,
        );
        let (shape, extent) = match footprint.shape {
            Shape::Circle { radius } => (BodyShape::Circle, Vec2::new(radius.0, 0.0)),
            Shape::Rect { width, height } => (BodyShape::Rect, Vec2::new(width.0, height.0)),
            Shape::Cone { angle, length } => {
                (BodyShape::Cone, Vec2::new(angle.radians().0, length.0))
            }
            Shape::Line { length, width } => (BodyShape::Line, Vec2::new(length.0, width.0)),
        };
        SiteFrame {
            kind: SiteKind::Area,
            center,
            size,
            px: 1.0,
            facing: Radians(-footprint.facing.0),
            heading: Radians(-footprint.facing.0),
            body: SiteBody {
                shape,
                origin,
                extent,
                tiles: size / TILE.0,
            },
            path: SitePath::default(),
            texture: None,
            sprite: None,
            depth: None,
        }
    }

    /// The quad a path site is drawn on, half a tile clear of its points on every side, facing
    /// from its first point to its last.
    pub fn path(points: &[Pos<Meters>]) -> SiteFrame {
        let (center, size) = bounds(points);
        let size = size + Vec2::splat(TILE.0);
        let corner = Vec2::new(center.x - size.x / 2.0, center.y + size.y / 2.0);
        let uv = |at: Pos<Meters>| {
            let screen = at.to_screen();
            Vec2::new(
                (screen.x - corner.x) / size.x,
                (corner.y - screen.y) / size.y,
            )
        };
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
                let heading = last.to_screen() - first.to_screen();
                heading.y.atan2(heading.x)
            }
            _ => 0.0,
        };
        SiteFrame {
            kind: SiteKind::Path,
            center,
            size,
            px: 1.0,
            facing: Radians(facing),
            heading: Radians(facing),
            body: SiteBody {
                shape: BodyShape::None,
                origin: points.first().map_or(Vec2::splat(0.5), |first| uv(*first)),
                extent: Vec2::ZERO,
                tiles: size / TILE.0,
            },
            path,
            texture: None,
            sprite: None,
            depth: None,
        }
    }

    /// A screen `size` pixels across, `px` of them to a world pixel, drawn over `texture` when
    /// there is a frame to work on.
    pub fn screen(size: Vec2, texture: Option<SiteTexture>, px: f32) -> SiteFrame {
        SiteFrame {
            kind: SiteKind::Screen,
            center: Vec2::ZERO,
            size,
            px,
            facing: Radians::default(),
            heading: Radians::default(),
            body: SiteBody::image(size / px / TILE.0),
            path: SitePath::default(),
            texture,
            sprite: None,
            depth: None,
        }
    }

    /// A line of text drawn on the screen at `center`, in window pixels from the window's middle
    /// with y up, over `texture`, the text's coverage as an image's alpha.
    pub fn text(center: Vec2, size: Vec2, texture: SiteTexture) -> SiteFrame {
        SiteFrame {
            kind: SiteKind::Text,
            center,
            size,
            px: 1.0,
            facing: Radians::default(),
            heading: Radians::default(),
            body: SiteBody::image(size / TILE.0),
            path: SitePath::default(),
            texture: Some(texture),
            sprite: None,
            depth: None,
        }
    }

    pub fn head(&self) -> Vec2 {
        self.center + Vec2::new(0.0, self.size.y / 2.0)
    }

    /// How many times its own size the site is drawn: the screen and text as they are, a world
    /// site with its margin.
    pub fn overscan(&self) -> Vec2 {
        if matches!(self.kind, SiteKind::Screen | SiteKind::Text) {
            Vec2::ONE
        } else {
            quad_overscan(self.size)
        }
    }

    pub fn uniform(&self, run: Run) -> SiteUniform {
        let (rect, pixel) = self.texture.as_ref().map_or(
            (Rect::from_corners(Vec2::ZERO, Vec2::ONE), Vec2::ONE),
            |texture| (texture.rect, texture.pixel),
        );
        SiteUniform {
            rect: Vec4::new(rect.min.x, rect.min.y, rect.max.x, rect.max.y),
            size: self.size,
            pixel,
            kind: self.kind as u32,
            facing: self.facing.0,
            heading: self.heading.0,
            progress: run.progress,
            time: run.time.0,
            overscan: self.overscan(),
            shape: self.body.shape as u32,
            point_count: self.path.count,
            origin: self.body.origin,
            extent: self.body.extent,
            tiles: self.body.tiles,
            points: self.path.points,
        }
    }
}

impl SiteBody {
    /// A site that is no area, drawn over its image: its body is the image's box, `tiles` across.
    pub fn image(tiles: Vec2) -> SiteBody {
        SiteBody {
            shape: BodyShape::None,
            origin: Vec2::splat(0.5),
            extent: Vec2::ZERO,
            tiles,
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

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct VfxShaderKey(Handle<Shader>);

impl From<&VfxMaterial> for VfxShaderKey {
    fn from(material: &VfxMaterial) -> VfxShaderKey {
        VfxShaderKey(material.shader.clone())
    }
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
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader = key.bind_group_data.0;
        }
        Ok(())
    }
}

const WIDGET_PULSE: f32 = 0.6;

// A shader asset is keyed by its path, so each load of the content gets paths of its own and the
// pipelines compiled for the last load are not reused for new text.
#[derive(Resource, Default)]
struct VfxShaders {
    by_id: HashMap<VfxShaderId, Handle<Shader>>,
    generation: u32,
}

/// The client's side of an instance or an overlay: one render piece per layer, children of the
/// entity so they go when it goes.
#[derive(Component, Clone)]
struct VfxView {
    pieces: Vec<Entity>,
}

#[derive(Component)]
struct VfxPiece;

/// On a sprite a shader layer is drawing in place of it: the sprite itself is hidden meanwhile.
#[derive(Component, Clone, Copy)]
struct Overdrawn {
    alpha: f32,
}

fn load_shaders(
    content: Res<Content>,
    mut shaders: ResMut<VfxShaders>,
    mut assets: ResMut<Assets<Shader>>,
) {
    shaders.generation += 1;
    let generation = shaders.generation;
    let compiled = content.built::<CompiledVfxShaders>();
    shaders.by_id = compiled
        .0
        .iter()
        .map(|(id, source)| {
            let path = format!("vfx_shader/{}-{generation}.wgsl", id.path(&content));
            (*id, assets.add(Shader::from_wgsl(source.clone(), path)))
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
    let mut overdrawn = HashSet::new();
    let screen_layer = screen_layer();
    for (entity, instance) in instances {
        let def = instance.vfx.get(&content);
        let pieces = match world.get::<VfxView>(entity) {
            Some(view) => view.pieces.clone(),
            None => attach(world, entity, def),
        };
        let frame = SiteFrame::of(world, &instance.site, instance.heading);
        let elapsed = now.since(instance.started);
        let shaders = world.resource::<VfxShaders>().by_id.clone();
        present_layers(
            world,
            &Presentation {
                layers: &def.layers,
                pieces: &pieces,
                frame: frame.as_ref(),
                elapsed,
                params: &instance.params,
                content: &content,
                screen_layer: &screen_layer,
            },
            &|shader| shaders.get(&shader).cloned(),
            &mut overdrawn,
        );
    }
    let uptime = Seconds(world.resource::<Time>().elapsed_secs());
    let overlays: Vec<(Entity, ShaderOverlay)> = world
        .query::<(Entity, &ShaderOverlay)>()
        .iter(world)
        .map(|(entity, overlay)| (entity, overlay.clone()))
        .collect();
    for (entity, overlay) in overlays {
        let piece = match world.get::<VfxView>(entity) {
            Some(view) => view.pieces.first().copied(),
            None => {
                let piece = shader_piece(world);
                attach_pieces(world, entity, vec![piece]);
                Some(piece)
            }
        };
        let Some(piece) = piece else {
            continue;
        };
        let compiled = world
            .resource::<VfxShaders>()
            .by_id
            .get(&overlay.shader)
            .cloned();
        let Some((frame, compiled)) =
            SiteFrame::of(world, &overlay.site, Radians::default()).zip(compiled)
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
        place_in_depth(world, piece, &frame, 1.0, &screen_layer);
        overdrawn.extend(frame.sprite);
    }
    settle_overdrawn(world, &overdrawn);
}

fn attach(world: &mut World, entity: Entity, def: &VfxDef) -> Vec<Entity> {
    let assets = world.resource::<AssetServer>().clone();
    let pieces = pieces_for(world, &def.layers, &mut |_, sheet| {
        assets.load(sheet.path())
    });
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

/// A world piece sorts just above its site's row; a screen piece sits over the presented frame,
/// on the layer the frame is presented on.
pub fn place_in_depth(
    world: &mut World,
    piece: Entity,
    frame: &SiteFrame,
    screen_z: f32,
    screen_layer: &RenderLayers,
) {
    let mut entity = world.entity_mut(piece);
    match frame.depth {
        Some(depth) => {
            entity.insert(RowDepth(depth.next_up()));
        }
        None => {
            entity.remove::<RowDepth>();
            if let Some(mut transform) = entity.get_mut::<Transform>() {
                transform.translation.z = screen_z;
            }
            entity.insert(screen_layer.clone());
        }
    }
}

/// Hides the sprites a shader layer is drawing over this frame and shows again those it no longer
/// is, so a sprite is never seen twice.
pub fn settle_overdrawn(world: &mut World, overdrawn: &HashSet<Entity>) {
    let was: Vec<Entity> = world
        .query_filtered::<Entity, With<Overdrawn>>()
        .iter(world)
        .collect();
    for entity in was {
        if overdrawn.contains(&entity) {
            continue;
        }
        let mut entity = world.entity_mut(entity);
        let restored = entity.take::<Overdrawn>();
        if let (Some(mut sprite), Some(restored)) = (entity.get_mut::<Sprite>(), restored) {
            sprite.color.set_alpha(restored.alpha);
        }
    }
    for &entity in overdrawn {
        let Ok(mut entity) = world.get_entity_mut(entity) else {
            continue;
        };
        let Some(mut sprite) = entity.get_mut::<Sprite>() else {
            continue;
        };
        let alpha = sprite.color.alpha();
        sprite.color.set_alpha(0.0);
        if !entity.contains::<Overdrawn>() {
            entity.insert(Overdrawn { alpha });
        }
    }
}

fn bounds(points: &[Pos<Meters>]) -> (Vec2, Vec2) {
    let screen: Vec<Vec2> = points.iter().map(|point| point.to_screen()).collect();
    let min = screen.iter().fold(Vec2::MAX, |min, at| min.min(*at));
    let max = screen.iter().fold(Vec2::MIN, |max, at| max.max(*at));
    if screen.is_empty() {
        return (Vec2::ZERO, Vec2::ZERO);
    }
    ((min + max) / 2.0, max - min)
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
