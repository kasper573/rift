//! Plays every vfx instance on the client: each frame, each of its layers is drawn where its site
//! and its placement put it, as far through its run as the instance is.

use std::collections::BTreeMap;

use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::world_serialization::WorldAssetRoot;
use bevy_hanabi::prelude::{EffectAsset, EffectMesh, EffectProperties, ParticleEffect};

use super::super::standing::HeldStill;
use super::super::uniform::{EffectFrame, ParamsUniform, ShaderValue};
use super::super::{
    LayerMotion, LayerPlace, LightLayer, MeshFit, MeshLayer, MeshShape, ModelLayer, ParamFill,
    ParticleLayer, ParticleLook, ParticleSource, ParticleVelocity, ShakeLayer, SurfaceLayer,
    VfxColor, VfxDef, VfxId, VfxInstance, VfxLayer, VfxPart, VfxShaderId,
};
use super::effect::EffectMaterial;
use super::particles::{self, AreaKind, SURFACE_POINTS};
use super::scene::{DrawnElsewhere, PartFrame, Placing, Scene};
use super::{compiled_shader, repaint, shapes, surface};
use crate::core::content::Content;
use crate::core::geometry::Shape;
use crate::core::math::Pos;
use crate::core::render::camera::CameraShake;
use crate::core::render::world::decal::LIFT;
use crate::core::render::world::lights;
use crate::core::render::world::opacity::Opacity;
use crate::core::render::world::outline::Unoutlined;
use crate::core::render::world::shine::{SurfaceLook, SurfaceShine};
use crate::core::render::world::terrain;
use crate::core::render::{CameraRig, GroundHeights};
use crate::core::time::server_now;
use crate::core::units::{Meters, Seconds};

/// The client's side of an instance: what each of its layers drew, and the seed its shapes and
/// shaders vary by.
#[derive(Component)]
pub(super) struct VfxView {
    layers: Vec<Pieces>,
    seed: f32,
}

/// What a layer drew: its pieces in the world and the material they share, the bolt or the patch
/// they were last built for, and its particle emitters, one for each run of the vfx that started
/// one, each kept until its last particle dies.
#[derive(Default)]
struct Pieces {
    shown: Vec<Entity>,
    material: Option<Handle<EffectMaterial>>,
    built: Option<Built>,
    emitters: Vec<(u64, Entity)>,
}

/// What a layer's pieces were built for, so they are built again only when it changes.
#[derive(Clone, Copy, PartialEq)]
enum Built {
    Bolt(i64),
    Patch(Vec3, Vec2, i32),
}

/// The compiled particle effects every play of a layer shares, and the meshes particles are drawn
/// as.
#[derive(Resource, Default)]
pub(super) struct ParticleAssets {
    effects: HashMap<ParticleKey, Handle<EffectAsset>>,
    puff: Option<Handle<Mesh>>,
    shard: Option<Handle<Mesh>>,
    spike: Option<Handle<Mesh>>,
    sources: HashMap<String, Handle<Mesh>>,
    laid: HashMap<String, Handle<Mesh>>,
}

/// What sets one compiled particle effect apart from another.
#[derive(Clone, PartialEq, Eq, Hash)]
struct ParticleKey {
    vfx: VfxId,
    layer: usize,
    area: Option<AreaKind>,
    colors: Vec<[u32; 5]>,
}

/// The things lit by a vfx's surface layers last frame, so one no longer lit loses its look.
#[derive(Resource, Default)]
pub(super) struct Shining(HashSet<Entity>);

/// Despawns what an instance drew once the entity holding its view is despawned. A view moved
/// onto another entity, as onto the stand-in a client keeps of what a server despawned, keeps
/// drawing with its pieces.
pub(super) fn forget(gone: On<Despawn<VfxView>>, views: Query<&VfxView>, mut commands: Commands) {
    let Ok(view) = views.get(gone.entity) else {
        return;
    };
    for pieces in &view.layers {
        for entity in pieces
            .shown
            .iter()
            .chain(pieces.emitters.iter().map(|(_, entity)| entity))
        {
            commands.entity(*entity).try_despawn();
        }
    }
}

/// Forgets every compiled particle effect when the content is loaded anew.
pub(super) fn recompile(mut assets: ResMut<ParticleAssets>) {
    assets.effects.clear();
}

