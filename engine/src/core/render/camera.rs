//! The camera: a perspective view pitched down on a point of the ground, turned to any yaw, tilted
//! between looking down and looking out toward the horizon, set to one of three distances, and the
//! ray it casts through the pointer. The view is a pure
//! projection of the rig, so the probe, the plates and the picking all project the way the camera
//! draws.

use bevy::anti_alias::smaa::Smaa;
use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::camera::Hdr;
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::light::{ShadowFilteringMethod, VolumetricFog};
use bevy::pbr::ScreenSpaceAmbientOcclusion;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::renderer::RenderAdapterInfo;
use bevy::render::settings::Backends;
use bevy::render::view::{ColorGrading, ColorGradingGlobal, ColorGradingSection};
use bevy::window::PrimaryWindow;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::ground::Heightfield;
use crate::core::gui;
use crate::core::math::{Pos, Rect, Rgb};
use crate::core::render::video::VideoPreferences;
use crate::core::units::{Degrees, Meters, Radians, Seconds, Stops};

const NEAR: f32 = 0.5;
/// How far a ray is followed through the world before it is given up as hitting nothing.
const REACH: f32 = 200.0;
const MARCH_STEP: f32 = 0.25;
const BISECTIONS: u32 = 12;

/// How the camera frames the world.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CameraFraming {
    /// How much of the world the camera takes in, top to bottom.
    pub fov: Degrees,
    /// How far down it looks when brought home.
    pub pitch: Degrees,
    /// The least it may look down, out over the ground, and the most, toward straight down.
    pub lowest_pitch: Degrees,
    pub highest_pitch: Degrees,
    /// How far one press of a turn key turns it.
    pub turn_step: Degrees,
    /// How far one press of a tilt key tilts it.
    pub tilt_step: Degrees,
    /// How far it stands back from what it looks at, at each zoom.
    pub zooms: Zooms,
    /// How long a zoom step takes to ease over.
    pub zoom_ease: Seconds,
}

/// The camera's distances along its pitch line, nearest first; it is brought home to the middle.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Zooms {
    pub close: Meters,
    pub middle: Meters,
    pub wide: Meters,
}

/// How the world's picture is finished.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PictureGrade {
    /// How much of the light blooms over its rim, so what the light catches glows softly into the
    /// shade around it.
    pub bloom: f32,
    /// How saturated the colours stay after the tone mapping, of its own: a little under it keeps
    /// a painted colour reading as painted, as the curve deepens colour as it lifts contrast.
    pub saturation: f32,
    /// How far the picture is opened over the light the world's lights are measured in.
    pub exposure: Stops,
    /// How much the air itself glows, of the sky's light, where no sun shaft crosses it, and in
    /// what colour.
    pub air_glow: f32,
    pub air_color: Rgb,
    /// What shows where nothing of the world is drawn.
    pub void: Rgb,
}

#[derive(Component)]
pub struct WorldCamera;

/// A camera that draws the world the way the player sees it, finished by `picture` in its grade.
#[derive(Component, Clone, Copy)]
pub struct WorldPicture(pub PictureGrade);

/// Whether the GPU draws the full picture: the sky scattering the sun's light and lighting the
/// world, the sun's shafts in the air, creases shaded by ambient occlusion, and the picture
/// settling over the frames by temporal anti-aliasing and the temporal shadow filter, which the
/// clearings' frame-stepped dither relies on to read as a translucency. GL has no compute shaders
/// for the sky, cannot read the depth the shafts are marched against, has too few storage
/// textures for the occlusion, and binds a texture with one sampler where the temporal pass
/// samples its history two ways, so there the picture is drawn plain: the authored ambient alone
/// lights what the sun does not, the shadows take a gaussian filter and the dither stands still.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct FullPicture(pub bool);

impl FullPicture {
    pub fn on(adapter: Option<&RenderAdapterInfo>) -> FullPicture {
        FullPicture(adapter.is_none_or(|adapter| Backends::from(adapter.backend) != Backends::GL))
    }
}

