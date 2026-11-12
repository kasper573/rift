//! The rain as the client watches it fall over the shown area: how hard it rains over each
//! stretch of the ground round the camera's subject, as the cloud that holds water drifts over
//! and the lie of the land shares the downpour out; how wet the ground lies there, soaking through
//! under the rain and drying after it; where the props that shelter keep it off the ground; the
//! lightning striking under storm cloud, flashing over the world and its thunder rolling in after;
//! and the rain the listener hears. Beside it, the rain as the server of the area knows it: how
//! hard it falls on a spot, for the scripts that ask.

use std::sync::{Arc, OnceLock};

use bevy::math::Affine3A;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, TaskPool};

use super::weather::{RainSound, ServerSky, ThunderHeard};
use super::{Area, AreaId, Rainfall, Weather};
use crate::core::audio::mix::AudioCategory;
use crate::core::audio::playback::{self, PlaySfx, SfxPlace};
use crate::core::audio::soundscape::{
    SoundscapeChannel, SoundscapeLayer, SoundscapeShape, SoundscapeZone,
};
use crate::core::content::Content;
use crate::core::content::gltf::ModelTop;
use crate::core::content::media::GltfInfos;
use crate::core::jobs::Handoff;
use crate::core::math::{Pos, Rect, Rng, Size};
use crate::core::render::world::clouds::{CLOUD_TILE, CloudDrift, CloudReading, CloudSheet};
use crate::core::render::{CameraRig, Look};
use crate::core::spatial::SpatialIndex;
use crate::core::units::Meters;
use crate::core::vfx::rain::{
    self as shelter, OPEN_TOP, RAIN_CELL, RAIN_SIDE, RainCell, RainField, SHELTER_CELL,
    SHELTER_SIDE, Shelter,
};
use crate::systems::area::scenery::{prop_transform, shown_weather};
use crate::systems::{GameSettings, WorldArea};

/// How hard it rains at `at` in `area` under `weather`, 0 none to 1 a cloudburst, with the field
/// of cloud `sheet` carried `drift` along and the rain carried `lean` sideways for every metre it
/// falls: as hard as the downpour under the cloud that holds water upwind of it, by as far as the
/// rain is carried on its way down from the clouds' height to the ground, as the area's rain map
/// shares it out where it lands.
pub fn rain_at(
    area: &Area,
    weather: &Weather,
    sheet: &CloudReading,
    drift: Vec2,
    lean: Vec2,
    at: Pos<Meters>,
) -> f32 {
    let fallen = (sheet.height().0 - area.heights.height_at(at).0).max(0.0);
    let source = Vec2::new(at.x, at.y) - lean * fallen;
    let under = (source - drift) / CLOUD_TILE.0;
    weather.rain * sheet.water(under) * area.rain_share(at)
}

/// How hard rain falls on the ground at `at` in the area a server's world serves, 0 none to 1 a
/// cloudburst, under the sky the server keeps: as hard as it rains over the stretch of ground
/// `at` lies in, as every client's field of rain has it, and not at all where a prop that
/// shelters keeps it off.
pub fn falling_on(world: &World, at: Pos<Meters>) -> f32 {
    let content = world.resource::<Content>();
    let area = super::load(content, world.resource::<WorldArea>().0);
    let sky = world.resource::<ServerSky>();
    let (Some(weather), Some(rainfall)) = (sky.weather(), &area.climate.rain) else {
        return 0.0;
    };
    if weather.rain <= 0.0 {
        return 0.0;
    }
    let look = content.built::<GameSettings>().look.clouds;
    let clouds = CloudReading::clear(look).covering(weather.clouds, weather.water);
    let lean = rainfall.lean(weather.wind.blowing());
    let cell = (Vec2::new(at.x, at.y) / RAIN_CELL.0).floor();
    let middle = (cell + 0.5) * RAIN_CELL.0;
    let rain = rain_at(
        area,
        weather,
        &clouds,
        sky.drift(),
        lean,
        Pos::new(middle.x, middle.y),
    );
    if rain <= 0.0 {
        return 0.0;
    }
    let shelters = world
        .resource::<ServerShelters>()
        .0
        .get_or_init(|| Shelters::of(content, area));
    let stood = Vec3::new(at.x, area.heights.height_at(at).0, at.y);
    if shelter::kept_off(stood, lean, |ground| shelters.top_over(ground)) {
        0.0
    } else {
        rain
    }
}

