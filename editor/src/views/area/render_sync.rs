//! How a viewport tells the world it draws what the edited scene holds: the scene standing as a
//! picture of it shows, the sight it is seen from and the weather it is lit under, and each
//! change to it as the change it is (an instance moved, a scatter strewn anew, a module taken
//! away, a region of ground laid anew, the modules outlined), so a world standing the scene piece
//! by piece changes only that piece; and what module the world shows under the pointer.
//! [`Respawn`] is the world as it stands today: a change shows once a picture holding it stands,
//! every piece anew.

use std::sync::Arc;

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use engine::core::content::Content;
use engine::core::ground::{Heightfield, PaintMap};
use engine::core::math::Rect;
use engine::core::render::world::gpu::pick::{self, Outline, Picking};
use engine::core::render::world::sky::Daylight;
use engine::core::units::Meters;
use engine::systems::area::prop::Scatter;
use engine::systems::area::scenery::{self, Scenery, Sight};
use engine::systems::area::{Area, Weather};

use super::overlays::OutlineSet;
use crate::doc::NodeId;

pub trait WorldSink {
    /// Stands `area` as the scene now pictures it, in place of what stood for an earlier picture;
    /// whether it stands, which waits on its shaders.
    fn stand(&mut self, world: &mut World, area: &Arc<Area>, content: &Content) -> bool;

    /// Lights what stands under `weather`, as seen by `camera`.
    fn light(&mut self, world: &mut World, weather: &Weather, camera: Entity);

    /// Stands what stands for `sight`.
    fn see(&mut self, world: &mut World, sight: Sight);

    /// Moves what the module `id` stands as to `transform`.
    fn move_instance(&mut self, world: &mut World, id: NodeId, transform: Transform);

    /// Strews the copies of the scatter `id` anew as `scatter` strews them.
    fn set_scatter(&mut self, world: &mut World, id: NodeId, scatter: &Scatter);

    /// Takes away what the module `id` stands as.
    fn remove(&mut self, world: &mut World, id: NodeId);

    /// Draws `outlines` as the set `set`, in place of what it drew as that set before.
    fn outline(&mut self, world: &mut World, set: usize, outlines: &OutlineSet);

    /// Points at the pixel `pointer` of the picture, none while the pointer is off it: what the
    /// world last found under where it pointed, which follows a frame or two behind.
    fn pick(&mut self, world: &mut World, pointer: Option<UVec2>) -> Option<StagePick>;

    /// Lays the ground within `region` anew as `heights` and `paint` shape it.
    fn upload_region(
        &mut self,
        world: &mut World,
        region: Rect<Meters>,
        heights: &Heightfield,
        paint: &PaintMap,
    );

    /// Takes everything it stood out of the world.
    fn despawn(&mut self, world: &mut World);
}

/// What the world showed at a pixel of the picture: the module standing what it showed there, by
/// where the module was written, none where it showed nothing a module stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagePick {
    pub at: UVec2,
    pub origin: Option<String>,
}

/// The world standing the scene as the picture of it shows, every piece anew each time a picture
/// stands: a moved instance, a scatter strewn anew or a module taken away shows once the picture
/// holding the change stands. Props are outlined and picked by what the GPU shows of them through
/// its camera, the rest of the outlines laid as lines over the ground.
pub struct Respawn {
    root: Entity,
    layer: usize,
    camera: Entity,
    stood: Option<Stood>,
    /// For each set of outlines, the revision laid, and the thin and the bold lines' gizmos.
    lines: [Option<(u64, [Entity; 2])>; 2],
    /// For each set of outlines, the revision outlined on the GPU and that of the props standing
    /// then.
    outlined: [Option<(u64, u64)>; 2],
}

struct Stood {
    scenery: Scenery,
    area: Arc<Area>,
    /// The weather the scenery was last lit under.
    lit: Option<Weather>,
}

impl Respawn {
    /// Stands everything under `root`, on the render layer `layer`, seen through `camera`.
    pub fn new(root: Entity, layer: usize, camera: Entity) -> Respawn {
        Respawn {
            root,
            layer,
            camera,
            stood: None,
            lines: [None, None],
            outlined: [None, None],
        }
    }

    /// The area last stood.
    pub fn area(&self) -> Option<&Arc<Area>> {
        self.stood.as_ref().map(|stood| &stood.area)
    }