/// How many steps the air is marched in along each pixel's ray for the sun's shafts.
const AIR_STEPS: u32 = 32;

/// How every camera that draws the world finishes its picture: in a high range of light, the air
/// lit by the sun's shafts, creases shaded by what stands around them, soft shadows, and where
/// the GPU draws the full picture (`finish_new_picture`) under the sky's own scattering and lighting
/// and all of it settled over the frames by temporal anti-aliasing; then what the light catches
/// blooms over its rim, the light is opened up and mapped to the screen's range by a filmic
/// curve, which deepens the shade and lifts the light, dithered
/// against banding, and smoothed once more by morphological anti-aliasing over the finished
/// picture, which takes the clearings along with the geometry's edges. The shafts, the shaded
/// creases, the bloom and the smoothing are drawn as the player's `VideoPreferences` ask.
pub fn picture(grade: PictureGrade) -> impl Bundle {
    (
        WorldPicture(grade),
        Hdr,
        Msaa::Off,
        Tonemapping::AcesFitted,
        grading(Tone::NEUTRAL, grade),
        DebandDither::Enabled,
    )
}

/// How a picture is finished beside the engine's own finish: opened `exposure` brighter, its
/// colours `saturation` times as saturated, its midtones `contrast` times as contrasted and its
/// light warmed toward amber by `warmth`, or cooled toward blue below 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tone {
    pub exposure: Stops,
    pub saturation: f32,
    pub contrast: f32,
    pub warmth: f32,
}

impl Tone {
    pub const NEUTRAL: Tone = Tone {
        exposure: Stops(0.0),
        saturation: 1.0,
        contrast: 1.0,
        warmth: 0.0,
    };
}

/// The grading a picture of `grade` finished in `tone` is drawn through.
pub fn grading(tone: Tone, grade: PictureGrade) -> ColorGrading {
    ColorGrading {
        global: ColorGradingGlobal {
            exposure: grade.exposure.0 + tone.exposure.0,
            post_saturation: grade.saturation * tone.saturation,
            temperature: tone.warmth,
            ..default()
        },
        midtones: ColorGradingSection {
            contrast: tone.contrast,
            ..default()
        },
        ..default()
    }
}

/// A new picture is finished at once, before it draws a frame: what settles it over the frames
/// where the GPU draws the full picture, a gaussian shadow filter where it does not, and the rest
/// as the player's video preferences ask. The sky it sees is the shown area's (`sky::show`).
pub(super) fn finish_new_picture(
    add: On<Add<WorldPicture>>,
    full: Res<FullPicture>,
    video: Res<VideoPreferences>,
    grades: Query<&WorldPicture>,
    mut commands: Commands,
) {
    let Ok(&WorldPicture(grade)) = grades.get(add.entity) else {
        return;
    };
    let mut picture = commands.entity(add.entity);
    if full.0 {
        picture.insert((
            TemporalAntiAliasing::default(),
            ShadowFilteringMethod::Temporal,
        ));
    } else {
        picture.insert(ShadowFilteringMethod::Gaussian);
    }
    finish(&mut picture, *full, *video, grade);
}

/// Every picture is finished anew as the player's video preferences or its grade change.
pub(super) fn refinish_pictures(
    full: Res<FullPicture>,
    video: Res<VideoPreferences>,
    pictures: Query<(Entity, Ref<WorldPicture>)>,
    mut commands: Commands,
) {
    for (picture, grade) in &pictures {
        if video.is_changed() || (grade.is_changed() && !grade.is_added()) {
            finish(&mut commands.entity(picture), *full, *video, grade.0);
        }
    }
}

