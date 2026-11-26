//! What a vfx row's layers are: shapes drawn with an effect shader, particles, coats over a thing's
//! surfaces, shaders on the ground, tints of a thing's own surface, lights and camera shake. Every
//! one stands in the world in three dimensions: none is a picture facing the camera.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{ParamFill, ParamKind, SiteKind, VfxDef, VfxShaderId};
use crate::core::content::{ModelRef, RowCheck, SchemaMark, serial};
use crate::core::math::Rgba;
use crate::core::units::{Degrees, DegreesPerSec, Hertz, Meters, MetersPerSec, Seconds};

#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum VfxLayer {
    /// A shape drawn in the world with an effect shader.
    Mesh(MeshLayer),
    /// A model drawn as the world draws it: an arrow in flight, a thrown knife.
    Model(ModelLayer),
    /// Particles flung from an emitter, each drawn in a look of its own.
    Particles(ParticleLayer),
    /// An effect shader drawn over the surfaces of the part of the thing the vfx plays on, a hull
    /// standing just off them that moves with them; on an area, over its ground.
    Coat(CoatLayer),
    /// An effect shader laid on the ground under the site.
    Ground(GroundLayer),
    /// A tint, a rim of light and a glow laid over the thing's own lit surface.
    Surface(SurfaceLayer),
    /// A light cast on everything round it.
    Light(LightLayer),
    /// Shakes the camera of whoever sees it, less the farther they stand.
    Shake(ShakeLayer),
}

/// When a layer runs within its vfx: `delay` seconds after the vfx starts, for `duration`.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerRun {
    #[serde(default, skip_serializing_if = "Seconds::is_zero")]
    pub delay: Seconds,
    #[schemars(extend("minimum" = super::SHORTEST_RUN))]
    pub duration: Seconds,
}

#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeshLayer {
    pub shape: MeshShape,
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub fit: MeshFit,
    pub shader: VfxShaderId,
    /// Keyed by the shader's declared parameters, each filled or left to its default.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(transform = SchemaMark::ParamsOf.with("shader"))]
    pub params: BTreeMap<String, ParamFill>,
    pub blend: VfxBlend,
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub place: LayerPlace,
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub motion: LayerMotion,
    pub run: LayerRun,
}

#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelLayer {
    pub model: ModelRef,
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub place: LayerPlace,
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub motion: LayerMotion,
    pub run: LayerRun,
}

/// How a shape or a model grows, moves and turns over its run, from where it is placed.
#[derive(JsonSchema, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerMotion {
    /// How many times its own measures it is drawn over the run, across its own x, y and z.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<VfxTrack<[f32; 3]>>,
    /// How far it moves from where it stands over the run, in its own frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drift: Option<VfxTrack<[Meters; 3]>>,
    /// Pitch, yaw and roll added to its turn over the run: a jaw snapping shut.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swing: Option<VfxTrack<[Degrees; 3]>>,
    /// How fast it turns round its own x, y and z, before its turn.
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub spin: [DegreesPerSec; 3],
}

/// A shape a mesh layer draws, at a size of one metre unless its measures say otherwise.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum MeshShape {
    /// A flat band round the y axis, centred ahead: a ring, a disc, an arc.
    Band {
        angle: Degrees,
        inner: Meters,
        outer: Meters,
    },
    /// An open tube up the y axis: a cylinder, a cone, a twister, a curved wall.
    Tube {
        angle: Degrees,
        bottom: Meters,
        top: Meters,
        height: Meters,
        #[serde(default, skip_serializing_if = "serial::is_default")]
        twist: Degrees,
        /// Flat sides round it, as a prism; round when none.
        #[serde(default, skip_serializing_if = "serial::is_default")]
        sides: u32,
    },
    /// A sphere, or the part of one between two latitudes and across an angle round it.
    Sphere {
        angle: Degrees,
        from: Degrees,
        to: Degrees,
    },
    /// A tube along the site's path, jagged by `jitter` and drawn anew `rate` times a second,
    /// with `branches` forks off each stretch.
    PathTube {
        radius: Meters,
        jitter: Meters,
        rate: Hertz,
        #[serde(default, skip_serializing_if = "serial::is_default")]
        branches: f32,
    },
    /// An upper jaw: a ridge bent in a U of `angle`, `radius` round, set with `teeth` pointing
    /// down, its fangs the longest. The U opens toward its hinge at the origin; the mouth lies
    /// ahead.
    Jaw {
        radius: Meters,
        angle: Degrees,
        teeth: u32,
    },
    /// A starburst: `spikes` points flung out every way from a core `inner` round, each reaching
    /// up to `outer`.
    Burst {
        spikes: u32,
        inner: Meters,
        outer: Meters,
    },
}

