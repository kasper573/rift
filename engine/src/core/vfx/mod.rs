pub mod board;
pub mod compile;
pub mod field;
pub mod layer;
pub mod play;
pub mod render;
pub mod shader;
pub mod stage;
pub mod standing;
pub mod uniform;

pub use layer::{
    CoatLayer, ColorFill, GroundLayer, LayerMotion, LayerPlace, LayerRun, LightLayer, MeshFit,
    MeshLayer, MeshShape, ModelLayer, ParticleEmit, ParticleLayer, ParticleLook, ParticleSource,
    ParticleVelocity, ShakeLayer, SurfaceLayer, Travel, VfxAnchor, VfxBlend, VfxColor, VfxEase,
    VfxLayer, VfxTrack,
};
pub use play::LayerTime;
pub use shader::{
    BodyShape, CompiledVfxShaders, ParamKind, ShaderError, ShaderParam, SiteKind, VfxShaderDef,
    VfxShaderId, prelude_names, shader_file,
};
pub use stage::{ShaderPass, Stage, StageKind, StageSpec, Stages};

use std::collections::BTreeMap;

use crate::core::content::SchemaMark;
use crate::core::content::fixture::FixtureLock;
use crate::core::math::PosSchema;
use bevy_ecs::entity::MapEntities;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{
    Content, ContentFile, ContentId, ContentRegistry, Fixture, LoadStep, Loading, NamedContent,
    Nearby, RowCheck, serial,
};
use crate::core::geometry::PlacedShape;
use crate::core::math::{Pos, Rgba};
use crate::core::time::UnixMillis;
use crate::core::units::{Meters, Radians, Seconds};

pub type VfxId = ContentId<VfxDef>;

pub const MAX_PARAMS: usize = 32;
pub const MAX_LAYERS: usize = 24;

/// The shortest a layer may run: long enough to be seen.
// An f64 so the schema states the bound as written; an f32 widened would read 0.0500000007.
const SHORTEST_RUN: f64 = 0.05;
// As f64 for the same reason: the bounds of a light's reach, in metres.
const SHORTEST_REACH: f64 = 0.5;
const LONGEST_REACH: f64 = 32.0;
/// The nearest to a light that something may start casting its shadow, in metres.
const NEAREST_SHADOW: f64 = 0.02;
/// The narrowest an aura may be, in metres.
const NARROWEST_AURA: f64 = 0.1;

pub fn content(registry: &mut ContentRegistry) {
    registry
        .file::<VfxDef>()
        .named::<VfxDef>()
        .bundle::<VfxShaderDef>()
        .named::<VfxShaderDef>()
        .hook(LoadStep::Files, shader::compile_all)
        .hook(LoadStep::Across, check_all);
}

/// A visual effect: a stack of layers played together on one site. The row declares the
/// parameters whoever plays it may fill, each with the value it holds otherwise: what makes one
/// row a fire, a frost or a venom.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VfxDef {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<VfxParam>,
    pub layers: Vec<VfxLayer>,
    /// The point of the thing the vfx plays on, by the name its model gives one of its nodes,
    /// where layers anchored at the socket stand: the head of a torch in a hand, the wick of a
    /// candle. What the thing holds counts as part of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socket: Option<String>,
    /// Played, with the same parameters, on each actor this vfx's area reaches when it plays on
    /// an area: the targets a cast struck in it, else everyone standing in it. It plays on each
    /// headed the area's way, so a blow lands from where it came: what a blast does to whoever
    /// it catches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_actors_inside: Option<VfxId>,
}

/// A parameter a vfx row leaves to whoever plays it, and the value it holds when they don't.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VfxParam {
    pub name: String,
    pub default: ParamValue,
}

/// Which of a thing's surfaces a vfx plays on: chosen by whoever plays it, so one row plays on a
/// blade, a hand, a whole body or a prop alike.
#[derive(JsonSchema, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum VfxPart {
    /// Everything the thing is drawn with, what it holds included.
    #[default]
    Whole,
    /// What it holds.
    Held,
    /// Its own body, not what it holds.
    Body,
    /// Its body within `radius` of one of its model's nodes: a hand, the head.
    Node { node: String, radius: Meters },
}