/// The props that shelter from the rain in the area a server's world serves, found when a script
/// first asks about the rain there.
#[derive(Resource, Default)]
pub struct ServerShelters(OnceLock<Shelters>);

/// How loud each of `sounds` plays where it rains `rain` hard, in their order: each loudest at its
/// own rain, fading out toward the next sound's either way, the lightest fading out toward no rain
/// and the heaviest holding on harder.
pub fn rain_sound_levels(sounds: &[RainSound], rain: f32) -> Vec<f32> {
    let mut order: Vec<usize> = (0..sounds.len()).collect();
    order.sort_by(|&a, &b| sounds[a].rain.total_cmp(&sounds[b].rain));
    let mut levels = vec![0.0; sounds.len()];
    if rain <= 0.0 {
        return levels;
    }
    for (place, &index) in order.iter().enumerate() {
        let peak = sounds[index].rain;
        let below = place
            .checked_sub(1)
            .map_or(0.0, |lighter| sounds[order[lighter]].rain);
        let above = order.get(place + 1).map(|&heavier| sounds[heavier].rain);
        levels[index] = if rain <= peak {
            ramp(below, peak, rain)
        } else {
            above.map_or(1.0, |above| 1.0 - ramp(peak, above, rain))
        };
    }
    levels
}

/// The rain the listener hears, 0 none to 1 a cloudburst.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct RainHeard(pub f32);

/// The zone the shown area's rain sounds in, everywhere over it, each of its rainfall's sounds as
/// loud as the rain `heard` makes it; none where it rains too lightly for any to sound.
pub fn rain_zone(area: &Area, heard: RainHeard) -> Option<SoundscapeZone> {
    let rainfall = area.climate.rain.as_ref()?;
    let levels = rain_sound_levels(&rainfall.sounds, heard.0);
    let channels: std::collections::BTreeMap<SoundscapeChannel, SoundscapeLayer> = rainfall
        .sounds
        .iter()
        .zip(levels)
        .filter(|(_, level)| *level > 0.0)
        .map(|(sound, level)| {
            (
                SoundscapeChannel(sound.channel),
                SoundscapeLayer {
                    src: sound.file.clone(),
                    volume: sound.volume * level,
                    pan: 0.0,
                    category: AudioCategory::Ambience,
                },
            )
        })
        .collect();
    (!channels.is_empty()).then(|| SoundscapeZone {
        shape: SoundscapeShape::Rect(Rect::from_size(area.size)),
        reach: Meters(1.0),
        channels,
    })
}

/// Lets the rain fall over the shown `area` under `weather`: the field of rain and the shelter
/// round the camera's subject laid for the shaders, the rain's `shells` shown while rain falls
/// near, lightning struck and its thunder played, and the rain the listener hears told. Returns
/// how brightly lightning lights the world this moment, 0 none to 1 a strike close by, and lights
/// `camera`'s ambient light with it.
pub fn fall(
    world: &mut World,
    area_id: AreaId,
    area: &Arc<Area>,
    weather: &Weather,
    shells: Option<Entity>,
    camera: Entity,
) -> f32 {
    let Some(subject) = world
        .get_resource::<CameraRig>()
        .map(|rig| rig.target().xz())
    else {
        return 0.0;
    };
    let now = world.resource::<Time>().elapsed_secs();
    let mut falling = world
        .remove_resource::<FallingRain>()
        .filter(|falling| falling.area == area_id)
        .unwrap_or_else(|| FallingRain::new(world, area_id, area));
    let flash = match &area.climate.rain {
        Some(rainfall) => {
            falling.lay_field(world, area, weather, rainfall, subject, now);
            falling.lay_shelter(world, subject);
            let lean = rainfall.lean(shown_weather(weather, 0.0).wind);
            let mut shelter = world.resource_mut::<Shelter>();
            if shelter.lean != lean {
                shelter.lean = lean;
            }
            let near = falling.near(subject);
            if let Some(mut shown) = shells.and_then(|shells| world.get_mut::<Visibility>(shells)) {
                shown.set_if_neq(if near > 0.0 {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                });
            }
            falling.strike(world, area, weather, subject, now)
        }
        None => {
            world
                .resource_mut::<RainField>()
                .set_if_neq(RainField::default());
            world
                .resource_mut::<Shelter>()
                .set_if_neq(Shelter::default());
            0.0
        }
    };
    let heard = world
        .resource::<playback::Listener>()
        .0
        .map_or(0.0, |at| falling.rain_over(at));
    let heard = RainHeard((heard * HEARD_STEPS).round() / HEARD_STEPS);
    if world.get_resource::<RainHeard>() != Some(&heard) {
        world.insert_resource(heard);
    }
    if let Some(lightning) = area.climate.lightning
        && flash > 0.0
    {
        let day = world.resource::<Look>().light.ambient_nits;
        if let Some(mut ambient) = world.get_mut::<AmbientLight>(camera) {
            ambient.brightness += flash * lightning.flash * day;
        }
    }
    world.insert_resource(falling);
    flash
}