/// What a mesh layer's shape takes from its site.
#[derive(JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeshFit {
    /// Drawn at its own measures.
    #[default]
    None,
    /// Scaled across to an area: a circle's radius, a cone's length.
    Area,
    /// A band takes an area cone's angle and length.
    Cone,
    /// Wrapped round the part of the thing it plays on: the shape's unit extent spans the part's
    /// bounds, standing on their floor.
    Wrap,
}

#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParticleLayer {
    pub emit: ParticleEmit,
    pub from: ParticleSource,
    pub velocity: ParticleVelocity,
    /// How fast each particle falls; a negative pull rises, as smoke and flame do.
    #[serde(default, skip_serializing_if = "serial::is_zero_f32")]
    pub gravity: f32,
    /// How much of its speed each particle loses a second.
    #[serde(default, skip_serializing_if = "serial::is_zero_f32")]
    #[schemars(extend("minimum" = 0.0))]
    pub drag: f32,
    /// How long each particle lives, least and most.
    #[schemars(extend("default" = [0.4, 0.8]))]
    pub life: [Seconds; 2],
    /// How big each particle is across over its life, from 0 at its birth to 1 at its end.
    #[schemars(extend("default" = [[0.0, 0.1], [1.0, 0.0]]))]
    pub size: Vec<(f32, Meters)>,
    /// How much bigger or smaller one particle is drawn than another, least and most.
    #[serde(
        default = "ParticleLayer::even",
        skip_serializing_if = "ParticleLayer::is_even"
    )]
    pub size_spread: [f32; 2],
    /// Its colour over its life.
    #[schemars(extend("default" = [
        [0.0, { "color": [255, 255, 255, 255] }],
        [1.0, { "color": [255, 255, 255, 255], "alpha": 0.0 }],
    ]))]
    pub color: Vec<(f32, VfxColor)>,
    pub look: ParticleLook,
    pub blend: VfxBlend,
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub place: LayerPlace,
    pub run: LayerRun,
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ParticleEmit {
    /// All at once as the layer starts.
    Burst {
        #[schemars(extend("default" = 24))]
        count: u32,
    },
    /// Steadily for the layer's run.
    Rate {
        #[schemars(extend("default" = 30.0))]
        per_second: f32,
    },
}

