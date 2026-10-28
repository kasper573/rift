//! A piece of a body: skinned meshes bound to one skeleton's rig, one for each build it is cut for
//! and one for every build without its own. A model is put together from parts, and so are the
//! looks that change it.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::material::MaterialId;
use super::skeleton::{BodyRegion, BuildName, SkeletonId};
use crate::core::assets::{FileSize, ModelRef};
use crate::core::content::{ContentFile, ContentId, Nearby};

pub type ModelPartId = ContentId<ModelPartDef>;

/// Skinned meshes bound to the rig of `skeleton`, each a glTF file of the game folder: `mesh` for
/// every build, and one in `builds` for each build cut its own way. A build with neither does not
/// draw the part.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelPartDef {
    pub skeleton: SkeletonId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh: Option<ModelRef>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub builds: BTreeMap<BuildName, ModelRef>,
    /// The regions of its skeleton it takes: a part laid over it that covers any of them replaces
    /// it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub covers: Vec<BodyRegion>,
    /// The regions it leaves bare of every other part, as a hood leaves the hair.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hides: Vec<BodyRegion>,
    /// What its meshes are painted with, made on its skeleton; without one they draw in their own
    /// material, untinted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<MaterialId>,
}

// A part is downloaded by every client that sees a body wearing it.
pub const MAX_PART_BYTES: FileSize = FileSize::kib(4 * 1024);

impl ModelPartDef {
    /// The mesh a body of `build` wears, when the part draws on it.
    pub fn mesh_for(&self, build: Option<&BuildName>) -> Option<&ModelRef> {
        build
            .and_then(|build| self.builds.get(build))
            .or(self.mesh.as_ref())
    }

    /// Every mesh of the part, with the build it is cut for, `None` for the one every build wears.
    pub fn meshes(&self) -> impl Iterator<Item = (Option<&BuildName>, &ModelRef)> {
        self.mesh
            .iter()
            .map(|mesh| (None, mesh))
            .chain(self.builds.iter().map(|(build, mesh)| (Some(build), mesh)))
    }
}

impl ContentFile for ModelPartDef {
    const TABLE: &'static str = "model_part";

    fn fresh(near: &Nearby) -> Result<ModelPartDef, String> {
        Ok(ModelPartDef {
            skeleton: near.file()?,
            mesh: None,
            builds: BTreeMap::new(),
            covers: Vec::new(),
            hides: Vec::new(),
            material: None,
        })
    }
}