    fn lay_lines(&mut self, world: &mut World, set: usize, lines: &OutlineSet) {
        if self.lines[set].is_some_and(|(revision, _)| revision == lines.revision) {
            return;
        }
        let mut drawn = [GizmoAsset::default(), GizmoAsset::default()];
        for line in lines.runs.iter() {
            let [r, g, b, a] = line.color.to_srgba_unmultiplied();
            drawn[usize::from(line.bold)]
                .linestrip(line.points.iter().copied(), Color::srgba_u8(r, g, b, a));
        }
        let handles = drawn.map(|asset| world.resource_mut::<Assets<GizmoAsset>>().add(asset));
        let root = self.root;
        let entities = match self.lines[set] {
            Some((_, entities)) => entities,
            None => [LINE_PIXELS, BOLD_LINE_PIXELS].map(|width| {
                world
                    .spawn((
                        Gizmo {
                            line_config: GizmoLineConfig { width, ..default() },
                            depth_bias: -1.0,
                            ..default()
                        },
                        ChildOf(root),
                    ))
                    .id()
            }),
        };
        for (entity, handle) in entities.into_iter().zip(handles) {
            if let Some(mut gizmo) = world.get_mut::<Gizmo>(entity) {
                gizmo.handle = handle;
            }
        }
        self.lines[set] = Some((lines.revision, entities));
    }
}

impl WorldSink for Respawn {
    fn stand(&mut self, world: &mut World, area: &Arc<Area>, content: &Content) -> bool {
        if self
            .stood
            .as_ref()
            .is_some_and(|stood| Arc::ptr_eq(&stood.area, area))
        {
            return true;
        }
        if !scenery::shaders_ready(world, content, area) {
            return false;
        }
        let scenery = scenery::spawn(
            world,
            content,
            area,
            RenderLayers::from_layers(&[self.layer]),
        );
        if let Some(old) = self.stood.take() {
            scenery::despawn(world, old.scenery);
        }
        world.entity_mut(scenery.root).insert(ChildOf(self.root));
        self.stood = Some(Stood {
            scenery,
            area: area.clone(),
            lit: None,
        });
        self.outlined = [None, None];
        true
    }

    fn light(&mut self, world: &mut World, weather: &Weather, camera: Entity) {
        let Some(stood) = self.stood.as_mut() else {
            return;
        };
        if stood.lit.as_ref() == Some(weather) {
            return;
        }
        scenery::light(
            world,
            &stood.area,
            weather,
            Daylight::Fixed,
            stood.scenery,
            camera,
        );
        stood.lit = Some(*weather);
    }

    fn see(&mut self, world: &mut World, sight: Sight) {
        if let Some(stood) = &self.stood {
            scenery::view_from(world, stood.scenery, sight);
        }
    }

    fn move_instance(&mut self, _: &mut World, _: NodeId, _: Transform) {}

    fn set_scatter(&mut self, _: &mut World, _: NodeId, _: &Scatter) {}

    fn remove(&mut self, _: &mut World, _: NodeId) {}

    fn outline(&mut self, world: &mut World, set: usize, outlines: &OutlineSet) {
        let standing = self
            .stood
            .as_ref()
            .and_then(|stood| scenery::props_revision(world, stood.scenery));
        let wanted = (outlines.revision, standing.unwrap_or_default());
        if self.outlined[set] != Some(wanted) {
            self.outlined[set] = Some(wanted);
            let instances: Arc<[(Entity, u32)]> = match &self.stood {
                Some(stood) if !outlines.standing.is_empty() => {
                    scenery::instances_of(world, stood.scenery, |origin| {
                        outlines.standing.contains(origin)
                    })
                    .into_iter()
                    .map(|instance| (stood.scenery.root, instance))
                    .collect()
                }
                _ => Arc::new([]),
            };
            let [r, g, b, a] = outlines.color.to_srgba_unmultiplied();
            if let Some(mut picking) = world.get_mut::<Picking>(self.camera) {
                picking.outlines[set] = Outline {
                    color: Color::srgba_u8(r, g, b, a).to_linear(),
                    instances,
                };
            }
        }
        self.lay_lines(world, set, outlines);
    }

    fn pick(&mut self, world: &mut World, pointer: Option<UVec2>) -> Option<StagePick> {
        if let Some(mut picking) = world.get_mut::<Picking>(self.camera)
            && picking.pointer != pointer
        {
            picking.pointer = pointer;
        }
        let stood = self.stood.as_ref()?;
        let picked = pick::picked(world, self.camera)?;
        let origin = picked
            .hit
            .filter(|hit| hit.scenery == stood.scenery.root)
            .and_then(|hit| scenery::origin_of(world, stood.scenery, hit.instance));
        Some(StagePick {
            at: picked.at,
            origin,
        })
    }

    fn upload_region(
        &mut self,
        world: &mut World,
        _: Rect<Meters>,
        heights: &Heightfield,
        paint: &PaintMap,
    ) {
        if let Some(stood) = &self.stood {
            scenery::lay_ground(world, stood.scenery, heights, paint);
        }
    }

    fn despawn(&mut self, world: &mut World) {
        if let Some(stood) = self.stood.take() {
            scenery::despawn(world, stood.scenery);
        }
        self.outlined = [None, None];
    }
}

/// How wide the outlines over the ground are drawn, and those of what is selected.
const LINE_PIXELS: f32 = 1.0;
const BOLD_LINE_PIXELS: f32 = 2.0;
