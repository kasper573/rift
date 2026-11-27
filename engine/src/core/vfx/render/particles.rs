//! A particle layer as a hanabi effect: its emitter, flight and look compiled into GPU particles.
//! What a site sets apart (the measures of its area, the frame it is turned to, the points of the
//! surface it emits from) the effect reads as properties, so every play of a layer shares one
//! compiled effect.

use std::borrow::Cow;
use std::f32::consts::TAU;

use bevy::prelude::*;
use bevy_hanabi::prelude::*;
use bevy_hanabi::{
    Attribute, BoxedModifier, ExprError, Modifier, ModifierContext, Module, RenderContext,
    RenderModifier, ShaderWriter,
};
use serde::{Deserialize, Serialize};

use super::super::VfxBlend;
use super::super::{ParticleEmit, ParticleLayer, ParticleLook, ParticleSource, ParticleVelocity};

/// How many points of its part's surface a `part` emitter reads each frame.
pub const SURFACE_POINTS: usize = 32;

/// The property a `part` emitter reads its `k`th surface point from, relative to the emitter.
pub fn surface_point(k: usize) -> String {
    format!("surface_{k}")
}

/// The property holding the way the surface faces at a `part` emitter's `k`th point.
pub fn surface_normal(k: usize) -> String {
    format!("surface_normal_{k}")
}

/// The way a cone of velocities points in the world.
pub const WAY: &str = "way";
/// How fast the place a layer emits from moves through the world, in meters a second.
pub const MOTION: &str = "motion";
/// An area's measures: a circle's radius, a rectangle's width and depth, a cone's half angle in
/// radians and its length.
pub const AREA: &str = "area";
/// The ways right and ahead run in the world, for a cone of ground to emit across.
pub const RIGHT: &str = "right";
pub const AHEAD: &str = "ahead";

/// The body of the area a particle layer emits across, when it emits from its site's area.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AreaKind {
    Circle,
    Rect,
    Cone,
}

