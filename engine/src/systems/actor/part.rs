//! A piece of a body: skinned meshes made on one skeleton's rig, one for each build it is cut for
//! and one for every build without its own. A model is put together from parts, and so are the
//! looks that change it.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::skeleton::{self, BuildName, SkeletonId};
use crate::core::assets::{FileSize, ModelRef};
use crate::core::content::gltf::GltfInfo;
use crate::core::content::{
    Bundle, ContentFile, ContentId, ContentProblem, ErrorPlace, FileWriter, Nearby, PartSpec,
    Parts, Shape,
};

pub type ModelPartId = ContentId<ModelPartDef>;

/// Skinned meshes made on the rig of `skeleton`: `mesh.glb` for every build, and
/// `builds/<build>.glb` for a build cut its own way. A build with neither does not draw the part.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelPartDef {
    pub skeleton: SkeletonId,
    #[serde(skip)]
    pub mesh: Option<ModelRef>,
    #[serde(skip)]
    pub builds: BTreeMap<BuildName, ModelRef>,
}

pub const PART_MESH: PartSpec = PartSpec::File {
    name: "mesh",
    exts: &["glb"],
};

pub const BUILD_MESHES: PartSpec = PartSpec::Keyed {
    folder: "builds",
    exts: &["glb"],
};

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

impl Bundle for ModelPartDef {
    const PARTS: &'static [PartSpec] = &[PART_MESH, BUILD_MESHES];

    fn read_parts(&mut self, parts: &mut Parts) {
        let mut meshes: Vec<&str> = Vec::new();
        if let Some(mesh) = parts.file(PART_MESH) {
            self.mesh = Some(ModelRef::new(parts.path(mesh)));
            meshes.push(mesh);
        }
        for (build, file) in parts.keyed(BUILD_MESHES) {
            self.builds
                .insert(BuildName::new(build), ModelRef::new(parts.path(file)));
            meshes.push(file);
        }
        if meshes.is_empty() {
            parts.error(
                "",
                ErrorPlace::WholeFile,
                ContentProblem::new(
                    "missing file",
                    format!(
                        "neither {} nor {} in {}/",
                        PART_MESH.describe(),
                        BUILD_MESHES.describe(),
                        parts.bundle()
                    ),
                    "a mesh for every build, or one for each build the part is cut for",
                ),
            );
        }
        for file in meshes {
            let Some(info) = parts.gltf_info(file) else {
                continue;
            };
            for problem in mesh_problems(file, &info, parts.weight(file)) {
                parts.error(file, ErrorPlace::WholeFile, problem);
            }
        }
    }

    fn write_parts(&self, _: &str, _: &mut FileWriter) -> std::io::Result<()> {
        Ok(())
    }
}

impl ContentFile for ModelPartDef {
    const TABLE: &'static str = "model_part";
    const SHAPE: Shape = Shape::Bundle;

    fn fresh(near: &Nearby) -> Result<ModelPartDef, String> {
        Ok(ModelPartDef {
            skeleton: near.file()?,
            mesh: None,
            builds: BTreeMap::new(),
        })
    }
}

/// What a part's mesh must be on its own: light enough to download, within the joint budget, and
/// carrying no clips, which only its skeleton's libraries play.
fn mesh_problems(file: &str, info: &GltfInfo, weight: Option<FileSize>) -> Vec<ContentProblem> {
    let mut problems = Vec::new();
    if let Some(weight) = weight
        && weight > MAX_PART_BYTES
    {
        problems.push(ContentProblem::new(
            "mesh too heavy",
            format!("{file} weighs {weight}"),
            format!("at most {MAX_PART_BYTES}"),
        ));
    }
    if info.joints() > skeleton::MAX_JOINTS {
        problems.push(ContentProblem::new(
            "too many joints",
            format!("{file} has {} joints", info.joints()),
            format!("at most {}", skeleton::MAX_JOINTS),
        ));
    }
    if !info.clips.is_empty() {
        problems.push(ContentProblem::new(
            "clips in a part",
            format!("{file} carries {}", info.clip_names().join(", ")),
            "no clips: a body plays only its skeleton's libraries",
        ));
    }
    problems
}
