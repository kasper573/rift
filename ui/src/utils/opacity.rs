use bevy_app::{App, Plugin, PostUpdate};
use bevy_color::{Alpha, Color};
use bevy_ecs::entity::EntityHashMap;
use bevy_ecs::prelude::*;
use bevy_text::TextColor;
use bevy_ui::widget::ImageNode;
use bevy_ui::{BackgroundColor, BorderColor};

/// An entity's own opacity. Descendants multiply theirs into it, and the effective product is
/// applied to every colored ui component under it each frame.
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
