use std::collections::HashSet;
use std::time::Duration;

use bevy::prelude::*;
use bevy_scene::{CommandsSceneExt, EntityScene, Scene, bsn, on, template_value};
use ui::button::intent as button_intent;
use ui::card::intent as card_intent;
use ui::theme::theme;
use ui::tokens::palette;
use ui::{
    Align, Announcement, AnnouncementKind, AnnouncementLane, ButtonIntent, ButtonSize, CardOptions,
    Carriable, Carried, CarryTarget, CastDepth, CastMember, Check, ChipOptions, ChoiceOptions,
    ConfirmOptions, DialogueBoxOptions, MotionPreference, OnSettle, OnTap, Orientation, RichPiece,
    RichSpan, RichText, Side, SonnerPosition, TextMotion, TextVoice, Typewriter, WidgetOptions,
    accordion, accordion_body, accordion_content, accordion_header, accordion_item,
    accordion_trigger, alert_dialog, alert_dialog_action, alert_dialog_cancel, announcement_lane,
    avatar, avatar_fallback, button, button_styled, card, cast, checkbox, checkbox_indicator, chip,
    choice_list, collapsible, collapsible_content, collapsible_trigger, component, confirm_dialog,
    dialog, dialog_close, dialogue_box, key_hint, list_header, popover, popover_content,
    popover_trigger, progress, progress_indicator, radio_circle, radio_group, radio_indicator,
    radio_item, rich_text, scroll_area, scroll_bar, scroll_thumb, scroll_viewport, separator,
    slider, slider_range, slider_thumb, slider_track, sonner_close, split_view, switch,
    switch_thumb, tabs, tabs_list, tabs_trigger, text, text_colored, toast, toaster, tooltip,
    tooltip_content, widget, window,
};

const WINDOW: Vec2 = Vec2::new(1600.0, 900.0);

const TOAST_MESSAGES: &[(&str, &str)] = &[
    ("Event created", "Monday, January 6 at 9:00 AM"),
    ("Changes saved", "Your project is up to date."),
    ("Copied to clipboard", "The share link is ready."),
    ("Upload complete", "report-q3.pdf finished uploading."),
];

const SLOTS: [(f32, f32, Side); 5] = [
    (0.5, 0.02, Side::Top),
    (0.94, 0.5, Side::Right),
    (0.5, 0.95, Side::Bottom),
    (0.03, 0.5, Side::Left),
    (0.5, 0.5, Side::Bottom),
];
const FLIP_NOTE: &str = "This floating panel flips to the opposite side when its preferred side would overflow the viewport.";

#[derive(Resource)]
struct CurrentScene(usize);

#[derive(Component, Default, Clone)]
struct GalleryRoot;

#[derive(Component, Default, Clone)]
struct SceneRoot;

#[derive(Component, Default, Clone)]
struct SceneTab(usize);

#[derive(Component, Default, Clone)]
struct ToasterEntity;

fn main() {
    let opened = std::env::args().nth(1).map_or(0, |name| scene_index(&name));
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "rift ui gallery".to_owned(),
                        resolution: WINDOW.as_uvec2().into(),
                        ..default()
                    }),
                    ..default()
                })
                .set(bevy::asset::AssetPlugin {
                    file_path: concat!(env!("CARGO_MANIFEST_DIR"), "/../assets").to_owned(),
                    ..default()
                }),
        )
        .insert_resource(ClearColor(theme().surface_inset.base))
        .insert_resource(CurrentScene(opened))
        .add_plugins(ui::UiPlugin)
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                rebuild_scene,
                animate_progress,
                stage_keys,
                run_down_lanes,
                rotate_cast,
            ),
        )
        .run();
}

fn boxed(scene: impl Scene + 'static) -> Box<dyn Scene> {
    Box::new(scene)
}

fn scene_index(name: &str) -> usize {
    SCENES
        .iter()
        .position(|(scene, _)| scene.eq_ignore_ascii_case(name))
        .unwrap_or_else(|| {
            let names: Vec<&str> = SCENES.iter().map(|(scene, _)| *scene).collect();
            panic!("no gallery scene {name:?}, pick one of {names:?}")
        })
}

fn setup(current: Res<CurrentScene>, assets: Res<AssetServer>, mut commands: Commands) {
    let _ = GALLERY_ASSETS.set(assets.clone());
    commands.spawn((Camera2d, IsDefaultUiCamera));
    let tab_buttons: Vec<Box<dyn Scene>> = SCENES
        .iter()
        .enumerate()
        .map(|(index, (name, _))| {
            boxed(bsn! {
                {tabs_trigger(index.to_string())}
                SceneTab({index})
                on(on_tab)
                Children [ {EntityScene(text(*name))} ]
            })
        })
        .collect();
    commands.spawn_scene(bsn! {
        GalleryRoot
        Node { width: Val::Percent(100.0), height: Val::Percent(100.0), flex_direction: FlexDirection::Column }
        Children [
            ( {tabs(Some(current.0.to_string()))}
              Children [
                ( Node {
                      flex_direction: FlexDirection::Row,
                      flex_wrap: FlexWrap::Wrap,
                      width: Val::Percent(100.0),
                      column_gap: Val::Px(4.0),
                      row_gap: Val::Px(4.0),
                      padding: {UiRect::all(Val::Px(12.0))},
                  }
                  Children [ {tab_buttons} ]
                )
              ]
            )
        ]
    });
}

fn animate_progress(time: Res<Time>, mut fractions: Query<&mut ui::ProgressFraction>) {
    let fraction = (time.elapsed_secs() % 2.5) / 2.5;
    for mut progress in &mut fractions {
        progress.0 = fraction;
    }
}

fn on_tab(event: On<ui::Activate>, tabs: Query<&SceneTab>, mut current: ResMut<CurrentScene>) {
    if let Ok(tab) = tabs.get(event.entity) {
        current.0 = tab.0;
    }
}

