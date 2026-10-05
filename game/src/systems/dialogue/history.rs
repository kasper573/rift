use bevy::prelude::*;
use bevy::scene::EntityScene;
use ui::tokens::{palette, spacing, typography};
use ui::{RichSpan, RichText};

use super::text::LineText;
use crate::core::sfx::SfxId;
use crate::core::sfx::playback::{PlaySfx, SfxPlace};
use crate::systems::hud::reconcile_children;
use crate::systems::notice::{Notice, NoticeTone};
use crate::systems::scene::Scene as GameScene;

const KEPT: usize = 200;
const WIDTH: f32 = 920.0;

pub struct HistoryPlugin;

impl Plugin for HistoryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ConversationHistory>()
            .add_systems(
                Update,
                (
                    note_notices,
                    toggle_key.run_if(not(ui::typing)),
                    show_panel,
                    sync_log,
                )
                    .chain()
                    .run_if(in_state(GameScene::Area)),
            )
            .add_systems(
                OnExit(GameScene::Area),
                (crate::systems::scene::despawn_all::<HistoryPanel>, forget),
            );
    }
}

#[derive(Clone)]
pub enum HistoryEntry {
    Began(String),
    Said {
        who: Option<String>,
        text: LineText,
    },
    Announced {
        who: String,
        text: LineText,
        missed: bool,
    },
    Picked(LineText),
    Noted(String, NoticeTone),
}

#[derive(Resource, Default)]
pub struct ConversationHistory {
    entries: Vec<(u64, HistoryEntry)>,
    next: u64,
    open: bool,
}

impl ConversationHistory {
    fn push(&mut self, entry: HistoryEntry) {
        self.entries.push((self.next, entry));
        self.next += 1;
        let overflow = self.entries.len().saturating_sub(KEPT);
        self.entries.drain(..overflow);
    }
}

pub fn record(world: &mut World, entry: HistoryEntry) {
    world.resource_mut::<ConversationHistory>().push(entry);
}

pub fn toggle(world: &mut World) {
    let mut history = world.resource_mut::<ConversationHistory>();
    history.open = !history.open;
    if history.open {
        world.write_message(PlaySfx {
            id: SfxId::UiPage,
            place: SfxPlace::Interface,
        });
    }
}

pub fn close(world: &mut World) -> bool {
    let mut history = world.resource_mut::<ConversationHistory>();
    std::mem::replace(&mut history.open, false)
}

pub fn is_open(world: &World) -> bool {
    world.resource::<ConversationHistory>().open
}

#[derive(Component, Default, Clone)]
struct HistoryPanel;

#[derive(Component, Default, Clone)]
struct HistoryLog;

fn note_notices(mut notices: MessageReader<Notice>, mut history: ResMut<ConversationHistory>) {
    for notice in notices.read() {
        history.push(HistoryEntry::Noted(notice.text.clone(), notice.tone));
    }
}

fn toggle_key(keys: Res<ButtonInput<KeyCode>>, mut commands: Commands) {
    if keys.just_pressed(KeyCode::KeyH) {
        commands.queue(toggle);
    }
}

fn show_panel(
    history: Res<ConversationHistory>,
    panels: Query<Entity, With<HistoryPanel>>,
    mut commands: Commands,
) {
    if !history.is_changed() {
        return;
    }
    match (history.open, panels.single()) {
        (true, Err(_)) => {
            commands.spawn_scene(panel());
        }
        (false, Ok(panel)) => commands.entity(panel).despawn(),
        _ => {}
    }
}

fn sync_log(world: &mut World) {
    let Ok(log) = world
        .query_filtered::<Entity, With<HistoryLog>>()
        .single(world)
    else {
        return;
    };
    let entries = world.resource::<ConversationHistory>().entries.clone();
    let keys: Vec<u64> = entries.iter().map(|(key, _)| *key).collect();
    reconcile_children(world, log, &keys, |index| entry(&entries[index].1));
}

fn forget(mut history: ResMut<ConversationHistory>) {
    *history = ConversationHistory::default();
}

