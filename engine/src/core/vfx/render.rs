use std::collections::{HashMap, HashSet};

use bevy::camera::visibility::RenderLayers;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::{Shader, ShaderRef};
use bevy::sprite::Anchor;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey, Material2dPipeline};
use bevy::window::PrimaryWindow;
use bevy_egui::EguiPrimaryContextPass;
use design_tokens::family;
use egui::emath::TSTransform;
use egui::load::SizedTexture;
use egui::{Color32, FontId};

use super::shader::PARAMS_BYTES;
use super::{
    ParamFill, ParamKind, ParamValue, Play, SpriteAnchor, VfxDef, VfxInstance, VfxLayer, VfxParams,
    VfxShaderDef, VfxShaderId, VfxSite,
};
use crate::core::content::Content;
use crate::core::geometry::{Footprint, Shape};
use crate::core::gui::{self, Band, WidgetRects};
use crate::core::math::{Pos, Rgba};
use crate::core::render::present::{self, PRESENT_LAYER, Viewport};
use crate::core::render::screen::ToTile;
use crate::core::render::transition::WorldViewSystems;
use crate::core::render::{
    self as render, GroundDepth, RowDepth, RowDepths, TILE, ToScreen, tile_to_window,
};
use crate::core::text::{self, TextShadow};
use crate::core::tiling::Tiles;
use crate::core::time::{Seconds, ServerClock, UnixMillis};
use crate::core::vfx::CompiledVfxShaders;

/// The margin around every world site that a shader layer may draw on, so spent matter (smoke,
/// sparks, mist) can drift clear of the body and dissolve; the prelude fades the margin's outer
/// edge, and the screen is drawn as is.
pub const MARGIN: Tiles = Tiles(2.0);

/// How many times its own size a site of `size` world pixels is drawn, per axis, with `MARGIN`
/// on every side.
pub fn quad_overscan(size: Vec2) -> Vec2 {
    let size = size.max(Vec2::ONE);
    (size + Vec2::splat(2.0 * MARGIN.0 * TILE.0)) / size
}
const SHADOW: TextShadow = TextShadow {
    offset: egui::vec2(1.0, 1.0),
    color: Color32::BLACK,
};
const WIDGET_PULSE: f32 = 0.6;

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

/// On a sprite a shader layer is drawing in place of it: the sprite itself is hidden meanwhile.
#[derive(Component, Clone, Copy)]
pub struct Overdrawn {
    alpha: f32,
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

/// The site a shader layer draws over, as the shader reads it: the texel rect of the site's image
/// within its texture, its size in world pixels, where the layer is in its run, and the geometry
/// of an area or path site.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct SiteUniform {
    pub rect: Vec4,
    pub size: Vec2,
    pub pixel: Vec2,
    pub kind: u32,
    pub facing: f32,
    pub progress: f32,
    pub time: f32,
    pub overscan: Vec2,
    pub shape: u32,
    pub point_count: u32,
    pub origin: Vec2,
    pub extent: Vec2,
    pub tiles: Vec2,
    pub points: [Vec4; MAX_PATH_POINTS / 2],
}

/// How many points of a path site a shader can read; a longer path keeps its first ones.
pub const MAX_PATH_POINTS: usize = 16;

#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct ParamsUniform {
    words: [Vec4; PARAMS_BYTES / 16],
}

#[derive(Resource, Default)]
struct VfxShaders {
    by_id: HashMap<VfxShaderId, Handle<Shader>>,
    generation: u32,
}

#[derive(Resource)]
struct VfxAssets {
    quad: Handle<Mesh>,
    blank: Handle<Image>,
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

/// The client's side of an instance: one render piece per layer, children of the instance so they
/// go when it goes.
#[derive(Component, Clone)]
struct VfxView {
    pieces: Vec<Option<Entity>>,
}

#[derive(Component)]
struct VfxPiece;

#[derive(Clone, Copy)]
struct SpriteSpec {
    cell: Vec2,
    frames: u32,
    fps: f32,
    anchor: SpriteAnchor,
    scale: f32,
    play: Play,
}

#[derive(Clone)]
struct SiteFrame {
    kind: u32,
    center: Vec2,
    size: Vec2,
    facing: f32,
    depth: Option<f32>,
    texture: Option<(Handle<Image>, Rect, Vec2)>,
    sprite: Option<Entity>,
    area: AreaShape,
    path: SitePath,
}

/// Where a shader layer is in its run: how far through its duration, and the seconds since its
/// instance started.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Run {
    progress: f32,
    time: Seconds,
}