/// Where particles are born.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ParticleSource {
    Point,
    Sphere {
        radius: Meters,
    },
    /// A flat disc, or a ring when only its rim emits.
    Disc {
        radius: Meters,
        #[serde(default, skip_serializing_if = "serial::is_false")]
        rim: bool,
    },
    /// Round the thing the vfx plays on: a cylinder this wide and tall.
    Body {
        radius: Meters,
        height: Meters,
    },
    /// Across the area the vfx plays on.
    Area,
    /// Over the surface of the part of the thing the vfx plays on, wherever it is and however it
    /// moves; across an area's ground.
    Part,
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ParticleVelocity {
    /// Away from the emitter's centre, and up.
    Out {
        speed: [MetersPerSec; 2],
        #[serde(default, skip_serializing_if = "serial::is_default")]
        up: MetersPerSec,
    },
    /// Within `spread` of a way in the layer's frame.
    Cone {
        way: [f32; 3],
        spread: Degrees,
        speed: [MetersPerSec; 2],
    },
    /// Round the y axis, in toward it, and up.
    Swirl {
        speed: MetersPerSec,
        #[serde(default, skip_serializing_if = "serial::is_default")]
        up: MetersPerSec,
        #[serde(default, skip_serializing_if = "serial::is_default")]
        inward: MetersPerSec,
    },
    /// Straight off the surface where each particle is born, and up: from a part emitter, along
    /// the part's surface there; from any other, away from the emitter's centre.
    Normal {
        speed: [MetersPerSec; 2],
        #[serde(default, skip_serializing_if = "serial::is_default")]
        up: MetersPerSec,
    },
}

/// How a particle is drawn.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ParticleLook {
    /// A soft round point of light.
    Glow,
    /// A four-pointed glint that flares and turns.
    Twinkle,
    /// A spark drawn long along its flight, the longer the faster it flies.
    Streak { stretch: f32 },
    /// A rounded ball lit in two tones: dust, steam.
    Puff,
    /// A sharp shard that tumbles: stone, glass, ice.
    Shard,
    /// A thorn laid along its flight.
    Spike,
    /// A model's mesh, laid along its flight.
    Model { model: ModelRef },
    /// A glyph of a few strokes, different for each particle.
    Rune,
    /// A tongue of flame, ragged and swaying as it rises.
    Flame,
    /// A billow of smoke, soft-edged and torn by noise that churns as it lives, lit from above.
    Smoke,
}

#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoatLayer {
    pub shader: VfxShaderId,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(transform = SchemaMark::ParamsOf.with("shader"))]
    pub params: BTreeMap<String, ParamFill>,
    pub blend: VfxBlend,
    /// How far off the surface the coat stands.
    #[serde(default, skip_serializing_if = "serial::is_default")]
    #[schemars(extend("minimum" = 0.0))]
    pub inflate: Meters,
    pub run: LayerRun,
}

#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroundLayer {
    pub shader: VfxShaderId,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(transform = SchemaMark::ParamsOf.with("shader"))]
    pub params: BTreeMap<String, ParamFill>,
    pub blend: VfxBlend,
    /// The side of the square patch it is drawn on; an area's own bounds, this many times over,
    /// when it is fitted to the area.
    pub size: Meters,
    /// Drawn over the area it plays on, its size a share of the area's bounds.
    #[serde(default, skip_serializing_if = "serial::is_false")]
    pub fit: bool,
    /// How far ahead, up and to the right of the site the patch is centred.
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub offset: [Meters; 3],
    /// Left where it was laid rather than following the thing it plays on.
    #[serde(default, skip_serializing_if = "serial::is_false")]
    pub stays: bool,
    pub run: LayerRun,
}

#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceLayer {
    /// The colour laid over the surface, and how much of it, over the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tint: Option<VfxTrack<VfxColor>>,
    /// A glow round the silhouette, brightest where the surface turns away from the eye.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rim: Option<VfxTrack<VfxColor>>,
    /// Light added evenly over the whole surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glow: Option<VfxTrack<VfxColor>>,
    pub run: LayerRun,
}