fn finish(
    picture: &mut EntityCommands,
    full: FullPicture,
    video: VideoPreferences,
    grade: PictureGrade,
) {
    match video.ambient_occlusion.level().filter(|_| full.0) {
        Some(quality_level) => picture.insert(ScreenSpaceAmbientOcclusion {
            quality_level,
            ..default()
        }),
        None => picture.remove::<ScreenSpaceAmbientOcclusion>(),
    };
    if video.bloom {
        picture.insert(Bloom {
            intensity: grade.bloom,
            ..Bloom::NATURAL
        });
    } else {
        picture.remove::<Bloom>();
    }
    match video.anti_aliasing.preset() {
        Some(preset) => picture.insert(Smaa { preset }),
        None => picture.remove::<Smaa>(),
    };
    if full.0 && video.light_shafts {
        picture.insert(VolumetricFog {
            ambient_color: grade.air_color.color(),
            ambient_intensity: grade.air_glow,
            jitter: 0.5,
            step_count: AIR_STEPS,
        });
    } else {
        picture.remove::<VolumetricFog>();
    }
}

/// A perspective `fov` high over a viewport `width` by `height`.
pub fn lens(fov: Degrees, width: u32, height: u32) -> Projection {
    Projection::Perspective(PerspectiveProjection {
        fov: fov.radians().0,
        near: NEAR,
        aspect_ratio: width as f32 / height.max(1) as f32,
        ..default()
    })
}

/// Which of its three distances the camera is kept at, along its pitch line.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Zoom {
    Close,
    #[default]
    Middle,
    Wide,
}

impl Zoom {
    /// How many steps the zoom has.
    pub const STEPS: u8 = 3;

    pub fn distance(self, zooms: &Zooms) -> Meters {
        match self {
            Zoom::Close => zooms.close,
            Zoom::Middle => zooms.middle,
            Zoom::Wide => zooms.wide,
        }
    }

    pub fn closer(self) -> Zoom {
        match self {
            Zoom::Close | Zoom::Middle => Zoom::Close,
            Zoom::Wide => Zoom::Middle,
        }
    }

    pub fn wider(self) -> Zoom {
        match self {
            Zoom::Close => Zoom::Middle,
            Zoom::Middle | Zoom::Wide => Zoom::Wide,
        }
    }

    /// The step as the probe reports it: 0 close, 1 middle, 2 wide.
    pub fn step(self) -> u8 {
        match self {
            Zoom::Close => 0,
            Zoom::Middle => 1,
            Zoom::Wide => 2,
        }
    }
}

/// Where the camera looks from: at a point of the world, from a yaw and a distance that ease toward
/// what the player last asked for, framed as the game frames it. A client resource, never
/// replicated, there once the game's look is known.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct CameraRig {
    framing: CameraFraming,
    target: Vec3,
    yaw: Radians,
    wanted_yaw: Radians,
    pitch: Radians,
    wanted_pitch: Radians,
    distance: Meters,
    zoom: Zoom,
}

/// How the player likes the camera to move.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct CameraPreferences {
    /// Degrees a second the camera turns at.
    pub turn_speed: Degrees,
}

impl Default for CameraPreferences {
    fn default() -> CameraPreferences {
        CameraPreferences {
            turn_speed: Degrees(180.0),
        }
    }
}

impl CameraRig {
    /// A camera framed by `framing`, brought home.
    pub fn new(framing: CameraFraming) -> CameraRig {
        CameraRig {
            framing,
            target: Vec3::ZERO,
            yaw: Radians(0.0),
            wanted_yaw: Radians(0.0),
            pitch: framing.pitch.radians(),
            wanted_pitch: framing.pitch.radians(),
            distance: Zoom::default().distance(&framing.zooms),
            zoom: Zoom::default(),
        }
    }

    pub fn framing(&self) -> &CameraFraming {
        &self.framing
    }

    /// Frames the camera anew, easing from where it stands to where the new framing keeps it.
    pub fn reframe(&mut self, framing: CameraFraming) {
        self.framing = framing;
        self.tilt(Degrees(0.0));
    }