impl SiteFrame {
    fn on_screen(&self) -> bool {
        self.kind == SITE_SCREEN
    }

    fn feet(&self) -> Vec2 {
        self.center - Vec2::new(0.0, self.size.y / 2.0)
    }

    fn head(&self) -> Vec2 {
        self.center + Vec2::new(0.0, self.size.y / 2.0)
    }
}

pub const SITE_ACTOR: u32 = 0;
pub const SITE_OBJECT: u32 = 1;
pub const SITE_AREA: u32 = 2;
pub const SITE_PATH: u32 = 3;
pub const SITE_SCREEN: u32 = 4;

pub const SHAPE_NONE: u32 = 0;
pub const SHAPE_CIRCLE: u32 = 1;
pub const SHAPE_RECT: u32 = 2;
pub const SHAPE_CONE: u32 = 3;
pub const SHAPE_LINE: u32 = 4;

/// How a site's body lies on the quad drawn over it, as a shader reads it through the prelude's
/// `site_*` functions: an area's shape and its measures in tiles, where the site's point sits in
/// site uv, and the quad's size in tiles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AreaShape {
    pub shape: u32,
    pub origin: Vec2,
    pub extent: Vec2,
    pub tiles: Vec2,
}

impl AreaShape {
    /// A site that is no area, drawn over its image: its body is the image's box, `tiles` across.
    pub fn image(tiles: Vec2) -> AreaShape {
        AreaShape {
            tiles,
            ..AreaShape::default()
        }
    }
}

impl Default for AreaShape {
    fn default() -> AreaShape {
        AreaShape {
            shape: SHAPE_NONE,
            origin: Vec2::splat(0.5),
            extent: Vec2::ZERO,
            tiles: Vec2::ONE,
        }
    }
}

/// The points of a path site in site uv, two to a word, as a shader reads them through the
/// prelude's `path_point`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SitePath {
    pub count: u32,
    pub points: [Vec4; MAX_PATH_POINTS / 2],
}

impl Default for SitePath {
    fn default() -> SitePath {
        SitePath {
            count: 0,
            points: [Vec4::ZERO; MAX_PATH_POINTS / 2],
        }
    }
}