#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightLayer {
    pub color: VfxColor,
    /// How bright it shines over the run.
    #[schemars(extend("default" = { "keys": [[0.0, 1.0], [1.0, 0.0]] }))]
    pub intensity: VfxTrack<f32>,
    #[schemars(extend("minimum" = super::SHORTEST_REACH, "maximum" = super::LONGEST_REACH))]
    pub radius: Meters,
    /// How much of its brightness comes and goes, as a flame's does.
    #[serde(default, skip_serializing_if = "serial::is_zero_f32")]
    #[schemars(extend("minimum" = 0.0, "maximum" = 1.0))]
    pub flicker: f32,
    /// How far from the light whatever stands in it starts casting a shadow away from it; left
    /// out, nothing casts one. What carries the light stands nearer and is spared, as the head of
    /// a torch is; the frame of a lantern stands farther, and throws its bars.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("minimum" = super::NEAREST_SHADOW))]
    pub shadows_from: Option<Meters>,
    /// How far out the light shines in from on every side, and from twice as far above, rather
    /// than from one point: an aura, lighting whatever stands in its midst. Left out, it shines
    /// from where it is placed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("minimum" = super::NARROWEST_AURA))]
    pub aura: Option<Meters>,
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub place: LayerPlace,
    pub run: LayerRun,
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShakeLayer {
    /// How far the camera is thrown at the start, easing to rest over the run.
    #[schemars(extend("minimum" = 0.0, "default" = 0.05))]
    pub strength: Meters,
    /// How far from the site it is felt, less and less until nothing at this distance.
    #[schemars(extend("exclusiveMinimum" = 0.0, "default" = 8.0))]
    pub reach: Meters,
    pub run: LayerRun,
}

/// Where a layer stands, in the frame of the vfx's heading: x to the right, y up, z ahead.
#[derive(JsonSchema, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerPlace {
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub anchor: VfxAnchor,
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub offset: [Meters; 3],
    /// Pitch, yaw and roll, after facing the heading.
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub turn: [Degrees; 3],
    /// How far the layer, its offset with it, is turned about the way it faces, before its turn:
    /// which way across a slash cuts.
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub roll: Degrees,
    /// One of the row's numbers, in degrees, added to `roll` as whoever plays the row fills it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roll_by: Option<String>,
    /// Left where it started rather than following the thing it plays on.
    #[serde(default, skip_serializing_if = "serial::is_false")]
    pub stays: bool,
    /// Flies along the site's path from its first point to its last over the layer's run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub travel: Option<Travel>,
}

/// Which point of its site a layer stands at.
#[derive(JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VfxAnchor {
    #[default]
    Feet,
    Center,
    Head,
    /// The hand of whoever dealt the vfx: where a cast leaves from.
    Hand,
    /// The vfx's socket on the thing it plays on.
    Socket,
    /// The middle of the part of the thing the vfx plays on.
    Part,
    /// Just over the top of the thing it plays on, however tall it stands.
    Top,
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Travel {
    /// How high the flight bows above the straight line at its middle.
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub arc: Meters,
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub ease: VfxEase,
}

/// How a value eases from one key to the next.
#[derive(JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VfxEase {
    #[default]
    Linear,
    /// Slow to start.
    In,
    /// Slow to stop.
    Out,
    InOut,
    /// Overshoots and settles back.
    Back,
}

/// A value over a layer's run: keys from 0 at its start to 1 at its end, eased between.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VfxTrack<T> {
    pub keys: Vec<(f32, T)>,
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub ease: VfxEase,
}

/// How a layer is laid over what is behind it.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VfxBlend {
    /// Adds light: glows, flashes, sparks.
    Add,
    /// Covers what is behind it: smoke, dust, solid shapes.
    Alpha,
}

/// A colour a layer draws in: a colour of its own or one of the row's colours as whoever plays the
/// row fills it, made `brightness` times as bright, above 1 glowing, and seen through `alpha`.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VfxColor {
    pub color: ColorFill,
    #[serde(
        default = "serial::one_f32",
        skip_serializing_if = "serial::is_one_f32"
    )]
    #[schemars(extend("minimum" = 0.0, "default" = serial::one_f32()))]
    pub brightness: f32,
    #[serde(
        default = "serial::one_f32",
        skip_serializing_if = "serial::is_one_f32"
    )]
    #[schemars(extend("minimum" = 0.0, "maximum" = 1.0, "default" = serial::one_f32()))]
    pub alpha: f32,
}

