//! Water as content: a body of water is a file of its own, which an area lays over its ground
//! wherever its water map names it. What the water looks like is worked out from the few plain
//! things the file says of it, the ground under it and the light.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{
    ContentFile, ContentId, ContentRegistry, LoadStep, Loading, NamedContent, Nearby, RowCheck,
    SchemaMark,
};
use crate::core::math::Rgb;
use crate::core::units::Meters;
use crate::core::vfx::{ParamValue, VfxShaderDef, VfxShaderId};
use crate::systems::area::terrain::SurfaceId;

pub mod ripples;
pub mod stage;

pub use stage::WATER;

pub type WaterId = ContentId<WaterDef>;

/// A body of water: how far down it is seen through, the colour its depths take, how milky it
/// is, what it sounds like to wade, and the shader of the water stage that draws its surface,
/// with the parameters the water fills it with.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WaterDef {
    pub name: String,
    /// How deep a white bed still shows through it, looking straight down under a noon sun.
    pub clarity: Meters,
    /// The colour of its depths under a white light.
    pub depths: Rgb,
    /// How much light it sends back from within: 0 a clear water, dark where it is deep, to 1
    /// a milk of silt or rock flour.
    pub murk: f32,
    pub surface: SurfaceId,
    pub shader: VfxShaderId,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(transform = SchemaMark::ParamsOf.with("shader"))]
    pub params: BTreeMap<String, ParamValue>,
}

/// The clearest and the cloudiest a water may be: how deep a white bed shows through it.
pub const CLOUDIEST: Meters = Meters(0.05);
pub const CLEAREST: Meters = Meters(200.0);
/// How many waters an area may name: as many as a pixel of its water map tells apart.
pub const MAX_WATERS: usize = 256;

impl ContentFile for WaterDef {
    const TABLE: &'static str = "water";

    fn fresh(near: &Nearby) -> Result<WaterDef, String> {
        Ok(WaterDef {
            name: near.name(),
            clarity: Meters(6.0),
            depths: Rgb([22, 74, 112]),
            murk: 0.25,
            surface: near.file()?,
            shader: near.file_where("a shader of the water stage", |shader: &VfxShaderDef| {
                shader.stage == WATER
            })?,
            params: BTreeMap::new(),
        })
    }

    fn check(&self, cx: &mut RowCheck) {
        cx.text("/name", &self.name, 1, 60);
        cx.range(
            "/clarity",
            self.clarity.0,
            CLOUDIEST.0,
            CLEAREST.0,
            "metres down a white bed still shows",
        );
        cx.range("/murk", self.murk, 0.0, 1.0, "of the way to a milk");
    }
}

impl NamedContent for WaterDef {
    fn shown_name(&self) -> &str {
        &self.name
    }
}

pub fn content(registry: &mut ContentRegistry) {
    registry
        .file::<WaterDef>()
        .named::<WaterDef>()
        .hook(LoadStep::Across, check_all);
}

/// Every water is drawn by a shader of the water stage, filled only with the parameters it
/// declares, each with a value of its kind and within its range.
fn check_all(loading: &mut Loading) {
    let content = loading.content();
    for (id, water) in loading.rows::<WaterDef>(&content) {
        let mut check = RowCheck::default();
        let shader = water.shader.get(&content);
        if shader.stage != WATER {
            check.error(
                "/shader",
                "a shader of another stage",
                shader.stage.label(),
                format!("a shader of the {} stage", WATER.label()),
            );
        }
        for (name, value) in &water.params {
            shader.check_fill(&mut check, &format!("/params/{name}"), name, Some(value));
        }
        loading.row_errors(id, check);
    }
}
