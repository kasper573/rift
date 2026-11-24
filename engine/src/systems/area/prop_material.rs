//! How the game draws a material of a prop's model in place of the world's own look: in pictures
//! of the game's, by a shader of the prop stage filled with the material's values, or both. Many
//! props share one, as many models share a material, so a kit of models that draws in the same
//! few pictures holds each of them once.

use crate::systems::area::stages::PROP;
use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::SchemaMark;
use crate::core::content::{ContentFile, ContentId, ImageRef, Loading, Nearby, RowCheck, serial};
use crate::core::math::ColorGradient;
use crate::core::units::Meters;
use crate::core::vfx::VfxShaderDef;
use crate::core::vfx::{ParamValue, VfxShaderId};

pub type PropMaterialId = ContentId<PropMaterialDef>;

/// What draws a material of a model in place of what the model gives it. `color` is the picture
/// its colours are read from and `normal` the one its relief is, each laid by the model's own
/// texture coordinates and repeating past their edges, in place of the model's own pictures: a
/// model drawn this way needs to carry none. `shader` is a shader of the prop stage, drawing in
/// place of the world's own look, and `params` the values it fills the shader's parameters with; a
/// parameter left out keeps the shader's default. A thin material, a leaf or a petal, is
/// `translucency` translucent: that share of the light falling on either side of a face shines
/// through to the other, so a crown seen from beneath glows where the sun stands over it. A shader
/// that draws past the mesh, spreading its leaves into cards facing the eye, says how far at most
/// in `overhang`, in the model's metres: the prop's bounds and its impostor's frame grow by it.
/// `wear` lies over the colours by the model's second texture coordinates.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropMaterialDef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ImageRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal: Option<ImageRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shader: Option<VfxShaderId>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(transform = SchemaMark::ParamsOf.with("shader"))]
    pub params: BTreeMap<String, ParamValue>,
    #[serde(default, skip_serializing_if = "serial::is_zero_f32")]
    pub translucency: f32,
    #[serde(default, skip_serializing_if = "serial::is_zero_meters")]
    pub overhang: Meters,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wear: Option<PropWear>,
}

/// Weathering over a material's colours, laid by its model's second texture coordinates so it
/// falls over a whole roof or beam however the first set repeats the colours' picture: moss on a
/// roof's eaves, grime at a wall's foot. Where `mask` is white the surface takes the wear's colour,
/// which `mottle` picks between `colors` at each point, black for its `from`.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropWear {
    pub mask: ImageRef,
    pub mottle: ImageRef,
    pub colors: ColorGradient,
}

impl ContentFile for PropMaterialDef {
    const TABLE: &'static str = "prop_material";

    fn fresh(near: &Nearby) -> Result<PropMaterialDef, String> {
        Ok(PropMaterialDef {
            color: None,
            normal: None,
            shader: Some(
                near.file_where("a shader of the prop stage", |shader: &VfxShaderDef| {
                    shader.stage == PROP
                })?,
            ),
            params: BTreeMap::new(),
            translucency: 0.0,
            overhang: Meters(0.0),
            wear: None,
        })
    }

    fn check(&self, cx: &mut RowCheck) {
        if self.color.is_none() && self.normal.is_none() && self.shader.is_none() {
            cx.error(
                "",
                "a material that draws nothing",
                "no picture and no shader",
                "a colour picture, a normal picture or a shader",
            );
        }
        cx.range(
            "/translucency",
            self.translucency,
            0.0,
            1.0,
            "of the light shining through",
        );
        cx.at_least("/overhang", self.overhang.0, 0.0, "metres past the mesh");
    }
}

/// Every prop material that names a shader names one of the prop stage and fills only the
/// parameters it declares, each with a value of its kind and within its range; one that names
/// none fills none.
pub fn check_all(loading: &mut Loading) {
    let content = loading.content();
    for (id, material) in loading.rows::<PropMaterialDef>(&content) {
        let mut check = RowCheck::default();
        match material.shader {
            Some(shader) => {
                let shader = shader.get(&content);
                if shader.stage != PROP {
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
            }
            None => {
                for name in material.params.keys() {
                    check.error(
                        format!("/params/{name}"),
                        "a value with no shader to fill",
                        name,
                        "no values, or a shader that declares them",
                    );
                }
            }
        }
        loading.row_errors(id, check);
    }
}