/// A colour of its own, or one of the row's colours by name.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ColorFill {
    Value(Rgba),
    Param { param: String },
}

impl ParticleLayer {
    fn even() -> [f32; 2] {
        [1.0, 1.0]
    }

    fn is_even(spread: &[f32; 2]) -> bool {
        *spread == ParticleLayer::even()
    }
}

impl VfxLayer {
    pub fn run(&self) -> LayerRun {
        match self {
            VfxLayer::Mesh(layer) => layer.run,
            VfxLayer::Model(layer) => layer.run,
            VfxLayer::Particles(layer) => layer.run,
            VfxLayer::Coat(layer) => layer.run,
            VfxLayer::Ground(layer) => layer.run,
            VfxLayer::Surface(layer) => layer.run,
            VfxLayer::Light(layer) => layer.run,
            VfxLayer::Shake(layer) => layer.run,
        }
    }

    /// The effect shader the layer draws with, and the fills of its parameters.
    pub fn shader(&self) -> Option<(VfxShaderId, &BTreeMap<String, ParamFill>)> {
        match self {
            VfxLayer::Mesh(MeshLayer { shader, params, .. })
            | VfxLayer::Coat(CoatLayer { shader, params, .. })
            | VfxLayer::Ground(GroundLayer { shader, params, .. }) => Some((*shader, params)),
            VfxLayer::Model(_)
            | VfxLayer::Particles(_)
            | VfxLayer::Surface(_)
            | VfxLayer::Light(_)
            | VfxLayer::Shake(_) => None,
        }
    }

    pub fn place(&self) -> Option<&LayerPlace> {
        match self {
            VfxLayer::Mesh(layer) => Some(&layer.place),
            VfxLayer::Model(layer) => Some(&layer.place),
            VfxLayer::Particles(layer) => Some(&layer.place),
            VfxLayer::Light(layer) => Some(&layer.place),
            VfxLayer::Coat(_) | VfxLayer::Ground(_) | VfxLayer::Surface(_) | VfxLayer::Shake(_) => {
                None
            }
        }
    }

    /// Every colour the layer draws in.
    pub fn colors(&self) -> Vec<&VfxColor> {
        fn track(track: &Option<VfxTrack<VfxColor>>) -> Vec<&VfxColor> {
            track
                .iter()
                .flat_map(|track| track.keys.iter().map(|(_, color)| color))
                .collect()
        }
        match self {
            VfxLayer::Particles(layer) => layer.color.iter().map(|(_, color)| color).collect(),
            VfxLayer::Surface(layer) => [&layer.tint, &layer.rim, &layer.glow]
                .into_iter()
                .flat_map(track)
                .collect(),
            VfxLayer::Light(layer) => vec![&layer.color],
            VfxLayer::Mesh(_)
            | VfxLayer::Model(_)
            | VfxLayer::Coat(_)
            | VfxLayer::Ground(_)
            | VfxLayer::Shake(_) => Vec::new(),
        }
    }
}

impl VfxLayer {
    /// How long what the layer flung lives on once its run is over: its particles' longest life.
    pub fn lingers(&self) -> Seconds {
        match self {
            VfxLayer::Particles(layer) => layer.life[1],
            _ => Seconds(0.0),
        }
    }
}

impl LayerRun {
    pub fn ends(&self) -> Seconds {
        self.delay + self.duration
    }
}