fn rebuild_scene(
    current: Res<CurrentScene>,
    root: Query<Entity, With<GalleryRoot>>,
    scenes: Query<Entity, With<SceneRoot>>,
    mut shown: Local<Option<usize>>,
    mut commands: Commands,
) {
    if *shown == Some(current.0) {
        return;
    }
    *shown = Some(current.0);
    let Ok(root) = root.single() else {
        return;
    };
    for scene in &scenes {
        commands.entity(scene).despawn();
    }
    let content = (SCENES[current.0].1)();
    commands
        .spawn_scene(bsn! {
            SceneRoot
            Node {
                flex_grow: 1.0,
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
            }
            Children [ {EntityScene(content)} ]
        })
        .insert(ChildOf(root));
}

fn show_toast(
    _event: On<ui::Activate>,
    toasters: Query<Entity, With<ToasterEntity>>,
    mut next: Local<usize>,
    mut commands: Commands,
) {
    let Ok(toaster) = toasters.single() else {
        return;
    };
    let (title, body) = TOAST_MESSAGES[*next % TOAST_MESSAGES.len()];
    *next += 1;
    commands
        .spawn_scene(bsn! {
            {toast()}
            Children [
                ( Node {
                      flex_direction: FlexDirection::Row,
                      justify_content: JustifyContent::SpaceBetween,
                      align_items: AlignItems::Center,
                      column_gap: Val::Px(12.0),
                  }
                  Children [
                    {EntityScene(text(title))},
                    ( {sonner_close()}
                      Children [ {EntityScene(button_styled(button_intent::SECONDARY, ButtonSize::Sm, "close"))} ]
                    )
                  ]
                ),
                {EntityScene(text_colored(body, theme().surface_canvas.on))}
            ]
        })
        .insert(ChildOf(toaster));
}

type SceneBuilder = fn() -> Box<dyn Scene>;

const SCENES: &[(&str, SceneBuilder)] = &[
    ("Button intents", button_intents_scene),
    ("Button sizes", button_sizes_scene),
    ("Tabs", tabs_scene),
    ("Checkbox", checkbox_scene),
    ("Switch", switch_scene),
    ("Radio group", radio_scene),
    ("Slider", slider_scene),
    ("Progress", progress_scene),
    ("Avatar", avatar_scene),
    ("Separator", separator_scene),
    ("Accordion", accordion_scene),
    ("Collapsible", collapsible_scene),
    ("Dialog", dialog_scene),
    ("Alert dialog", alert_dialog_scene),
    ("Card", card_scene),
    ("Tooltip", tooltip_scene),
    ("Popover", popover_scene),
    ("Tooltip + card", tooltip_card_scene),
    ("Popover + card", popover_card_scene),
    ("Toasts (sonner)", toasts_scene),
    ("Scroll area", scroll_area_scene),
    ("Pinned scroll", pinned_scroll_scene),
    ("Text input", text_input_scene),
    ("Widget", widget_scene),
    ("Window", window_scene),
    ("Rich text", rich_text_scene),
    ("Typewriter", typewriter_scene),
    ("Chips", chips_scene),
    ("Choices", choices_scene),
    ("Dialogue box", dialogue_box_scene),
    ("Cast", cast_scene),
    ("Banners", banners_scene),
    ("Toasts (top center)", top_toasts_scene),
    ("List detail", list_detail_scene),
    ("Confirm dialog", confirm_dialog_scene),
    ("Drag and drop", drag_and_drop_scene),
];

const BUTTON_INTENTS: &[(ButtonIntent, &str)] = &[
    (button_intent::PRIMARY, "primary"),
    (button_intent::SECONDARY, "secondary"),
    (button_intent::DANGER, "danger"),
    (button_intent::MUTED, "muted"),
    (button_intent::PLAIN, "plain"),
];

const BUTTON_SIZES: &[(ButtonSize, &str)] = &[
    (ButtonSize::Sm, "sm"),
    (ButtonSize::Md, "md"),
    (ButtonSize::Lg, "lg"),
];

fn wrap(kids: Vec<Box<dyn Scene>>) -> Box<dyn Scene> {
    boxed(bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            column_gap: Val::Px(18.0),
            row_gap: Val::Px(18.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            max_width: Val::Px(1360.0),
        }
        Children [ {kids} ]
    })
}

fn col(width: f32, kids: Vec<Box<dyn Scene>>) -> Box<dyn Scene> {
    boxed(bsn! {
        Node { width: Val::Px({width}), flex_direction: FlexDirection::Column, align_items: AlignItems::Center }
        Children [ {kids} ]
    })
}

fn button_intents_scene() -> Box<dyn Scene> {
    let mut buttons: Vec<Box<dyn Scene>> = BUTTON_INTENTS
        .iter()
        .map(|&(intent, label)| boxed(button_styled(intent, ButtonSize::Md, label)))
        .collect();
    buttons.push(boxed(bsn! {
        {button_styled(button_intent::PRIMARY, ButtonSize::Md, "disabled")}
        bevy::ui::InteractionDisabled
    }));
    wrap(buttons)
}

fn button_sizes_scene() -> Box<dyn Scene> {
    wrap(
        BUTTON_SIZES
            .iter()
            .map(|&(size, label)| boxed(button_styled(button_intent::PRIMARY, size, label)))
            .collect(),
    )
}

fn tabs_scene() -> Box<dyn Scene> {
    col(
        520.0,
        vec![boxed(bsn! {
            {tabs(Some("overview".to_owned()))}
            Children [
                ( {tabs_list()}
                  Children [
                    ( {tabs_trigger("overview")} Children [ {EntityScene(text("Overview"))} ] ),
                    ( {tabs_trigger("activity")} Children [ {EntityScene(text("Activity"))} ] ),
                    ( {tabs_trigger("settings")} Children [ {EntityScene(text("Settings"))} ] ),
                  ]
                )
            ]
        })],
    )
}