/// How often the field of rain is laid again, in seconds: the clouds drift slowly.
const FIELD_EVERY: f32 = 0.25;
/// How far the camera's subject moves before the shelter is laid again round it.
const SHELTER_SLIP: Meters = Meters(8.0);
/// How finely the rain heard is told, so its sounds follow it without a new soundscape a frame.
const HEARD_STEPS: f32 = 50.0;
/// How long a stroke of lightning takes to fade to a third, in seconds.
const STROKE_FADE: f32 = 0.06;
/// How many places lightning looks over for the wettest cloud to strike from.
const STRIKE_TRIES: usize = 8;
/// How hard it must rain where lightning strikes: storm cloud alone throws it.
const STORM_RAIN: f32 = 0.05;

/// The rain falling over the shown area as the client keeps it: the field of rain laid last, its
/// cells by their place on the ground, and when, with the next one being laid off the main thread;
/// where the shelter was laid round; the props that shelter, filed by the ground they cover; the
/// strokes of lightning still flashing and the thunder still to come.
#[derive(Resource)]
struct FallingRain {
    area: AreaId,
    field: Arc<HashMap<IVec2, RainCell>>,
    corner: IVec2,
    laid: Option<f32>,
    laying: Option<Task<LaidRain>>,
    sheltered_round: Option<Vec2>,
    /// The area's sheltering props, found off the main thread: an area holds hundreds of
    /// thousands of props, and the rain may not hold a frame up to file them.
    shelters: Option<Shelters>,
    finding: Option<Task<Shelters>>,
    strokes: Vec<Stroke>,
    thunder: Vec<Rolling>,
    rng: Rng,
}

/// What laying a field of rain reads: the area and its weather, the clouds as they drifted, the
/// field's north-west cell, and the field laid before it, `since` seconds earlier.
struct RainJob {
    area: Arc<Area>,
    weather: Weather,
    rainfall: Rainfall,
    clouds: CloudReading,
    drift: Vec2,
    corner: IVec2,
    was: Arc<HashMap<IVec2, RainCell>>,
    since: f32,
}

/// A field of rain as laid: its cells by their place on the ground, its north-west cell, and the
/// field as the shaders read it.
struct LaidRain {
    field: Arc<HashMap<IVec2, RainCell>>,
    corner: IVec2,
    shown: RainField,
}

impl RainJob {
    fn lay(self) -> LaidRain {
        let lean = self.rainfall.lean(shown_weather(&self.weather, 0.0).wind);
        let mut field = HashMap::with_capacity(RAIN_SIDE * RAIN_SIDE);
        let mut cells = Vec::with_capacity(RAIN_SIDE * RAIN_SIDE);
        for row in 0..RAIN_SIDE as i32 {
            for column in 0..RAIN_SIDE as i32 {
                let at = self.corner + IVec2::new(column, row);
                let center = (at.as_vec2() + 0.5) * RAIN_CELL.0;
                let center = Pos::new(center.x, center.y);
                let rain = rain_at(
                    &self.area,
                    &self.weather,
                    &self.clouds,
                    self.drift,
                    lean,
                    center,
                );
                let wetness = match self.was.get(&at) {
                    Some(was) => soaked(was.wetness, rain, self.since, &self.rainfall),
                    None => (rain / STORM_RAIN).min(1.0),
                };
                let cell = RainCell {
                    share: self.area.rain_share(center),
                    wetness,
                    rain,
                };
                field.insert(at, cell);
                cells.push(cell);
            }
        }
        LaidRain {
            field: Arc::new(field),
            corner: self.corner,
            shown: RainField {
                corner: self.corner.as_vec2() * RAIN_CELL.0,
                cells,
            },
        }
    }
}