/// The effect a particle layer compiles to, emitting from an area of `area`'s kind when its site
/// is one, coloured as `colors` resolves its colours, in linear light.
pub fn effect(
    layer: &ParticleLayer,
    area: Option<AreaKind>,
    colors: &[(f32, Vec4)],
) -> EffectAsset {
    let writer = ExprWriter::new();
    let [least, most] = layer.life;
    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let init_life = SetAttributeModifier::new(
        Attribute::LIFETIME,
        writer.lit(least.0).uniform(writer.lit(most.0)).expr(),
    );
    let [low, high] = layer.size_spread;
    let init_size = SetAttributeModifier::new(
        Attribute::F32_0,
        writer.lit(low).uniform(writer.lit(high)).expr(),
    );
    let init_seed = SetAttributeModifier::new(
        Attribute::F32_1,
        (writer.rand(ScalarType::Float) * writer.lit(100.0)).expr(),
    );
    // Which of a part emitter's surface points the particle is born at, kept so its position and
    // its velocity read the same point.
    let init_pick = SetAttributeModifier::new(
        Attribute::F32_2,
        (writer.rand(ScalarType::Float) * writer.lit(SURFACE_POINTS as f32)).expr(),
    );
    let from = match (layer.from, area) {
        (ParticleSource::Part, Some(_)) => ParticleSource::Area,
        (from, _) => from,
    };
    let position = position(&writer, from, area);
    let velocity: BoxedModifier = match layer.velocity {
        ParticleVelocity::Normal { speed, up } => {
            let way = if from == ParticleSource::Part {
                surface_sum(&writer, surface_normal).normalized()
            } else {
                writer.attr(Attribute::POSITION).normalized()
            };
            let speed = writer.lit(speed[0].0.0).uniform(writer.lit(speed[1].0.0));
            let v = way * speed + writer.lit(Vec3::Y * up.0.0);
            Box::new(SetAttributeModifier::new(Attribute::VELOCITY, v.expr()))
        }
        ParticleVelocity::Out { speed, up } => {
            let way = writer.attr(Attribute::POSITION).normalized();
            let speed = writer.lit(speed[0].0.0).uniform(writer.lit(speed[1].0.0));
            let v = way * speed + writer.lit(Vec3::Y * up.0.0);
            Box::new(SetAttributeModifier::new(Attribute::VELOCITY, v.expr()))
        }
        ParticleVelocity::Cone { spread, speed, .. } => {
            let way = writer.prop(writer.add_property(WAY, Vec3::Y.into()));
            let jitter = (writer.rand(VectorType::VEC3F) * writer.lit(2.0) - writer.lit(1.0))
                * writer.lit((spread.radians().0 / 2.0).tan());
            let way = (way + jitter).normalized();
            let speed = writer.lit(speed[0].0.0).uniform(writer.lit(speed[1].0.0));
            Box::new(SetAttributeModifier::new(
                Attribute::VELOCITY,
                (way * speed).expr(),
            ))
        }
        ParticleVelocity::Swirl { speed, up, inward } => {
            let p = writer.attr(Attribute::POSITION);
            let flat = p * writer.lit(Vec3::new(1.0, 0.0, 1.0));
            let tangent = writer.lit(Vec3::Y).cross(flat.clone()).normalized();
            let v = tangent * writer.lit(speed.0.0) - flat.normalized() * writer.lit(inward.0.0)
                + writer.lit(Vec3::Y * up.0.0);
            Box::new(SetAttributeModifier::new(Attribute::VELOCITY, v.expr()))
        }
    };
    let accel = AccelModifier::new(writer.lit(Vec3::Y * -layer.gravity).expr());
    let air = (layer.carried > 0.0).then(|| {
        writer.prop(writer.add_property(MOTION, Vec3::ZERO.into())) * writer.lit(layer.carried)
    });
    let born_in_air = air.clone().map(|air| {
        SetAttributeModifier::new(
            Attribute::VELOCITY,
            (writer.attr(Attribute::VELOCITY) + air).expr(),
        )
    });
    let drag: BoxedModifier = match air {
        Some(air) => {
            let kept = (writer.lit(1.0) - writer.lit(layer.drag) * writer.delta_time())
                .max(writer.lit(0.0));
            let against = writer.attr(Attribute::VELOCITY) - air.clone();
            Box::new(SetAttributeModifier::new(
                Attribute::VELOCITY,
                (air + against * kept).expr(),
            ))
        }
        None => Box::new(LinearDragModifier::new(writer.lit(layer.drag).expr())),
    };
    let mut color = bevy_hanabi::Gradient::new();
    for (t, c) in colors {
        color.add_key(*t, *c);
    }
    let mut size = bevy_hanabi::Gradient::new();
    for (t, s) in &layer.size {
        size.add_key(*t, Vec3::splat(s.0));
    }
    let run = layer.run.duration.0;
    let spawner = match layer.emit {
        ParticleEmit::Burst { count } => SpawnerSettings::once((count as f32).into()),
        ParticleEmit::Rate { per_second } => {
            SpawnerSettings::new((per_second * run).into(), run.into(), run.into(), 1)
        }
    };
    let capacity = match layer.emit {
        ParticleEmit::Burst { count } => count,
        ParticleEmit::Rate { per_second } => (per_second * most.0 * 1.3) as u32 + 8,
    }
    .max(16);
    let orient = match layer.look {
        ParticleLook::Streak { .. } | ParticleLook::Model { .. } | ParticleLook::Spike => {
            OrientModifier::new(OrientMode::AlongVelocity)
        }
        _ => OrientModifier::new(OrientMode::FaceCameraPosition),
    };
    let module = writer.finish();
    let effect = EffectAsset::new(capacity, spawner, module)
        .with_simulation_space(SimulationSpace::Global)
        .with_alpha_mode(match layer.blend {
            VfxBlend::Add => bevy_hanabi::AlphaMode::Add,
            VfxBlend::Alpha => bevy_hanabi::AlphaMode::Blend,
        })
        .init(init_pick)
        .add_modifier(ModifierContext::Init, position)
        .add_modifier(ModifierContext::Init, velocity)
        .init(init_age)
        .init(init_life)
        .init(init_size)
        .init(init_seed)
        .update(accel)
        .add_modifier(ModifierContext::Update, drag)
        .render(ColorOverLifetimeModifier {
            gradient: color,
            blend: ColorBlendMode::Overwrite,
            mask: ColorBlendMask::RGBA,
        })
        .render(SizeOverLifetimeModifier {
            gradient: size,
            screen_space_size: false,
        })
        .render(orient)
        .render(LookModifier::from(&layer.look));
    match born_in_air {
        Some(born) => effect.init(born),
        None => effect,
    }
}