fn panel() -> impl Scene {
    let surface = ui::theme::theme().surface_trough;
    let frame = ui::Style::new()
        .background(surface.base.with_alpha(0.96))
        .border_color(surface.on.with_alpha(0.55))
        .node(|node| {
            node.position_type = PositionType::Absolute;
            node.top = Val::Px(spacing::XL);
            node.left = Val::Percent(50.0);
            node.margin = UiRect::left(Val::Px(-WIDTH / 2.0));
            node.width = Val::Px(WIDTH);
            node.max_width = Val::Vw(94.0);
            node.height = Val::Vh(52.0);
            node.flex_direction = FlexDirection::Column;
            node.border = UiRect::all(Val::Px(2.0));
            node.border_radius = BorderRadius::all(Val::Px(ui::tokens::radius::M));
        });
    let close_button =
        ui::button_styled(ui::button::intent::SECONDARY, ui::ButtonSize::Sm, "Close");
    bsn! {
        HistoryPanel
        template_value(frame)
        Pickable { should_block_lower: true, is_hoverable: true }
        Children [
            (
                Node {
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    padding: {UiRect::axes(Val::Px(spacing::XL), Val::Px(spacing::L))},
                }
                Children [
                    {EntityScene(ui::styled_text("History", surface.on, typography::NAME))},
                    (
                        {close_button}
                        on(|_: On<ui::Activate>, mut commands: Commands| {
                            commands.queue(|world: &mut World| {
                                close(world);
                            });
                        })
                    ),
                ]
            ),
            ( Node { flex_grow: 1.0, min_height: Val::Px(0.0), width: Val::Percent(100.0) }
              Children [
                ( {ui::scroll_area()}
                  Children [
                    ( {ui::scroll_viewport()}
                      {ui::component(ui::PinToBottom::default())}
                      Children [
                        (
                            HistoryLog
                            Node {
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px({spacing::M}),
                                width: Val::Percent(100.0),
                                padding: {UiRect::axes(Val::Px(spacing::XL), Val::Px(spacing::L))},
                            }
                        )
                      ]
                    ),
                    ( {ui::scroll_bar()} Children [ {EntityScene(ui::scroll_thumb())} ] )
                  ]
                )
              ]
            ),
        ]
    }
}

fn entry(entry: &HistoryEntry) -> Box<dyn Scene> {
    let ink = ui::theme::theme().surface_trough.on;
    let text = match entry {
        HistoryEntry::Began(with) => {
            return Box::new(ui::styled_text(
                format!("— {with} —"),
                ink.with_alpha(0.5),
                typography::CAPTION,
            ));
        }
        HistoryEntry::Said { who, text } => {
            let mut said = text.rich();
            if let Some(who) = who {
                said.pieces.insert(
                    0,
                    RichSpan::plain(format!("{who}  "))
                        .color(palette::AMBER_80)
                        .into(),
                );
            }
            said
        }
        HistoryEntry::Announced { who, text, missed } => {
            let mut announced = text.rich();
            let tag = if *missed { " (missed)" } else { "" };
            announced.pieces.insert(
                0,
                RichSpan::plain(format!("{who}{tag}  "))
                    .color(palette::VIOLET_80)
                    .into(),
            );
            announced
        }
        HistoryEntry::Picked(label) => {
            let mut picked = label.rich();
            picked
                .pieces
                .insert(0, RichSpan::plain("You  ").color(palette::AZURE_80).into());
            picked
        }
        HistoryEntry::Noted(note, tone) => RichText::new(vec![
            RichSpan::plain(note.clone())
                .color(match tone {
                    NoticeTone::Bad => palette::CRIMSON_80,
                    NoticeTone::Good => palette::EMERALD_80,
                    NoticeTone::Info => ink.with_alpha(0.6),
                })
                .into(),
        ]),
    };
    Box::new(ui::rich_text(
        RichText {
            size: typography::BODY.font_size,
            color: ink,
            ..text
        },
        false,
    ))
}
