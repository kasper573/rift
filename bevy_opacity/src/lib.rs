//! Hierarchical opacity for `bevy_ui`.
//!
//! A single [`Opacity`] component fades an entity and everything beneath it: effective opacity is
//! `parent_effective × local`, so descendants need no component of their own. Opacity *multiplies*
//! into each node's existing alpha (it does not replace it), so intentionally transparent or
//! semi-transparent paints fade proportionally and never become opaque. The value can be set
//! directly or animated with a built-in linear fade that can despawn the entity when it reaches 0.
//!
//! A simplified, `bevy_ui`-only take on `bevy_mod_opacity`.

use bevy_app::{App, Plugin, PostUpdate};
use bevy_color::{Alpha, Color};
use bevy_ecs::entity::EntityHashMap;
use bevy_ecs::prelude::*;
use bevy_text::TextColor;
use bevy_time::Time;
use bevy_ui::widget::ImageNode;
use bevy_ui::{BackgroundColor, BorderColor};

/// Opacity of an entity and its descendants, in `0.0..=1.0`.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct Opacity {
    current: f32,
    target: f32,
    speed: f32,
    despawns: bool,
}

impl Opacity {
    pub const INVISIBLE: Opacity = Opacity::new(0.0);
    pub const OPAQUE: Opacity = Opacity::new(1.0);

    /// A static opacity with no animation.
    pub const fn new(opacity: f32) -> Opacity {
        Opacity {
            current: opacity,
            target: opacity,
            speed: 0.0,
            despawns: false,
        }
    }

    /// Starts invisible and fades to `1.0` over `seconds`.
    pub const fn new_fade_in(seconds: f32) -> Opacity {
        Opacity {
            current: 0.0,
            target: 1.0,
            speed: 1.0 / seconds,
            despawns: false,
        }
    }

    pub const fn get(&self) -> f32 {
        self.current
    }

    pub const fn get_target(&self) -> f32 {
        self.target
    }

    pub const fn is_invisible(&self) -> bool {
        self.current <= 0.0
    }

    /// Sets the value immediately, cancelling any animation.
    pub fn set(&mut self, opacity: f32) {
        *self = Opacity::new(opacity);
    }

    /// Fades to `1.0` over `seconds`.
    pub fn fade_in(&mut self, seconds: f32) {
        self.target = 1.0;
        self.despawns = false;
        self.speed = (1.0 - self.current) / seconds;
    }

    /// Fades to `0.0` over `seconds`, then despawns the entity. Cancel with [`Opacity::set`],
    /// [`Opacity::fade_in`], or [`Opacity::interpolate_to`].
    pub fn fade_out(&mut self, seconds: f32) {
        self.target = 0.0;
        self.despawns = true;
        self.speed = -self.current / seconds;
    }

    /// Fades to `opacity` over `seconds` without despawning.
    pub fn interpolate_to(&mut self, opacity: f32, seconds: f32) {
        self.target = opacity;
        self.despawns = false;
        self.speed = (opacity - self.current) / seconds;
    }
}

/// Default to visible: better to show something than hide it implicitly.
impl Default for Opacity {
    fn default() -> Opacity {
        Opacity::OPAQUE
    }
}

/// Effective opacity per entity, rebuilt every frame. Entities absent from the map are outside any
/// opacity hierarchy and left untouched.
#[derive(Resource, Default)]
pub struct OpacityMap(EntityHashMap<f32>);

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OpacitySet {
    /// Advances fades. Despawns entities whose fade-out completed.
    Fade,
    /// Rebuilds the [`OpacityMap`] from the hierarchy.
    Calculate,
    /// Writes effective opacity into rendered alpha.
    Apply,
}

fn advance_fades(
    mut commands: Commands,
    time: Res<Time>,
    mut query: Query<(Entity, &mut Opacity)>,
) {
    let dt = time.delta_secs();
    for (entity, mut opacity) in &mut query {
        if opacity.speed != 0.0 {
            opacity.current += opacity.speed * dt;
            let reached = (opacity.speed > 0.0 && opacity.current >= opacity.target)
                || (opacity.speed < 0.0 && opacity.current <= opacity.target);
            if reached {
                opacity.current = opacity.target;
                opacity.speed = 0.0;
            }
        }
        if opacity.despawns && opacity.current <= 0.0 {
            commands.entity(entity).try_despawn();
        }
    }
}

fn calculate(
    mut map: ResMut<OpacityMap>,
    roots: Query<(Entity, &Opacity)>,
    opacities: Query<&Opacity>,
    children: Query<&Children>,
) {
    map.0.clear();
    let mut stack = Vec::new();
    for (entity, opacity) in &roots {
        if map.0.contains_key(&entity) {
            continue;
        }
        stack.push((entity, opacity.get()));
        while let Some((entity, effective)) = stack.pop() {
            map.0.insert(entity, effective);
            if let Ok(kids) = children.get(entity) {
                for kid in kids.iter() {
                    let local = opacities.get(kid).map_or(1.0, Opacity::get);
                    stack.push((kid, effective * local));
                }
            }
        }
    }
}

#[allow(clippy::type_complexity)]
fn apply(
    map: Res<OpacityMap>,
    mut nodes: Query<(
        Entity,
        Option<&mut BackgroundColor>,
        Option<&mut BorderColor>,
        Option<&mut TextColor>,
        Option<&mut ImageNode>,
    )>,
) {
    for (entity, background, border, text, image) in &mut nodes {
        let Some(&opacity) = map.0.get(&entity) else {
            continue;
        };
        if opacity >= 1.0 {
            continue;
        }
        if let Some(mut background) = background {
            background.0 = fade(background.0, opacity);
        }
        if let Some(mut border) = border {
            border.top = fade(border.top, opacity);
            border.right = fade(border.right, opacity);
            border.bottom = fade(border.bottom, opacity);
            border.left = fade(border.left, opacity);
        }
        if let Some(mut text) = text {
            text.0 = fade(text.0, opacity);
        }
        if let Some(mut image) = image {
            image.color = fade(image.color, opacity);
        }
    }
}

fn fade(color: Color, opacity: f32) -> Color {
    color.with_alpha(color.alpha() * opacity)
}

/// Registers opacity fading and propagation in [`PostUpdate`]. Anything that writes a node's base
/// colour each frame must run before [`OpacitySet::Apply`], which multiplies opacity into it.
pub struct OpacityPlugin;

impl Plugin for OpacityPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<OpacityMap>()
            .configure_sets(
                PostUpdate,
                (OpacitySet::Fade, OpacitySet::Calculate, OpacitySet::Apply).chain(),
            )
            .add_systems(
                PostUpdate,
                (
                    advance_fades.in_set(OpacitySet::Fade),
                    calculate.in_set(OpacitySet::Calculate),
                    apply.in_set(OpacitySet::Apply),
                ),
            );
    }
}