/// Where a particle is born, relative to its emitter.
fn position(writer: &ExprWriter, from: ParticleSource, area: Option<AreaKind>) -> BoxedModifier {
    let random = || writer.rand(ScalarType::Float);
    match (from, area) {
        (ParticleSource::Point, _) => Box::new(SetAttributeModifier::new(
            Attribute::POSITION,
            writer.lit(Vec3::ZERO).expr(),
        )),
        (ParticleSource::Sphere { radius }, _) => Box::new(SetPositionSphereModifier {
            center: writer.lit(Vec3::ZERO).expr(),
            radius: writer.lit(radius.0).expr(),
            dimension: ShapeDimension::Volume,
        }),
        (ParticleSource::Disc { radius, rim }, _) => Box::new(SetPositionCircleModifier {
            center: writer.lit(Vec3::ZERO).expr(),
            axis: writer.lit(Vec3::Y).expr(),
            radius: writer.lit(radius.0).expr(),
            dimension: if rim {
                ShapeDimension::Surface
            } else {
                ShapeDimension::Volume
            },
        }),
        (ParticleSource::Body { radius, height }, _) => {
            let a = random() * writer.lit(TAU);
            let r = random().sqrt() * writer.lit(radius.0);
            let y = random() * writer.lit(height.0);
            let x = a.clone().cos() * r.clone();
            let z = a.sin() * r;
            Box::new(SetAttributeModifier::new(
                Attribute::POSITION,
                x.vec3(y, z).expr(),
            ))
        }
        (ParticleSource::Area | ParticleSource::Part, Some(kind)) => {
            let area = writer.prop(writer.add_property(AREA, Vec4::ZERO.into()));
            match kind {
                AreaKind::Circle => Box::new(SetPositionCircleModifier {
                    center: writer.lit(Vec3::ZERO).expr(),
                    axis: writer.lit(Vec3::Y).expr(),
                    radius: area.x().expr(),
                    dimension: ShapeDimension::Volume,
                }),
                AreaKind::Rect => {
                    let x = (random() - writer.lit(0.5)) * area.clone().x();
                    let z = (random() - writer.lit(0.5)) * area.y();
                    let p = writer.prop(writer.add_property(RIGHT, Vec3::X.into())) * x
                        + writer.prop(writer.add_property(AHEAD, Vec3::Z.into())) * z;
                    Box::new(SetAttributeModifier::new(Attribute::POSITION, p.expr()))
                }
                AreaKind::Cone => {
                    let a = (random() * writer.lit(2.0) - writer.lit(1.0)) * area.clone().x();
                    let r = random().sqrt() * area.y();
                    let across = a.clone().sin() * r.clone();
                    let along = a.cos() * r;
                    let p = writer.prop(writer.add_property(RIGHT, Vec3::X.into())) * across
                        + writer.prop(writer.add_property(AHEAD, Vec3::Z.into())) * along;
                    Box::new(SetAttributeModifier::new(Attribute::POSITION, p.expr()))
                }
            }
        }
        // A site with no area has none to emit across: what plays there is born at its point.
        (ParticleSource::Area, None) => Box::new(SetAttributeModifier::new(
            Attribute::POSITION,
            writer.lit(Vec3::ZERO).expr(),
        )),
        (ParticleSource::Part, None) => {
            let p = surface_sum(writer, surface_point);
            let jitter =
                (writer.rand(VectorType::VEC3F) - writer.lit(0.5)) * writer.lit(PART_JITTER);
            Box::new(SetAttributeModifier::new(
                Attribute::POSITION,
                (p + jitter).expr(),
            ))
        }
    }
}

/// How far round its surface point a particle of a part emitter may be born, in metres.
const PART_JITTER: f32 = 0.02;

/// How a particle is drawn: shaped in the fragment, stretched or tumbled in the vertex.
#[derive(Clone, Copy, Debug, Reflect, Serialize, Deserialize)]
pub struct LookModifier {
    look: LookKind,
    stretch: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Reflect, Serialize, Deserialize)]
enum LookKind {
    Glow,
    Streak,
    Puff,
    Shard,
    Solid,
    Twinkle,
    Rune,
    Flame,
    Smoke,
}