pub(super) fn play(world: &mut World) {
    let Some(content) = world.get_resource::<Content>().cloned() else {
        return;
    };
    let Some(now) = server_now(world) else {
        return;
    };
    lay_particle_meshes(world);
    let instances: Vec<(Entity, VfxInstance)> = world
        .query::<(Entity, &VfxInstance)>()
        .iter(world)
        .map(|(entity, instance)| (entity, instance.clone()))
        .collect();
    let mut shines: HashMap<Entity, SurfaceShine> = HashMap::default();
    let mut shake = Vec3::ZERO;
    for (entity, instance) in instances {
        let def = instance.vfx.get(&content);
        if world.get::<VfxView>(entity).is_none() {
            let seed = (entity.to_bits() % 997) as f32 * 0.37;
            let layers = def.layers.iter().map(|_| Pieces::default()).collect();
            world.entity_mut(entity).insert(VfxView { layers, seed });
        }
        let Some(mut scene) = Scene::of(world, &instance) else {
            for index in 0..def.layers.len() {
                clear(world, entity, index);
            }
            continue;
        };
        scene.faded = scene
            .faded
            .max(1.0 - Opacity::of(world, entity).clamp(0.0, 1.0));
        let held = world.get::<HeldStill>(entity).is_some();
        let clock = Clock::of(&instance, def, now.since(instance.started), held);
        for (index, layer) in def.layers.iter().enumerate() {
            let cx = Cx {
                content: &content,
                entity,
                index,
                instance: &instance,
                def,
                scene: &scene,
                clock,
                held,
            };
            if let VfxLayer::Particles(layer) = layer {
                emit(world, &cx, layer);
                continue;
            }
            let Some(time) = layer.at(clock.into_cycle) else {
                clear(world, entity, index);
                continue;
            };
            let life = time.progress;
            match layer {
                VfxLayer::Mesh(mesh) => draw_mesh(world, &cx, mesh, time.local, life),
                VfxLayer::Model(model) => draw_model(world, &cx, model, time.local, life),
                VfxLayer::Coat(coat) => {
                    let frame = |placing: &Placing| {
                        coat_frame(placing, &cx, coat.inflate, life, time.local)
                    };
                    draw_coat(world, &cx, coat.shader, &coat.params, coat.blend, frame);
                }
                VfxLayer::Ground(ground) => draw_ground(world, &cx, ground, life, time.local),
                VfxLayer::Light(light) => draw_light(world, &cx, light, life),
                VfxLayer::Surface(surface) => {
                    shine(world, &cx, surface, life, &mut shines);
                }
                VfxLayer::Shake(layer) => shake += shaking(world, &cx, layer, time.local, life),
                VfxLayer::Particles(_) => {}
            }
        }
    }
    let lit: HashSet<Entity> = shines.keys().copied().collect();
    let was = std::mem::take(&mut world.resource_mut::<Shining>().0);
    for gone in was.difference(&lit) {
        if let Ok(mut thing) = world.get_entity_mut(*gone) {
            thing.remove::<SurfaceShine>();
        }
    }
    for (thing, shine) in shines {
        if let Ok(mut thing) = world.get_entity_mut(thing)
            && thing.get::<SurfaceShine>() != Some(&shine)
        {
            thing.insert(shine);
        }
    }
    world.resource_mut::<Shining>().0 = lit;
    world
        .resource_mut::<CameraShake>()
        .set_if_neq(CameraShake(shake));
}

/// Where an instance is in its run: the seconds since it started, how many times a vfx played
/// over and over has started again, and how far into the run it is now.
#[derive(Clone, Copy)]
struct Clock {
    elapsed: Seconds,
    cycle: u64,
    into_cycle: Seconds,
    period: Seconds,
}

impl Clock {
    fn of(instance: &VfxInstance, def: &VfxDef, elapsed: Seconds, held: bool) -> Clock {
        let period = Seconds(def.cycle().0.max(f32::EPSILON));
        if held {
            return Clock {
                elapsed,
                cycle: 0,
                into_cycle: period * 0.5,
                period,
            };
        }
        match instance.until {
            Some(_) => Clock {
                elapsed,
                cycle: 0,
                into_cycle: elapsed,
                period,
            },
            None => {
                let cycle = (elapsed.0 / period.0).floor().max(0.0);
                Clock {
                    elapsed,
                    cycle: cycle as u64,
                    into_cycle: Seconds(elapsed.0 - cycle * period.0),
                    period,
                }
            }
        }
    }
}

/// One layer of one instance, as this frame draws it.
struct Cx<'a> {
    content: &'a Content,
    entity: Entity,
    index: usize,
    instance: &'a VfxInstance,
    def: &'a VfxDef,
    scene: &'a Scene,
    clock: Clock,
    held: bool,
}

impl Cx<'_> {
    fn placing<'w>(&'w self, world: &'w World) -> Placing<'w> {
        Placing {
            world,
            instance: self.instance,
            def: self.def,
            scene: self.scene,
        }
    }

    fn seed(&self, world: &World) -> f32 {
        let base = world
            .get::<VfxView>(self.entity)
            .map_or(0.0, |view| view.seed);
        base + self.index as f32 * 3.1
    }

    fn pieces<'w>(&self, world: &'w mut World) -> Option<Mut<'w, VfxView>> {
        world.get_mut::<VfxView>(self.entity)
    }

    /// A shader's parameters as the layer fills them and the instance fills the row's.
    fn params(&self, shader: VfxShaderId, fills: &BTreeMap<String, ParamFill>) -> ParamsUniform {
        ParamsUniform::pack(shader.get(self.content), |name| match fills.get(name)? {
            ParamFill::Param { param } => self
                .def
                .param(param, &self.instance.params)
                .map(ShaderValue::from),
            ParamFill::Color(color) => Some(ShaderValue::Light(self.color(color))),
            ParamFill::Value(value) => Some(ShaderValue::from(*value)),
        })
    }

    fn color(&self, color: &VfxColor) -> LinearRgba {
        color.linear(self.def, &self.instance.params)
    }

    /// The age at which the layer ends for good: none for one that plays on.
    fn ends(&self) -> Seconds {
        if self.instance.until.is_none() && !self.held {
            return Seconds(0.0);
        }
        self.def.layers[self.index].run().duration
    }

    /// How long the layer has played: across every cycle of a vfx that loops, so what animates by
    /// it never jumps as one cycle gives way to the next.
    fn age(&self, local: Seconds) -> Seconds {
        if self.held {
            return local;
        }
        Seconds(local.0 + self.clock.cycle as f32 * self.clock.period.0)
    }

    fn frame(
        &self,
        placed: (Vec3, Quat),
        scale: Vec3,
        seed: f32,
        life: f32,
        local: Seconds,
    ) -> EffectFrame {
        EffectFrame {
            origin: placed.0.extend(self.instance.heading.0),
            rotation: Vec4::from_array(placed.1.to_array()),
            scale: scale.extend(seed),
            life: Vec4::new(life, self.age(local).0, self.ends().0, 0.0),
            site: self.scene.measures(),
            extra: Vec4::new(0.0, 0.0, self.scene.faded, 0.0),
            mask: Vec4::ZERO,
        }
    }
}

