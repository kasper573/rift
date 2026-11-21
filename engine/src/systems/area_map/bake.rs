//! Baking the maps: drawing each square of every level of an area as a survey map draws it (see
//! [`super::survey`]), into the area's baked map (see [`super::baked`]). It is the designer's to
//! do, in the editor or with `rift`, and needs the GPU: each prop's plan from above is read from
//! its impostor first, then the squares are drawn on every core. A square whose key is the one
//! it was baked with is kept as it is, so a bake after an edit draws again only what the edit
//! changed.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::gltf::Gltf;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use rayon::prelude::*;

use super::baked::{self, BakedMap, BakedMapSquare, MAP_PICTURES, MapPictureFile};
use super::pictures::{self, MapDetail, MapTile};
use super::survey::{PropPlans, Survey};
use crate::core::assets;
use crate::core::content::Content;
use crate::core::os::Profile;
use crate::core::render::camera;
use crate::core::render::world::gpu::arena::PropModels;
use crate::core::render::world::gpu::impostor::{
    AtlasKey, ImpostorStore, PlanAsked, PlanFromAbove,
};
use crate::core::units::MeterSize;
use crate::systems::GameSettings;
use crate::systems::area::prop::Footprint;
use crate::systems::area::scenery;
use crate::systems::area::{self, Area, AreaDef, AreaId, PropId};

/// Bakes the maps a [`begin`] asks for. A world that bakes them draws props as the game does.
pub struct MapBakePlugin;

impl Plugin for MapBakePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, advance.run_if(resource_exists::<MapBake>));
    }
}

/// How far a bake has come: how many of the props' plans from above have come, of how many,
/// then how many of the squares it looks at are done, of how many.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapBakeProgress {
    Planning { come: u32, total: u32 },
    Drawing { drawn: u32, total: u32 },
}

/// The maps a bake made, each area's whole: the squares it kept as they were baked before, and
/// the rest drawn again.
pub struct BakedMaps {
    pub areas: Vec<BakedArea>,
    /// How long the bake took, and how long of that the props' plans took to come.
    pub took: Duration,
    pub planned: Duration,
}

/// One area's map as a bake made it.
pub struct BakedArea {
    pub area: AreaId,
    pub map: BakedMap,
    /// The squares kept as they were.
    pub kept: BTreeSet<MapTile>,
}

impl BakedArea {
    /// Which files the area at `bundle` writes anew with its map: its keys, and every picture not
    /// kept as it was, those it bakes no longer among them.
    pub fn changes(&self, bundle: &str) -> impl Fn(&str) -> bool + Send + Sync + 'static {
        let keys = baked::keys_file(bundle);
        let pictures = format!("{bundle}/");
        let kept: HashSet<String> = self
            .kept
            .iter()
            .map(|tile| baked::picture_file(bundle, *tile))
            .collect();
        move |file: &str| {
            let picture = file
                .strip_prefix(&pictures)
                .is_some_and(|part| MAP_PICTURES.matches(part));
            file == keys || (picture && !kept.contains(file))
        }
    }

    /// How many squares the bake drew.
    pub fn drawn(&self) -> usize {
        self.map.squares.len() - self.kept.len()
    }
}

/// Starts baking the maps of `areas` of `content`, in place of any bake under way.
pub fn begin(world: &mut World, content: Content, areas: Vec<AreaId>) {
    if let Some(eye) = world
        .remove_resource::<MapBake>()
        .and_then(|under_way| under_way.eye)
    {
        world.despawn(eye);
    }
    let mut wanted = HashMap::new();
    for prop in drawn_props(&content, &areas) {
        let model = assets::model(
            world.resource::<AssetServer>(),
            prop.get(&content).model.path(),
        );
        world.resource_mut::<PropModels>().want(&model);
        wanted.insert(prop, Plan::Loading(model));
    }
    let eye = (!wanted.is_empty()).then(|| eye(world, &content));
    world.insert_resource(MapBake {
        content,
        areas,
        eye,
        began: Instant::now(),
        step: Step::Planning(wanted),
    });
}

/// How far the bake under way has come, until it is done; none without one.
pub fn progress(world: &World) -> Option<MapBakeProgress> {
    match &world.get_resource::<MapBake>()?.step {
        Step::Planning(wanted) => Some(MapBakeProgress::Planning {
            come: wanted
                .values()
                .filter(|plan| matches!(plan, Plan::Come(_) | Plan::Missing))
                .count() as u32,
            total: wanted.len() as u32,
        }),
        Step::Drawing { drawn, total, .. } => Some(MapBakeProgress::Drawing {
            drawn: drawn.load(Ordering::Relaxed),
            total: *total,
        }),
        Step::Done(_) => None,
    }
}

/// The maps the bake under way made, once it is done, ending it.
pub fn take_baked(world: &mut World) -> Option<BakedMaps> {
    if !matches!(world.get_resource::<MapBake>()?.step, Step::Done(_)) {
        return None;
    }
    match world.remove_resource::<MapBake>()?.step {
        Step::Done(baked) => Some(baked),
        _ => None,
    }
}