fn checkbox_scene() -> Box<dyn Scene> {
    wrap(vec![
        boxed(bsn! { {checkbox(Check::Off)} Children [ {EntityScene(checkbox_indicator())} ] }),
        boxed(bsn! { {checkbox(Check::On)} Children [ {EntityScene(checkbox_indicator())} ] }),
        boxed(bsn! {
            {checkbox(Check::Indeterminate)}
            Children [
                ( Node { width: Val::Px(10.0), height: Val::Px(2.0), border_radius: {BorderRadius::all(Val::Px(999.0))} }
                  template_value(ui::Style::new().background(theme().primary.on))
                )
            ]
        }),
    ])
}

fn switch_scene() -> Box<dyn Scene> {
    wrap(vec![
        boxed(bsn! { {switch(false)} Children [ {EntityScene(switch_thumb())} ] }),
        boxed(bsn! { {switch(true)} Children [ {EntityScene(switch_thumb())} ] }),
    ])
}

fn radio_scene() -> Box<dyn Scene> {
    let items = ["apple", "banana", "cherry"].map(|name| {
        let label = format!("{}{}", name[..1].to_uppercase(), &name[1..]);
        boxed(bsn! {
            {radio_item(name)}
            Children [
                ( {radio_circle()} Children [ {EntityScene(radio_indicator())} ] ),
                {EntityScene(text(label))}
            ]
        })
    });
    col(
        240.0,
        vec![boxed(bsn! {
            {radio_group(Some("apple".to_owned()))}
            Children [ {items.into_iter().collect::<Vec<_>>()} ]
        })],
    )
}

fn slider_scene() -> Box<dyn Scene> {
    col(
        360.0,
        vec![boxed(bsn! {
            {slider(35.0, 0.0, 100.0)}
            Children [
                ( {slider_track()}
                  Children [ {EntityScene(slider_range())}, {EntityScene(slider_thumb())} ]
                )
            ]
        })],
    )
}

fn progress_scene() -> Box<dyn Scene> {
    col(
        360.0,
        vec![boxed(bsn! {
            {progress(0.0, 100.0)}
            Children [ {EntityScene(progress_indicator())} ]
        })],
    )
}

fn avatar_scene() -> Box<dyn Scene> {
    let one = |initials: &'static str| {
        boxed(bsn! {
            {avatar()}
            Children [
                ( {avatar_fallback()} Children [ {EntityScene(text_colored(initials, theme().primary.on))} ] )
            ]
        })
    };
    wrap(vec![one("KS"), one("AB")])
}

fn separator_scene() -> Box<dyn Scene> {
    col(
        360.0,
        vec![
            boxed(text("Above")),
            boxed(separator(Orientation::Horizontal)),
            boxed(text("Below")),
        ],
    )
}

fn accordion_scene() -> Box<dyn Scene> {
    let item = |value: &'static str, q: &'static str, a: &'static str| {
        boxed(bsn! {
            {accordion_item()}
            Children [
                ( {accordion_header()}
                  Children [ ( {accordion_trigger(value)} Children [ {EntityScene(text(q))} ] ) ]
                ),
                ( {accordion_content(value)}
                  Children [ ( {accordion_body()} Children [ {EntityScene(text_colored(a, theme().surface_canvas.on))} ] ) ]
                )
            ]
        })
    };
    col(
        440.0,
        vec![boxed(bsn! {
            {accordion(HashSet::from(["shipping".to_owned()]), false)}
            Children [
                {EntityScene(item("shipping", "Is shipping free?", "Yes, on orders over $50."))},
                {EntityScene(item("returns", "Can I return it?", "Within 30 days, no questions."))},
                {EntityScene(item("styled", "Is it themed?", "Every color comes from the theme."))}
            ]
        })],
    )
}

fn collapsible_scene() -> Box<dyn Scene> {
    col(
        360.0,
        vec![boxed(bsn! {
            {collapsible(false)}
            Children [
                ( {collapsible_trigger()} Children [ {EntityScene(text("Notification settings"))} ] ),
                ( {collapsible_content()}
                  Children [ {EntityScene(text_colored("Email me about replies and mentions.", theme().surface_canvas.on))} ]
                )
            ]
        })],
    )
}

fn dialog_actions(kids: Vec<Box<dyn Scene>>) -> Box<dyn Scene> {
    boxed(bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(12.0),
            row_gap: Val::Px(12.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::FlexEnd,
            flex_wrap: FlexWrap::Wrap,
            max_width: Val::Px(1360.0),
        }
        Children [ {kids} ]
    })
}

fn dialog_scene() -> Box<dyn Scene> {
    boxed(dialog(
        false,
        button_styled(button_intent::PRIMARY, ButtonSize::Md, "Delete project"),
        bsn! {
            Children [
                {EntityScene(text("Delete project?"))},
                {EntityScene(text_colored("This permanently removes the project and its data.", theme().surface_canvas.on))},
                {EntityScene(dialog_actions(vec![
                    boxed(bsn! {
                        {dialog_close()}
                        Children [ {EntityScene(button_styled(button_intent::PLAIN, ButtonSize::Md, "Cancel"))} ]
                    }),
                    boxed(button_styled(button_intent::DANGER, ButtonSize::Md, "Delete")),
                ]))}
            ]
        },
    ))
}

fn alert_dialog_scene() -> Box<dyn Scene> {
    boxed(alert_dialog(
        false,
        button_styled(button_intent::DANGER, ButtonSize::Md, "Reset everything"),
        bsn! {
            Children [
                {EntityScene(text("Are you absolutely sure?"))},
                {EntityScene(text_colored("This action cannot be undone.", theme().surface_canvas.on))},
                {EntityScene(dialog_actions(vec![
                    boxed(bsn! {
                        {alert_dialog_cancel()}
                        Children [ {EntityScene(button_styled(button_intent::PLAIN, ButtonSize::Md, "Cancel"))} ]
                    }),
                    boxed(bsn! {
                        {alert_dialog_action()}
                        Children [ {EntityScene(button_styled(button_intent::PRIMARY, ButtonSize::Md, "Continue"))} ]
                    }),
                ]))}
            ]
        },
    ))
}