/// How far a shape softens where it meets what stands behind it, in metres.
const SHAPE_SOFTNESS: f32 = 0.25;

/// How an effect layer's frame reads what it is drawn on, beside a shape of its own: a thing's
/// surface, or the ground.
const ON_SURFACE: f32 = 1.0;
const ON_GROUND: f32 = 2.0;

fn draw_mesh(world: &mut World, cx: &Cx, mesh: &MeshLayer, local: Seconds, life: f32) {
    let Some(shader) = compiled_shader(world, mesh.shader) else {
        clear(world, cx.entity, cx.index);
        return;
    };
    let seed = cx.seed(world);
    let (placed, scale) = {
        let placing = cx.placing(world);
        let (placed, mut scale) = moved(&placing, &mesh.place, &mesh.motion, cx.age(local), life);
        let placed = match (mesh.fit, placing.part()) {
            (MeshFit::Wrap, Some(part)) => {
                let across = part.size.x.max(part.size.z);
                scale *= Vec3::new(across, part.size.y, across);
                let floor = part.center - Vec3::Y * part.size.y / 2.0;
                let offset = Vec3::from_array(mesh.place.offset.map(|metres| metres.0));
                (floor + placed.1 * (offset * scale), placed.1)
            }
            _ => placed,
        };
        if let (MeshFit::Area, Some(area)) = (mesh.fit, &cx.scene.area) {
            scale.x *= area.across / 2.0;
            scale.z *= area.along / 2.0;
        }
        (placed, scale)
    };
    let mut frame = cx.frame(placed, scale, seed, life, local);
    frame.life.w = SHAPE_SOFTNESS;
    let params = cx.params(mesh.shader, &mesh.params);
    let material = EffectMaterial {
        frame,
        params,
        shader,
        blend: mesh.blend,
        coat: false,
    };
    if let MeshShape::PathTube {
        radius,
        jitter,
        rate,
        branches,
    } = mesh.shape
    {
        let tick = (local.0 * rate.0).floor() as i64;
        let material = EffectMaterial {
            frame: EffectFrame {
                origin: Vec4::new(0.0, 0.0, 0.0, cx.instance.heading.0),
                rotation: Vec4::W,
                scale: Vec4::new(1.0, 1.0, 1.0, seed + tick as f32 * 17.0),
                ..frame
            },
            ..material
        };
        if built(world, cx) != Some(Built::Bolt(tick)) {
            clear(world, cx.entity, cx.index);
            let path = cx.scene.path.clone();
            let bolt_seed = (seed.to_bits() as u64) << 20 | (tick as u64 & 0xfffff);
            let tubes = shapes::bolts(&path, radius.0, jitter.0, branches, bolt_seed);
            let handle = world.resource_mut::<Assets<EffectMaterial>>().add(material);
            let mut shown = Vec::new();
            for tube in tubes {
                let tube = world.resource_mut::<Assets<Mesh>>().add(tube);
                shown.push(spawn_shape(
                    world,
                    tube,
                    handle.clone(),
                    Transform::IDENTITY,
                ));
            }
            set_pieces(world, cx, shown, Some(handle), Some(Built::Bolt(tick)));
        } else {
            paint(world, cx, material);
        }
        return;
    }
    let transform = Transform::from_translation(placed.0)
        .with_rotation(placed.1)
        .with_scale(scale);
    if shown(world, cx).is_empty() {
        let built = match (&mesh.shape, mesh.fit, cx.scene.area.map(|area| area.shape)) {
            (
                MeshShape::Band { inner, outer, .. },
                MeshFit::Cone,
                Some(Shape::Cone { angle, length }),
            ) => Some(shapes::band(
                angle.0,
                inner.0 * length.0,
                outer.0 * length.0,
            )),
            (shape, _, _) => shapes::build(shape),
        };
        let Some(built) = built else {
            return;
        };
        let built = world.resource_mut::<Assets<Mesh>>().add(built);
        let handle = world.resource_mut::<Assets<EffectMaterial>>().add(material);
        let piece = spawn_shape(world, built, handle.clone(), transform);
        set_pieces(world, cx, vec![piece], Some(handle), None);
    } else {
        paint(world, cx, material);
        place_all(world, cx, transform);
    }
}

fn draw_model(world: &mut World, cx: &Cx, model: &ModelLayer, local: Seconds, life: f32) {
    let (placed, scale) = moved(
        &cx.placing(world),
        &model.place,
        &model.motion,
        cx.age(local),
        life,
    );
    let transform = Transform::from_translation(placed.0)
        .with_rotation(placed.1)
        .with_scale(scale);
    if shown(world, cx).is_empty() {
        let scene = world
            .resource::<AssetServer>()
            .load(GltfAssetLabel::Scene(0).from_asset(model.model.path()));
        let piece = world
            .spawn((
                WorldAssetRoot(scene),
                transform,
                Visibility::default(),
                Unoutlined,
            ))
            .id();
        set_pieces(world, cx, vec![piece], None, None);
    } else {
        place_all(world, cx, transform);
    }
}

