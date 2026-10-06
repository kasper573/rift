use std::time::Duration;

use bevy::prelude::*;
use bevy::scene::EntityScene;
use ui::tokens::{palette, spacing, typography};
use ui::{RichSpan, RichText, component};

use super::{Notification, NotificationKind, NotificationSent};
use crate::core::time::Seconds;
use crate::systems::dialogue::stage;
use crate::systems::history::RecordTally;
use crate::systems::scene::Scene as GameScene;

const READING_BASE: Seconds = Seconds(2.0);
const READING_PER_WORD: Seconds = Seconds(0.3);
const READING_MIN: Seconds = Seconds(3.0);
const READING_MAX: Seconds = Seconds(6.0);
const EDGE: f32 = 24.0;
const WIDEST: f32 = 320.0;
const NARROWEST: f32 = 140.0;
const ICON: f32 = 16.0;
const MORE_PRESENCE: ui::Presence = ui::Presence::new(
    ui::PresenceMove::new(
        ui::Transform2d::new(Vec2::new(16.0, 0.0), 0.96),
        ui::Timing::new(260, ui::Easing::Standard),
    ),
    ui::PresenceMove::new(
        ui::Transform2d::new(Vec2::new(16.0, 0.0), 0.96),
        ui::Timing::new(160, ui::Easing::StandardAccelerate),
    ),
)
.hidden();

pub struct FeedPlugin;

impl Plugin for FeedPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameScene::Area), spawn_feed)
            .add_systems(
                OnExit(GameScene::Area),
                crate::systems::scene::despawn_all::<FeedRoot>,
            )
            .add_systems(
                Update,
                (count_more, fit_beside_box).run_if(in_state(GameScene::Area)),
            );
    }
}

pub fn show(world: &mut World, notification_sent: NotificationSent) {
    let Some(toaster) = world
        .query_filtered::<Entity, With<FeedToaster>>()
        .iter(world)
        .next()
    else {
        return;
    };
    show_in(world, toaster, notification_sent.notification);
}

pub fn rows(world: &mut World) -> Vec<String> {
    let mut rows: Vec<(Entity, String)> = world
        .query::<(Entity, &FeedRow, &ui::Toast)>()
        .iter(world)
        .filter(|(_, _, toast)| !toast.leaving)
        .map(|(entity, row, _)| (entity, label(&row.0)))
        .collect();
    let order: Vec<Entity> = world
        .query_filtered::<&Children, With<FeedToaster>>()
        .iter(world)
        .flat_map(|children| children.iter().collect::<Vec<_>>())
        .collect();
    rows.sort_by_key(|(entity, _)| order.iter().position(|kid| kid == entity));
    rows.into_iter().map(|(_, label)| label).collect()
}

#[derive(Component, Default, Clone)]
struct FeedRoot;

#[derive(Component, Default, Clone)]
struct FeedToaster;

#[derive(Component, Default, Clone)]
struct MoreChip;

#[derive(Component, Clone)]
struct FeedRow(Notification);

fn spawn_feed(mut commands: Commands) {
    commands.spawn_scene(bsn! {
        FeedRoot
        Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0) }
        Pickable::IGNORE
        GlobalZIndex({ui::tokens::layer::NOTIFICATIONS})
        Children [
            (
                {ui::compact_toaster(WIDEST)}
                FeedToaster
                Children [ {EntityScene(more_chip())} ]
            )
        ]
    });
}

fn show_in(world: &mut World, toaster: Entity, notification: Notification) {
    let alive: Vec<(Entity, Notification)> = world
        .query::<(Entity, &FeedRow, &ui::Toast)>()
        .iter(world)
        .filter(|(_, _, toast)| !toast.leaving)
        .map(|(entity, row, _)| (entity, row.0.clone()))
        .collect();
    let merge = alive
        .into_iter()
        .find_map(|(entity, shown)| merged(&shown, &notification).map(|merged| (entity, merged)));
    if let Some((row, merged)) = merge {
        world.entity_mut(row).despawn_related::<Children>();
        world.entity_mut(row).insert(FeedRow(merged.clone()));
        let assets = world.resource::<AssetServer>().clone();
        if let Ok(mut content) = world.spawn_scene(content(&assets, &merged)) {
            content.insert(ChildOf(row));
        }
        ui::bump_toast(world, row);
        return;
    }
    let lasts = Duration::from_secs_f32(reading_time(&notification).0);
    let failure = matches!(
        notification.kind,
        NotificationKind::Feed { failure: true, .. }
    );
    let border = match failure {
        true => palette::CRIMSON_70,
        false => ui::theme::theme().surface_elevated.border.with_alpha(0.6),
    };
    let body = content(world.resource::<AssetServer>(), &notification);
    let row = bsn! {
        {ui::compact_toast(lasts)}
        component(FeedRow(notification.clone()))
        component(BorderColor::all(border))
        Children [ {EntityScene(body)} ]
    };
    if let Ok(mut spawned) = world.spawn_scene(row) {
        spawned.insert(ChildOf(toaster));
    }
}

fn merged(shown: &Notification, newer: &Notification) -> Option<Notification> {
    let (
        NotificationKind::Feed {
            topic,
            icon,
            tally: Some(old),
            failure,
            ..
        },
        NotificationKind::Feed {
            icon: new_icon,
            tally: Some(new),
            failure: new_failure,
            sfx,
            ..
        },
    ) = (&shown.kind, &newer.kind)
    else {
        return None;
    };
    let same = shown.text == newer.text && icon == new_icon && failure == new_failure;
    let tally = old.merged(*new)?;
    same.then(|| Notification {
        kind: NotificationKind::Feed {
            topic: *topic,
            icon: new_icon.clone(),
            tally: Some(tally),
            failure: *new_failure,
            sfx: *sfx,
        },
        ..newer.clone()
    })
}

