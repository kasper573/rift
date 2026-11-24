//! Where a prop that snaps to the ground stands: on the ground under it or on a prop placed there,
//! as a warp's shine lies on a floor laid over the ground, and along it where it slopes. Under each
//! point of its model, a foot inside another model (the first of its faces over the foot looks up,
//! out of it) rises out onto that face, and a foot inside nothing is let down onto the highest face
//! looking up under it, never under the ground. The prop then lies along the plane closest to
//! where its points rest, lifted until none of them rests over it, its shape as it is.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;

use crate::core::content::Content;
use crate::core::content::gltf::{FaceCrossing, ModelTop};
use crate::core::content::media::GltfInfos;
use crate::core::ground::Heightfield;
use crate::core::math::{Pos, Rect};
use crate::core::render::world_point;
use crate::core::spatial::SpatialIndex;
use crate::core::units::Meters;
use crate::systems::area::prop::{PropDef, PropId, PropPlacement};

/// What the props of an area that snap to the ground rest on besides the ground: the props placed
/// there that don't snap, filed by the ground each covers, and the model of every prop that snaps.
pub struct SnapSurfaces {
    snapping: HashMap<PropId, Arc<ModelTop>>,
    floors: Vec<Floor>,
    index: SpatialIndex,
}

impl SnapSurfaces {
    /// What snapping props rest on among `placed`, the props standing in an area.
    pub fn of<'a>(
        content: &Content,
        placed: impl IntoIterator<Item = &'a PropPlacement>,
    ) -> SnapSurfaces {
        let models = content.built::<GltfInfos>();
        let snapping = content
            .table::<PropDef>()
            .iter()
            .filter(|(_, def)| def.snap_to_ground)
            .filter_map(|(id, def)| {
                let model = models.0.get(def.model.as_str())?;
                Some((id, model.top.clone()))
            })
            .collect::<HashMap<_, _>>();
        let floors: Vec<Floor> = placed
            .into_iter()
            .filter(|placed| !snapping.contains_key(&placed.prop))
            .filter_map(|placed| {
                let model = models.0.get(placed.prop.get(content).model.as_str())?;
                Some(Floor {
                    placed: placed.clone(),
                    top: model.top.clone(),
                    low: model.bounds.min.xz(),
                    high: model.bounds.max.xz(),
                })
            })
            .collect();
        let mut index = SpatialIndex::new(Meters(FILED_CELL));
        index.refile(floors.iter().map(Floor::ground));
        SnapSurfaces {
            snapping,
            floors,
            index,
        }
    }

    /// How `placed` stands on the ground of `heights`: its foot under its middle, `sunk` into it,
    /// for a prop that doesn't snap; one that does rests on what lies under its model.
    pub fn stand(&self, heights: &Heightfield, placed: &PropPlacement) -> Transform {
        let laid = heights.height_at(placed.at) - placed.sunk;
        let flat = stood(placed, laid);
        let Some(top) = self.snapping.get(&placed.prop) else {
            return flat;
        };
        let rests: Vec<Vec3> = top
            .covered()
            .map(|local| {
                let ground = flat.transform_point(Vec3::new(local.x, 0.0, local.y)).xz();
                let rest = self.rest_at(heights, ground, laid.0);
                Vec3::new(ground.x - placed.at.x, rest, ground.y - placed.at.y)
            })
            .collect();
        let Some(slope) = slope_of(&rests) else {
            return flat;
        };
        let foot = rests
            .iter()
            .map(|rest| rest.y - slope.dot(rest.xz()))
            .fold(f32::MIN, f32::max);
        let up = Vec3::new(-slope.x, 1.0, -slope.y).normalize();
        Transform {
            translation: world_point(placed.at, Meters(foot)),
            rotation: Quat::from_rotation_arc(Vec3::Y, up) * flat.rotation,
            scale: flat.scale,
        }
    }

    /// Where a foot standing at `foot` over the ground point `ground` rests: out of every model it
    /// lies inside, one after another, as out of a floor laid on another; else on the highest face
    /// looking up under it; and never under the ground.
    fn rest_at(&self, heights: &Heightfield, ground: Vec2, foot: f32) -> f32 {
        let under = heights.height_at(Pos::new(ground.x, ground.y)).0;
        let models: Vec<Vec<FaceCrossing>> = self
            .index
            .within(Rect::new(Pos::new(ground.x, ground.y), Default::default()))
            .map(|index| self.floors[index].faces_over(heights, ground))
            .collect();
        let mut rest = foot.max(under);
        let mut risen = foot < under;
        while let Some(out) = models
            .iter()
            .filter_map(|faces| way_out(faces, rest))
            .reduce(f32::max)
        {
            rest = out;
            risen = true;
        }
        if risen {
            return rest;
        }
        models
            .iter()
            .flatten()
            .filter(|face| face.looks_up && face.height <= foot + LEVEL)
            .map(|face| face.height)
            .fold(under, f32::max)
    }
}