fn card_scene() -> Box<dyn Scene> {
    let variants: [(&str, &str, Color, CardOptions); 8] = [
        (
            "Surface",
            "Default, bordered",
            theme().surface_elevated.on,
            CardOptions::default(),
        ),
        (
            "Floating",
            "Elevation shadow",
            theme().surface_elevated.on,
            CardOptions {
                floating: true,
                ..default()
            },
        ),
        (
            "Compact",
            "Tighter padding",
            theme().surface_elevated.on,
            CardOptions {
                compact: true,
                ..default()
            },
        ),
        (
            "Interactive",
            "Hover & press me",
            theme().surface_elevated.on,
            CardOptions {
                interactive: true,
                ..default()
            },
        ),
        (
            "Floating + interactive",
            "Lifts higher on hover",
            theme().surface_elevated.on,
            CardOptions {
                floating: true,
                interactive: true,
                ..default()
            },
        ),
        (
            "Success",
            "Intent color",
            theme().success_soft.on,
            CardOptions {
                intent: card_intent::SUCCESS,
                ..default()
            },
        ),
        (
            "Error",
            "Intent color",
            theme().error_soft.on,
            CardOptions {
                intent: card_intent::ERROR,
                ..default()
            },
        ),
        (
            "Info",
            "Intent color",
            theme().info_soft.on,
            CardOptions {
                intent: card_intent::INFO,
                ..default()
            },
        ),
    ];
    wrap(variants
        .into_iter()
        .map(|(title, desc, on, opts)| {
            boxed(bsn! {
                {card(opts)}
                Children [
                    ( Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(4.0), width: Val::Px(150.0) }
                      Children [
                        {EntityScene(text_colored(title, on))},
                        {EntityScene(text_colored(desc, on))}
                      ]
                    )
                ]
            })
        })
        .collect())
}

fn floating(make: impl Fn(Side) -> Box<dyn Scene>) -> Box<dyn Scene> {
    let slots: Vec<Box<dyn Scene>> = SLOTS
        .iter()
        .map(|&(fx, fy, side)| {
            let left = (fx - 0.5) * WINDOW.x;
            let top = (fy - 0.5) * WINDOW.y;
            boxed(bsn! {
                Node { position_type: PositionType::Absolute, left: Val::Px({left}), top: Val::Px({top}) }
                Children [ ( Node Children [ {EntityScene(make(side))} ] ) ]
            })
        })
        .collect();
    boxed(bsn! {
        Node { position_type: PositionType::Relative }
        Children [ {slots} ]
    })
}

fn tooltip_overlay(side: Side, panel: Box<dyn Scene>) -> Box<dyn Scene> {
    boxed(bsn! {
        {tooltip(false)}
        Children [
            {EntityScene(button_styled(button_intent::PRIMARY, ButtonSize::Md, "Hover me"))},
            ( {tooltip_content(side, Align::Center, 8.0)} Children [ {EntityScene(panel)} ] )
        ]
    })
}

fn popover_overlay(side: Side, panel: Box<dyn Scene>) -> Box<dyn Scene> {
    boxed(bsn! {
        {popover(false)}
        Children [
            ( {popover_trigger()} Children [ {EntityScene(button("Open"))} ] ),
            ( {popover_content(side, Align::Center, 8.0)} Children [ {EntityScene(panel)} ] )
        ]
    })
}

fn note_panel() -> Box<dyn Scene> {
    boxed(bsn! {
        Node { width: Val::Px(220.0), padding: {UiRect::all(Val::Px(12.0))} }
        Children [ {EntityScene(text(FLIP_NOTE))} ]
    })
}

fn dimensions_panel() -> Box<dyn Scene> {
    boxed(bsn! {
        Node { width: Val::Px(220.0), flex_direction: FlexDirection::Column, row_gap: Val::Px(8.0), padding: {UiRect::all(Val::Px(12.0))} }
        Children [
            {EntityScene(text("Dimensions"))},
            {EntityScene(text_colored(FLIP_NOTE, theme().surface_canvas.on))}
        ]
    })
}

fn in_card(panel: Box<dyn Scene>) -> Box<dyn Scene> {
    boxed(bsn! {
        {card(CardOptions { floating: true, ..default() })}
        Children [ {EntityScene(panel)} ]
    })
}

fn tooltip_scene() -> Box<dyn Scene> {
    floating(|side| tooltip_overlay(side, note_panel()))
}

fn popover_scene() -> Box<dyn Scene> {
    floating(|side| popover_overlay(side, dimensions_panel()))
}

fn tooltip_card_scene() -> Box<dyn Scene> {
    floating(|side| tooltip_overlay(side, in_card(note_panel())))
}

fn popover_card_scene() -> Box<dyn Scene> {
    floating(|side| popover_overlay(side, in_card(dimensions_panel())))
}

fn toasts_scene() -> Box<dyn Scene> {
    boxed(bsn! {
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
        }
        Children [
            ( {button("Show toast")} on(show_toast) ),
            ( {toaster(SonnerPosition::BottomRight)} ToasterEntity )
        ]
    })
}

fn top_toasts_scene() -> Box<dyn Scene> {
    boxed(bsn! {
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
        }
        Children [
            ( {button("Show toast")} on(show_toast) ),
            ( {toaster(SonnerPosition::TopCenter)} ToasterEntity )
        ]
    })
}

fn widget_scene() -> Box<dyn Scene> {
    col(
        160.0,
        vec![boxed(bsn! {
            Node {
                width: Val::Px(160.0),
                height: Val::Px(120.0),
                position_type: PositionType::Relative,
            }
            Children [
                {EntityScene(widget(WidgetOptions {
                    pos: Vec2::new(56.0, 36.0),
                    icon: Handle::default(),
                    badge: "I".into(),
                    tooltip: "Inventory".into(),
                    on_tap: OnTap::new(|_| {}),
                    on_settle: OnSettle::new(|_, geom| geom),
                }))}
            ]
        })],
    )
}

#[derive(Component, Default, Clone)]
struct PinnedLog;

#[derive(Component, Default, Clone)]
struct SubmittedText;