/// The quad a path site is drawn on, half a tile clear of its points on every side: its centre and
/// size in world pixels, where its points lie on it and the quad's size in tiles.
pub fn path_quad(points: &[Pos<Tiles>]) -> (Vec2, Vec2, SitePath, AreaShape) {
    let (center, size) = bounds(points);
    let size = size + Vec2::splat(TILE.0);
    let corner = Vec2::new(center.x - size.x / 2.0, center.y + size.y / 2.0);
    let uv = |at: Pos<Tiles>| {
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
    let area = AreaShape {
        shape: SHAPE_NONE,
        origin: points.first().map_or(Vec2::splat(0.5), |first| uv(*first)),
        extent: Vec2::ZERO,
        tiles: size / TILE.0,
    };
    (center, size, path, area)
}

/// The quad an area site is drawn on: its centre and size in world pixels, and the footprint on it.
pub fn area_quad(footprint: &Footprint) -> (Vec2, Vec2, AreaShape) {
    let (center, size) = bounds(&footprint.outline(32));
    let size = size.max(Vec2::ONE);
    let at = footprint.at.to_screen();
    let origin = Vec2::new(
        (at.x - (center.x - size.x / 2.0)) / size.x,
        ((center.y + size.y / 2.0) - at.y) / size.y,
    );
    let (shape, extent) = match footprint.shape {
        Shape::Circle { radius } => (SHAPE_CIRCLE, Vec2::new(radius.0, 0.0)),
        Shape::Rect { width, height } => (SHAPE_RECT, Vec2::new(width.0, height.0)),
        Shape::Cone { angle, length } => (SHAPE_CONE, Vec2::new(angle.radians().0, length.0)),
        Shape::Line { length, width } => (SHAPE_LINE, Vec2::new(length.0, width.0)),
    };
    let area = AreaShape {
        shape,
        origin,
        extent,
        tiles: size / TILE.0,
    };
    (center, size, area)
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
            let path = format!("vfx_shader/{}-{generation}.wgsl", id.key(&content));
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
    for (entity, instance) in instances {
        let def = instance.vfx.get(&content);
        if world.get::<VfxView>(entity).is_none() {
            attach(world, entity, def);
        }
        let view = world
            .get::<VfxView>(entity)
            .cloned()
            .expect("attached above");
        let frame = site_frame(world, &instance.site);
        let elapsed = now.since(instance.started);
        for (index, layer) in def.layers.iter().enumerate() {
            let Some(piece) = view.pieces[index] else {
                continue;
            };
            let local = elapsed - layer.delay();
            let active = local.0 >= 0.0 && local <= layer.duration();
            let Some(frame) = frame.as_ref().filter(|_| active) else {
                hide(world, piece);
                continue;
            };
            match layer {
                VfxLayer::Sprite {
                    frame: cell,
                    frames,
                    fps,
                    anchor,
                    scale,
                    play,
                    ..
                } => {
                    let spec = SpriteSpec {
                        cell: Vec2::new(cell.width as f32, cell.height as f32),
                        frames: *frames,
                        fps: *fps,
                        anchor: *anchor,
                        scale: *scale,
                        play: *play,
                    };
                    show_sprite(world, piece, frame, &spec, local);
                }
                VfxLayer::Shader {
                    shader,
                    params,
                    duration,
                    ..
                } => {
                    let run = Run {
                        progress: (local.0 / duration.0.max(0.001)).clamp(0.0, 1.0),
                        time: elapsed,
                    };
                    let shown = show_shader(
                        world,
                        piece,
                        frame,
                        shader.get(&content),
                        *shader,
                        params,
                        &instance.params,
                        run,
                    );
                    if shown && let Some(sprite) = frame.sprite {
                        overdrawn.insert(sprite);
                    }
                }
                VfxLayer::Text { .. } => {}
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
        if world.get::<VfxView>(entity).is_none() {
            let piece = shader_piece(world, entity);
            world.entity_mut(entity).insert((
                VfxView {
                    pieces: vec![Some(piece)],
                },
                Transform::default(),
                Visibility::default(),
            ));
        }
        let Some(piece) = world
            .get::<VfxView>(entity)
            .and_then(|view| view.pieces.first().copied().flatten())
        else {
            continue;
        };
        let Some(frame) = site_frame(world, &overlay.site) else {
            hide(world, piece);
            continue;
        };
        let fills = overlay
            .params
            .0
            .iter()
            .map(|(name, value)| (name.clone(), ParamFill::Value(*value)))
            .collect();
        let run = Run {
            progress: 0.0,
            time: uptime - overlay.since,
        };
        let shown = show_shader(
            world,
            piece,
            &frame,
            overlay.shader.get(&content),
            overlay.shader,
            &fills,
            &VfxParams::default(),
            run,
        );
        if shown && let Some(sprite) = frame.sprite {
            overdrawn.insert(sprite);
        }
    }
    settle_overdrawn(world, &overdrawn);
}

fn attach(world: &mut World, entity: Entity, def: &VfxDef) {
    let assets = world.resource::<AssetServer>().clone();
    let pieces: Vec<Option<Entity>> = def
        .layers
        .iter()
        .map(|layer| match layer {
            VfxLayer::Sprite { sheet, .. } => Some(
                world
                    .spawn((
                        Sprite {
                            image: assets.load(sheet.path()),
                            ..default()
                        },
                        Transform::default(),
                        Visibility::Hidden,
                        VfxPiece,
                        ChildOf(entity),
                    ))
                    .id(),
            ),
            VfxLayer::Shader { .. } => Some(shader_piece(world, entity)),
            VfxLayer::Text { .. } => None,
        })
        .collect();
    world.entity_mut(entity).insert((
        VfxView { pieces },
        Transform::default(),
        Visibility::default(),
    ));
}

fn shader_piece(world: &mut World, parent: Entity) -> Entity {
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
            ChildOf(parent),
        ))
        .id()
}

fn hide(world: &mut World, piece: Entity) {
    if let Some(mut visibility) = world.get_mut::<Visibility>(piece) {
        visibility.set_if_neq(Visibility::Hidden);
    }
}

fn show_sprite(
    world: &mut World,
    piece: Entity,
    frame: &SiteFrame,
    spec: &SpriteSpec,
    local: Seconds,
) {
    let SpriteSpec {
        cell,
        frames,
        fps,
        anchor,
        scale,
        play,
    } = *spec;
    let image = world
        .get::<Sprite>(piece)
        .map(|sprite| sprite.image.clone());
    let Some(sheet) = image.and_then(|image| {
        world
            .resource::<Assets<Image>>()
            .get(&image)
            .map(Image::size_f32)
    }) else {
        hide(world, piece);
        return;
    };
    let index = (local.0 * fps) as u32;
    let index = match play {
        Play::Once if index >= frames => {
            hide(world, piece);
            return;
        }
        Play::Once => index,
        Play::Loop { .. } => index % frames.max(1),
    };
    let columns = ((sheet.x / cell.x) as u32).max(1);
    let min = Vec2::new(
        (index % columns) as f32 * cell.x,
        (index / columns) as f32 * cell.y,
    );
    let drawn = cell
        * scale
        * if frame.on_screen() {
            present::SCALE
        } else {
            1.0
        };
    let at = match anchor {
        SpriteAnchor::Center => frame.center,
        SpriteAnchor::Feet => frame.feet() + Vec2::new(0.0, drawn.y / 2.0),
        SpriteAnchor::Head => frame.head() + Vec2::new(0.0, drawn.y / 2.0),
    };
    let mut entity = world.entity_mut(piece);
    if let Some(mut sprite) = entity.get_mut::<Sprite>() {
        sprite.rect = Some(Rect::from_corners(min, min + cell));
        sprite.custom_size = Some(drawn);
    }
    if let Some(mut transform) = entity.get_mut::<Transform>() {
        transform.translation.x = at.x;
        transform.translation.y = at.y;
    }
    place_in_depth(&mut entity, frame, 2.0);
    if let Some(mut visibility) = entity.get_mut::<Visibility>() {
        visibility.set_if_neq(Visibility::Inherited);
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "one layer's whole state, read once"
)]
fn show_shader(
    world: &mut World,
    piece: Entity,
    frame: &SiteFrame,
    shader: &VfxShaderDef,
    shader_id: VfxShaderId,
    fills: &std::collections::BTreeMap<String, ParamFill>,
    given: &VfxParams,
    run: Run,
) -> bool {
    let Some(compiled) = world
        .resource::<VfxShaders>()
        .by_id
        .get(&shader_id)
        .cloned()
    else {
        hide(world, piece);
        return false;
    };
    let blank = world.resource::<VfxAssets>().blank.clone();
    let Some(handle) = world
        .get::<MeshMaterial2d<VfxMaterial>>(piece)
        .map(|material| material.0.clone())
    else {
        return false;
    };
    let overscan = if frame.on_screen() {
        Vec2::ONE
    } else {
        quad_overscan(frame.size)
    };
    let (image, rect, pixel) = frame.texture.clone().unwrap_or((
        blank,
        Rect::from_corners(Vec2::ZERO, Vec2::ONE),
        Vec2::ONE,
    ));
    let site = SiteUniform {
        rect: Vec4::new(rect.min.x, rect.min.y, rect.max.x, rect.max.y),
        size: frame.size,
        pixel,
        kind: frame.kind,
        facing: frame.facing,
        progress: run.progress,
        time: run.time.0,
        overscan,
        shape: frame.area.shape,
        point_count: frame.path.count,
        origin: frame.area.origin,
        extent: frame.area.extent,
        tiles: frame.area.tiles,
        points: frame.path.points,
    };
    let params = fill_params(shader, fills, given);
    if let Some(mut material) = world.resource_mut::<Assets<VfxMaterial>>().get_mut(&handle) {
        material.image = image;
        material.site = site;
        material.params = params;
        material.shader = compiled;
    }
    let mut entity = world.entity_mut(piece);
    if let Some(mut transform) = entity.get_mut::<Transform>() {
        transform.translation.x = frame.center.x;
        transform.translation.y = frame.center.y;
        transform.scale = (frame.size * overscan).extend(1.0);
    }
    place_in_depth(&mut entity, frame, 1.0);
    if let Some(mut visibility) = entity.get_mut::<Visibility>() {
        visibility.set_if_neq(Visibility::Inherited);
    }
    true
}

/// A world piece sorts just above its site's row; a screen piece sits over the presented frame.
fn place_in_depth(entity: &mut EntityWorldMut, frame: &SiteFrame, screen_z: f32) {
    match frame.depth {
        Some(depth) => {
            entity.insert(RowDepth(depth.next_up()));
        }
        None => {
            entity.remove::<RowDepth>();
            if let Some(mut transform) = entity.get_mut::<Transform>() {
                transform.translation.z = screen_z;
            }
            entity.insert(RenderLayers::layer(PRESENT_LAYER));
        }
    }
}

/// The parameter uniform of one shader layer: each declared parameter from the layer's fill, the
/// caller's parameters or the shader's default, laid out as the shader's `Params` struct.
pub fn fill_params(
    shader: &VfxShaderDef,
    fills: &std::collections::BTreeMap<String, ParamFill>,
    given: &VfxParams,
) -> ParamsUniform {
    let mut floats = [0.0f32; PARAMS_BYTES / 4];
    for slot in shader.layout() {
        let default = shader
            .param(&slot.name)
            .map(|param| param.default)
            .unwrap_or(ParamValue::Number(0.0));
        let value = match fills.get(&slot.name) {
            Some(ParamFill::Value(value)) => *value,
            Some(ParamFill::Param { param }) => given.get(param).unwrap_or(default),
            None => default,
        };
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

fn settle_overdrawn(world: &mut World, overdrawn: &HashSet<Entity>) {
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

fn site_frame(world: &World, site: &VfxSite) -> Option<SiteFrame> {
    match site {
        VfxSite::Actor(entity) => sprite_frame(world, *entity, SITE_ACTOR),
        VfxSite::Object(entity) => sprite_frame(world, *entity, SITE_OBJECT),
        VfxSite::Area(footprint) => {
            let (center, size, area) = area_quad(footprint);
            Some(SiteFrame {
                kind: SITE_AREA,
                center,
                size,
                facing: -footprint.facing.0,
                depth: Some(world.resource::<GroundDepth>().0),
                texture: None,
                sprite: None,
                area,
                path: SitePath::default(),
            })
        }
        VfxSite::Path(points) => {
            let (center, size, path, area) = path_quad(points);
            let facing = match (points.first(), points.last()) {
                (Some(first), Some(last)) if first != last => {
                    let heading = last.to_screen() - first.to_screen();
                    heading.y.atan2(heading.x)
                }
                _ => 0.0,
            };
            let south = points.iter().map(|point| point.y).fold(f32::MIN, f32::max);
            Some(SiteFrame {
                kind: SITE_PATH,
                center,
                size,
                facing,
                depth: Some(world.resource::<RowDepths>().at(Tiles(south))),
                texture: None,
                sprite: None,
                area,
                path,
            })
        }
        VfxSite::Screen => {
            let window = world
                .iter_entities()
                .find(|entity| entity.contains::<PrimaryWindow>())
                .and_then(|entity| entity.get::<Window>())?;
            let size = Vec2::new(window.resolution.width(), window.resolution.height());
            let live = present::world_texture(world)?;
            let px = present::SCALE
                * world
                    .get_resource::<Viewport>()
                    .map_or(1.0, |viewport| viewport.scale.max(0.01));
            Some(SiteFrame {
                kind: SITE_SCREEN,
                center: Vec2::ZERO,
                size,
                facing: 0.0,
                depth: None,
                texture: Some((
                    live,
                    Rect::from_corners(Vec2::ZERO, Vec2::ONE),
                    Vec2::ONE / size,
                )),
                sprite: None,
                area: AreaShape::image(size / px),
                path: SitePath::default(),
            })
        }
        VfxSite::Widget(_) => None,
    }
}

fn sprite_frame(world: &World, entity: Entity, kind: u32) -> Option<SiteFrame> {
    let sprite = world.get::<Sprite>(entity)?;
    let transform = world.get::<Transform>(entity)?;
    let depth = world.get::<RowDepth>(entity)?.0;
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
        facing: 0.0,
        depth: Some(depth),
        texture: Some((
            sprite.image.clone(),
            Rect::from_corners(rect.min / sheet, rect.max / sheet),
            Vec2::ONE / sheet,
        )),
        sprite: Some(entity),
        area: AreaShape::image(size / TILE.0),
        path: SitePath::default(),
    })
}

fn bounds(points: &[Pos<Tiles>]) -> (Vec2, Vec2) {
    let screen: Vec<Vec2> = points.iter().map(|point| point.to_screen()).collect();
    let min = screen.iter().fold(Vec2::MAX, |min, at| min.min(*at));
    let max = screen.iter().fold(Vec2::MIN, |max, at| max.max(*at));
    if screen.is_empty() {
        return (Vec2::ZERO, Vec2::ZERO);
    }
    ((min + max) / 2.0, max - min)
}

fn server_now(world: &World) -> Option<UnixMillis> {
    let uptime = Seconds(world.get_resource::<Time>()?.elapsed_secs());
    world.get_resource::<ServerClock>()?.now(uptime)
}

/// Text layers, and every layer of a widget site, draw through the interface rather than the world.
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
        let Some(anchor) = overlay_anchor(world, &gui, &instance.site) else {
            continue;
        };
        for (index, layer) in def.layers.iter().enumerate() {
            let local = elapsed - layer.delay();
            if local.0 < 0.0 || local > layer.duration() {
                continue;
            }
            let progress = (local.0 / layer.duration().0.max(0.001)).clamp(0.0, 1.0);
            let area = gui::world_area(world, ("vfx", entity, index));
            match layer {
                VfxLayer::Text {
                    template,
                    color,
                    size,
                    rise,
                    ..
                } => {
                    let text = VfxDef::fill(template, &instance.params);
                    let risen =
                        anchor.head - egui::vec2(0.0, rise.0 * TILE.0 * anchor.px * ease(progress));
                    let alpha = 1.0 - ((progress - 0.6) / 0.4).clamp(0.0, 1.0);
                    let font = FontId::new(*size, family::prose_bold());
                    let width = gui
                        .fonts_mut(|fonts| {
                            fonts.layout_no_wrap(text.clone(), font.clone(), Color32::WHITE)
                        })
                        .size();
                    area.fixed_pos(risen - egui::vec2(width.x / 2.0, width.y))
                        .show(&gui, |ui| {
                            text::show_shadowed(
                                ui,
                                &text,
                                font,
                                color32(*color).gamma_multiply(alpha),
                                SHADOW,
                                TSTransform::IDENTITY,
                            );
                        });
                }
                VfxLayer::Sprite {
                    sheet,
                    frame,
                    frames,
                    fps,
                    scale,
                    play,
                    ..
                } if anchor.rect.is_some() => {
                    let Some(image) = gui::image_file(world, &sheet.path()) else {
                        continue;
                    };
                    let index = (local.0 * fps) as u32;
                    let index = match play {
                        Play::Once if index >= *frames => continue,
                        Play::Once => index,
                        Play::Loop { .. } => index % (*frames).max(1),
                    };
                    let cell = egui::vec2(frame.width as f32, frame.height as f32);
                    let columns = ((image.size.x / cell.x) as u32).max(1);
                    let min = egui::pos2(
                        (index % columns) as f32 * cell.x,
                        (index / columns) as f32 * cell.y,
                    );
                    let uv = egui::Rect::from_min_size(min, cell);
                    let uv = egui::Rect::from_min_max(
                        egui::pos2(uv.min.x / image.size.x, uv.min.y / image.size.y),
                        egui::pos2(uv.max.x / image.size.x, uv.max.y / image.size.y),
                    );
                    let drawn = cell * *scale * anchor.px;
                    area.fixed_pos(anchor.center - drawn / 2.0)
                        .show(&gui, |ui| {
                            ui.add(
                                egui::Image::new(SizedTexture::new(image.texture, drawn)).uv(uv),
                            );
                        });
                }
                VfxLayer::Shader { shader, params, .. } if anchor.rect.is_some() => {
                    let Some(rect) = anchor.rect else {
                        continue;
                    };
                    let shader = shader.get(&content);
                    let color = params
                        .values()
                        .filter_map(|fill| match fill {
                            ParamFill::Value(ParamValue::Color(color)) => Some(*color),
                            ParamFill::Param { param } => match instance.params.get(param) {
                                Some(ParamValue::Color(color)) => Some(color),
                                _ => None,
                            },
                            ParamFill::Value(ParamValue::Number(_) | ParamValue::Bool(_)) => None,
                        })
                        .next()
                        .or_else(|| {
                            shader.params.iter().find_map(|param| match param.default {
                                ParamValue::Color(color) => Some(color),
                                ParamValue::Number(_) | ParamValue::Bool(_) => None,
                            })
                        })
                        .unwrap_or(Rgba::WHITE);
                    area.fixed_pos(rect.min).show(&gui, |ui| {
                        ui.painter().rect_filled(
                            rect,
                            4.0,
                            color32(color).gamma_multiply(WIDGET_PULSE * (1.0 - progress)),
                        );
                    });
                }
                VfxLayer::Sprite { .. } | VfxLayer::Shader { .. } => {}
            }
        }
    }
}

