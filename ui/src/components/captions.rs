use bevy_color::{Alpha, Color};
use bevy_ecs::hierarchy::{ChildOf, Children};
use bevy_ecs::prelude::*;
use bevy_math::Vec2;
use bevy_picking::hover::Hovered;
use bevy_picking::prelude::Pickable;
use bevy_scene::{CommandsSceneExt, EntityScene, Scene, bsn, template_value};
use bevy_ui::{
    AlignItems, BorderRadius, Display, FlexDirection, GridPlacement, Node, Overflow,
    RepeatedGridTrack, UiRect, Val,
};

use crate::component;
use crate::components::rich_text::{RichText, rich_text};
use crate::components::text::styled_text;
use crate::cursor::ClickThrough;
use crate::motion::{Easing, Timing, Transform2d};
use crate::presence::{Leaving, Presence, PresenceMove};
use crate::style::Style;
use crate::theme::theme;
use crate::tokens::{palette, radius, spacing, typography};

const WIDEST: f32 = 640.0;
const ROW_SIZE: f32 = 17.0;
const ROW_LINE_HEIGHT: f32 = 23.0;
const SHOWN_TEXT_LINES: f32 = 2.0;
const SCRIM_PRESENCE: Presence = Presence::new(
    PresenceMove::new(
        Transform2d::new(Vec2::new(0.0, 12.0), 0.97),
        Timing::new(360, Easing::Standard),
    ),
    PresenceMove::new(
        Transform2d::new(Vec2::new(0.0, 8.0), 0.98),
        Timing::new(240, Easing::StandardAccelerate),
    ),
)
.hidden();
const ROW_PRESENCE: Presence = Presence::new(
    PresenceMove::new(
        Transform2d::new(Vec2::new(0.0, 8.0), 1.0),
        Timing::new(340, Easing::Standard),
    ),
    PresenceMove::new(
        Transform2d::new(Vec2::new(0.0, -6.0), 1.0),
        Timing::new(220, Easing::StandardAccelerate),
    ),
)
.collapsing();
const SWAP_PRESENCE: Presence = Presence::new(
    PresenceMove::new(
        Transform2d::new(Vec2::new(0.0, 4.0), 1.0),
        Timing::new(240, Easing::Standard),
    ),
    PresenceMove::new(
        Transform2d::new(Vec2::new(0.0, -4.0), 1.0),
        Timing::new(160, Easing::StandardAccelerate),
    ),
);
const COUNT_PRESENCE: Presence = Presence::new(
    PresenceMove::new(
        Transform2d::new(Vec2::ZERO, 0.9),
        Timing::new(260, Easing::Standard),
    ),
    PresenceMove::new(
        Transform2d::new(Vec2::ZERO, 0.92),
        Timing::new(160, Easing::StandardAccelerate),
    ),
);

#[derive(Component, Clone, Default, PartialEq)]
#[require(Node)]
pub struct Captions {
    pub rows: Vec<LabelledLine>,
    pub more: Option<RichText>,
}

#[derive(Clone, PartialEq)]
pub struct LabelledLine {
    pub key: u64,
    pub label: String,
    pub replaced: u32,
    pub text: RichText,
}

pub fn captions() -> impl Scene {
    bsn! {
        Captions
        Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: Val::Px({spacing::M}),
            max_width: Val::Vw(94.0),
        }
        component(Hovered::default())
        ClickThrough
        Pickable { should_block_lower: false, is_hoverable: true }
        Children [
            (
                LineHost
                template_value(scrim_style())
                component(SCRIM_PRESENCE)
                Pickable { should_block_lower: false, is_hoverable: true }
            ),
            ( CountHost Node Pickable { should_block_lower: false, is_hoverable: true } ),
        ]
    }
}

pub(crate) fn sync_captions(
    captions: Query<(Entity, &Captions), Changed<Captions>>,
    mut places: PlaceParts,
    mut commands: Commands,
) {
    for (root, captions) in &captions {
        places.show_lines(&mut commands, root, &captions.rows, ROW_PRESENCE, |line| {
            Box::new(caption_row(line.clone()))
        });
        places.show_count(&mut commands, root, captions.more.clone());
    }
}

#[derive(Component, Default, Clone)]
pub(crate) struct LineHost;

#[derive(Component, Default, Clone)]
pub(crate) struct CountHost;

#[derive(Component, Clone, PartialEq)]
pub(crate) struct ShownLine(LabelledLine);

#[derive(Component, Default, Clone)]
pub(crate) struct CountRow;

#[derive(bevy_ecs::system::SystemParam)]
pub(crate) struct PlaceParts<'w, 's> {
    children: Query<'w, 's, &'static Children>,
    line_hosts: Query<'w, 's, (), With<LineHost>>,
    count_hosts: Query<'w, 's, (), With<CountHost>>,
    count_rows: Query<'w, 's, (), (With<CountRow>, Without<Leaving>)>,
    shown: Query<'w, 's, &'static ShownLine, Without<Leaving>>,
    presences: Query<'w, 's, &'static mut Presence>,
}

impl PlaceParts<'_, '_> {
    fn part(&self, root: Entity, is: impl Fn(Entity) -> bool) -> Option<Entity> {
        self.children
            .get(root)
            .into_iter()
            .flatten()
            .copied()
            .find(|&kid| is(kid))
    }