fn pinned_scroll_scene() -> Box<dyn Scene> {
    let mut items: Vec<Box<dyn Scene>> = (1..=30)
        .map(|n| {
            boxed(text_colored(
                format!("Log line {n}"),
                theme().surface_canvas.on,
            ))
        })
        .collect();
    items.push(boxed(text_colored(
        "A deliberately long log line that must wrap onto several rows instead of overflowing the scroll area horizontally",
        theme().surface_canvas.on,
    )));
    col(
        320.0,
        vec![
            boxed(bsn! {
                Node { width: Val::Px(300.0), height: Val::Px(200.0) }
                Children [
                    ( {scroll_area()}
                      Children [
                        ( {scroll_viewport()}
                          {ui::component(ui::PinToBottom::default())}
                          Children [
                            ( Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(6.0), width: Val::Percent(100.0), padding: {UiRect::all(Val::Px(8.0))} }
                              PinnedLog
                              Children [ {items} ]
                            )
                          ]
                        ),
                        ( {scroll_bar()} Children [ {EntityScene(scroll_thumb())} ] )
                      ]
                    )
                ]
            }),
            boxed(bsn! {
                {button_styled(button_intent::PRIMARY, ButtonSize::Md, "append line")}
                on(|_: On<ui::Activate>, logs: Query<(Entity, Option<&Children>), With<PinnedLog>>, mut commands: Commands| {
                    for (log, children) in &logs {
                        let line = children.map_or(0, Children::len) + 1;
                        commands
                            .spawn_scene(text_colored(
                                format!("Appended line {line} which is deliberately long enough that it must wrap onto several rows instead of overflowing"),
                                theme().surface_canvas.on,
                            ))
                            .insert(ChildOf(log));
                    }
                })
            }),
        ],
    )
}

fn text_input_scene() -> Box<dyn Scene> {
    col(
        360.0,
        vec![boxed(bsn! {
            Node {
                width: Val::Px(320.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
            }
            Children [
                {EntityScene(ui::text_input(ui::TextInputOptions {
                    on_submit: ui::OnSubmit::new(|world, submitted| {
                        let mut labels = world.query_filtered::<&mut Text, With<SubmittedText>>();
                        for mut label in labels.iter_mut(world) {
                            label.0 = format!("submitted: {submitted}");
                        }
                    }),
                }))},
                ( {text("submitted: —")} SubmittedText )
            ]
        })],
    )
}

fn window_scene() -> Box<dyn Scene> {
    col(
        360.0,
        vec![boxed(bsn! {
            Node {
                width: Val::Px(520.0),
                height: Val::Px(300.0),
                position_type: PositionType::Relative,
            }
            Children [
                {EntityScene(window(ui::WindowOptions {
                    pos: Vec2::ZERO,
                    size: Vec2::new(520.0, 300.0),
                    on_close: OnTap::new(|_| {}),
                    on_settle: OnSettle::new(|_, geom| geom),
                    content: vec![
                        window_tab("Inventory", 12),
                        window_tab("Equipment", 4),
                        log_tab(),
                    ],
                }))}
            ]
        })],
    )
}

fn window_tab(title: &str, count: u32) -> ui::WindowContent {
    let items: Vec<Box<dyn Scene>> = (1..=count)
        .map(|n| {
            boxed(text_colored(
                format!("{title} item {n}"),
                theme().surface_floating.on,
            ))
        })
        .collect();
    ui::WindowContent {
        title: title.into(),
        scene: Box::new(ui::scrolled(Box::new(bsn! {
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                width: Val::Percent(100.0),
            }
            Children [ {items} ]
        }))),
    }
}

fn log_tab() -> ui::WindowContent {
    let mut items: Vec<Box<dyn Scene>> = (1..=8)
        .map(|n| {
            boxed(text_colored(
                format!("log line {n}"),
                theme().surface_floating.on,
            ))
        })
        .collect();
    items.push(boxed(text_colored(
        "a deliberately long log line that must wrap onto several rows instead of being clipped at the window edge",
        theme().surface_floating.on,
    )));
    ui::WindowContent {
        title: "Log".into(),
        scene: Box::new(bsn! {
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
            }
            Children [
                ( Node { flex_grow: 1.0, min_height: Val::Px(0.0), width: Val::Percent(100.0) }
                  Children [
                    ( {scroll_area()}
                      Children [
                        ( {scroll_viewport()}
                          {ui::component(ui::PinToBottom::default())}
                          Children [
                            (
                                Node {
                                    flex_direction: FlexDirection::Column,
                                    width: Val::Percent(100.0),
                                    padding: {UiRect::all(Val::Px(4.0))},
                                }
                                Children [ {items} ]
                            )
                          ]
                        ),
                        ( {scroll_bar()} Children [ {EntityScene(scroll_thumb())} ] )
                      ]
                    )
                  ]
                ),
                {EntityScene(ui::text_input(ui::TextInputOptions {
                    on_submit: ui::OnSubmit::new(|_, _| {}),
                }))}
            ]
        }),
    }
}

fn scroll_area_scene() -> Box<dyn Scene> {
    let items: Vec<Box<dyn Scene>> = (1..=16)
        .map(|n| boxed(text_colored(format!("Item {n}"), theme().surface_canvas.on)))
        .collect();
    col(
        320.0,
        vec![boxed(bsn! {
            Node { width: Val::Px(300.0), height: Val::Px(220.0) }
            Children [
                ( {scroll_area()}
                  Children [
                    ( {scroll_viewport()}
                      Children [
                        ( Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(10.0), width: Val::Percent(100.0), padding: {UiRect::all(Val::Px(8.0))} }
                          Children [ {items} ]
                        )
                      ]
                    ),
                    ( {scroll_bar()} Children [ {EntityScene(scroll_thumb())} ] )
                  ]
                )
            ]
        })],
    )
}

fn ink(text: &'static str, color: Color) -> RichPiece {
    RichSpan::plain(text).color(color).into()
}

fn plain(text: &'static str) -> RichPiece {
    RichSpan::plain(text).into()
}

