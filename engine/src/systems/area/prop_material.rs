//! How the game draws a material of a prop's model in place of the world's own look: a shader of
//! the prop stage, filled with the material's values. Many props share one, as many models share
//! a material.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{ContentFile, ContentId, Loading, Nearby, RowCheck, serial};
use crate::core::vfx::shader::{Stage, VfxShaderDef};
use crate::core::vfx::{ParamValue, VfxShaderId};

pub type PropMaterialId = ContentId<PropMaterialDef>;

/// A shader of the prop stage, and the values it fills the shader's parameters with; a parameter
/// left out keeps the shader's default. A thin material, a leaf or a petal, is `translucency`
/// translucent: that share of the light falling on either side of a face shines through to the
/// other, so a crown seen from beneath glows where the sun stands over it.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropMaterialDef {
    pub shader: VfxShaderId,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(extend("x-params-of" = "shader"))]
    pub params: BTreeMap<String, ParamValue>,
    #[serde(default, skip_serializing_if = "serial::is_zero_f32")]
    pub translucency: f32,
}

impl ContentFile for PropMaterialDef {
    const TABLE: &'static str = "prop_material";

    fn fresh(near: &Nearby) -> Result<PropMaterialDef, String> {
        Ok(PropMaterialDef {
            shader: near.file_where("a shader of the prop stage", |shader: &VfxShaderDef| {
                shader.stage == Stage::Prop
            })?,
            params: BTreeMap::new(),
            translucency: 0.0,
        })
    }

    fn check(&self, cx: &mut RowCheck) {
        cx.range(
            "/translucency",
            self.translucency,
            0.0,
            1.0,
            "of the light shining through",
        );
    }
}

/// Every prop material names a shader of the prop stage and fills only the parameters it
/// declares, each with a value of its kind and within its range.
pub fn check_all(loading: &mut Loading) {
    let content = loading.content();
    for (id, material) in loading.rows::<PropMaterialDef>(&content) {
        let mut check = RowCheck::default();
        let shader = material.shader.get(&content);
        if shader.stage != Stage::Prop {
            check.error(
                "/shader",
                "a shader of another stage",
                shader.stage.label(),
                "a shader of the prop stage",
            );
        }
        for (name, value) in &material.params {
            shader.check_fill(&mut check, &format!("/params/{name}"), name, Some(value));
        }
        loading.row_errors(id, check);
    }
}
