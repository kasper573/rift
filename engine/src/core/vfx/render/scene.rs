//! Where a vfx stands in the world this frame: its site measured, and each of its layers placed on
//! it by its anchor, its offset and its turn, or flown along its path.

use bevy::camera::primitives::Aabb;
use bevy::prelude::*;

use super::super::{
    LayerPlace, ParamValue, PathStop, VfxAnchor, VfxDef, VfxInstance, VfxPart, VfxSite,
};
use crate::core::geometry::Shape;
use crate::core::render::world::material::WorldMaterial;
use crate::core::render::world::opacity::Opacity;
use crate::core::render::world::shine::HeldPiece;
use crate::core::render::{facing_of, facing_rotation, ground_height, world_point};
use crate::core::units::Radians;

/// A thing whose meshes stand only for its vfx to wrap: drawn somewhere else, it holds them
/// hidden, and what its coats wrap them in shows of its own.
#[derive(Component, Clone, Copy, Debug)]
pub struct DrawnElsewhere;

/// How big a thing a vfx plays on is. Whoever draws a thing says, as it stands at its entity's
/// transform.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct SiteExtent {
    /// How wide and deep the thing is on the ground, in metres.
    pub footprint: Vec2,
    /// How tall it stands, in metres.
    pub height: f32,
}

/// The node of a thing that what it deals leaves from: the hand it casts with. Whoever draws the
/// thing says which.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct VfxHand(pub Entity);

/// A vfx's site as it stands in the world this frame, and the way its vfx travels there.
pub(super) struct Scene {
    pub thing: Option<Thing>,
    pub area: Option<Area>,
    pub path: Vec<Vec3>,
    /// The way the vfx heads, as the world's facings count it.
    pub facing: Radians,
    /// The vfx's heading, as a turn about the vertical from facing +Z.
    pub heading: Quat,
    /// How far the thing it plays on has faded from sight.
    pub faded: f32,
}

/// A thing a vfx plays on: where its feet stand, how it is turned and how tall it is.
#[derive(Clone, Copy)]
pub(super) struct Thing {
    pub entity: Entity,
    pub feet: Vec3,
    pub turn: Quat,
    pub height: f32,
}

/// An area a vfx plays on: the middle of the patch that covers it, how it is turned, how far it
/// spans across and along its facing, and its shape as an effect reads it from its frame.
#[derive(Clone, Copy)]
pub(super) struct Area {
    pub middle: Vec3,
    pub turn: Quat,
    pub across: f32,
    pub along: f32,
    pub shape: Shape,
}

/// Where a part of a thing is: its middle, how it is turned, how big it is across each of its own
/// axes, and the sphere it is seen within, when only a sphere of it is.
#[derive(Clone, Copy, Debug)]
pub(super) struct PartFrame {
    pub center: Vec3,
    pub rotation: Quat,
    pub size: Vec3,
    pub mask: Vec4,
}

/// A vfx caught this frame, to place its layers on.
pub(super) struct Placing<'w> {
    pub world: &'w World,
    pub instance: &'w VfxInstance,
    pub def: &'w VfxDef,
    pub scene: &'w Scene,
}

impl Scene {
    /// The site of `instance` as it stands this frame, or nothing while it cannot be drawn: a
    /// thing not yet placed, or a path with a stop gone.
    pub fn of(world: &World, instance: &VfxInstance) -> Option<Scene> {
        let scene = match &instance.site {
            VfxSite::Actor(entity) | VfxSite::Object(entity) => {
                let thing = thing(world, *entity)?;
                let facing = instance.heading.unwrap_or_else(|| facing_of(thing.turn));
                Scene {
                    thing: Some(thing),
                    area: None,
                    path: Vec::new(),
                    facing,
                    heading: facing_rotation(facing),
                    faded: 1.0 - Opacity::of(world, *entity).clamp(0.0, 1.0),
                }
            }
            VfxSite::Area(footprint) => {
                let turn = facing_rotation(footprint.facing);
                let ground = |at| world_point(at, ground_height(world, at));
                let at = ground(footprint.at);
                let (across, along, ahead) = match footprint.shape {
                    Shape::Circle { radius } => (2.0 * radius.0, 2.0 * radius.0, 0.0),
                    Shape::Rect { width, height } => (height.0, width.0, 0.0),
                    Shape::Cone { length, .. } => (2.0 * length.0, 2.0 * length.0, 0.0),
                    Shape::Line { length, width } => (width.0, length.0, length.0 / 2.0),
                };
                Scene {
                    thing: None,
                    area: Some(Area {
                        middle: at + turn * Vec3::Z * ahead,
                        turn,
                        across,
                        along,
                        shape: footprint.shape,
                    }),
                    path: Vec::new(),
                    facing: footprint.facing,
                    heading: turn,
                    faded: 0.0,
                }
            }
            VfxSite::Path(stops) => Scene {
                thing: None,
                area: None,
                path: stops
                    .iter()
                    .enumerate()
                    .map(|(index, stop)| stop_point(world, stop, index == 0))
                    .collect::<Option<Vec<Vec3>>>()?,
                facing: instance.heading.unwrap_or_default(),
                heading: facing_rotation(instance.heading.unwrap_or_default()),
                faded: 0.0,
            },
        };
        Some(scene)
    }