    pub(crate) fn show_lines(
        &mut self,
        commands: &mut Commands,
        root: Entity,
        lines: &[LabelledLine],
        presence: Presence,
        build: impl Fn(&LabelledLine) -> Box<dyn Scene>,
    ) {
        let Some(host) = self.part(root, |kid| self.line_hosts.contains(kid)) else {
            return;
        };
        if let Ok(mut host_presence) = self.presences.get_mut(host)
            && host_presence.shown == lines.is_empty()
        {
            host_presence.shown = !lines.is_empty();
        }
        let kept: Vec<(Entity, LabelledLine)> = self
            .children
            .get(host)
            .into_iter()
            .flatten()
            .filter_map(|&kid| Some((kid, self.shown.get(kid).ok()?.0.clone())))
            .collect();
        for (kid, shown) in &kept {
            match lines.iter().find(|line| line.key == shown.key) {
                None => {
                    commands.entity(*kid).insert(Leaving);
                }
                Some(line) if line != shown => {
                    for &cell in self.children.get(*kid).into_iter().flatten() {
                        commands.entity(cell).insert(Leaving);
                    }
                    commands.entity(*kid).insert(ShownLine(line.clone()));
                    commands
                        .spawn_scene(cell(build(line)))
                        .insert(ChildOf(*kid));
                }
                Some(_) => {}
            }
        }
        for line in lines {
            if kept.iter().any(|(_, shown)| shown.key == line.key) {
                continue;
            }
            let row = commands
                .spawn_scene(bsn! {
                    component(presence)
                    component(ShownLine(line.clone()))
                    Node { display: Display::Grid, grid_template_columns: {vec![RepeatedGridTrack::flex(1, 1.0)]} }
                    Pickable::IGNORE
                })
                .insert(ChildOf(host))
                .id();
            commands.spawn_scene(cell(build(line))).insert(ChildOf(row));
        }
    }

    pub(crate) fn show_count(
        &mut self,
        commands: &mut Commands,
        root: Entity,
        more: Option<RichText>,
    ) {
        let Some(host) = self.part(root, |kid| self.count_hosts.contains(kid)) else {
            return;
        };
        let row = self.part(host, |kid| self.count_rows.contains(kid));
        match (row, more) {
            (Some(row), Some(more)) => {
                commands.entity(row).despawn_related::<Children>();
                commands.spawn_scene(count_text(more)).insert(ChildOf(row));
            }
            (None, Some(more)) => {
                let row = commands.spawn_scene(count_row()).insert(ChildOf(host)).id();
                commands.spawn_scene(count_text(more)).insert(ChildOf(row));
            }
            (Some(row), None) => {
                commands.entity(row).insert(Leaving);
            }
            (None, None) => {}
        }
    }
}

fn cell(content: Box<dyn Scene>) -> impl Scene {
    bsn! {
        Node {
            flex_direction: FlexDirection::Column,
            grid_row: {GridPlacement::start(1)},
            grid_column: {GridPlacement::start(1)},
        }
        component(SWAP_PRESENCE)
        Pickable::IGNORE
        Children [ {EntityScene(content)} ]
    }
}

pub(crate) fn label_text(label: &str, replaced: u32, color: Color) -> impl Scene + use<> {
    let label = match replaced {
        0 => label.to_uppercase(),
        replaced => format!("{} +{replaced}", label.to_uppercase()),
    };
    styled_text(label, color, typography::LABEL)
}

fn count_row() -> impl Scene {
    let frame = Style::new()
        .background(theme().surface_elevated.base.with_alpha(0.55))
        .node(|node| {
            node.padding = UiRect::axes(Val::Px(spacing::L), Val::Px(spacing::S));
            node.border_radius = BorderRadius::all(Val::Px(radius::M));
        });
    bsn! {
        CountRow
        template_value(frame)
        component(COUNT_PRESENCE)
        Pickable { should_block_lower: false, is_hoverable: true }
    }
}

fn count_text(text: RichText) -> impl Scene {
    rich_text(
        RichText {
            size: typography::CAPTION.font_size,
            color: theme().surface_elevated.on.with_alpha(0.75),
            ..text
        },
        false,
    )
}

fn caption_row(row: LabelledLine) -> impl Scene {
    let ink = palette::SLATE_100;
    let text = RichText {
        size: ROW_SIZE,
        color: ink,
        ..row.text
    };
    bsn! {
        Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: Val::Px({spacing::S}),
        }
        Pickable::IGNORE
        Children [
            {EntityScene(label_text(&row.label, row.replaced, ink.with_alpha(0.7)))},
            (
                Node { max_height: Val::Px({ROW_LINE_HEIGHT * SHOWN_TEXT_LINES}), overflow: {Overflow::clip()} }
                Pickable::IGNORE
                Children [ {EntityScene(rich_text(text, false))} ]
            ),
        ]
    }
}

fn scrim_style() -> Style {
    Style::new()
        .background(Color::BLACK.with_alpha(0.45))
        .node(|node| {
            node.flex_direction = FlexDirection::Column;
            node.align_items = AlignItems::Center;
            node.row_gap = Val::Px(spacing::L);
            node.max_width = Val::Px(WIDEST);
            node.padding = UiRect::axes(Val::Px(spacing::XXL), Val::Px(spacing::L));
            node.border_radius = BorderRadius::all(Val::Px(radius::M));
        })
}