/// Where a shape or a model stands, how it turns and how big it is `life` through its run, as it
/// grows, drifts, swings and spins from where it is placed.
fn moved(
    placing: &Placing,
    place: &LayerPlace,
    motion: &LayerMotion,
    age: Seconds,
    life: f32,
) -> ((Vec3, Quat), Vec3) {
    let spun = Quat::from_euler(
        EulerRot::XYZ,
        motion.spin[0].0.to_radians() * age.0,
        motion.spin[1].0.to_radians() * age.0,
        motion.spin[2].0.to_radians() * age.0,
    );
    let mut place = place.clone();
    if let Some(drift) = &motion.drift {
        let drifted = drift.at(life, |offset| offset.map(|metres| metres.0));
        for (offset, by) in place.offset.iter_mut().zip(drifted) {
            offset.0 += by;
        }
    }
    if let Some(swing) = &motion.swing {
        let swung = swing.at(life, |turn| turn.map(|degrees| degrees.0));
        for (turn, by) in place.turn.iter_mut().zip(swung) {
            turn.0 += by;
        }
    }
    let scale = motion.size.as_ref().map_or(Vec3::ONE, |size| {
        Vec3::from_array(size.at(life, |scale| *scale))
    });
    (placing.place(&place, life, spun), scale)
}

/// The frame of a coat over the part of the thing it plays on, or over an area's ground.
fn coat_frame(
    placing: &Placing,
    cx: &Cx,
    inflate: Meters,
    life: f32,
    local: Seconds,
) -> Option<EffectFrame> {
    let seed = cx.seed(placing.world);
    if let Some(area) = &cx.scene.area {
        let mut frame = cx.frame(
            (area.middle, area.turn),
            Vec3::new(area.across, 1.0, area.along),
            seed,
            life,
            local,
        );
        frame.extra.y = ON_GROUND;
        return Some(frame);
    }
    let PartFrame {
        center,
        rotation,
        size,
        mask,
    } = placing.part()?;
    let mut frame = cx.frame((center, rotation), size, seed, life, local);
    frame.extra.x = inflate.0;
    frame.extra.y = ON_SURFACE;
    frame.mask = mask;
    Some(frame)
}

/// Draws a coat: a hull over each mesh of the part of the thing it plays on, riding the mesh's
/// skin, or a patch over an area's ground.
fn draw_coat(
    world: &mut World,
    cx: &Cx,
    shader: VfxShaderId,
    fills: &BTreeMap<String, ParamFill>,
    blend: super::super::VfxBlend,
    frame: impl Fn(&Placing) -> Option<EffectFrame>,
) {
    let Some(compiled) = compiled_shader(world, shader) else {
        clear(world, cx.entity, cx.index);
        return;
    };
    let Some(frame) = frame(&cx.placing(world)) else {
        clear(world, cx.entity, cx.index);
        return;
    };
    let material = EffectMaterial {
        frame,
        params: cx.params(shader, fills),
        shader: compiled,
        blend,
        coat: cx.scene.area.is_none(),
    };
    if let Some(area) = cx.scene.area {
        lay_patch(
            world,
            cx,
            material,
            area.middle,
            Vec2::new(area.across, area.along),
            area.turn,
        );
        return;
    }
    if !shown(world, cx).is_empty() {
        paint(world, cx, material);
        return;
    }
    let sources: Vec<(Entity, Handle<Mesh>, Option<SkinnedMesh>)> = cx
        .placing(world)
        .part_meshes()
        .into_iter()
        .filter_map(|source| {
            let mesh = world.get::<Mesh3d>(source)?.0.clone();
            Some((source, mesh, world.get::<SkinnedMesh>(source).cloned()))
        })
        .collect();
    if sources.is_empty() {
        return;
    }
    let handle = world.resource_mut::<Assets<EffectMaterial>>().add(material);
    let shown = match cx
        .scene
        .thing
        .filter(|thing| world.get::<DrawnElsewhere>(thing.entity).is_some())
    {
        Some(_) => Visibility::Visible,
        None => Visibility::Inherited,
    };
    let hulls = sources
        .into_iter()
        .map(|(source, mesh, skin)| {
            let mut hull = world.spawn((
                Mesh3d(mesh),
                MeshMaterial3d(handle.clone()),
                Transform::IDENTITY,
                shown,
                ChildOf(source),
                NotShadowCaster,
                NotShadowReceiver,
                Unoutlined,
            ));
            if let Some(skin) = skin {
                hull.insert(skin);
            }
            hull.id()
        })
        .collect();
    set_pieces(world, cx, hulls, Some(handle), None);
}