/// The props of an area that keep the rain off the ground, filed by the ground each covers.
struct Shelters {
    props: Vec<Sheltering>,
    index: SpatialIndex,
}

impl Shelters {
    fn of(content: &Content, area: &Area) -> Shelters {
        let models = content.built::<GltfInfos>();
        let props: Vec<Sheltering> = area
            .props
            .iter()
            .filter_map(|placed| {
                let def = placed.prop.get(content);
                if !def.shelter {
                    return None;
                }
                let model = models.0.get(&def.model.path())?;
                let placed = prop_transform(&area.heights, placed);
                Some(Sheltering {
                    into: placed.compute_affine().inverse(),
                    placed,
                    low: model.bounds.min.xz(),
                    high: model.bounds.max.xz(),
                    top: model.top.clone(),
                })
            })
            .collect();
        let mut index = SpatialIndex::new(Meters(SHELTER_CELL.0 * SHELTER_SIDE as f32 / 4.0));
        index.refile(props.iter().map(Sheltering::ground));
        Shelters { props, index }
    }

    /// How high the tallest of the props stands over the ground point `ground`; none where none
    /// stands over it.
    fn top_over(&self, ground: Vec2) -> Option<f32> {
        self.index
            .within(Rect::new(Pos::new(ground.x, ground.y), Size::zero()))
            .filter_map(|index| self.props[index].top_over(ground))
            .max_by(f32::total_cmp)
    }
}

/// A prop that keeps the rain off what lies under it: where its model's box lies, as the prop
/// stands, the way from the world into the model, and how high the model's top stands over the
/// ground it covers.
struct Sheltering {
    placed: Transform,
    into: Affine3A,
    low: Vec2,
    high: Vec2,
    top: Arc<ModelTop>,
}

/// A stroke of lightning flashing from `at` seconds on, `nearness` as bright as a strike close by.
struct Stroke {
    at: f32,
    nearness: f32,
}

/// The thunder of a strike due to be heard at `due` from `toward`, its clap and its roll as loud
/// as `heard` says.
struct Rolling {
    due: f32,
    toward: Pos<Meters>,
    heard: ThunderHeard,
}

impl FallingRain {
    fn new(world: &mut World, area_id: AreaId, area: &Arc<Area>) -> FallingRain {
        let (content, area) = (world.resource::<Content>().clone(), area.clone());
        world
            .resource_mut::<Shelter>()
            .set_if_neq(Shelter::default());
        FallingRain {
            area: area_id,
            field: Arc::default(),
            corner: IVec2::ZERO,
            laid: None,
            laying: None,
            sheltered_round: None,
            shelters: None,
            finding: area.climate.rain.is_some().then(|| {
                AsyncComputeTaskPool::get_or_init(TaskPool::default)
                    .spawn(async move { Shelters::of(&content, &area) })
            }),
            strokes: Vec::new(),
            thunder: Vec::new(),
            rng: Rng::fork_of(world),
        }
    }

    /// Lays the field of rain round `subject` anew, every [`FIELD_EVERY`] seconds, off the main
    /// thread, and takes in the one laid the frame before: each cell's share of the downpour and
    /// its rain under the clouds as they have drifted, its ground wetter for the rain since it was
    /// last laid, or drier without it. A cell newly taken in lies as wet as its rain makes it.
    fn lay_field(
        &mut self,
        world: &mut World,
        area: &Arc<Area>,
        weather: &Weather,
        rainfall: &Rainfall,
        subject: Vec2,
        now: f32,
    ) {
        if let Some(laying) = self.laying.take() {
            match Handoff::of(world).take(laying) {
                Ok(laid) => {
                    self.field = laid.field;
                    self.corner = laid.corner;
                    world.resource_mut::<RainField>().set_if_neq(laid.shown);
                }
                Err(laying) => self.laying = Some(laying),
            }
        }
        if self.laying.is_some() || self.laid.is_some_and(|laid| now - laid < FIELD_EVERY) {
            return;
        }
        let since = self.laid.map_or(0.0, |laid| now - laid);
        self.laid = Some(now);
        let job = RainJob {
            area: area.clone(),
            weather: *weather,
            rainfall: rainfall.clone(),
            clouds: world.resource::<CloudSheet>().reading(),
            drift: world.resource::<CloudDrift>().drift(),
            corner: (subject / RAIN_CELL.0).floor().as_ivec2() - IVec2::splat(RAIN_SIDE as i32 / 2),
            was: self.field.clone(),
            since,
        };
        self.laying = Some(
            AsyncComputeTaskPool::get_or_init(TaskPool::default).spawn(async move { job.lay() }),
        );
    }

