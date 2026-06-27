use bevy_app::{App, Plugin, PostUpdate};
use bevy_color::{Alpha, Color};
use bevy_ecs::entity::EntityHashMap;
use bevy_ecs::prelude::*;
use bevy_text::TextColor;
use bevy_time::Time;
use bevy_ui::widget::ImageNode;
use bevy_ui::{BackgroundColor, BorderColor};

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

    pub const fn new(opacity: f32) -> Opacity {
        Opacity {
            current: opacity,
            target: opacity,
            speed: 0.0,
            despawns: false,
        }
    }

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

    pub fn set(&mut self, opacity: f32) {
        *self = Opacity::new(opacity);
    }

    pub fn fade_in(&mut self, seconds: f32) {
        self.target = 1.0;
        self.despawns = false;
        self.speed = (1.0 - self.current) / seconds;
    }

    pub fn fade_out(&mut self, seconds: f32) {
        self.target = 0.0;
        self.despawns = true;
        self.speed = -self.current / seconds;
    }

    pub fn interpolate_to(&mut self, opacity: f32, seconds: f32) {
        self.target = opacity;
        self.despawns = false;
        self.speed = (opacity - self.current) / seconds;
    }
}

impl Default for Opacity {
    fn default() -> Opacity {
        Opacity::OPAQUE
    }
}

#[derive(Resource, Default)]
pub struct OpacityMap(EntityHashMap<f32>);

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OpacitySet {
    Fade,
    Calculate,
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