impl VfxLayer {
    /// What the layer is, as a sentence names it.
    pub fn label(&self) -> &'static str {
        match self {
            VfxLayer::Mesh(_) => "mesh",
            VfxLayer::Model(_) => "model",
            VfxLayer::Particles(_) => "particles",
            VfxLayer::Coat(_) => "coat",
            VfxLayer::Ground(_) => "ground",
            VfxLayer::Surface(_) => "surface",
            VfxLayer::Light(_) => "light",
            VfxLayer::Shake(_) => "shake",
        }
    }

    /// Whether the layer can play on a site of `kind`: a coat needs surfaces or ground, a surface
    /// a thing's own, a fitted shape or an area emitter an area, a travel or a path tube a path.
    pub fn plays_on(&self, kind: SiteKind) -> bool {
        let thing = matches!(kind, SiteKind::Actor | SiteKind::Object);
        let world = thing || matches!(kind, SiteKind::Area | SiteKind::Path);
        let placed = |place: &LayerPlace| place.travel.is_none() || kind == SiteKind::Path;
        match self {
            VfxLayer::Mesh(layer) => {
                let shape = match layer.shape {
                    MeshShape::PathTube { .. } => kind == SiteKind::Path,
                    _ => true,
                };
                let fit = match layer.fit {
                    MeshFit::None => true,
                    MeshFit::Area | MeshFit::Cone => kind == SiteKind::Area,
                    MeshFit::Wrap => thing,
                };
                world && shape && fit && placed(&layer.place)
            }
            VfxLayer::Model(layer) => world && placed(&layer.place),
            VfxLayer::Particles(layer) => {
                let source = match layer.from {
                    ParticleSource::Area => kind == SiteKind::Area,
                    ParticleSource::Part => thing || kind == SiteKind::Area,
                    ParticleSource::Body { .. } => thing,
                    _ => true,
                };
                world && source && placed(&layer.place)
            }
            VfxLayer::Coat(_) => thing || kind == SiteKind::Area,
            VfxLayer::Ground(layer) => world && (!layer.fit || kind == SiteKind::Area),
            VfxLayer::Surface(_) => thing,
            VfxLayer::Light(layer) => world && placed(&layer.place),
            VfxLayer::Shake(_) => world,
        }
    }

    /// Refuses, under `pointer`, what the layer holds that no site would mend: a run too short,
    /// tracks out of order, measures out of range, and parameters of the row it names that the
    /// row doesn't declare or declares as another kind.
    pub(super) fn check(&self, def: &VfxDef, pointer: &str, check: &mut RowCheck) {
        let at = |field: &str| format!("{pointer}/{field}");
        let run = self.run();
        check.at_least(at("run/delay"), run.delay.0, 0.0, "seconds");
        check.at_least(
            at("run/duration"),
            f64::from(run.duration.0),
            super::SHORTEST_RUN,
            "seconds",
        );
        for (index, color) in self.colors().into_iter().enumerate() {
            if let ColorFill::Param { param } = &color.color {
                check_param(
                    def,
                    check,
                    at(&format!("color/{index}")),
                    param,
                    ParamKind::Color,
                );
            }
        }
        if let Some((_, params)) = self.shader() {
            for (name, fill) in params {
                if let ParamFill::Param { param } = fill {
                    if def.declares(param).is_none() {
                        check.error(
                            at(&format!("params/{name}")),
                            "undeclared parameter",
                            param,
                            "a name the row declares under params",
                        );
                    }
                } else if let ParamFill::Color(VfxColor {
                    color: ColorFill::Param { param },
                    ..
                }) = fill
                {
                    check_param(
                        def,
                        check,
                        at(&format!("params/{name}")),
                        param,
                        ParamKind::Color,
                    );
                }
            }
        }
        if let Some(place) = self.place()
            && let Some(by) = &place.roll_by
        {
            check_param(def, check, at("place/roll_by"), by, ParamKind::Number);
        }
        match self {
            VfxLayer::Mesh(MeshLayer { motion, .. })
            | VfxLayer::Model(ModelLayer { motion, .. }) => {
                for (field, keys) in [
                    ("motion/size", motion.size.as_ref().map(VfxTrack::times)),
                    ("motion/drift", motion.drift.as_ref().map(VfxTrack::times)),
                    ("motion/swing", motion.swing.as_ref().map(VfxTrack::times)),
                ] {
                    if let Some(keys) = keys {
                        check_keys(check, at(field), &keys);
                    }
                }
            }
            VfxLayer::Particles(layer) => {
                match layer.emit {
                    ParticleEmit::Burst { count } => {
                        check.range(at("emit/count"), count, 1, MAX_BURST, "particles");
                    }
                    ParticleEmit::Rate { per_second } => {
                        let most = MAX_RATE;
                        check.range(at("emit/per_second"), per_second, 0.1, most, "a second");
                    }
                }
                let [least, most] = layer.life;
                check.range(at("life/0"), least.0, 0.01, most.0.max(0.01), "seconds");
                let [low, high] = layer.size_spread;
                check.range(at("size_spread/0"), low, 0.01, high.max(0.01), "times");
                let sizes: Vec<f32> = layer.size.iter().map(|(t, _)| *t).collect();
                check_keys(check, at("size"), &sizes);
                let colors: Vec<f32> = layer.color.iter().map(|(t, _)| *t).collect();
                check_keys(check, at("color"), &colors);
            }
            VfxLayer::Surface(layer) => {
                for (field, track) in [
                    ("tint", &layer.tint),
                    ("rim", &layer.rim),
                    ("glow", &layer.glow),
                ] {
                    if let Some(track) = track {
                        check_keys(check, at(field), &track.times());
                    }
                }
            }
            VfxLayer::Light(layer) => {
                if let Some(aura) = layer.aura {
                    let narrowest = super::NARROWEST_AURA as f32;
                    check.range(at("aura"), aura.0, narrowest, layer.radius.0, "m");
                }
                if let Some(from) = layer.shadows_from {
                    let nearest = super::NEAREST_SHADOW as f32;
                    check.range(at("shadows_from"), from.0, nearest, layer.radius.0, "m");
                }
                let (nearest, farthest) =
                    (super::SHORTEST_REACH as f32, super::LONGEST_REACH as f32);
                check.range(at("radius"), layer.radius.0, nearest, farthest, "m");
                check.range(at("flicker"), layer.flicker, 0.0, 1.0, "flicker");
                check_keys(check, at("intensity"), &layer.intensity.times());
            }
            VfxLayer::Shake(layer) => {
                check.range(
                    at("reach"),
                    layer.reach.0,
                    0.1,
                    super::LONGEST_REACH as f32,
                    "m",
                );
            }
            VfxLayer::Coat(_) | VfxLayer::Ground(_) => {}
        }
    }
}