/// A vfx as a carrier plays it: on which part of the thing it carries, and with which of the
/// row's parameters filled.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VfxPlay {
    pub vfx: VfxId,
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub part: VfxPart,
    /// Keyed by the row's declared parameters.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(transform = SchemaMark::ParamsOf.with("vfx"))]
    pub fill: BTreeMap<String, ParamValue>,
}

/// What a layer hands one of its shader's parameters: one of the row's declared parameters,
/// filled by whoever plays the vfx; a colour made brighter or seen through; or a value of its own.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
#[schemars(transform = SchemaMark::ParamFill.with(true))]
pub enum ParamFill {
    Param { param: String },
    Color(VfxColor),
    Value(ParamValue),
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParamValue {
    Bool(bool),
    Number(f32),
    Color(Rgba),
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(from = "WireParams", into = "WireParams")]
pub struct VfxParams(pub Vec<(String, ParamValue)>);

/// One playing vfx on one site, spawned by the server and drawn by every client that sees it.
#[derive(Component, Serialize, Deserialize, MapEntities, Clone, Debug, PartialEq)]
pub struct VfxInstance {
    pub vfx: VfxId,
    #[entities]
    pub site: VfxSite,
    /// The part of the thing the site is that the vfx plays on.
    #[serde(with = "wire_part")]
    pub part: VfxPart,
    pub params: VfxParams,
    /// Whoever dealt it: where a layer anchored at the hand leaves from.
    #[entities]
    pub dealer: Option<Entity>,
    /// The way the vfx travels: from whoever dealt it toward the one it lands on. None for a vfx
    /// on a thing that no one else dealt, which heads the way that thing faces as it turns. A vfx
    /// on an area heads the way the area faces, whatever this says.
    pub heading: Option<Radians>,
    pub started: UnixMillis,
    /// When its last layer ends; none for one that plays over and over until it is removed.
    pub until: Option<UnixMillis>,
}

/// Where a vfx plays: always one thing, or one place of the world.
#[derive(Serialize, Deserialize, JsonSchema, MapEntities, Clone, Debug, PartialEq)]
pub enum VfxSite {
    Actor(
        #[entities]
        #[schemars(with = "u64")]
        Entity,
    ),
    Object(
        #[entities]
        #[schemars(with = "u64")]
        Entity,
    ),
    Area(PlacedShape),
    /// From whoever deals it through what it reaches, in order.
    Path(
        #[entities]
        #[schemars(with = "Vec<PathStopSchema>")]
        Vec<PathStop>,
    ),
}

/// A point a path passes through: an actor, wherever it stands as the path is drawn, or a place
/// on the ground.
#[derive(Serialize, Deserialize, MapEntities, Clone, Copy, Debug, PartialEq)]
pub enum PathStop {
    Actor(#[entities] Entity),
    Point(Pos<Meters>),
}

#[derive(JsonSchema)]
#[allow(dead_code)]
enum PathStopSchema {
    Actor(u64),
    Point(PosSchema),
}

impl ParamValue {
    pub fn kind(self) -> ParamKind {
        match self {
            ParamValue::Bool(_) => ParamKind::Bool,
            ParamValue::Number(_) => ParamKind::Number,
            ParamValue::Color(_) => ParamKind::Color,
        }
    }
}

impl VfxParams {
    pub fn get(&self, name: &str) -> Option<ParamValue> {
        self.0
            .iter()
            .find(|(known, _)| known == name)
            .map(|(_, value)| *value)
    }
}

impl VfxPlay {
    /// `vfx` played on the whole of what carries it, with nothing filled.
    pub fn of(vfx: VfxId) -> VfxPlay {
        VfxPlay {
            vfx,
            part: VfxPart::Whole,
            fill: BTreeMap::new(),
        }
    }