    /// The site's measures as an effect reads them from its frame: an area's radius, a cone's
    /// angle in degrees (360 for a circle, 0 for a rectangle), how far it spans along its facing
    /// and across it; a path's length; a thing's height.
    pub fn measures(&self) -> Vec4 {
        if let Some(area) = &self.area {
            return match area.shape {
                Shape::Circle { radius } => Vec4::new(radius.0, 360.0, radius.0, radius.0),
                Shape::Rect { .. } | Shape::Line { .. } => Vec4::new(
                    area.across.max(area.along) / 2.0,
                    0.0,
                    area.along,
                    area.across,
                ),
                Shape::Cone { angle, length } => Vec4::new(length.0, angle.0, length.0, 0.0),
            };
        }
        if let Some(thing) = &self.thing {
            return Vec4::new(0.5, 0.0, thing.height, 0.0);
        }
        let length = self
            .path
            .windows(2)
            .map(|pair| pair[0].distance(pair[1]))
            .sum();
        Vec4::new(0.0, 0.0, 0.0, length)
    }
}

impl Placing<'_> {
    /// Where the hand `actor` deals from stands now: the node its drawer names, or the middle of
    /// a thing drawn with none.
    pub fn hand(&self, actor: Entity) -> Option<Vec3> {
        hand(self.world, actor)
    }

    /// Where the vfx's socket stands on the thing it plays on.
    pub fn socket(&self) -> Option<Vec3> {
        let name = self.def.socket.as_deref()?;
        let node = named_node(self.world, self.scene.thing?.entity, name)?;
        Some(posed(self.world, node).translation())
    }

    /// The meshes the part of the thing the vfx plays on is drawn with: under what it holds for
    /// what it holds, the rest for its body, every one for the whole of it or a node's round.
    pub fn part_meshes(&self) -> Vec<Entity> {
        let Some(thing) = self.scene.thing else {
            return Vec::new();
        };
        let mut found = Vec::new();
        let mut pending = vec![(thing.entity, false)];
        while let Some((at, held)) = pending.pop() {
            let held = held || self.world.get::<HeldPiece>(at).is_some();
            let wanted = match self.instance.part {
                VfxPart::Whole | VfxPart::Node { .. } => true,
                VfxPart::Held => held,
                VfxPart::Body => !held,
            };
            if wanted
                && self
                    .world
                    .get::<MeshMaterial3d<WorldMaterial>>(at)
                    .is_some()
                && self.world.get::<Aabb>(at).is_some()
            {
                found.push(at);
            }
            if let Some(children) = self.world.get::<Children>(at) {
                pending.extend(children.iter().map(|child| (child, held)));
            }
        }
        found
    }

    /// Where the part of the thing the vfx plays on is: the bounds of its meshes as they were
    /// last drawn, turned as the thing is; a node's round; or, for a thing drawn without meshes
    /// of its own, the box its extent gives.
    pub fn part(&self) -> Option<PartFrame> {
        let thing = self.scene.thing?;
        let frame = self
            .world
            .get::<GlobalTransform>(thing.entity)
            .map(GlobalTransform::compute_transform)
            .unwrap_or(Transform::from_translation(thing.feet));
        if let VfxPart::Node { node, radius } = &self.instance.part {
            let center = named_node(self.world, thing.entity, node)
                .map(|node| posed(self.world, node).translation())
                .or_else(|| self.hand(thing.entity))?;
            return Some(PartFrame {
                center,
                rotation: frame.rotation,
                size: Vec3::splat(radius.0 * 2.0),
                mask: center.extend(radius.0),
            });
        }
        let into = frame.to_matrix().inverse();
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for mesh in self.part_meshes() {
            let (Some(aabb), Some(global)) = (
                self.world.get::<Aabb>(mesh),
                self.world.get::<GlobalTransform>(mesh),
            ) else {
                continue;
            };
            let (c, h) = (Vec3::from(aabb.center), Vec3::from(aabb.half_extents));
            for k in 0..8 {
                let corner = c + h * Vec3::new(
                    if k & 1 == 0 { -1.0 } else { 1.0 },
                    if k & 2 == 0 { -1.0 } else { 1.0 },
                    if k & 4 == 0 { -1.0 } else { 1.0 },
                );
                let p = into.transform_point3(global.transform_point(corner));
                lo = lo.min(p);
                hi = hi.max(p);
            }
        }
        if lo.x > hi.x {
            let extent = self.world.get::<SiteExtent>(thing.entity)?;
            let half = extent.footprint / 2.0;
            let scale = frame.scale.max(Vec3::splat(1e-4));
            lo = Vec3::new(-half.x, 0.0, -half.y) / scale;
            hi = Vec3::new(half.x, extent.height, half.y) / scale;
        }
        Some(PartFrame {
            center: frame.transform_point((lo + hi) / 2.0),
            rotation: frame.rotation,
            size: (hi - lo) * frame.scale,
            mask: Vec4::ZERO,
        })
    }

    /// Where a layer anchored at `anchor` stands before its offset.
    pub fn anchor(&self, anchor: VfxAnchor) -> Vec3 {
        let scene = self.scene;
        let fallback = || self.anchor(VfxAnchor::Center);
        match anchor {
            VfxAnchor::Hand => {
                if let Some(at) = self.instance.dealer.and_then(|dealer| self.hand(dealer)) {
                    return at;
                }
                if let Some(first) = scene.path.first() {
                    return *first;
                }
                return fallback();
            }
            VfxAnchor::Socket => return self.socket().unwrap_or_else(fallback),
            VfxAnchor::Part => {
                if let Some(part) = self.part() {
                    return part.center;
                }
            }
            VfxAnchor::Top => {
                if let Some(whole) = self.whole() {
                    return whole.center + Vec3::Y * whole.size.y / 2.0;
                }
            }
            VfxAnchor::Feet | VfxAnchor::Center | VfxAnchor::Head => {}
        }
        if let Some(thing) = scene.thing {
            return thing.feet
                + Vec3::Y
                    * thing.height
                    * match anchor {
                        VfxAnchor::Feet => 0.0,
                        VfxAnchor::Center => 0.5,
                        _ => 1.0,
                    };
        }
        if let Some(area) = &scene.area {
            let rise = match anchor {
                VfxAnchor::Feet | VfxAnchor::Part => 0.0,
                VfxAnchor::Center => area.across.min(area.along) / 2.0,
                _ => area.across.min(area.along),
            };
            return self.area_point() + Vec3::Y * rise;
        }
        scene.path.first().copied().unwrap_or_default()
    }

    /// Where a layer stands and how it turns, `life` through its run, turned by `spun` before its
    /// own turn: at its anchor, its offset turned with the heading, or along the path as it
    /// travels it.
    pub fn place(&self, place: &LayerPlace, life: f32, spun: Quat) -> (Vec3, Quat) {
        let roll = place.roll.0 + self.number(place.roll_by.as_deref());
        let heading = self.scene.heading * Quat::from_rotation_z(roll.to_radians());
        let turn = spun
            * Quat::from_euler(
                EulerRot::YXZ,
                place.turn[1].0.to_radians(),
                place.turn[0].0.to_radians(),
                place.turn[2].0.to_radians(),
            );
        let offset = Vec3::from_array(place.offset.map(|metres| metres.0));
        if let (Some(travel), [from, .., to]) = (place.travel, self.scene.path.as_slice()) {
            let (a, b) = (*from, *to);
            let at = |t: f32| {
                let eased = travel.ease.apply(t);
                a.lerp(b, eased) + Vec3::Y * travel.arc.0 * 4.0 * eased * (1.0 - eased)
            };
            let ahead =
                (at((life + 0.02).min(1.0)) - at((life - 0.02).max(0.0))).normalize_or(b - a);
            let yaw = ahead.x.atan2(ahead.z);
            let pitch = -ahead.y.clamp(-1.0, 1.0).asin();
            let way = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
            return (at(life) + way * offset, way * turn);
        }
        (self.anchor(place.anchor) + heading * offset, heading * turn)
    }

    /// The whole of the thing the vfx plays on, whichever part it plays on.
    fn whole(&self) -> Option<PartFrame> {
        let whole = VfxInstance {
            part: VfxPart::Whole,
            ..self.instance.clone()
        };
        Placing {
            instance: &whole,
            ..*self
        }
        .part()
    }

    /// Where the area's own point lies on the ground: a circle's or a rectangle's middle, a cone's
    /// tip, a line's start.
    pub fn area_point(&self) -> Vec3 {
        let Some(area) = &self.scene.area else {
            return Vec3::ZERO;
        };
        match area.shape {
            Shape::Line { length, .. } => area.middle - area.turn * Vec3::Z * length.0 / 2.0,
            _ => area.middle,
        }
    }

    /// The value of the row's number `name` as the instance fills it, 0 for none.
    fn number(&self, name: Option<&str>) -> f32 {
        match name.and_then(|name| self.def.param(name, &self.instance.params)) {
            Some(ParamValue::Number(number)) => number,
            _ => 0.0,
        }
    }
}