impl<T> VfxTrack<T> {
    fn times(&self) -> Vec<f32> {
        self.keys.iter().map(|(t, _)| *t).collect()
    }
}

/// The most particles a burst may fling at once, and a rate may give a second.
const MAX_BURST: u32 = 4096;
const MAX_RATE: f32 = 4096.0;

/// Refuses keys that are none, or that leave the run or go back in it.
fn check_keys(check: &mut RowCheck, pointer: String, keys: &[f32]) {
    if keys.is_empty() {
        check.error(pointer, "no keys", "none", "at least one key");
        return;
    }
    for (index, pair) in keys.windows(2).enumerate() {
        if pair[1] < pair[0] {
            check.error(
                format!("{pointer}/keys/{}", index + 1),
                "keys out of order",
                pair[1],
                format!("at least {}", pair[0]),
            );
        }
    }
    for (index, key) in keys.iter().enumerate() {
        if !(0.0..=1.0).contains(key) {
            check.error(
                format!("{pointer}/keys/{index}"),
                "key out of the run",
                key,
                "0..1",
            );
        }
    }
}

fn check_param(def: &VfxDef, check: &mut RowCheck, pointer: String, name: &str, kind: ParamKind) {
    match def.declares(name) {
        None => check.error(
            pointer,
            "undeclared parameter",
            name,
            "a name the row declares under params",
        ),
        Some(declared) if declared != kind => check.error(
            pointer,
            "wrong parameter kind",
            declared.label(),
            format!("a {}", kind.label()),
        ),
        Some(_) => {}
    }
}