fn effect_rows() -> Vec<(&'static str, Vec<RichPiece>)> {
    vec![
        (
            "ink danger",
            vec![plain("the "), ink("chief's blade", palette::CRIMSON_80)],
        ),
        (
            "ink item",
            vec![plain("a "), ink("Greater Health Potion", palette::AMBER_80)],
        ),
        (
            "ink place",
            vec![plain("by "), ink("the old well", palette::AZURE_80)],
        ),
        (
            "ink name",
            vec![plain("ask "), ink("Mara", palette::EMERALD_80)],
        ),
        (
            "ink magic",
            vec![
                plain("the "),
                ink("drums", palette::VIOLET_80),
                plain(" grow louder"),
            ],
        ),
        (
            "wave",
            vec![
                RichSpan::plain("a little game")
                    .motion(TextMotion::Wave)
                    .into(),
            ],
        ),
        (
            "shake",
            vec![
                plain("you'll "),
                RichSpan::plain("lose").motion(TextMotion::Shake).into(),
            ],
        ),
        (
            "pulse",
            vec![
                RichSpan::plain("something stirs")
                    .motion(TextMotion::Pulse)
                    .into(),
            ],
        ),
        (
            "whisper",
            vec![
                RichSpan::plain("everyone does")
                    .voice(TextVoice::Whisper)
                    .into(),
            ],
        ),
        (
            "shout",
            vec![RichSpan::plain("CHEAT?!").voice(TextVoice::Shout).into()],
        ),
        (
            "slow",
            vec![RichSpan::plain("very... slowly").slow().into()],
        ),
        (
            "pause",
            vec![
                plain("wait"),
                RichPiece::Pause(Duration::from_millis(900)),
                plain(" for it"),
            ],
        ),
    ]
}

fn pell_line() -> Vec<RichPiece> {
    vec![
        plain("Care for a little "),
        RichSpan::plain("game of chance")
            .motion(TextMotion::Wave)
            .into(),
        plain("? Ten "),
        ink("Gold", palette::AMBER_80),
        plain(" says you'll "),
        RichSpan::plain("lose").motion(TextMotion::Shake).into(),
        plain("."),
        RichPiece::Pause(Duration::from_millis(700)),
        RichSpan::plain(" Everyone does.")
            .voice(TextVoice::Whisper)
            .into(),
    ]
}

fn rich_text_scene() -> Box<dyn Scene> {
    let rows: Vec<Box<dyn Scene>> = effect_rows()
        .into_iter()
        .map(|(label, pieces)| {
            boxed(bsn! {
                Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(16.0), align_items: AlignItems::Center }
                Children [
                    ( Node { width: Val::Px(110.0) } Children [ {EntityScene(text(label))} ] ),
                    {EntityScene(rich_text(RichText::new(pieces), false))},
                ]
            })
        })
        .collect();
    col(
        560.0,
        vec![
            boxed(bsn! {
                Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(10.0), width: Val::Percent(100.0) }
                Children [ {rows} ]
            }),
            boxed(bsn! {
                Node { margin: {UiRect::top(Val::Px(24.0))} }
                Children [ (
                    {button_styled(button_intent::SECONDARY, ButtonSize::Sm, "toggle reduced motion")}
                    on(|_: On<ui::Activate>, mut preference: ResMut<MotionPreference>| {
                        preference.reduced = !preference.reduced;
                    })
                ) ]
            }),
        ],
    )
}

#[derive(Component, Default, Clone)]
struct ReplayLine;

fn typewriter_scene() -> Box<dyn Scene> {
    let line = RichText {
        size: 20.0,
        ..RichText::new(pell_line())
    };
    col(
        620.0,
        vec![
            boxed(bsn! {
                Node {
                    width: Val::Percent(100.0),
                    min_height: Val::Px(90.0),
                    padding: {UiRect::all(Val::Px(18.0))},
                }
                BackgroundColor({theme().surface_floating.base})
                Children [ ( {rich_text(line, true)} ReplayLine ) ]
            }),
            boxed(bsn! {
                Node { margin: {UiRect::top(Val::Px(18.0))}, column_gap: Val::Px(12.0) }
                Children [
                    (
                        {button_styled(button_intent::PRIMARY, ButtonSize::Sm, "replay")}
                        on(|_: On<ui::Activate>, mut lines: Query<&mut RichText, With<ReplayLine>>| {
                            for mut line in &mut lines {
                                line.set_changed();
                            }
                        })
                    ),
                    (
                        {button_styled(button_intent::SECONDARY, ButtonSize::Sm, "skip")}
                        on(|_: On<ui::Activate>, mut typewriters: Query<&mut Typewriter, With<ReplayLine>>| {
                            for mut typewriter in &mut typewriters {
                                typewriter.finish();
                            }
                        })
                    ),
                ]
            }),
        ],
    )
}

fn tag(label: &str, color: Color) -> ChipOptions {
    ChipOptions {
        label: label.to_owned(),
        icon: None,
        family: ui::Family::outline(color),
    }
}

fn cost(assets: &AssetServer, count: u32, met: bool) -> ChipOptions {
    ChipOptions {
        label: count.to_string(),
        icon: Some(assets.load("icons/misc/golden_coin.png")),
        family: ui::Family::outline(if met {
            palette::AMBER_70
        } else {
            palette::CRIMSON_70
        }),
    }
}

fn sample_choices(assets: &AssetServer) -> Vec<ChoiceOptions> {
    let choice = |label: &'static str, chips: Vec<ChipOptions>, locked: bool| ChoiceOptions {
        label: RichText::new(vec![plain(label)]),
        icon: None,
        chips,
        locked,
    };
    vec![
        choice(
            "I'm ready for the forest road.",
            vec![
                tag("Level 3", palette::CRIMSON_70),
                tag("DIALOGUE", palette::AZURE_70),
            ],
            true,
        ),
        choice(
            "The Orc Chief won't trouble anyone now.",
            vec![
                tag("Wearing Tribal Helmet", palette::EMERALD_70),
                tag("DIALOGUE", palette::AZURE_70),
            ],
            false,
        ),
        choice(
            "Here, for your trouble.",
            vec![cost(assets, 20, true), tag("DIALOGUE", palette::AZURE_70)],
            false,
        ),
        choice(
            "Never mind.",
            vec![tag("DIALOGUE", palette::AZURE_70)],
            false,
        ),
    ]
}