/// A thing as it stands: where its transform stands it, or, held by another, where its holder
/// had it as the last frame was drawn, and how tall its extent says it is.
fn thing(world: &World, entity: Entity) -> Option<Thing> {
    let (feet, turn) = match world.get::<ChildOf>(entity) {
        Some(_) => {
            let placed = world.get::<GlobalTransform>(entity)?;
            (placed.translation(), placed.rotation())
        }
        None => {
            let placed = world.get::<Transform>(entity)?;
            (placed.translation, placed.rotation)
        }
    };
    let height = world.get::<SiteExtent>(entity)?.height;
    Some(Thing {
        entity,
        feet,
        turn,
        height,
    })
}

/// Where a stop of a path stands: the first actor's hand, where the path leaves from, the middle
/// of every other actor, or a point on the ground.
fn stop_point(world: &World, stop: &PathStop, first: bool) -> Option<Vec3> {
    match stop {
        PathStop::Actor(actor) if first => hand(world, *actor),
        PathStop::Actor(actor) => middle(world, *actor),
        PathStop::Point(at) => Some(world_point(*at, ground_height(world, *at))),
    }
}

fn hand(world: &World, actor: Entity) -> Option<Vec3> {
    match world.get::<VfxHand>(actor) {
        Some(hand) => Some(posed(world, hand.0).translation()),
        None => middle(world, actor),
    }
}