fn draw_ground(
    world: &mut World,
    cx: &Cx,
    ground: &super::super::GroundLayer,
    life: f32,
    local: Seconds,
) {
    let Some(compiled) = compiled_shader(world, ground.shader) else {
        clear(world, cx.entity, cx.index);
        return;
    };
    let place = LayerPlace {
        offset: ground.offset,
        ..LayerPlace::default()
    };
    let (at, _) = cx.placing(world).place(&place, life, Quat::IDENTITY);
    let turn = cx.scene.heading;
    let size = match (ground.fit, &cx.scene.area) {
        (true, Some(area)) => Vec2::new(area.across, area.along) * ground.size.0,
        _ => Vec2::splat(ground.size.0),
    };
    let at = match (ground.fit, &cx.scene.area) {
        (true, Some(area)) => area.middle,
        _ => at,
    };
    let seed = cx.seed(world);
    let mut frame = cx.frame(
        (at, turn),
        Vec3::new(size.x, 1.0, size.y),
        seed,
        life,
        local,
    );
    frame.extra.y = ON_GROUND;
    let material = EffectMaterial {
        frame,
        params: cx.params(ground.shader, &ground.params),
        shader: compiled,
        blend: ground.blend,
        coat: false,
    };
    let laid = world
        .get::<VfxView>(cx.entity)
        .and_then(|view| view.layers[cx.index].material.as_ref())
        .and_then(|handle| world.resource::<Assets<EffectMaterial>>().get(handle))
        .map(|material| material.frame);
    match laid {
        Some(laid) if ground.stays => {
            let material = EffectMaterial {
                frame: EffectFrame {
                    life: material.frame.life,
                    extra: material.frame.extra,
                    ..laid
                },
                ..material
            };
            paint(world, cx, material);
        }
        _ => lay_patch(world, cx, material, at, size, turn),
    }
}

/// Lays an effect over the ground or the water `size` metres round `at`, turned by `turn`, its
/// patch built again only when it moved.
fn lay_patch(
    world: &mut World,
    cx: &Cx,
    material: EffectMaterial,
    at: Vec3,
    size: Vec2,
    turn: Quat,
) {
    let yaw = -turn.to_euler(EulerRot::YXZ).0;
    let wanted = Built::Patch(at, size, (yaw * 1000.0) as i32);
    if built(world, cx) == Some(wanted) {
        paint(world, cx, material);
        return;
    }
    let Some(heights) = world.get_resource::<GroundHeights>() else {
        return;
    };
    let patch = terrain::patch(
        |point: Pos<Meters>| heights.surface_at(point),
        Vec2::new(at.x, at.z),
        size,
        yaw,
        LIFT,
    );
    clear(world, cx.entity, cx.index);
    let patch = world.resource_mut::<Assets<Mesh>>().add(patch);
    let handle = world.resource_mut::<Assets<EffectMaterial>>().add(material);
    let piece = spawn_shape(world, patch, handle.clone(), Transform::IDENTITY);
    set_pieces(world, cx, vec![piece], Some(handle), Some(wanted));
}

fn draw_light(world: &mut World, cx: &Cx, light: &LightLayer, life: f32) {
    let (at, _) = cx.placing(world).place(&light.place, life, Quat::IDENTITY);
    let brightest = light
        .intensity
        .keys
        .iter()
        .map(|(_, intensity)| *intensity)
        .fold(0.0, f32::max);
    let level = if brightest > 0.0 {
        light.intensity.at(life, |intensity| *intensity) / brightest
    } else {
        0.0
    };
    if shown(world, cx).is_empty() {
        let color = cx.color(&light.color);
        let lamp = lights::Lamp {
            color: Color::LinearRgba(LinearRgba::rgb(color.red, color.green, color.blue)),
            intensity: brightest,
            radius: light.radius,
            flicker: if cx.held { 0.0 } else { light.flicker },
            shadows_from: light.shadows_from,
            aura: light.aura,
        };
        let piece = lamp.spawn(world, cx.seed(world));
        world.entity_mut(piece).insert(Unoutlined);
        set_pieces(world, cx, vec![piece], None, None);
    }
    place_all(world, cx, Transform::from_translation(at));
    let shown_level = Opacity(level * (1.0 - cx.scene.faded));
    for piece in shown(world, cx) {
        if world.get::<Opacity>(piece) != Some(&shown_level) {
            world.entity_mut(piece).insert(shown_level);
        }
    }
}

/// Lays a surface layer's tint, rim and glow over the part of the thing it plays on, on top of
/// whatever other layers lay this frame.
fn shine(
    world: &World,
    cx: &Cx,
    layer: &SurfaceLayer,
    life: f32,
    shines: &mut HashMap<Entity, SurfaceShine>,
) {
    let Some(thing) = cx.scene.thing else {
        return;
    };
    let color = |track: &Option<super::super::VfxTrack<VfxColor>>| {
        track
            .as_ref()
            .map(|track| track.at(life, |color| cx.color(color)).to_vec4())
    };
    let (tint, rim, glow) = (color(&layer.tint), color(&layer.rim), color(&layer.glow));
    let mask = match &cx.instance.part {
        VfxPart::Node { .. } => cx
            .placing(world)
            .part()
            .map_or(Vec4::ZERO, |part| part.mask),
        _ => Vec4::ZERO,
    };
    let (body, held) = match cx.instance.part {
        VfxPart::Held => (false, true),
        VfxPart::Body => (true, false),
        VfxPart::Whole | VfxPart::Node { .. } => (true, true),
    };
    let lit = shines.entry(thing.entity).or_default();
    for (look, covered) in [(&mut lit.body, body), (&mut lit.held, held)] {
        if !covered {
            continue;
        }
        look.mask = mask;
        laid_over(look, tint, rim, glow, 1.0 - cx.scene.faded);
    }
}

