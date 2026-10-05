use bevy_app::{App, Plugin, PostUpdate};
use bevy_color::{Alpha, Color};
use bevy_ecs::entity::EntityHashMap;
use bevy_ecs::prelude::*;
use bevy_text::TextColor;
use bevy_ui::widget::ImageNode;
use bevy_ui::{BackgroundColor, BorderColor};

/// An entity's own opacity. Descendants multiply theirs into it, and the effective product is
/// applied to every colored ui component under it each frame, always from the color's own value
/// rather than last frame's faded one.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct Opacity(pub f32);

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OpacitySet {
    Calculate,
    Apply,
}

pub struct OpacityPlugin;

impl Plugin for OpacityPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<OpacityMap>()
            .configure_sets(
                PostUpdate,
                (OpacitySet::Calculate, OpacitySet::Apply).chain(),
            )
            .add_systems(
                PostUpdate,
                (
                    calculate.in_set(OpacitySet::Calculate),
                    apply.in_set(OpacitySet::Apply),
                ),
            );
    }
}

#[derive(Resource, Default)]
struct OpacityMap(EntityHashMap<f32>);

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
        stack.push((entity, opacity.0));
        while let Some((entity, effective)) = stack.pop() {
            map.0.insert(entity, effective);
            if let Ok(kids) = children.get(entity) {
                for kid in kids.iter() {
                    let local = opacities.get(kid).map_or(1.0, |opacity| opacity.0);
                    stack.push((kid, effective * local));
                }
            }
        }
    }
}

#[derive(Component, Default)]
struct Faded {
    background: Option<Fade<Color>>,
    border: Option<Fade<[Color; 4]>>,
    text: Option<Fade<Color>>,
    image: Option<Fade<Color>>,
}

#[derive(Clone, Copy)]
struct Fade<T> {
    base: T,
    written: T,
}

#[allow(clippy::type_complexity)]
fn apply(
    map: Res<OpacityMap>,
    mut commands: Commands,
    mut nodes: Query<(
        Entity,
        Option<&mut Faded>,
        Option<&mut BackgroundColor>,
        Option<&mut BorderColor>,
        Option<&mut TextColor>,
        Option<&mut ImageNode>,
    )>,
) {
    for (entity, faded, background, border, text, image) in &mut nodes {
        let opacity = map.0.get(&entity).copied().unwrap_or(1.0);
        let mut fresh = Faded::default();
        let faded = match faded {
            Some(faded) => faded.into_inner(),
            None if opacity < 1.0 => &mut fresh,
            None => continue,
        };
        if let Some(mut background) = background {
            background.0 = refade(&mut faded.background, background.0, opacity);
        }
        if let Some(mut border) = border {
            let sides = [border.top, border.right, border.bottom, border.left];
            let [top, right, bottom, left] = refade(&mut faded.border, sides, opacity);
            *border = BorderColor {
                top,
                right,
                bottom,
                left,
            };
        }
        if let Some(mut text) = text {
            text.0 = refade(&mut faded.text, text.0, opacity);
        }
        if let Some(mut image) = image {
            image.color = refade(&mut faded.image, image.color, opacity);
        }
        if opacity >= 1.0 {
            commands.entity(entity).remove::<Faded>();
        } else if fresh.background.is_some()
            || fresh.border.is_some()
            || fresh.text.is_some()
            || fresh.image.is_some()
        {
            commands.entity(entity).insert(fresh);
        }
    }
}

trait Fadable: Copy + PartialEq {
    fn faded(self, opacity: f32) -> Self;
}

impl Fadable for Color {
    fn faded(self, opacity: f32) -> Color {
        self.with_alpha(self.alpha() * opacity.clamp(0.0, 1.0))
    }
}

impl Fadable for [Color; 4] {
    fn faded(self, opacity: f32) -> [Color; 4] {
        self.map(|color| color.faded(opacity))
    }
}

fn refade<T: Fadable>(slot: &mut Option<Fade<T>>, current: T, opacity: f32) -> T {
    let base = match *slot {
        Some(fade) if fade.written == current => fade.base,
        _ => current,
    };
    let written = base.faded(opacity);
    *slot = Some(Fade { base, written });
    written
}