    /// Points the camera at `at`, `height` above the ground there.
    pub fn look_at(&mut self, at: Pos<Meters>, height: Meters) {
        self.target = world_point(at, height);
    }

    pub fn target(&self) -> Vec3 {
        self.target
    }

    pub fn turn(&mut self, by: Degrees) {
        self.wanted_yaw = Radians(self.wanted_yaw.0 + by.radians().0);
    }

    /// Turns the camera straight away, as a drag does.
    pub fn turn_now(&mut self, by: Degrees) {
        self.turn(by);
        self.yaw = self.wanted_yaw;
    }

    /// Tilts the camera `by` degrees further down, or up toward the horizon when negative, within
    /// its limits.
    pub fn tilt(&mut self, by: Degrees) {
        self.wanted_pitch = Radians((self.wanted_pitch.0 + by.radians().0).clamp(
            self.framing.lowest_pitch.radians().0,
            self.framing.highest_pitch.radians().0,
        ));
    }

    /// Tilts the camera straight away.
    pub fn tilt_now(&mut self, by: Degrees) {
        self.tilt(by);
        self.pitch = self.wanted_pitch;
    }

    pub fn home(&mut self) {
        self.wanted_yaw = Radians(0.0);
        self.wanted_pitch = self.framing.pitch.radians();
        self.zoom = Zoom::default();
    }

    pub fn zoom_in(&mut self) {
        self.zoom = self.zoom.closer();
    }

    pub fn zoom_out(&mut self) {
        self.zoom = self.zoom.wider();
    }

    pub fn yaw(&self) -> Radians {
        self.yaw
    }

    pub fn pitch(&self) -> Radians {
        self.pitch
    }

    pub fn zoom(&self) -> Zoom {
        self.zoom
    }

    /// Whether the camera has come to rest where it was last asked to go.
    pub fn settled(&self) -> bool {
        self.yaw == self.wanted_yaw
            && self.pitch == self.wanted_pitch
            && self.distance == self.zoom.distance(&self.framing.zooms)
    }

    /// Whether the camera rests where going home puts it.
    pub fn at_home(&self) -> bool {
        self.settled()
            && self.yaw == Radians(0.0)
            && self.pitch == self.framing.pitch.radians()
            && self.zoom == Zoom::default()
    }

    pub fn eye(&self) -> Vec3 {
        let (sin, cos) = self.pitch.0.sin_cos();
        let back = Vec3::new(0.0, self.distance.0 * sin, self.distance.0 * cos);
        self.target + Quat::from_rotation_y(self.yaw.0) * back
    }

    pub fn transform(&self) -> Transform {
        Transform::from_translation(self.eye()).looking_at(self.target, Vec3::Y)
    }

    pub fn view(&self, viewport: Vec2) -> View {
        View::new(self.transform(), viewport, self.framing.fov)
    }

    fn ease(&mut self, dt: Seconds, preferences: &CameraPreferences) {
        let wanted = self.zoom.distance(&self.framing.zooms);
        let turn = preferences.turn_speed.radians().0 * dt.0;
        let toward = |from: Radians, to: Radians| {
            let apart = to.0 - from.0;
            if apart.abs() <= turn {
                to
            } else {
                Radians(from.0 + turn.copysign(apart))
            }
        };
        self.yaw = toward(self.yaw, self.wanted_yaw);
        self.pitch = toward(self.pitch, self.wanted_pitch);
        let caught = (dt.0 / self.framing.zoom_ease.0.max(f32::EPSILON)).min(1.0);
        let left = wanted.0 - self.distance.0;
        self.distance = if left.abs() < 0.01 {
            wanted
        } else {
            Meters(self.distance.0 + left * caught)
        };
    }
}

/// A camera's projection of the world onto a viewport, as the renderer and everything that places
/// points in the window share it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    world_from_view: GlobalTransform,
    clip_from_view: Mat4,
    viewport: Vec2,
}