/// A look with `tint`, `rim` and `glow` laid over it: the strongest tint wins, light adds.
fn laid_over(
    look: &mut SurfaceLook,
    tint: Option<Vec4>,
    rim: Option<Vec4>,
    glow: Option<Vec4>,
    shown: f32,
) {
    if let Some(tint) = tint
        && tint.w * shown > look.tint.w
    {
        look.tint = tint.truncate().extend(tint.w * shown);
    }
    let add = |was: Vec4, light: Option<Vec4>| match light {
        Some(light) => (was.truncate() * was.w + light.truncate() * light.w * shown).extend(1.0),
        None => was,
    };
    look.rim = add(look.rim, rim);
    look.glow = add(look.glow, glow);
}

/// How far a shake layer throws the camera this frame: hardest at its start, easing to rest, and
/// less the farther the camera's subject stands from where it plays, nothing past its reach.
fn shaking(world: &World, cx: &Cx, layer: &ShakeLayer, local: Seconds, life: f32) -> Vec3 {
    let at = cx.placing(world).anchor(super::super::VfxAnchor::Feet);
    let near = world.get_resource::<CameraRig>().map_or(0.0, |rig| {
        1.0 - (rig.target().distance(at) / layer.reach.0).min(1.0)
    });
    let fall = (1.0 - life) * (1.0 - life);
    let t = local.0;
    Vec3::new(
        (t * 61.0 + 1.0).sin() + 0.5 * (t * 97.0).sin(),
        (t * 53.0 + 2.0).sin() * 0.6,
        (t * 71.0 + 3.0).sin() + 0.5 * (t * 89.0 + 1.0).sin(),
    ) * layer.strength.0
        * fall
        * near
}

/// Runs a particle layer's emitters: one starts as each run of the vfx reaches the layer, follows
/// where the layer stands while it emits, and goes once its last particle has died.
fn emit(world: &mut World, cx: &Cx, layer: &ParticleLayer) {
    let run = layer.run;
    let started = cx.clock.into_cycle >= run.delay;
    let emitting = cx.def.layers[cx.index].at(cx.clock.into_cycle).is_some();
    let lingers = run.ends() + layer.life[1];
    let cycle = cx.clock.cycle;
    let period = cx.clock.period;
    let Some(view) = world.get::<VfxView>(cx.entity) else {
        return;
    };
    let done: Vec<Entity> = view.layers[cx.index]
        .emitters
        .iter()
        .filter(|(started_in, _)| {
            let since = cx.clock.elapsed.0 - *started_in as f32 * period.0;
            since > lingers.0
        })
        .map(|(_, entity)| *entity)
        .collect();
    let has = view.layers[cx.index]
        .emitters
        .iter()
        .find(|(started_in, _)| *started_in == cycle)
        .map(|(_, entity)| *entity);
    for entity in &done {
        if let Ok(emitter) = world.get_entity_mut(*entity) {
            emitter.despawn();
        }
    }
    if let Some(mut view) = cx.pieces(world) {
        view.layers[cx.index]
            .emitters
            .retain(|(_, entity)| !done.contains(entity));
    }
    let life = cx.def.layers[cx.index]
        .at(cx.clock.into_cycle)
        .map_or(1.0, |time| time.progress);
    let placed = {
        let placing = cx.placing(world);
        let (at, turn) = placing.place(&layer.place, life, Quat::IDENTITY);
        let surface = (layer.from == ParticleSource::Part && cx.scene.thing.is_some())
            .then(|| sample_surface(&placing, world))
            .flatten();
        (at, turn, surface)
    };
    match has {
        Some(entity) if emitting && !layer.place.stays => {
            set_emitter(world, entity, cx, layer, placed);
        }
        Some(_) => {}
        None if started && emitting => {
            let Some(effect) = compiled_effect(world, cx, layer) else {
                return;
            };
            let mut emitter = world.spawn((
                ParticleEffect::new(effect),
                EffectProperties::default(),
                Transform::default(),
                Visibility::default(),
            ));
            if let Some(mesh) =
                particle_mesh(&layer.look, emitter.world().resource::<ParticleAssets>())
            {
                emitter.insert(EffectMesh(mesh));
            }
            let entity = emitter.id();
            set_emitter(world, entity, cx, layer, placed);
            if let Some(mut view) = cx.pieces(world) {
                view.layers[cx.index].emitters.push((cycle, entity));
            }
        }
        None => {}
    }
}