/// Where a foot at `at` comes out of the model whose faces cross the line up through it at `faces`,
/// lowest first: onto the first face over it looking up, where more of those over it look up than
/// down, so it lies inside the model; none where it lies outside.
fn way_out(faces: &[FaceCrossing], at: f32) -> Option<f32> {
    let over = || faces.iter().filter(|face| face.height > at + LEVEL);
    let up = over().filter(|face| face.looks_up).count();
    if up * 2 <= over().count() {
        return None;
    }
    over().find(|face| face.looks_up).map(|face| face.height)
}

/// How near a face lies to a foot to count as level with it, in metres: as near as a model's own
/// rounding puts a face meant to lie at its foot.
const LEVEL: f32 = 0.001;

/// How steeply the plane lying closest to `rests` rises east and south, their heights over where
/// they lie off the prop's middle; none for no rests, and flat for rests that lie along a line.
fn slope_of(rests: &[Vec3]) -> Option<Vec2> {
    if rests.is_empty() {
        return None;
    }
    let count = rests.len() as f32;
    let middle = rests.iter().copied().sum::<Vec3>() / count;
    let (mut across, mut rise) = (Mat2::ZERO, Vec2::ZERO);
    for rest in rests {
        let off = rest.xz() - middle.xz();
        across += Mat2::from_cols(off * off.x, off * off.y);
        rise += off * (rest.y - middle.y);
    }
    if across.determinant().abs() <= f32::EPSILON {
        return Some(Vec2::ZERO);
    }
    Some(across.inverse() * rise)
}

/// The ground a filed floor's cells span each way, in metres.
const FILED_CELL: f32 = 8.0;

/// A prop placed in an area that something snapping may rest on: where it stands, its model seen
/// from above, and the corners of the ground its model covers, in the model's own metres.
struct Floor {
    placed: PropPlacement,
    top: Arc<ModelTop>,
    low: Vec2,
    high: Vec2,
}

impl Floor {
    fn ground(&self) -> Rect<Meters> {
        let standing = stood(&self.placed, Meters(0.0));
        let corners = [
            Vec3::new(self.low.x, 0.0, self.low.y),
            Vec3::new(self.high.x, 0.0, self.low.y),
            Vec3::new(self.low.x, 0.0, self.high.y),
            Vec3::new(self.high.x, 0.0, self.high.y),
        ]
        .map(|corner| standing.transform_point(corner).xz());
        Rect::from_points(corners.into_iter().map(|at| Pos::new(at.x, at.y)))
    }

    /// Where the model's faces cross the line straight up through the ground point `ground`,
    /// lowest first, as the prop stands on `heights`.
    fn faces_over(&self, heights: &Heightfield, ground: Vec2) -> Vec<FaceCrossing> {
        let standing = stood(&self.placed, self.foot(heights));
        let local = standing
            .compute_affine()
            .inverse()
            .transform_point3(Vec3::new(ground.x, 0.0, ground.y))
            .xz();
        self.top
            .faces_over(local)
            .iter()
            .map(|face| FaceCrossing {
                height: standing
                    .transform_point(Vec3::new(local.x, face.height, local.y))
                    .y,
                looks_up: face.looks_up,
            })
            .collect()
    }

    fn foot(&self, heights: &Heightfield) -> Meters {
        heights.height_at(self.placed.at) - self.placed.sunk
    }
}

/// How `placed` stands with its foot at `foot`.
fn stood(placed: &PropPlacement, foot: Meters) -> Transform {
    Transform {
        translation: world_point(placed.at, foot),
        rotation: Quat::from_rotation_y(-placed.yaw.radians().0),
        scale: Vec3::splat(placed.scale),
    }
}
