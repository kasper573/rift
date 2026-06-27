use std::collections::HashSet;

use bevy::prelude::*;
use bevy_scene::{CommandsSceneExt, EntityScene, Scene, bsn, on, template_value};
use ui::button::intent as button_intent;
use ui::card::intent as card_intent;
use ui::theme::theme;
use ui::{
    Align, ButtonIntent, ButtonSize, CardOptions, Check, OnSettle, OnTap, Orientation, Side,
    SonnerPosition, WidgetOptions, accordion, accordion_body, accordion_content, accordion_header,
    accordion_item, accordion_trigger, alert_dialog, alert_dialog_action, alert_dialog_cancel,
    avatar, avatar_fallback, button, button_styled, card, checkbox, checkbox_indicator,
    collapsible, collapsible_content, collapsible_trigger, dialog, dialog_close, popover,
    popover_content, popover_trigger, progress, progress_indicator, radio_circle, radio_group,
    radio_indicator, radio_item, scroll_area, scroll_bar, scroll_thumb, scroll_viewport, separator,
    slider, slider_range, slider_thumb, slider_track, sonner_close, switch, switch_thumb, tabs,
    tabs_list, tabs_trigger, text, text_colored, toast, toaster, tooltip, tooltip_content, widget,
    window,
};

const WINDOW: Vec2 = Vec2::new(1600.0, 900.0);

const TOAST_MESSAGES: &[(&str, &str)] = &[
    ("Event created", "Monday, January 6 at 9:00 AM"),
    ("Changes saved", "Your project is up to date."),
    ("Copied to clipboard", "The share link is ready."),
    ("Upload complete", "report-q3.pdf finished uploading."),
];

// Five anchor positions (four edges + center) used by the floating-overlay demos to show how a
// popper flips to the opposite side when its preferred side would overflow the viewport.
const SLOTS: [(f32, f32, Side); 5] = [
    (0.5, 0.02, Side::Top),
    (0.94, 0.5, Side::Right),
    (0.5, 0.95, Side::Bottom),
    (0.03, 0.5, Side::Left),
    (0.5, 0.5, Side::Bottom),
];
const FLIP_NOTE: &str = "This floating panel flips to the opposite side when its preferred side would overflow the viewport.";

#[derive(Resource, Default)]
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
                    // The gallery's only assets are the design fonts under the repo's `assets/`,
                    // resolved relative to this crate so `cargo run -p ui` just works from anywhere.
                    file_path: concat!(env!("CARGO_MANIFEST_DIR"), "/../assets").to_owned(),
                    ..default()
                }),
        )
        .insert_resource(ClearColor(theme().surface_inset.base))
        .init_resource::<CurrentScene>()
        .add_plugins(ui::UiPlugin)
        .add_systems(Startup, setup)
        .add_systems(Update, (rebuild_scene, animate_progress))
        .run();
}

fn boxed(scene: impl Scene + 'static) -> Box<dyn Scene> {
    Box::new(scene)
}

fn setup(mut commands: Commands) {
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
            ( {tabs(Some("0".to_owned()))}
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

// Loop the progress bars 0→100% so the scene is a live demo rather than a frozen empty bar.
fn animate_progress(time: Res<Time>, mut fractions: Query<&mut ui::ProgressFraction>) {
    let fraction = (time.elapsed_secs() % 2.5) / 2.5;
    for mut progress in &mut fractions {
        progress.0 = fraction;
    }
}

// Clicking a tab records its scene index; `rebuild_scene` rebuilds the scene tree when it changes.
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

// The toasts scene spawns a real toast on each "Show toast" press, cycling the example messages.
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
    ("Widget", widget_scene),
    ("Window", window_scene),
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

// A wrapping, centered row — the common showcase container for a set of variants.
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

// A fixed-width centered column.
fn col(width: f32, kids: Vec<Box<dyn Scene>>) -> Box<dyn Scene> {
    boxed(bsn! {
        Node { width: Val::Px({width}), flex_direction: FlexDirection::Column, align_items: AlignItems::Center }
        Children [ {kids} ]
    })
}

fn button_intents_scene() -> Box<dyn Scene> {
    wrap(
        BUTTON_INTENTS
            .iter()
            .map(|&(intent, label)| boxed(button_styled(intent, ButtonSize::Md, label)))
            .collect(),
    )
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

// A row of dialog action buttons, aligned to the trailing edge.
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

// Builds the five-position floating-overlay demo grid; `make` produces the anchored content per slot.
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
    // Fill the scene area so the toaster's absolute bottom-right anchors to the screen, not to a
    // content-sized box.
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

fn window_scene() -> Box<dyn Scene> {
    let items: Vec<Box<dyn Scene>> = (1..=12)
        .map(|n| {
            boxed(text_colored(
                format!("Item {n}"),
                theme().surface_floating.on,
            ))
        })
        .collect();
    col(
        360.0,
        vec![boxed(bsn! {
            Node {
                width: Val::Px(340.0),
                height: Val::Px(260.0),
                position_type: PositionType::Relative,
            }
            Children [
                {EntityScene(window(ui::WindowOptions {
                    pos: Vec2::ZERO,
                    size: Vec2::new(340.0, 260.0),
                    title: "Inventory".into(),
                    on_close: OnTap::new(|_| {}),
                    on_settle: OnSettle::new(|_, geom| geom),
                    content: Box::new(bsn! {
                        Node {
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(6.0),
                            width: Val::Percent(100.0),
                        }
                        Children [ {items} ]
                    }),
                }))}
            ]
        })],
    )
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