impl View {
    pub fn new(camera: Transform, viewport: Vec2, fov: Degrees) -> View {
        let aspect = viewport.x / viewport.y.max(1.0);
        View {
            world_from_view: GlobalTransform::from(camera),
            clip_from_view: bevy::math::proj::perspective_infinite_reverse(
                fov.radians().0,
                aspect,
                NEAR,
            ),
            viewport,
        }
    }

    pub fn viewport(&self) -> Vec2 {
        self.viewport
    }

    pub fn eye(&self) -> Vec3 {
        self.world_from_view.translation()
    }

    /// The farthest from `from` the view takes in of ground no lower than `lowest`; none while it
    /// takes in the sky over the horizon.
    pub fn ground_reach(&self, from: Pos<Meters>, lowest: Meters) -> Option<Meters> {
        let corners = [
            Vec2::ZERO,
            Vec2::new(self.viewport.x, 0.0),
            Vec2::new(0.0, self.viewport.y),
            self.viewport,
        ];
        corners
            .into_iter()
            .try_fold(Meters(0.0), |farthest, corner| {
                let ray = self.ray(corner);
                if ray.direction.y >= 0.0 {
                    return None;
                }
                let along = (lowest.0 - ray.origin.y) / ray.direction.y;
                let lands = ground_pos(ray.origin + *ray.direction * along.max(0.0));
                Some(Meters(farthest.0.max((lands - from).length())))
            })
    }

    /// How many pixels of the viewport a metre spans `distance` off the eye, across the view.
    pub fn pixels_per_meter(&self, distance: Meters) -> f32 {
        self.viewport.y / 2.0 * self.clip_from_view.y_axis.y / distance.0.max(NEAR)
    }

    pub fn view_proj(&self) -> Mat4 {
        self.clip_from_view * self.world_from_view.to_matrix().inverse()
    }

    /// The ray from the eye through a point of the viewport, in window pixels from its top left.
    pub fn ray(&self, window: Vec2) -> Ray3d {
        let ndc = (window / self.viewport * 2.0 - Vec2::ONE) * Vec2::new(1.0, -1.0);
        let view_from_clip = self.clip_from_view.inverse();
        let near = view_from_clip.project_point3(ndc.extend(1.0));
        let far = view_from_clip.project_point3(ndc.extend(f32::EPSILON));
        let origin = self.world_from_view.transform_point(near);
        let direction = self.world_from_view.affine().transform_vector3(far - near);
        Ray3d::new(origin, Dir3::new(direction).unwrap_or(Dir3::NEG_Y))
    }

    /// Where a point of the world shows in the viewport, none behind the eye.
    pub fn project(&self, world: Vec3) -> Option<Vec2> {
        let clip = self.view_proj() * world.extend(1.0);
        if clip.w <= 0.0 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(Vec2::new(
            (ndc.x + 1.0) / 2.0 * self.viewport.x,
            (1.0 - ndc.y) / 2.0 * self.viewport.y,
        ))
    }
}

/// The ground on view: the shown area's heights and how far it reaches, for everything that stands
/// on the ground or points at it.
#[derive(Resource, Clone)]
pub struct GroundHeights {
    pub heights: Heightfield,
    pub bounds: Rect<Meters>,
}

impl GroundHeights {
    pub fn height_at(&self, at: Pos<Meters>) -> Meters {
        self.heights.height_at(at)
    }

    /// Where the ray meets the ground, if it does within the area.
    pub fn hit(&self, ray: Ray3d) -> Option<Pos<Meters>> {
        ground_hit(&self.heights, self.bounds, ray)
    }
}