static GALLERY_ASSETS: std::sync::OnceLock<AssetServer> = std::sync::OnceLock::new();

fn assets() -> &'static AssetServer {
    GALLERY_ASSETS
        .get()
        .expect("gallery assets are ready before scenes are built")
}

fn chips_scene() -> Box<dyn Scene> {
    let assets = assets();
    let chips: Vec<Box<dyn Scene>> = [
        tag("QUEST", palette::AMBER_70),
        tag("SHOP", palette::AMBER_80),
        tag("DIALOGUE", palette::AZURE_70),
        tag("Level 3", palette::CRIMSON_70),
        tag("Wearing Tribal Helmet", palette::EMERALD_70),
        cost(assets, 20, true),
        cost(assets, 150, false),
    ]
    .into_iter()
    .map(|options| boxed(chip(options)))
    .chain(std::iter::once(boxed(key_hint(
        "↑ ↓ choose · Enter pick · 1-4 shortcut · Esc leave",
        theme().surface_trough,
    ))))
    .collect();
    wrap(chips)
}

fn choices_scene() -> Box<dyn Scene> {
    col(720.0, vec![boxed(choice_list(sample_choices(assets())))])
}

fn dialogue_box_scene() -> Box<dyn Scene> {
    let line = RichText::new(vec![
        plain("The forest road is "),
        ink("closed", palette::CRIMSON_80),
        plain(". Orders from the harbour master."),
    ]);
    let actions: Vec<Box<dyn Scene>> = vec![
        boxed(button_styled(
            button_intent::SECONDARY,
            ButtonSize::Sm,
            "History",
        )),
        boxed(button_styled(
            button_intent::SECONDARY,
            ButtonSize::Sm,
            "Leave",
        )),
    ];
    boxed(bsn! {
        Node { width: Val::Percent(100.0), height: Val::Percent(100.0), align_items: AlignItems::End, justify_content: JustifyContent::Center, padding: {UiRect::bottom(Val::Px(40.0))} }
        Children [ {EntityScene(dialogue_box(DialogueBoxOptions {
            speaker: Some(("Ilsa".to_owned(), Side::Right)),
            line,
            typed: true,
            choices: sample_choices(assets()),
            hint: "↑ ↓ choose · Enter pick · 1-4 shortcut · Esc leave".to_owned(),
            actions,
            status: None,
        }))} ]
    })
}

const CAST_SECONDS: f32 = 3.0;

fn cast_scene() -> Box<dyn Scene> {
    let line = RichText::new(vec![
        plain("Do you know what a crate of silk costs? I do. "),
        RichSpan::plain("To the copper.").slow().into(),
    ]);
    boxed(bsn! {
        Node { width: Val::Percent(100.0), height: Val::Percent(100.0), align_items: AlignItems::End, justify_content: JustifyContent::Center, padding: {UiRect::bottom(Val::Px(40.0))} }
        Children [
            {EntityScene(cast())},
            {EntityScene(dialogue_box(DialogueBoxOptions {
                speaker: Some(("Mara".to_owned(), Side::Right)),
                line,
                typed: false,
                choices: Vec::new(),
                hint: "Space next".to_owned(),
                actions: Vec::new(),
                status: None,
            }))},
        ]
    })
}

fn rotate_cast(time: Res<Time>, assets: Res<AssetServer>, mut casts: Query<&mut ui::Cast>) {
    let round = (time.elapsed_secs() / CAST_SECONDS) as u64;
    for mut cast in &mut casts {
        let members = cast_round(&assets, round);
        if cast.members != members {
            cast.members = members;
        }
    }
}

fn cast_round(assets: &AssetServer, round: u64) -> Vec<CastMember> {
    let mara_speaks = round.is_multiple_of(2);
    let member = |key: u64, art: &'static str, side: Side, lit: bool| CastMember {
        key,
        image: assets.load(art),
        side,
        depth: if lit {
            CastDepth::Front
        } else {
            CastDepth::Back
        },
        lit,
        flip: false,
    };
    vec![
        member(0, "busts/adventurer/thinking.png", Side::Left, !mara_speaks),
        member(
            1,
            if mara_speaks {
                "busts/mara/counting.png"
            } else {
                "busts/mara/smug.png"
            },
            Side::Right,
            mara_speaks,
        ),
    ]
}

const BANNER_SECONDS: f32 = 4.0;

fn banners_scene() -> Box<dyn Scene> {
    boxed(bsn! {
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            padding: {UiRect::top(Val::Px(ui::tokens::spacing::XL))},
        }
        Children [ {EntityScene(announcement_lane(banner_cycle(0)))} ]
    })
}

fn banner_cycle(round: u64) -> AnnouncementLane {
    let chief = |key: u64, text: RichText| Announcement {
        key,
        speaker: Some("Orc Chief".to_owned()),
        text,
        kind: AnnouncementKind::Speech,
    };
    let shout = chief(
        1,
        RichText::new(vec![
            RichSpan::plain("WHO DARES STEAL TUSKS FROM MY CLAN?!")
                .voice(TextVoice::Shout)
                .motion(TextMotion::Shake)
                .into(),
        ]),
    );
    let threat = chief(
        2,
        RichText::new(vec![plain("I'll grind your bones for soup!")]),
    );
    let narration = Announcement {
        key: 3,
        speaker: None,
        text: RichText::new(vec![plain(
            "The camp falls silent. Somewhere, a drum stops.",
        )]),
        kind: AnnouncementKind::Narration,
    };
    let system = Announcement {
        key: 4,
        speaker: None,
        text: RichText::new(vec![plain("The server restarts in five minutes.")]),
        kind: AnnouncementKind::System,
    };
    let (head, next) = match round % 4 {
        0 => (shout, Some(threat)),
        1 => (threat, Some(narration)),
        2 => (narration, Some(system)),
        _ => (system, None),
    };
    AnnouncementLane {
        head: Some(head),
        next,
        remaining: 1.0,
    }
}