/// Stands an emitter where its layer stands now and tells it what its site sets apart: the area it
/// emits across, the way its cone points and the points of the surface it emits from.
fn set_emitter(
    world: &mut World,
    entity: Entity,
    cx: &Cx,
    layer: &ParticleLayer,
    (at, turn, surface): (Vec3, Quat, Option<Surface>),
) {
    let mut values: Vec<(String, bevy_hanabi::prelude::Value)> = Vec::new();
    let mut stands = at;
    if let ParticleVelocity::Cone { way, .. } = layer.velocity {
        let way = (turn * Vec3::from_array(way)).normalize_or(Vec3::Y);
        values.push((particles::WAY.to_owned(), way.into()));
    }
    if let (Some(area), ParticleSource::Area | ParticleSource::Part) = (&cx.scene.area, layer.from)
    {
        let measures = match area.shape {
            Shape::Circle { radius } => Vec4::new(radius.0, 0.0, 0.0, 0.0),
            Shape::Rect { .. } | Shape::Line { .. } => Vec4::new(area.across, area.along, 0.0, 0.0),
            Shape::Cone { angle, length } => Vec4::new(angle.radians().0 / 2.0, length.0, 0.0, 0.0),
        };
        values.push((particles::AREA.to_owned(), measures.into()));
        values.push((particles::RIGHT.to_owned(), (area.turn * Vec3::X).into()));
        values.push((particles::AHEAD.to_owned(), (area.turn * Vec3::Z).into()));
        stands = area.middle + (at - cx.placing(world).anchor(layer.place.anchor));
    }
    if let Some(surface) = surface {
        stands = surface.center;
        for (k, (point, normal)) in surface.points.iter().zip(&surface.normals).enumerate() {
            values.push((
                particles::surface_point(k),
                (*point - surface.center).into(),
            ));
            values.push((particles::surface_normal(k), (*normal).into()));
        }
    }
    let mut emitter = world.entity_mut(entity);
    if let Some(mut transform) = emitter.get_mut::<Transform>() {
        transform.set_if_neq(Transform::from_translation(stands));
    }
    if let Some(mut properties) = emitter.get_mut::<EffectProperties>() {
        for (name, value) in values {
            properties.set(&name, value);
        }
    }
}

/// The effect a particle layer compiles to on this site, compiled once for every play alike.
fn compiled_effect(
    world: &mut World,
    cx: &Cx,
    layer: &ParticleLayer,
) -> Option<Handle<EffectAsset>> {
    if let ParticleLook::Model { model } = &layer.look
        && !world
            .resource::<ParticleAssets>()
            .laid
            .contains_key(&model.path())
    {
        let path = model.path();
        if !world
            .resource::<ParticleAssets>()
            .sources
            .contains_key(&path)
        {
            let source = world.resource::<AssetServer>().load(
                GltfAssetLabel::Primitive {
                    mesh: 0,
                    primitive: 0,
                }
                .from_asset(path.clone()),
            );
            world
                .resource_mut::<ParticleAssets>()
                .sources
                .insert(path, source);
        }
        return None;
    }
    let area = cx.scene.area.and_then(|area| match layer.from {
        ParticleSource::Area | ParticleSource::Part => Some(match area.shape {
            Shape::Circle { .. } => AreaKind::Circle,
            Shape::Rect { .. } | Shape::Line { .. } => AreaKind::Rect,
            Shape::Cone { .. } => AreaKind::Cone,
        }),
        _ => None,
    });
    let colors: Vec<(f32, Vec4)> = layer
        .color
        .iter()
        .map(|(t, color)| (*t, cx.color(color).to_vec4()))
        .collect();
    let key = ParticleKey {
        vfx: cx.instance.vfx,
        layer: cx.index,
        area,
        colors: colors
            .iter()
            .map(|(t, c)| {
                [
                    t.to_bits(),
                    c.x.to_bits(),
                    c.y.to_bits(),
                    c.z.to_bits(),
                    c.w.to_bits(),
                ]
            })
            .collect(),
    };
    if let Some(known) = world.resource::<ParticleAssets>().effects.get(&key) {
        return Some(known.clone());
    }
    let effect = particles::effect(layer, area, &colors);
    let handle = world.resource_mut::<Assets<EffectAsset>>().add(effect);
    world
        .resource_mut::<ParticleAssets>()
        .effects
        .insert(key, handle.clone());
    Some(handle)
}

fn particle_mesh(look: &ParticleLook, assets: &ParticleAssets) -> Option<Handle<Mesh>> {
    match look {
        ParticleLook::Puff => assets.puff.clone(),
        ParticleLook::Shard => assets.shard.clone(),
        ParticleLook::Spike => assets.spike.clone(),
        ParticleLook::Model { model } => assets.laid.get(&model.path()).cloned(),
        _ => None,
    }
}

/// Makes the meshes particles are drawn as, once, and lays each model a look names along its
/// flight once it has loaded.
fn lay_particle_meshes(world: &mut World) {
    if world.resource::<ParticleAssets>().puff.is_none() {
        let mut meshes = world.resource_mut::<Assets<Mesh>>();
        let (puff, shard, spike) = (
            meshes.add(shapes::ico(2)),
            meshes.add(shapes::shard()),
            meshes.add(shapes::spike()),
        );
        let mut assets = world.resource_mut::<ParticleAssets>();
        assets.puff = Some(puff);
        assets.shard = Some(shard);
        assets.spike = Some(spike);
    }
    let pending: Vec<(String, Handle<Mesh>)> = {
        let assets = world.resource::<ParticleAssets>();
        assets
            .sources
            .iter()
            .filter(|(path, _)| !assets.laid.contains_key(*path))
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect()
    };
    for (path, source) in pending {
        let Some(laid) = world
            .resource::<Assets<Mesh>>()
            .get(&source)
            .map(shapes::laid_model)
        else {
            continue;
        };
        let laid = world.resource_mut::<Assets<Mesh>>().add(laid);
        world
            .resource_mut::<ParticleAssets>()
            .laid
            .insert(path, laid);
    }
}

/// Points spread over the surface of the part a vfx plays on, posed as its meshes were last drawn,
/// each with the way the surface faces there; or over a node's round.
struct Surface {
    center: Vec3,
    points: Vec<Vec3>,
    normals: Vec<Vec3>,
}

/// How many points of each mesh's surface the points an emitter reads each frame are taken from:
/// each frame takes another share of them, so emission covers the whole surface within a few
/// frames.
const SURFACE_POOL: usize = 512;