/// Every square of every level of `area` whose maps are drawn at `detail`, the coarsest first.
pub fn squares_of(area: &Area, detail: MapDetail) -> Vec<MapTile> {
    let bounds = area.size.bounds();
    (0..=pictures::most_level(area, detail))
        .rev()
        .flat_map(|level| MapTile::within(level, bounds, bounds, detail))
        .collect()
}

/// How an area's baked map stands against the area as it is now, by its squares.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MapStanding {
    /// How many squares the map has.
    pub squares: u32,
    /// The squares it has no picture of.
    pub missing: u32,
    /// The squares whose picture was baked from what the area showed before.
    pub out_of_date: u32,
    /// The pictures of squares the area no longer has.
    pub extra: u32,
}

impl MapStanding {
    pub fn current(&self) -> bool {
        self.missing == 0 && self.out_of_date == 0 && self.extra == 0
    }

    /// Whether the area has no map at all.
    pub fn unbaked(&self) -> bool {
        self.missing == self.squares
    }
}

/// How the baked map of `area` of `content` stands, worked out from the area's files alone,
/// with no bake.
pub fn standing(content: &Content, area: AreaId) -> Option<MapStanding> {
    let def = content.table::<AreaDef>().get(area)?;
    let loaded = area::loaded(content, area)?;
    let detail = pictures::detail_of(content);
    let survey = Survey::new(content, loaded, detail, &PropPlans::new());
    let squares = squares_of(loaded, detail);
    let mut standing = MapStanding {
        squares: squares.len() as u32,
        ..MapStanding::default()
    };
    let known: BTreeSet<MapTile> = squares.iter().copied().collect();
    for tile in &squares {
        match def.map.squares.get(tile) {
            None => standing.missing += 1,
            Some(square) if square.key.as_ref() != Some(&survey.key(*tile)) => {
                standing.out_of_date += 1;
            }
            Some(_) => {}
        }
    }
    standing.extra = def
        .map
        .squares
        .keys()
        .filter(|tile| !known.contains(tile))
        .count() as u32;
    Some(standing)
}

fn advance(world: &mut World) {
    world.resource_scope(|world, mut bake: Mut<MapBake>| bake.advance(world));
}

#[derive(Resource)]
struct MapBake {
    content: Content,
    areas: Vec<AreaId>,
    /// The view the props' impostors are baked through, while the bake is under way; none where
    /// it draws no props.
    eye: Option<Entity>,
    began: Instant,
    step: Step,
}

enum Step {
    /// Each prop's plan from above on its way from its impostor.
    Planning(HashMap<PropId, Plan>),
    /// The squares being drawn on every core, how many of them are done, of how many, and how
    /// long the plans took to come.
    Drawing {
        task: Task<Vec<BakedArea>>,
        drawn: Arc<AtomicU32>,
        total: u32,
        planned: Duration,
    },
    Done(BakedMaps),
}

/// Where a prop's plan from above is: its model loading, its impostor read or baked, or come.
/// None comes for a prop whose model doesn't load.
enum Plan {
    Loading(Handle<Gltf>),
    /// The model is held until its impostor has come, so it stays on the GPU to be baked from.
    Dressed {
        _model: Handle<Gltf>,
        key: AtlasKey,
    },
    Come(Arc<PlanFromAbove>),
    Missing,
}

impl MapBake {
    fn advance(&mut self, world: &mut World) {
        match &mut self.step {
            Step::Planning(wanted) => {
                if let Some(plans) = planned(world, &self.content, wanted) {
                    let drawn = Arc::new(AtomicU32::new(0));
                    let total = self
                        .areas
                        .iter()
                        .filter_map(|id| area::loaded(&self.content, *id))
                        .map(|area| squares_of(area, pictures::detail_of(&self.content)).len())
                        .sum::<usize>() as u32;
                    let (content, areas) = (self.content.clone(), self.areas.clone());
                    let task = AsyncComputeTaskPool::get().spawn({
                        let drawn = drawn.clone();
                        async move {
                            areas
                                .iter()
                                .filter_map(|id| bake_area(&content, *id, &plans, &drawn))
                                .collect()
                        }
                    });
                    self.step = Step::Drawing {
                        task,
                        drawn,
                        total,
                        planned: self.began.elapsed(),
                    };
                }
            }
            Step::Drawing { task, planned, .. } => {
                if let Some(areas) = check_ready(task) {
                    let baked = BakedMaps {
                        areas,
                        took: self.began.elapsed(),
                        planned: *planned,
                    };
                    info!(
                        "baked the maps of {} areas, drawing {} squares in {:.1?}, of which \
                         finding the props' plans took {:.1?}",
                        baked.areas.len(),
                        baked.areas.iter().map(BakedArea::drawn).sum::<usize>(),
                        baked.took,
                        baked.planned,
                    );
                    if let Some(eye) = self.eye.take() {
                        world.despawn(eye);
                    }
                    self.step = Step::Done(baked);
                }
            }
            Step::Done(_) => {}
        }
    }
}