impl From<&ParticleLook> for LookModifier {
    fn from(look: &ParticleLook) -> LookModifier {
        let (look, stretch) = match look {
            ParticleLook::Glow => (LookKind::Glow, 0.0),
            ParticleLook::Twinkle => (LookKind::Twinkle, 0.0),
            ParticleLook::Streak { stretch } => (LookKind::Streak, *stretch),
            ParticleLook::Puff => (LookKind::Puff, 0.0),
            ParticleLook::Shard => (LookKind::Shard, 0.0),
            ParticleLook::Spike | ParticleLook::Model { .. } => (LookKind::Solid, 0.0),
            ParticleLook::Rune => (LookKind::Rune, 0.0),
            ParticleLook::Flame => (LookKind::Flame, 0.0),
            ParticleLook::Smoke => (LookKind::Smoke, 0.0),
        };
        LookModifier { look, stretch }
    }
}

impl Modifier for LookModifier {
    fn context(&self) -> ModifierContext {
        ModifierContext::Render
    }

    fn as_render(&self) -> Option<&dyn RenderModifier> {
        Some(self)
    }

    fn as_render_mut(&mut self) -> Option<&mut dyn RenderModifier> {
        Some(self)
    }

    fn attributes(&self) -> Cow<'_, [Attribute]> {
        Cow::Borrowed(&[
            Attribute::F32_0,
            Attribute::F32_1,
            Attribute::VELOCITY,
            Attribute::AGE,
        ])
    }

    fn boxed_clone(&self) -> BoxedModifier {
        Box::new(*self)
    }

    fn apply(&self, _: &mut Module, _: &mut ShaderWriter) -> Result<(), ExprError> {
        Err(ExprError::TypeError("Wrong modifier context".to_string()))
    }
}

impl RenderModifier for LookModifier {
    fn apply_render(&self, _: &mut Module, context: &mut RenderContext) -> Result<(), ExprError> {
        context.vertex_code += "size = size * particle.f32_0;\n";
        match self.look {
            LookKind::Glow => {
                context.set_needs_uv();
                context.fragment_code += GLOW;
            }
            LookKind::Streak => {
                context.set_needs_uv();
                context.vertex_code += &format!(
                    "size.x = size.x + length(particle.velocity) * {:.5}; size.y = size.y * 0.35;\n",
                    self.stretch
                );
                context.fragment_code += STREAK;
            }
            LookKind::Puff => {
                context.set_needs_normal();
                context.fragment_code += PUFF;
            }
            LookKind::Shard => {
                context.set_needs_normal();
                context.vertex_code += SHARD_TUMBLE;
                context.fragment_code += SHARD;
            }
            LookKind::Twinkle => {
                context.set_needs_uv();
                context.needs_particle_fragment = true;
                context.fragment_code += TWINKLE;
            }
            LookKind::Rune => {
                context.set_needs_uv();
                context.needs_particle_fragment = true;
                context.fragment_code += RUNE;
            }
            LookKind::Flame => {
                context.set_needs_uv();
                context.needs_particle_fragment = true;
                context.vertex_code +=
                    "size.y = size.y * (1.4 + length(particle.velocity) * 0.8);\n";
                context.fragment_code += FLAME;
            }
            LookKind::Smoke => {
                context.set_needs_uv();
                context.needs_particle_fragment = true;
                context.fragment_code += SMOKE;
            }
            LookKind::Solid => {
                context.set_needs_normal();
                context.fragment_code += SOLID;
            }
        }
        Ok(())
    }

    fn boxed_render_clone(&self) -> Box<dyn RenderModifier> {
        Box::new(*self)
    }

    fn as_modifier(&self) -> &dyn Modifier {
        self
    }
}

/// The value of the per-point properties `name` at the particle's picked surface point.
fn surface_sum(writer: &ExprWriter, name: fn(usize) -> String) -> WriterExpr {
    let pick = writer.attr(Attribute::F32_2);
    let mut sum = writer.lit(Vec3::ZERO);
    for k in 0..SURFACE_POINTS {
        let value = writer.prop(writer.add_property(name(k), Vec3::ZERO.into()));
        let here = (pick.clone() - writer.lit(k as f32 + 0.5)).abs();
        let weight = writer.lit(1.0) - (here * writer.lit(2.0)).min(writer.lit(1.0)).floor();
        sum = sum + value * weight;
    }
    sum
}

// Each look shapes the particle in the fragment hanabi draws it with, from its quad's uv or its
// mesh's normal, its colour and its own seed and age.
const GLOW: &str = "{ let d = length(uv * 2.0 - 1.0); let g = clamp(1.0 - d, 0.0, 1.0); \
    color = vec4<f32>(color.rgb * (1.0 + 1.5 * g * g * g), color.a * g * g); }\n";