    /// The parameters it fills, as an instance carries them.
    pub fn params(&self) -> VfxParams {
        VfxParams(
            self.fill
                .iter()
                .map(|(name, value)| (name.clone(), *value))
                .collect(),
        )
    }
}

impl VfxDef {
    /// How long the whole stack plays: until its last layer ends and the last particle any
    /// layer flung has died.
    pub fn lasts(&self) -> Seconds {
        Seconds(
            self.layers
                .iter()
                .map(|layer| (layer.run().ends() + layer.lingers()).0)
                .fold(0.0, f32::max),
        )
    }

    /// How long one run of the stack is, when its last layer ends: a vfx played over and over
    /// starts again this often, the particles of one run still flying as the next begins.
    pub fn cycle(&self) -> Seconds {
        Seconds(
            self.layers
                .iter()
                .map(|layer| layer.run().ends().0)
                .fold(0.0, f32::max),
        )
    }

    /// Whether a layer of it coats the surfaces of the thing it plays on.
    pub fn coats(&self) -> bool {
        self.layers
            .iter()
            .any(|layer| matches!(layer, VfxLayer::Coat(_)))
    }

    /// When the vfx lands: when the last of its layers that travel its path arrives, or, with none
    /// travelling, when it ends.
    pub fn lands(&self) -> Seconds {
        let travelling = self
            .layers
            .iter()
            .filter(|layer| layer.place().is_some_and(|place| place.travel.is_some()))
            .map(|layer| layer.run().ends().0)
            .reduce(f32::max);
        travelling.map_or(self.lasts(), Seconds)
    }

    pub fn declares(&self, name: &str) -> Option<ParamKind> {
        self.params
            .iter()
            .find(|param| param.name == name)
            .map(|param| param.default.kind())
    }

    /// The value of the row's parameter `name` as `given` fills it, else its default.
    pub fn param(&self, name: &str, given: &VfxParams) -> Option<ParamValue> {
        let declared = self.params.iter().find(|param| param.name == name)?;
        Some(
            given
                .get(name)
                .filter(|value| value.kind() == declared.default.kind())
                .unwrap_or(declared.default),
        )
    }
}

impl NamedContent for VfxDef {
    fn shown_name(&self) -> &str {
        &self.name
    }
}

impl ContentFile for VfxDef {
    const TABLE: &'static str = "vfx";

    fn fresh(near: &Nearby) -> Result<VfxDef, String> {
        let light = LightLayer {
            color: VfxColor {
                color: ColorFill::Value(Rgba::WHITE),
                brightness: 1.0,
                alpha: 1.0,
            },
            intensity: VfxTrack {
                keys: vec![(0.0, 1.0), (1.0, 0.0)],
                ease: VfxEase::Linear,
            },
            radius: Meters(4.0),
            flicker: 0.0,
            shadows_from: None,
            aura: None,
            place: LayerPlace::default(),
            run: LayerRun {
                delay: Seconds(0.0),
                duration: Seconds(1.0),
            },
        };
        Ok(VfxDef {
            name: near.name(),
            params: Vec::new(),
            layers: vec![VfxLayer::Light(light)],
            socket: None,
            on_actors_inside: None,
        })
    }

    fn check(&self, check: &mut RowCheck) {
        check.text("/name", &self.name, 1, 40);
        check.count("/layers", self.layers.len(), 1, MAX_LAYERS, "layers");
        check.count("/params", self.params.len(), 0, MAX_PARAMS, "parameters");
        for (index, param) in self.params.iter().enumerate() {
            shader::check_param_name(check, &format!("/params/{index}/name"), &param.name);
            if self.params[..index]
                .iter()
                .any(|other| other.name == param.name)
            {
                check.error(
                    format!("/params/{index}/name"),
                    "parameter listed twice",
                    &param.name,
                    "each parameter once",
                );
            }
        }
        if let Some(socket) = &self.socket {
            check.text("/socket", socket, 1, 40);
        }
        for (index, layer) in self.layers.iter().enumerate() {
            layer.check(self, &format!("/layers/{index}"), check);
        }
    }
}

impl VfxDef {
    /// What the engine reads of the file of a fixture vfx it fills `params` of: content tunes the
    /// rest, but never takes these away.
    pub fn fixture_locks(params: &[VfxParam]) -> Vec<FixtureLock> {
        params
            .iter()
            .map(|param| {
                FixtureLock::entry(
                    crate::field!(VfxDef, params),
                    param,
                    &[
                        crate::field!(VfxParam, name),
                        crate::field!(VfxParam, default),
                    ],
                )
            })
            .collect()
    }