/// The ground point a ray lands on: the ray is marched a quarter metre at a time from where it
/// enters the heights' range until it dips under the ground, then the crossing is bisected.
pub fn ground_hit(heights: &Heightfield, bounds: Rect<Meters>, ray: Ray3d) -> Option<Pos<Meters>> {
    let origin = ray.origin;
    let direction = *ray.direction;
    let under = |t: f32| {
        let point = origin + direction * t;
        let at = ground_pos(point);
        bounds.contains(at) && point.y <= heights.height_at(at).0
    };
    let range = heights.range();
    let mut t = if origin.y > range.highest.0 && direction.y < 0.0 {
        (range.highest.0 - origin.y) / direction.y
    } else {
        0.0
    };
    if under(t) {
        return Some(ground_pos(origin + direction * t));
    }
    let end = t + REACH;
    while t < end {
        let next = t + MARCH_STEP;
        let point = origin + direction * next;
        if point.y < range.lowest.0 && direction.y < 0.0 {
            return None;
        }
        if under(next) {
            let (mut above, mut below) = (t, next);
            for _ in 0..BISECTIONS {
                let mid = (above + below) / 2.0;
                if under(mid) {
                    below = mid;
                } else {
                    above = mid;
                }
            }
            return Some(ground_pos(origin + direction * below));
        }
        t = next;
    }
    None
}

/// A point of the ground `height` above it, in the world the camera sees: x east, y up, z south.
pub fn world_point(at: Pos<Meters>, height: Meters) -> Vec3 {
    Vec3::new(at.x, height.0, at.y)
}

pub fn ground_pos(point: Vec3) -> Pos<Meters> {
    Pos::new(point.x, point.z)
}

/// The turn that points a model's front, +Z as its glTF is authored, the way a `facing` on the
/// ground heads.
pub fn facing_rotation(facing: Radians) -> Quat {
    Quat::from_rotation_y(std::f32::consts::FRAC_PI_2 - facing.0)
}

pub fn ground_height(world: &World, at: Pos<Meters>) -> Meters {
    world
        .get_resource::<GroundHeights>()
        .map_or(Meters(0.0), |ground| ground.height_at(at))
}

/// The size of the primary window, in logical pixels.
pub fn window_size(world: &mut World) -> Option<Vec2> {
    let window = world
        .query_filtered::<&Window, With<PrimaryWindow>>()
        .single(world)
        .ok()?;
    Some(Vec2::new(window.width(), window.height()))
}

/// The camera's view through the primary window.
pub fn view_of(world: &mut World) -> Option<View> {
    let viewport = window_size(world)?;
    Some(world.get_resource::<CameraRig>()?.view(viewport))
}

/// The ray through the pointer, while it is over the world rather than the interface.
pub fn cursor_ray(world: &mut World) -> Option<Ray3d> {
    if gui::pointer_claimed(world) {
        return None;
    }
    let cursor = world
        .query_filtered::<&Window, With<PrimaryWindow>>()
        .single(world)
        .ok()?
        .cursor_position()?;
    Some(view_of(world)?.ray(cursor))
}

/// The ground under the pointer.
pub fn cursor_ground(world: &mut World) -> Option<Pos<Meters>> {
    let ray = cursor_ray(world)?;
    world.get_resource::<GroundHeights>()?.hit(ray)
}

/// Where a point `height` above the ground at `at` shows in the window.
pub fn window_point(world: &mut World, at: Pos<Meters>, height: Meters) -> Option<Vec2> {
    let ground = ground_height(world, at);
    view_of(world)?.project(world_point(at, ground + height))
}

pub(super) fn ease(
    time: Res<Time>,
    preferences: Res<CameraPreferences>,
    rig: Option<ResMut<CameraRig>>,
) {
    if let Some(mut rig) = rig
        && !rig.settled()
    {
        rig.ease(Seconds(time.delta_secs()), &preferences);
    }
}

pub(super) fn place(
    rig: Option<Res<CameraRig>>,
    mut camera: Query<&mut Transform, With<WorldCamera>>,
) {
    if let Some(rig) = rig
        && let Ok(mut transform) = camera.single_mut()
    {
        *transform = rig.transform();
    }
}
