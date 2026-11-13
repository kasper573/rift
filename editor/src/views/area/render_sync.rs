//! How a viewport tells the world it draws what the edited scene holds: the scene standing as a
//! picture of it shows, the sight it is seen from and the weather it is lit under, the ground as
//! a brush stroke shapes it, the modules outlined, and what module the world shows under the
//! pointer.

use std::sync::Arc;

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use engine::core::content::Content;
use engine::core::ground::{Heightfield, PaintMap};
use engine::core::render::world::gpu::pick::{self, Outline, Picking};
use engine::core::render::world::sky::Daylight;
use engine::systems::area::scenery::{self, Scenery, Sight};
use engine::systems::area::{Area, Weather};

use super::overlays::OutlineSet;

/// What the world showed at a pixel of the picture: the module standing what it showed there, by
/// where the module was written, none where it showed nothing a module stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagePick {
    pub at: UVec2,
    pub origin: Option<String>,
}

/// The scene standing in the world as the picture of it shows. A new picture stands in place of
/// the last, standing anew only what it changed. Props are outlined and picked by what the GPU
/// shows of them through the viewport's camera, the rest of the outlines laid as lines over the
/// ground.
pub struct StandingScene {
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
    /// The weather and the daylight the scenery was last lit under.
    lit: Option<(Weather, Daylight)>,
}

impl StandingScene {
    /// Stands everything under `root`, on the render layer `layer`, seen through `camera`.
    pub fn new(root: Entity, layer: usize, camera: Entity) -> StandingScene {
        StandingScene {
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

    /// Stands `area` as the scene now pictures it, in place of what stood for an earlier picture;
    /// whether it stands, which waits on its shaders when it stands anew.
    pub fn stand(&mut self, world: &mut World, area: &Arc<Area>, content: &Content) -> bool {
        if self
            .stood
            .as_ref()
            .is_some_and(|stood| Arc::ptr_eq(&stood.area, area))
        {
            return true;
        }
        self.outlined = [None, None];
        if let Some(stood) = self.stood.as_mut()
            && scenery::restand(world, stood.scenery, content, area)
        {
            stood.area = area.clone();
            stood.lit = None;
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
        true
    }

    /// Lights what stands under `weather` in `daylight`, as seen by `camera`.
    pub fn light(
        &mut self,
        world: &mut World,
        weather: &Weather,
        daylight: Daylight,
        camera: Entity,
    ) {
        let Some(stood) = self.stood.as_mut() else {
            return;
        };
        if stood.lit == Some((*weather, daylight)) {
            return;
        }
        scenery::light(world, &stood.area, weather, daylight, stood.scenery, camera);
        stood.lit = Some((*weather, daylight));
    }

    /// Stands what stands for `sight`.
    pub fn see(&mut self, world: &mut World, sight: Sight) {
        if let Some(stood) = &self.stood {
            scenery::view_from(world, stood.scenery, sight);
        }
    }

    /// Keeps what stands while no view shows it.
    pub fn unsee(&self, world: &mut World) {
        if let Some(stood) = &self.stood {
            scenery::unseen(world, stood.scenery);
        }
    }

    /// Draws `outlines` as the set `set`, in place of what it drew as that set before.
    pub fn outline(&mut self, world: &mut World, set: usize, outlines: &OutlineSet) {
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

    /// Points at the pixel `pointer` of the picture, none while the pointer is off it: what the
    /// world last found under where it pointed, which follows a frame or two behind.
    pub fn pick(&mut self, world: &mut World, pointer: Option<UVec2>) -> Option<StagePick> {
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

    /// Lays the ground anew as `heights` and `paint` shape it, standing anew only what stands
    /// where they changed.
    pub fn lay_ground(&mut self, world: &mut World, heights: &Heightfield, paint: &PaintMap) {
        if let Some(stood) = &self.stood {
            scenery::lay_ground(world, stood.scenery, heights, paint);
        }
    }

    /// Takes everything it stood out of the world.
    pub fn despawn(&mut self, world: &mut World) {
        if let Some(stood) = self.stood.take() {
            scenery::despawn(world, stood.scenery);
        }
        self.outlined = [None, None];
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

/// How wide the outlines over the ground are drawn, and those of what is selected.
const LINE_PIXELS: f32 = 1.0;
const BOLD_LINE_PIXELS: f32 = 2.0;