/// Every prop's plan from above once each has come or is known never to come; until then each
/// is moved on its way.
fn planned(
    world: &mut World,
    content: &Content,
    wanted: &mut HashMap<PropId, Plan>,
) -> Option<PropPlans> {
    for (prop, plan) in wanted.iter_mut() {
        if let Plan::Loading(model) = plan {
            let failed = world
                .resource::<AssetServer>()
                .get_load_state(model.id())
                .is_some_and(|state| state.is_failed());
            *plan = if failed {
                Plan::Missing
            } else {
                match scenery::impostor_of(world, content, *prop, model) {
                    Some(key) => Plan::Dressed {
                        _model: model.clone(),
                        key,
                    },
                    None => continue,
                }
            };
        }
        if let Plan::Dressed { key, .. } = plan {
            let profile = world.resource::<Profile>().clone();
            match world
                .resource_mut::<ImpostorStore>()
                .plan_from_above(&profile, *key)
            {
                PlanAsked::Coming => {}
                PlanAsked::Come(come) => *plan = Plan::Come(come),
                PlanAsked::Missing => *plan = Plan::Missing,
            }
        }
    }
    wanted
        .values()
        .all(|plan| matches!(plan, Plan::Come(_) | Plan::Missing))
        .then(|| {
            wanted
                .iter()
                .filter_map(|(prop, plan)| match plan {
                    Plan::Come(plan) => Some((*prop, plan.clone())),
                    _ => None,
                })
                .collect()
        })
}

/// The map of `area` of `content`, its props drawn from `plans`, on every core: each square kept
/// as it was baked where its key is the same, the rest drawn again, each counted done in
/// `drawn`. A square whose ground doesn't read is left out.
fn bake_area(
    content: &Content,
    area: AreaId,
    plans: &PropPlans,
    drawn: &AtomicU32,
) -> Option<BakedArea> {
    let def = content.table::<AreaDef>().get(area)?;
    let loaded = area::loaded(content, area)?;
    let detail = pictures::detail_of(content);
    let survey = Survey::new(content, loaded, detail, plans);
    let squares: BTreeMap<MapTile, (BakedMapSquare, bool)> = squares_of(loaded, detail)
        .par_iter()
        .filter_map(|tile| {
            let key = survey.key(*tile);
            let kept = def
                .map
                .squares
                .get(tile)
                .filter(|square| square.key.as_ref() == Some(&key));
            let square = match kept {
                Some(square) => Some((square.clone(), true)),
                None => match survey.draw(*tile) {
                    Ok(pixels) => Some((
                        BakedMapSquare {
                            picture: MapPictureFile::Drawn(baked::encode(&pixels)),
                            key: Some(key),
                        },
                        false,
                    )),
                    Err(problem) => {
                        warn!("Couldn't draw the map's square {tile:?}: {problem}");
                        None
                    }
                },
            };
            drawn.fetch_add(1, Ordering::Relaxed);
            Some((*tile, square?))
        })
        .collect();
    Some(BakedArea {
        area,
        kept: squares
            .iter()
            .filter(|(_, (_, kept))| *kept)
            .map(|(tile, _)| *tile)
            .collect(),
        map: BakedMap {
            squares: squares
                .into_iter()
                .map(|(tile, (square, _))| (tile, square))
                .collect(),
        },
    })
}

/// A view of nothing, drawn into a picture of a few pixels nobody sees, for the props' impostors
/// to be baked through as the game's own views bake them: in the picture its views are drawn in,
/// wherever the world has no view of its own.
fn eye(world: &mut World, content: &Content) -> Entity {
    let picture =
        Image::new_target_texture(EYE_PIXELS, EYE_PIXELS, TextureFormat::Rgba8UnormSrgb, None);
    let picture = world.resource_mut::<Assets<Image>>().add(picture);
    world
        .spawn((
            Camera3d::default(),
            RenderTarget::Image(picture.into()),
            camera::picture(content.built::<GameSettings>().look.picture),
            RenderLayers::none(),
        ))
        .id()
}

/// How many pixels across the picture of [`eye`] is: few, but enough for every pass a world's
/// picture takes, ambient occlusion's smaller copies among them.
const EYE_PIXELS: u32 = 64;

/// The props whose plans the maps of `areas` of `content` draw: those that take up ground.
fn drawn_props(content: &Content, areas: &[AreaId]) -> Vec<PropId> {
    let mut props: Vec<PropId> = areas
        .iter()
        .filter_map(|id| area::loaded(content, *id))
        .flat_map(|area| area.props.iter().chain(&area.stood))
        .map(|placed| placed.prop)
        .filter(|prop| !matches!(prop.get(content).footprint, Footprint::None))
        .collect();
    props.sort_unstable();
    props.dedup();
    props
}