fn middle(world: &World, actor: Entity) -> Option<Vec3> {
    let thing = thing(world, actor)?;
    Some(thing.feet + Vec3::Y * thing.height / 2.0)
}

/// The node named `name` among those of `thing` and of whatever hangs from it.
fn named_node(world: &World, thing: Entity, name: &str) -> Option<Entity> {
    let mut pending: Vec<Entity> = world.get::<Children>(thing)?.iter().collect();
    while let Some(node) = pending.pop() {
        if world
            .get::<Name>(node)
            .is_some_and(|named| named.as_str() == name)
        {
            return Some(node);
        }
        if let Some(children) = world.get::<Children>(node) {
            pending.extend(children.iter());
        }
    }
    None
}

/// Where `node` stands in the world as its own transform and those of everything that holds it
/// place it now, not as the transforms propagated last frame left it, which a moving body has
/// already left behind.
fn posed(world: &World, node: Entity) -> GlobalTransform {
    let mut at = GlobalTransform::from(world.get::<Transform>(node).copied().unwrap_or_default());
    let mut holder = world.get::<ChildOf>(node).map(ChildOf::parent);
    while let Some(held_by) = holder {
        let local = world.get::<Transform>(held_by).copied().unwrap_or_default();
        at = GlobalTransform::from(local) * at;
        holder = world.get::<ChildOf>(held_by).map(ChildOf::parent);
    }
    at
}