fn run_down_lanes(time: Res<Time>, mut lanes: Query<&mut AnnouncementLane>) {
    let elapsed = time.elapsed_secs();
    let round = (elapsed / BANNER_SECONDS) as u64;
    for mut lane in &mut lanes {
        let cycle = banner_cycle(round);
        if lane.head.as_ref().map(|head| head.key) != cycle.head.as_ref().map(|head| head.key) {
            *lane = cycle;
        }
        lane.remaining = 1.0 - (elapsed % BANNER_SECONDS) / BANNER_SECONDS;
    }
}

fn list_detail_scene() -> Box<dyn Scene> {
    let rows = |titles: &[&str]| -> Vec<Box<dyn Scene>> {
        titles
            .iter()
            .map(|title| {
                boxed(bsn! {
                    Node { padding: {UiRect::axes(Val::Px(16.0), Val::Px(4.0))} }
                    Children [ {EntityScene(text(*title))} ]
                })
            })
            .collect()
    };
    let list = boxed(bsn! {
        Node { flex_direction: FlexDirection::Column, width: Val::Percent(100.0) }
        Children [
            {EntityScene(list_header("Orc Trouble", "1"))},
            {rows(&["Tusks for the Chief"])},
            {EntityScene(list_header("Harbour Errands", "2"))},
            {rows(&["A Letter for the Captain", "Low Tide"])},
        ]
    });
    let detail = boxed(bsn! {
        Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(8.0), padding: {UiRect::all(Val::Px(12.0))} }
        Children [
            {EntityScene(text("Low Tide"))},
            {EntityScene(text("Bring Tobb three Fish Steaks before the tide turns."))},
        ]
    });
    boxed(bsn! {
        Node { width: Val::Px(640.0), height: Val::Px(360.0) }
        BackgroundColor({theme().surface_floating.base})
        Children [ {EntityScene(split_view(list, detail))} ]
    })
}

#[derive(Component, Default, Clone)]
struct ConfirmHost;

fn confirm_dialog_scene() -> Box<dyn Scene> {
    boxed(bsn! {
        ConfirmHost
        Node { flex_direction: FlexDirection::Column, align_items: AlignItems::Center }
        Children [ (
            {button("Abandon quest")}
            on(|_: On<ui::Activate>, hosts: Query<Entity, With<ConfirmHost>>, mut commands: Commands| {
                let Ok(host) = hosts.single() else { return };
                commands
                    .spawn_scene(confirm_dialog(ConfirmOptions {
                        title: "Abandon A Letter for the Captain?".to_owned(),
                        body: vec![
                            "Tobb's Letter is taken from your bag.".to_owned(),
                            "Tobb offers the quest again.".to_owned(),
                        ],
                        confirm: "Abandon".to_owned(),
                        cancel: "Keep quest".to_owned(),
                        on_confirm: OnTap::new(|_| {}),
                        on_cancel: OnTap::new(|_| {}),
                    }))
                    .insert(ChildOf(host));
            })
        ) ]
    })
}

#[derive(Component, Default, Clone)]
struct DropLog;

fn drag_and_drop_scene() -> Box<dyn Scene> {
    let assets = assets();
    let items: Vec<Box<dyn Scene>> = [
        ("icons/monster_part/bone.png", 1),
        ("icons/monster_part/feather.png", 2),
        ("icons/potion/red_potion.png", 3),
    ]
    .into_iter()
    .map(|(icon, payload)| {
        let image = assets.load(icon);
        boxed(bsn! {
            Node { width: Val::Px(48.0), height: Val::Px(48.0) }
            BackgroundColor({theme().surface_floating.base})
            component(Carriable { image: image.clone(), payload })
            Children [ ( Node { width: Val::Percent(100.0), height: Val::Percent(100.0) } component(ImageNode::new(image)) Pickable::IGNORE ) ]
        })
    })
    .collect();
    boxed(bsn! {
        Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(80.0), align_items: AlignItems::Center }
        Children [
            ( Node { column_gap: Val::Px(8.0) } Children [ {items} ] ),
            (
                CarryTarget
                Node { width: Val::Px(240.0), height: Val::Px(160.0), align_items: AlignItems::Center, justify_content: JustifyContent::Center }
                BackgroundColor({theme().surface_floating.base})
                on(|carried: On<Carried>, logs: Query<Entity, With<DropLog>>, mut texts: Query<&mut Text>| {
                    for log in &logs {
                        if let Ok(mut text) = texts.get_mut(log) {
                            text.0 = format!("received item {}", carried.payload);
                        }
                    }
                })
                Children [ ( {text("Drop items here")} DropLog ) ]
            ),
        ]
    })
}

fn stage_keys(keys: Res<ButtonInput<KeyCode>>, mut commands: Commands) {
    let pressed: Vec<KeyCode> = keys.get_just_pressed().copied().collect();
    if pressed.is_empty() {
        return;
    }
    commands.queue(move |world: &mut World| {
        let list = world
            .query_filtered::<Entity, With<ui::ChoiceList>>()
            .iter(world)
            .next();
        for key in pressed {
            match key {
                KeyCode::Escape => {
                    ui::dismiss_topmost(world);
                }
                KeyCode::ArrowUp => {
                    if let Some(list) = list {
                        ui::step_choice(world, list, -1);
                    }
                }
                KeyCode::ArrowDown => {
                    if let Some(list) = list {
                        ui::step_choice(world, list, 1);
                    }
                }
                KeyCode::Enter if !ui::modal_open(world) => {
                    if let Some(list) = list {
                        ui::pick_choice(world, list);
                    }
                }
                KeyCode::Digit1 | KeyCode::Digit2 | KeyCode::Digit3 | KeyCode::Digit4 => {
                    let index = match key {
                        KeyCode::Digit1 => 0,
                        KeyCode::Digit2 => 1,
                        KeyCode::Digit3 => 2,
                        _ => 3,
                    };
                    if let Some(list) = list {
                        ui::pick_choice_at(world, list, index);
                    }
                }
                _ => {}
            }
        }
    });
}
