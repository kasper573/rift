use bevy::image::Image;
use bevy_asset::Handle;
use bevy_ecs::hierarchy::Children;
use bevy_picking::prelude::Pickable;
use bevy_scene::{EntityScene, Scene, bsn, template_value};
use bevy_ui::widget::ImageNode;
use bevy_ui::{AlignItems, BorderRadius, Node, UiRect, Val};

use crate::component;
use crate::components::inspectable::{InspectableOptions, inspectable};
use crate::components::text::styled_text;
use crate::style::Style;
use crate::theme::Family;
use crate::tokens::{radius, spacing, typography};

const ICON: f32 = 14.0;

#[derive(Clone)]
pub struct ChipOptions {
    pub label: String,
    pub icon: Option<Handle<Image>>,
    pub family: Family,
    pub inspect: Option<InspectableOptions>,
}

pub fn chip(options: ChipOptions) -> Box<dyn Scene> {
    let ChipOptions {
        label,
        icon,
        family,
        inspect,
    } = options;
    match inspect {
        Some(inspect) => Box::new(inspectable(
            inspect,
            face(label, icon, family, Pickable::IGNORE),
        )),
        None => Box::new(face(
            label,
            icon,
            family,
            Pickable {
                should_block_lower: false,
                is_hoverable: true,
            },
        )),
    }
}

fn face(
    label: String,
    icon: Option<Handle<Image>>,
    family: Family,
    pickable: Pickable,
) -> impl Scene {
    let style = Style::new()
        .background(family.base)
        .border_color(family.border)
        .node(|node| {
            node.align_items = AlignItems::Center;
            node.column_gap = Val::Px(spacing::M);
            node.padding = UiRect::axes(Val::Px(spacing::L), Val::Px(1.0));
            node.border = UiRect::all(Val::Px(1.0));
            node.border_radius = BorderRadius::all(Val::Px(radius::PILL));
        });
    let icon = icon.map(|image| {
        bsn! {
            Node { width: Val::Px(ICON), height: Val::Px(ICON) }
            component(ImageNode::new(image))
        }
    });
    let label = styled_text(label, family.on, typography::LABEL);
    bsn! {
        template_value(style)
        template_value(pickable)
        Children [ {icon}, {EntityScene(label)} ]
    }
}

pub fn key_hint(hint: impl Into<String>, family: Family) -> impl Scene {
    let style = Style::new()
        .background(family.base)
        .border_color(family.border)
        .node(|node| {
            node.padding = UiRect::axes(Val::Px(spacing::L), Val::Px(spacing::S));
            node.border = UiRect::all(Val::Px(1.0));
            node.border_radius = BorderRadius::all(Val::Px(radius::S));
        });
    let text = styled_text(hint.into(), family.on, typography::HINT);
    bsn! {
        template_value(style)
        Pickable::IGNORE
        Children [ {EntityScene(text)} ]
    }
}