const STREAK: &str = "{ let across = abs(uv.y * 2.0 - 1.0); let along = uv.x; \
    let core = clamp(1.0 - across, 0.0, 1.0); let tail = smoothstep(0.0, 0.7, along) * (1.0 - smoothstep(0.9, 1.0, along)); \
    color = vec4<f32>(color.rgb * (1.0 + 2.0 * core * core), color.a * core * tail); }\n";

const PUFF: &str = "{ let sun = normalize(vec3<f32>(-0.45, 0.8, 0.4)); \
    let n = normalize(normal); let back = normalize(view.world_from_view[2].xyz); \
    let lit = smoothstep(-0.15, 0.25, dot(n, sun)); let rim = pow(1.0 - clamp(dot(n, back), 0.0, 1.0), 3.0); \
    let facing = clamp(dot(n, back), 0.0, 1.0); \
    let shade = (mix(0.6, 1.0, lit) + rim * 0.15) * mix(0.62, 1.12, pow(facing, 0.6)); \
    color = vec4<f32>(color.rgb * shade, color.a * smoothstep(0.0, 0.35, dot(n, back) + 0.15)); }\n";

const SHARD_TUMBLE: &str = "{ let s = particle.f32_1; let ang = s + particle.age * (4.0 + fract(s * 1.7) * 6.0); \
    let ax = normalize(vec3<f32>(sin(s * 7.1), cos(s * 3.3), sin(s * 5.9)) + vec3<f32>(0.01)); \
    let c = cos(ang); let si = sin(ang); \
    axis_x = vec3<f32>(1.0, 0.0, 0.0) * c + cross(ax, vec3<f32>(1.0, 0.0, 0.0)) * si + ax * dot(ax, vec3<f32>(1.0, 0.0, 0.0)) * (1.0 - c); \
    axis_y = vec3<f32>(0.0, 1.0, 0.0) * c + cross(ax, vec3<f32>(0.0, 1.0, 0.0)) * si + ax * dot(ax, vec3<f32>(0.0, 1.0, 0.0)) * (1.0 - c); \
    axis_z = cross(axis_x, axis_y); }\n";

const SHARD: &str = "{ let sun = normalize(vec3<f32>(-0.45, 0.8, 0.4)); let n = normalize(normal); \
    let back = normalize(view.world_from_view[2].xyz); \
    let lit = 0.5 + 0.5 * dot(n, sun); let rim = pow(1.0 - abs(dot(n, back)), 2.0); \
    color = vec4<f32>(color.rgb * (0.45 + 0.75 * lit) + color.rgb * rim * 0.6, color.a); }\n";

const TWINKLE: &str = "{ let q = uv * 2.0 - 1.0; let s = particle.f32_1; let a = s + particle.age * 3.0; \
    let r = vec2<f32>(q.x * cos(a) - q.y * sin(a), q.x * sin(a) + q.y * cos(a)); \
    let star = max(1.0 - abs(r.x) * 12.0 - abs(r.y) * 1.1, 0.0) + max(1.0 - abs(r.y) * 12.0 - abs(r.x) * 1.1, 0.0); \
    let core = clamp(1.0 - length(q) * 2.2, 0.0, 1.0); let g = clamp(star + core * core, 0.0, 1.0); \
    let blink = 0.55 + 0.45 * sin(particle.age * 14.0 + s * 3.0); \
    color = vec4<f32>(color.rgb * (1.0 + 2.0 * core), color.a * g * blink); }\n";

const RUNE: &str = "{ let q = uv * 2.0 - 1.0; let s = particle.f32_1; var ink = 0.0; \
    for (var k = 0u; k < 4u; k = k + 1u) { \
        let h = fract(sin(vec4<f32>(1.0, 2.0, 3.0, 4.0) * (s * 13.7 + f32(k) * 7.3)) * 43758.5453); \
        let a = (floor(h.xy * 3.0) - 1.0) * 0.62; let b = (floor(h.zw * 3.0) - 1.0) * 0.62; \
        let ab = b - a; let t = clamp(dot(q - a, ab) / max(dot(ab, ab), 1e-4), 0.0, 1.0); \
        let d = length(q - a - ab * t); ink = max(ink, 1.0 - smoothstep(0.07, 0.16, d)); } \
    let ring = 1.0 - smoothstep(0.05, 0.11, abs(length(q) - 0.92)); \
    let g = max(ink, ring * 0.7) * (1.0 - smoothstep(0.98, 1.0, length(q))); \
    color = vec4<f32>(color.rgb * (1.0 + g), color.a * g); }\n";