fn reading_time(notification: &Notification) -> Seconds {
    let words = notification.text.word_count() + 1;
    let seconds = READING_BASE.0 + READING_PER_WORD.0 * words as f32;
    Seconds(seconds.clamp(READING_MIN.0, READING_MAX.0))
}

fn label(notification: &Notification) -> String {
    let text = super::plain_words(&notification.text);
    match tally_of(notification) {
        Some(RecordTally::Change(change)) => format!("{} {text}", signed(change)),
        Some(RecordTally::Progress { have, need }) => format!("{text} {have}/{need}"),
        None => text,
    }
}

fn tally_of(notification: &Notification) -> Option<RecordTally> {
    match notification.kind {
        NotificationKind::Feed { tally, .. } => tally,
        _ => None,
    }
}

fn signed(change: i32) -> String {
    match change > 0 {
        true => format!("+{change}"),
        false => format!("\u{2212}{}", change.unsigned_abs()),
    }
}

fn content(assets: &AssetServer, notification: &Notification) -> impl Scene + use<> {
    let ink = ui::theme::theme().surface_elevated.on;
    let icon = match &notification.kind {
        NotificationKind::Feed { icon, .. } => icon.clone(),
        _ => None,
    };
    let mut text = notification.text.rich();
    match tally_of(notification) {
        Some(RecordTally::Change(change)) => {
            let color = match (change > 0, icon.is_some()) {
                (true, true) => palette::EMERALD_80,
                (true, false) => palette::AMBER_80,
                (false, _) => ink.with_alpha(0.65),
            };
            let sign = format!("{} ", signed(change));
            text.pieces
                .insert(0, RichSpan::plain(sign).color(color).into());
        }
        Some(RecordTally::Progress { have, need }) => {
            text.pieces
                .push(RichSpan::plain(format!(" {have}/{need}")).into());
        }
        None => {}
    }
    let line = ui::rich_text(
        RichText {
            size: typography::CAPTION.font_size,
            color: ink,
            ..text
        },
        false,
    );
    let icon: Vec<Box<dyn Scene>> = icon
        .map(|icon| -> Box<dyn Scene> {
            Box::new(bsn! {
                Node { width: Val::Px(ICON), height: Val::Px(ICON), flex_shrink: 0.0 }
                component(ImageNode::new(assets.load(icon)))
                Pickable::IGNORE
            })
        })
        .into_iter()
        .collect();
    bsn! {
        Node { column_gap: Val::Px({spacing::M}), align_items: AlignItems::Center, min_width: Val::Px(0.0) }
        Pickable::IGNORE
        Children [ {icon}, {EntityScene(line)} ]
    }
}

fn more_chip() -> impl Scene {
    let surface = ui::theme::theme().surface_elevated;
    let frame = ui::Style::new()
        .background(surface.base.with_alpha(0.55))
        .node(|node| {
            node.padding = UiRect::axes(Val::Px(spacing::L), Val::Px(spacing::S));
            node.border_radius = BorderRadius::all(Val::Px(ui::tokens::radius::M));
        });
    bsn! {
        MoreChip
        template_value(frame)
        component(MORE_PRESENCE)
        ui::ClickThrough
        Pickable { should_block_lower: false, is_hoverable: true }
    }
}

fn count_more(world: &mut World) {
    let Some((toaster, overflow)) = world
        .query_filtered::<(Entity, &ui::Toaster), With<FeedToaster>>()
        .iter(world)
        .next()
        .map(|(entity, toaster)| (entity, toaster.overflow))
    else {
        return;
    };
    let Some(chip) = world.get::<Children>(toaster).and_then(|kids| {
        kids.iter()
            .find(|&kid| world.get::<MoreChip>(kid).is_some())
    }) else {
        return;
    };
    let shown = world.get::<ShownMore>(chip).map(|shown| shown.0);
    if shown == Some(overflow) {
        return;
    }
    world.entity_mut(chip).insert(ShownMore(overflow));
    if let Some(mut presence) = world.get_mut::<ui::Presence>(chip) {
        presence.shown = overflow > 0;
    }
    if overflow == 0 {
        return;
    }
    world.entity_mut(chip).despawn_related::<Children>();
    let ink = ui::theme::theme().surface_elevated.on.with_alpha(0.75);
    let text = ui::rich_text(
        RichText {
            size: typography::CAPTION.font_size,
            color: ink,
            ..super::rows::more_text(overflow as u32)
        },
        false,
    );
    if let Ok(mut spawned) = world.spawn_scene(text) {
        spawned.insert(ChildOf(chip));
    }
}

#[derive(Component)]
struct ShownMore(usize);

fn fit_beside_box(world: &mut World) {
    let Ok(screen) = world
        .query::<&bevy::window::Window>()
        .single(world)
        .map(|window| window.resolution.width())
    else {
        return;
    };
    let room = stage::box_right(world).map_or(WIDEST, |right| {
        (screen - right - 2.0 * EDGE).clamp(NARROWEST, WIDEST)
    });
    let mut toasters = world.query_filtered::<&mut Node, With<FeedToaster>>();
    for mut node in toasters.iter_mut(world) {
        if node.max_width != Val::Px(room) {
            node.max_width = Val::Px(room);
        }
    }
}