    /// What a designer's fixture vfx holds where it is missing: a fresh vfx declaring `params`.
    pub fn fixture_fresh(near: &Nearby, params: Vec<VfxParam>) -> Result<VfxDef, String> {
        Ok(VfxDef {
            params,
            ..VfxDef::fresh(near)?
        })
    }
}

/// Where a vfx row's layers are refused for the sites they play on.
impl VfxDef {
    /// Refuses, at each layer, one that a site of `sites` the vfx plays on, `by` whom, cannot
    /// carry.
    pub fn check_sites(&self, sites: &[SiteKind], by: &str, check: &mut RowCheck) {
        for (index, layer) in self.layers.iter().enumerate() {
            for site in sites.iter().filter(|site| !layer.plays_on(**site)) {
                check.error(
                    format!("/layers/{index}"),
                    "layer off its site",
                    format!(
                        "a {} layer played on {} site by {by}",
                        layer.label(),
                        site.label()
                    ),
                    format!("a layer {} site carries", site.label()),
                );
            }
        }
    }
}

/// Refuses, on the vfx row, a layer that a site of `sites` the vfx is played on, `by` whom,
/// cannot carry.
pub fn check_played(loading: &mut Loading, vfx: VfxId, sites: &[SiteKind], by: &str) {
    let content = loading.content();
    let mut check = RowCheck::default();
    vfx.get(&content).check_sites(sites, by, &mut check);
    loading.row_errors(vfx, check);
}

/// Refuses, on the vfx row of each fixture of `F` that the game provides, a layer that a site the
/// engine plays it on, as `sites` says for each, cannot carry.
pub fn check_fixtures_played<F: Fixture<Row = VfxDef>>(
    loading: &mut Loading,
    sites: fn(F) -> &'static [SiteKind],
) {
    let content = loading.content();
    for fixture in F::VARIANTS {
        if let Some(vfx) = content.by_path::<VfxDef>(&fixture.path())
            && content.table::<VfxDef>().get(vfx).is_some()
        {
            let by = format!("the engine, as the {} fixture", fixture.name());
            check_played(loading, vfx, sites(*fixture), &by);
        }
    }
}

/// Refuses, at `pointer` of a carrier's row, a play whose fills the vfx doesn't declare or fills
/// with a value of another kind.
pub fn check_play(content: &Content, play: &VfxPlay, pointer: &str, check: &mut RowCheck) {
    let def = play.vfx.get(content);
    for (name, value) in &play.fill {
        match def.declares(name) {
            None => check.error(
                format!("{pointer}/fill/{name}"),
                "undeclared parameter",
                name,
                "a name the vfx row declares under params",
            ),
            Some(kind) if kind != value.kind() => check.error(
                format!("{pointer}/fill/{name}"),
                "wrong parameter kind",
                value.kind().label(),
                format!("a {}", kind.label()),
            ),
            Some(_) => {}
        }
    }
}

fn check_all(loading: &mut Loading) {
    let content = loading.content();
    for (id, def) in loading.rows::<VfxDef>(&content) {
        if let Some(inside) = def.on_actors_inside {
            let mut check = RowCheck::default();
            let by = format!("{} on the actors inside it", id.path(&content));
            inside
                .get(&content)
                .check_sites(&[SiteKind::Actor], &by, &mut check);
            loading.row_errors(inside, check);
        }
        let mut check = RowCheck::default();
        for (index, layer) in def.layers.iter().enumerate() {
            let Some((shader, params)) = layer.shader() else {
                continue;
            };
            let at = |field: &str| format!("/layers/{index}/{field}");
            let shader = shader.get(&content);
            if shader.stage != Stage::EFFECT {
                check.error(
                    at("shader"),
                    "shader of another stage",
                    shader.stage.label(),
                    format!("a shader of the {} stage", Stage::EFFECT.label()),
                );
            }
            for (name, fill) in params {
                let Some(declared) = shader.param(name) else {
                    check.error(
                        at(&format!("params/{name}")),
                        "unknown shader parameter",
                        name,
                        format!(
                            "one of {}",
                            shader
                                .params
                                .iter()
                                .map(|param| param.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    );
                    continue;
                };
                let given = match fill {
                    ParamFill::Param { param } => def.declares(param),
                    ParamFill::Color(_) => Some(ParamKind::Color),
                    ParamFill::Value(value) => Some(value.kind()),
                };
                if let Some(given) = given
                    && given != declared.kind
                {
                    check.error(
                        at(&format!("params/{name}")),
                        "wrong parameter kind",
                        given.label(),
                        format!("a {}", declared.kind.label()),
                    );
                }
                if let (ParamFill::Value(ParamValue::Number(number)), Some((min, max))) =
                    (fill, declared.range())
                    && (*number < min || *number > max)
                {
                    check.error(
                        at(&format!("params/{name}")),
                        "parameter out of range",
                        number,
                        format!("{min}..{max}"),
                    );
                }
            }
        }
        loading.row_errors(id, check);
    }
}

// Replication serializes with postcard, which cannot read the untagged value content uses.
#[derive(Serialize, Deserialize)]
struct WireParams(Vec<(String, WireValue)>);

#[derive(Serialize, Deserialize)]
enum WireValue {
    Bool(bool),
    Number(f32),
    Color(Rgba),
}

impl From<VfxParams> for WireParams {
    fn from(params: VfxParams) -> WireParams {
        WireParams(
            params
                .0
                .into_iter()
                .map(|(name, value)| {
                    let wire = match value {
                        ParamValue::Bool(flag) => WireValue::Bool(flag),
                        ParamValue::Number(number) => WireValue::Number(number),
                        ParamValue::Color(color) => WireValue::Color(color),
                    };
                    (name, wire)
                })
                .collect(),
        )
    }
}

impl From<WireParams> for VfxParams {
    fn from(wire: WireParams) -> VfxParams {
        VfxParams(
            wire.0
                .into_iter()
                .map(|(name, value)| {
                    let value = match value {
                        WireValue::Bool(flag) => ParamValue::Bool(flag),
                        WireValue::Number(number) => ParamValue::Number(number),
                        WireValue::Color(color) => ParamValue::Color(color),
                    };
                    (name, value)
                })
                .collect(),
        )
    }
}

/// The part an instance plays on crosses the wire as an externally tagged twin of `VfxPart`,
/// which postcard can read where it cannot read the internally tagged content form.
mod wire_part {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::VfxPart;
    use crate::core::units::Meters;

    #[derive(Serialize, Deserialize)]
    enum WirePart {
        Whole,
        Held,
        Body,
        Node(String, Meters),
    }

    pub fn serialize<S: Serializer>(part: &VfxPart, serializer: S) -> Result<S::Ok, S::Error> {
        let wire = match part {
            VfxPart::Whole => WirePart::Whole,
            VfxPart::Held => WirePart::Held,
            VfxPart::Body => WirePart::Body,
            VfxPart::Node { node, radius } => WirePart::Node(node.clone(), *radius),
        };
        wire.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<VfxPart, D::Error> {
        Ok(match WirePart::deserialize(deserializer)? {
            WirePart::Whole => VfxPart::Whole,
            WirePart::Held => VfxPart::Held,
            WirePart::Body => VfxPart::Body,
            WirePart::Node(node, radius) => VfxPart::Node { node, radius },
        })
    }
}