fn sample_surface(placing: &Placing, world: &World) -> Option<Surface> {
    let part = placing.part()?;
    let turn = world
        .get_resource::<Time>()
        .map_or(0, |time| (time.elapsed_secs() * 60.0) as usize);
    let points: Vec<(Vec3, Vec3)> = match &placing.instance.part {
        VfxPart::Node { radius, .. } => {
            let golden = std::f32::consts::PI * (3.0 - 5f32.sqrt());
            (0..SURFACE_POINTS)
                .map(|k| {
                    let y = 1.0 - 2.0 * (k as f32 + 0.5) / SURFACE_POINTS as f32;
                    let r = (1.0 - y * y).sqrt();
                    let a = k as f32 * golden;
                    let out = Vec3::new(a.cos() * r, y, a.sin() * r);
                    (part.center + out * radius.0 * 0.55, out)
                })
                .collect()
        }
        _ => {
            let meshes = world.resource::<Assets<Mesh>>();
            let bindposes = world.resource::<Assets<SkinnedMeshInverseBindposes>>();
            let joint = |entity: Entity| {
                world
                    .get::<GlobalTransform>(entity)
                    .map(GlobalTransform::to_matrix)
            };
            let sources: Vec<(Entity, &Mesh, f32, Vec<surface::Sample>)> = placing
                .part_meshes()
                .into_iter()
                .filter_map(|source| {
                    let mesh = meshes.get(&world.get::<Mesh3d>(source)?.0)?;
                    let (area, samples) = surface::spread(mesh, SURFACE_POOL)?;
                    let scale = world
                        .get::<GlobalTransform>(source)?
                        .scale()
                        .abs()
                        .max_element();
                    Some((source, mesh, area * scale * scale, samples))
                })
                .collect();
            let total: f32 = sources.iter().map(|source| source.2).sum();
            (0..SURFACE_POINTS)
                .filter_map(|k| {
                    let mut at = (k as f32 + 0.5) / SURFACE_POINTS as f32 * total;
                    let (source, mesh, _, samples) = sources.iter().find(|source| {
                        at -= source.2;
                        at <= 0.0
                    })?;
                    let skin = world
                        .get::<SkinnedMesh>(*source)
                        .and_then(|skin| Some((skin, bindposes.get(&skin.inverse_bindposes)?)));
                    surface::locate(
                        mesh,
                        &samples[(k * SURFACE_POOL / SURFACE_POINTS + turn) % SURFACE_POOL],
                        world.get::<GlobalTransform>(*source)?,
                        skin,
                        &joint,
                    )
                })
                .collect()
        }
    };
    let (points, normals) = if points.len() == SURFACE_POINTS {
        points.into_iter().unzip()
    } else {
        (
            vec![part.center; SURFACE_POINTS],
            vec![Vec3::Y; SURFACE_POINTS],
        )
    };
    Some(Surface {
        center: part.center,
        points,
        normals,
    })
}

fn spawn_shape(
    world: &mut World,
    mesh: Handle<Mesh>,
    material: Handle<EffectMaterial>,
    transform: Transform,
) -> Entity {
    world
        .spawn((
            Mesh3d(mesh),
            MeshMaterial3d(material),
            transform,
            Visibility::default(),
            NotShadowCaster,
            NotShadowReceiver,
            Unoutlined,
        ))
        .id()
}

fn shown(world: &World, cx: &Cx) -> Vec<Entity> {
    world
        .get::<VfxView>(cx.entity)
        .map_or(Vec::new(), |view| view.layers[cx.index].shown.clone())
}

fn built(world: &World, cx: &Cx) -> Option<Built> {
    world.get::<VfxView>(cx.entity)?.layers[cx.index].built
}

fn set_pieces(
    world: &mut World,
    cx: &Cx,
    shown: Vec<Entity>,
    material: Option<Handle<EffectMaterial>>,
    built: Option<Built>,
) {
    if let Some(mut view) = cx.pieces(world) {
        let pieces = &mut view.layers[cx.index];
        pieces.shown = shown;
        pieces.material = material;
        pieces.built = built;
    }
}

/// Paints a layer's material as `painted`.
fn paint(world: &mut World, cx: &Cx, painted: EffectMaterial) {
    let Some(handle) = world
        .get::<VfxView>(cx.entity)
        .and_then(|view| view.layers[cx.index].material.clone())
    else {
        return;
    };
    repaint(world, &handle, painted, |was, now| {
        (was.frame, was.params, &was.shader, was.blend)
            == (now.frame, now.params, &now.shader, now.blend)
    });
}

fn place_all(world: &mut World, cx: &Cx, transform: Transform) {
    for piece in shown(world, cx) {
        if let Ok(mut piece) = world.get_entity_mut(piece)
            && let Some(mut placed) = piece.get_mut::<Transform>()
        {
            placed.set_if_neq(transform);
        }
    }
}

/// Takes away what a layer drew, for a layer waiting for its delay or over.
fn clear(world: &mut World, entity: Entity, index: usize) {
    let Some(mut view) = world.get_mut::<VfxView>(entity) else {
        return;
    };
    let Some(pieces) = view.layers.get_mut(index) else {
        return;
    };
    let shown = std::mem::take(&mut pieces.shown);
    pieces.material = None;
    pieces.built = None;
    for piece in shown {
        if let Ok(piece) = world.get_entity_mut(piece) {
            piece.despawn();
        }
    }
}