const FLAME: &str = "{ let s = particle.f32_1; let t = particle.age; let along = 1.0 - uv.y; let y = uv.x * 2.0 - 1.0; \
    let sway = (sin(along * 5.0 - t * 9.0 + s * 20.0) * 0.5 + sin(along * 11.0 - t * 15.0 + s * 7.0) * 0.25) * along * 0.4; \
    let p = vec2<f32>(along * 5.0 - t * 7.0 + s * 3.0, (y - sway) * 2.5 + s * 13.0); let i0 = floor(p); let fr = fract(p); \
    let u = fr * fr * (3.0 - 2.0 * fr); \
    let h = fract(sin(vec4<f32>(dot(i0, vec2<f32>(127.1, 311.7)), dot(i0 + vec2<f32>(1.0, 0.0), vec2<f32>(127.1, 311.7)), \
        dot(i0 + vec2<f32>(0.0, 1.0), vec2<f32>(127.1, 311.7)), dot(i0 + vec2<f32>(1.0, 1.0), vec2<f32>(127.1, 311.7)))) * 43758.5453); \
    let n = mix(mix(h.x, h.y, u.x), mix(h.z, h.w, u.x), u.y); \
    let across = abs(y - sway); \
    let width = pow(clamp(1.0 - along, 0.0, 1.0), 0.85) * smoothstep(0.0, 0.28, along) * (0.7 + 0.6 * n); \
    let body = 1.0 - smoothstep(width * 0.45, width, across); \
    let heat = (1.0 - smoothstep(0.0, width * 0.65, across)) * (1.0 - smoothstep(0.1, 0.85, along)); \
    let glow = color.rgb * (0.7 + 1.1 * heat) + vec3<f32>(heat * heat * heat) * dot(color.rgb, vec3<f32>(0.3333)) * 0.5; \
    color = vec4<f32>(glow, color.a * body); }\n";

const SMOKE: &str = "{ let s = particle.f32_1; let spin = s * 6.28 + particle.age * (0.3 + fract(s * 3.7) * 0.6) * select(-1.0, 1.0, fract(s * 9.1) > 0.5); \
    let q0 = uv * 2.0 - 1.0; let q = vec2<f32>(q0.x * cos(spin) - q0.y * sin(spin), q0.x * sin(spin) + q0.y * cos(spin)); \
    var n = 0.0; var amp = 0.5; var f = 1.6; \
    for (var o = 0; o < 4; o = o + 1) { \
        let p = q * f + vec2<f32>(s * 17.0, s * 31.0 + particle.age * 0.35 * f); let i0 = floor(p); let fr = fract(p); let u = fr * fr * (3.0 - 2.0 * fr); \
        let h = vec4<f32>(dot(i0, vec2<f32>(127.1, 311.7)), dot(i0 + vec2<f32>(1.0, 0.0), vec2<f32>(127.1, 311.7)), dot(i0 + vec2<f32>(0.0, 1.0), vec2<f32>(127.1, 311.7)), dot(i0 + vec2<f32>(1.0, 1.0), vec2<f32>(127.1, 311.7))); \
        let v = fract(sin(h) * 43758.5453); \
        n = n + amp * mix(mix(v.x, v.y, u.x), mix(v.z, v.w, u.x), u.y); amp = amp * 0.5; f = f * 2.1; } \
    let r = length(q0); \
    let density = smoothstep(1.0, 0.25, r + (n - 0.5) * 0.9) * smoothstep(0.15, 0.6, n + 0.25 - r * 0.2); \
    let lit = 0.72 + 0.28 * clamp(-q0.y * 0.8 + 0.4, 0.0, 1.0) + 0.15 * (n - 0.5); \
    color = vec4<f32>(color.rgb * lit, color.a * density); }\n";

const SOLID: &str = "{ let sun = normalize(vec3<f32>(-0.45, 0.8, 0.4)); let n = normalize(normal); \
    let lit = 0.55 + 0.45 * max(dot(n, sun), 0.0); \
    color = vec4<f32>(color.rgb * lit, color.a); }\n";