    /// Lays the shelter round `subject` anew once it has slipped [`SHELTER_SLIP`] from where it
    /// was laid round: each cell a sheltering prop stands over topped as high as the prop's top
    /// stands there. None is laid until the area's sheltering props are found.
    fn lay_shelter(&mut self, world: &mut World, subject: Vec2) {
        if let Some(finding) = self.finding.take() {
            match Handoff::of(world).take(finding) {
                Ok(found) => self.shelters = Some(found),
                Err(finding) => self.finding = Some(finding),
            }
        }
        let Some(shelters) = &self.shelters else {
            return;
        };
        if self
            .sheltered_round
            .is_some_and(|round| round.distance(subject) < SHELTER_SLIP.0)
        {
            return;
        }
        self.sheltered_round = Some(subject);
        let corner = (subject / SHELTER_CELL.0).floor() - SHELTER_SIDE as f32 / 2.0;
        let corner = corner * SHELTER_CELL.0;
        let span = SHELTER_CELL.0 * SHELTER_SIDE as f32;
        let window = Rect::new(Pos::new(corner.x, corner.y), Size::new(span, span));
        let lean = world.resource::<Shelter>().lean;
        let mut tops = vec![OPEN_TOP; SHELTER_SIDE * SHELTER_SIDE];
        for index in shelters.index.within(window) {
            shelters.props[index].lay(&mut tops, corner);
        }
        world
            .resource_mut::<Shelter>()
            .set_if_neq(Shelter { corner, tops, lean });
    }

    /// The most it rains over the cells round `subject`'s.
    fn near(&self, subject: Vec2) -> f32 {
        let at = (subject / RAIN_CELL.0).floor().as_ivec2();
        (-1..=1)
            .flat_map(|dy| (-1..=1).map(move |dx| at + IVec2::new(dx, dy)))
            .filter_map(|cell| self.field.get(&cell))
            .map(|cell| cell.rain)
            .fold(0.0, f32::max)
    }

    /// How hard it rains over the cell at `at`, none outside the field.
    fn rain_over(&self, at: Pos<Meters>) -> f32 {
        let cell = (Vec2::new(at.x, at.y) / RAIN_CELL.0).floor().as_ivec2();
        self.field.get(&cell).map_or(0.0, |cell| cell.rain)
    }

    /// Strikes lightning as often as `weather` says, out of the wettest cloud of a few drawn over
    /// the field, a few strokes at a time, its thunder rolling after; plays the thunder now due;
    /// and returns how brightly the strokes still flashing light the world.
    fn strike(
        &mut self,
        world: &mut World,
        area: &Area,
        weather: &Weather,
        subject: Vec2,
        now: f32,
    ) -> f32 {
        let Some(lightning) = area.climate.lightning else {
            return 0.0;
        };
        let since = world.resource::<Time>().delta_secs();
        let chance = 1.0 - (-weather.lightning / 60.0 * since).exp();
        if weather.lightning > 0.0 && !self.field.is_empty() && self.rng.rand_float() < chance {
            let side = RAIN_SIDE as f32;
            let wettest = (0..STRIKE_TRIES)
                .map(|_| {
                    let at = Vec2::new(self.rng.rand_float(), self.rng.rand_float()) * side;
                    self.corner.as_vec2() + at
                })
                .map(|at| {
                    let rain = self
                        .field
                        .get(&at.floor().as_ivec2())
                        .map_or(0.0, |cell| cell.rain);
                    (at * RAIN_CELL.0, rain)
                })
                .max_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((at, _)) = wettest.filter(|(_, rain)| *rain >= STORM_RAIN) {
                let away = Meters(at.distance(subject));
                let nearness = lightning.thunder.nearness(away);
                let heard = lightning.thunder.heard(away);
                let strokes = 2 + (self.rng.rand_float() * 3.0) as usize;
                let mut stroke = now;
                for _ in 0..strokes {
                    self.strokes.push(Stroke {
                        at: stroke,
                        nearness: nearness * (0.6 + 0.4 * self.rng.rand_float()),
                    });
                    stroke += 0.04 + 0.1 * self.rng.rand_float();
                }
                self.thunder.push(Rolling {
                    due: now + heard.after.0,
                    toward: Pos::new(at.x, at.y),
                    heard,
                });
            }
        }
        let (due, waiting): (Vec<Rolling>, Vec<Rolling>) = std::mem::take(&mut self.thunder)
            .into_iter()
            .partition(|thunder| thunder.due <= now);
        self.thunder = waiting;
        for rolling in due {
            let sounds = [
                (lightning.thunder.clap, rolling.heard.clap),
                (lightning.thunder.roll, rolling.heard.roll),
            ];
            for (id, loudness) in sounds {
                playback::play(
                    world,
                    PlaySfx {
                        id,
                        place: SfxPlace::Afar {
                            toward: rolling.toward,
                            loudness,
                        },
                    },
                );
            }
        }
        self.strokes
            .retain(|stroke| now - stroke.at < STROKE_FADE * 12.0);
        self.strokes
            .iter()
            .filter(|stroke| stroke.at <= now)
            .map(|stroke| stroke.nearness * (-(now - stroke.at) / STROKE_FADE).exp())
            .sum::<f32>()
            .min(1.0)
    }
}