struct OverlayAnchor {
    center: egui::Pos2,
    head: egui::Pos2,
    /// Window pixels per world pixel at this site.
    px: f32,
    /// The rect of a widget or screen site, whose every layer draws here.
    rect: Option<egui::Rect>,
}

fn overlay_anchor(world: &mut World, gui: &egui::Context, site: &VfxSite) -> Option<OverlayAnchor> {
    let px = present::SCALE * world.resource::<Viewport>().scale.max(0.01);
    match site {
        VfxSite::Screen => {
            let rect = gui.content_rect();
            Some(OverlayAnchor {
                center: rect.center(),
                head: egui::pos2(rect.center().x, rect.min.y + rect.height() * 0.3),
                px: 1.0,
                rect: Some(rect),
            })
        }
        VfxSite::Widget(name) => {
            let rect = *world.resource::<WidgetRects>().0.get(&name.0)?;
            Some(OverlayAnchor {
                center: rect.center(),
                head: rect.center_top(),
                px: 1.0,
                rect: Some(rect),
            })
        }
        _ => {
            let frame = site_frame(world, site)?;
            let center = tile_to_window(world, frame.center.to_tile())?;
            let head = tile_to_window(world, frame.head().to_tile())?;
            Some(OverlayAnchor {
                center: egui::pos2(center.x, center.y),
                head: egui::pos2(head.x, head.y),
                px: px / present::SCALE,
                rect: None,
            })
        }
    }
}

fn ease(progress: f32) -> f32 {
    1.0 - (1.0 - progress) * (1.0 - progress)
}

fn color32(color: Rgba) -> Color32 {
    let [r, g, b, a] = color.0.to_be_bytes();
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

impl Default for SiteUniform {
    fn default() -> SiteUniform {
        SiteUniform {
            rect: Vec4::new(0.0, 0.0, 1.0, 1.0),
            size: Vec2::ONE,
            pixel: Vec2::ONE,
            kind: SITE_ACTOR,
            facing: 0.0,
            progress: 0.0,
            time: 0.0,
            overscan: Vec2::ONE,
            shape: SHAPE_NONE,
            point_count: 0,
            origin: Vec2::splat(0.5),
            extent: Vec2::ZERO,
            tiles: Vec2::ONE,
            points: [Vec4::ZERO; MAX_PATH_POINTS / 2],
        }
    }
}

impl Default for ParamsUniform {
    fn default() -> ParamsUniform {
        ParamsUniform {
            words: [Vec4::ZERO; PARAMS_BYTES / 16],
        }
    }
}