impl Sheltering {
    /// The ground the prop's box covers, as it stands.
    fn ground(&self) -> Rect<Meters> {
        let corners = [
            Vec3::new(self.low.x, 0.0, self.low.y),
            Vec3::new(self.high.x, 0.0, self.low.y),
            Vec3::new(self.low.x, 0.0, self.high.y),
            Vec3::new(self.high.x, 0.0, self.high.y),
        ]
        .map(|corner| self.placed.transform_point(corner).xz());
        Rect::from_points(corners.into_iter().map(|at| Pos::new(at.x, at.y)))
    }

    /// Tops each cell of the shelter laid from `corner` whose middle the prop stands over with
    /// the height of the prop's top there, where nothing taller stands over it.
    fn lay(&self, tops: &mut [f32], corner: Vec2) {
        let ground = self.ground();
        let side = SHELTER_SIDE as i32;
        let cell = |at: f32, from: f32| ((at - from) / SHELTER_CELL.0).floor() as i32;
        let (left, top) = (
            cell(ground.origin.x, corner.x),
            cell(ground.origin.y, corner.y),
        );
        let right = cell(ground.origin.x + ground.size.width, corner.x);
        let bottom = cell(ground.origin.y + ground.size.height, corner.y);
        for row in top.max(0)..=bottom.min(side - 1) {
            for column in left.max(0)..=right.min(side - 1) {
                let middle = corner + (Vec2::new(column as f32, row as f32) + 0.5) * SHELTER_CELL.0;
                if let Some(top) = self.top_over(middle) {
                    let at = (row * side + column) as usize;
                    tops[at] = tops[at].max(top);
                }
            }
        }
    }

    /// How high the prop's top stands over the ground point `ground`; none where it does not
    /// stand over it.
    fn top_over(&self, ground: Vec2) -> Option<f32> {
        let local = self
            .into
            .transform_point3(Vec3::new(ground.x, 0.0, ground.y))
            .xz();
        let height = self.top.over(local)?;
        Some(
            self.placed
                .transform_point(Vec3::new(local.x, height, local.y))
                .y,
        )
    }
}

/// How wet ground lying `wetness` wet lies `since` seconds on under `rain`: soaking through over
/// the rainfall's `wets` under a cloudburst, longer under lighter rain, and drying over its
/// `dries` where no rain falls.
fn soaked(wetness: f32, rain: f32, since: f32, rainfall: &Rainfall) -> f32 {
    let wetter = if rain > 0.0 {
        since * rain / rainfall.wets.0.max(f32::EPSILON)
    } else {
        -since / rainfall.dries.0.max(f32::EPSILON)
    };
    (wetness + wetter).clamp(0.0, 1.0)
}

/// How far `at` lies from `from` toward `to`, 0 to 1.
fn ramp(from: f32, to: f32, at: f32) -> f32 {
    if to <= from {
        return 1.0;
    }
    ((at - from) / (to - from)).clamp(0.0, 1.0)
}
