use std::collections::{HashMap, HashSet};
use std::time::Duration;

use bevy::prelude::*;
use bevy_scene::{CommandsSceneExt, EntityScene, Scene, bsn, on, template_value};
use ui::button::intent as button_intent;
use ui::card::intent as card_intent;
use ui::theme::theme;
use ui::tokens::palette;
use ui::{
    AlertBar, Align, BubbleLine, BubbleTail, ButtonIntent, ButtonSize, Captions, CardOptions,
    Carriable, Carried, CarryTarget, CastDepth, CastMember, Check, ChipOptions, ChoiceOptions,
    ConfirmOptions, DialogueBoxOptions, DialoguePanelOptions, FoldedSpeakers, Intro, LabelledLine,
    Leaving, Milestones, MotionPreference, OnSettle, OnTap, Orientation, RichPiece, RichSpan,
    RichText, Side, SonnerPosition, SpeechBubble, TextMotion, TextVoice, Toast, Typewriter,
    WidgetOptions, accordion, accordion_body, accordion_content, accordion_header, accordion_item,
    accordion_trigger, alert_bar, alert_dialog, alert_dialog_action, alert_dialog_cancel, avatar,
    avatar_fallback, button, button_styled, captions, card, cast, checkbox, checkbox_indicator,
    chip, choice_list, collapsible, collapsible_content, collapsible_trigger, compact_toast,
    compact_toaster, component, confirm_dialog, dialog, dialog_close, dialogue_box, dialogue_panel,
    intro, key_hint, list_header, milestones, popover, popover_content, popover_trigger, progress,
    progress_indicator, radio_circle, radio_group, radio_indicator, radio_item, rich_text,
    scroll_area, scroll_bar, scroll_thumb, scroll_viewport, separator, slider, slider_range,
    slider_thumb, slider_track, sonner_close, speech_bubble, split_view, switch, switch_thumb,
    tabs, tabs_list, tabs_trigger, text, text_colored, toast, toaster, tooltip, tooltip_content,
    widget, window,
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
                })
                .set(ImagePlugin::default_nearest()),
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
                rotate_cast,
                cycle_bubbles,
                visit_bubbles,
                cycle_captions,
                cycle_alerts,
                cycle_milestones,
                cycle_intro,
                feed_toasts,
            ),
        )
        .run();
}

const UP: ui::InputRef = ui::InputRef(0);
const DOWN: ui::InputRef = ui::InputRef(1);
const PICK: ui::InputRef = ui::InputRef(2);
const LEAVE: ui::InputRef = ui::InputRef(3);
const NEXT: ui::InputRef = ui::InputRef(4);
const INSPECT: ui::InputRef = ui::InputRef(5);
const CARRY: ui::InputRef = ui::InputRef(6);
const INVENTORY: ui::InputRef = ui::InputRef(7);
const SUBMIT: ui::InputRef = ui::InputRef(8);
const KEEP: ui::InputRef = ui::InputRef(9);
const HISTORY: ui::InputRef = ui::InputRef(14);
const CHOICES: [ui::InputRef; 4] = [
    ui::InputRef(10),
    ui::InputRef(11),
    ui::InputRef(12),
    ui::InputRef(13),
];
const CHOICE_KEYS: [KeyCode; 4] = [
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
];

fn sample_inputs() -> ui::InputCatalog {
    let key = |name: &str, key: KeyCode| ui::CatalogEntry {
        name: name.to_owned(),
        keys: vec![ui::KeyGesture {
            key,
            modifiers: ui::KeyModifiers::NONE,
        }],
        ..default()
    };
    let click = |button: MouseButton| ui::ClickGesture {
        button,
        times: 1,
        modifiers: ui::KeyModifiers::NONE,
    };
    let mut entries = HashMap::from([
        (UP, key("↑", KeyCode::ArrowUp)),
        (DOWN, key("↓", KeyCode::ArrowDown)),
        (
            PICK,
            ui::CatalogEntry {
                clicks: vec![click(MouseButton::Left)],
                ..key("Enter", KeyCode::Enter)
            },
        ),
        (LEAVE, key("Esc", KeyCode::Escape)),
        (
            NEXT,
            ui::CatalogEntry {
                clicks: vec![click(MouseButton::Left)],
                ..key("Space", KeyCode::Space)
            },
        ),
        (INVENTORY, key("I", KeyCode::KeyI)),
        (SUBMIT, key("Enter", KeyCode::Enter)),
        (KEEP, key("Enter", KeyCode::Enter)),
        (HISTORY, key("H", KeyCode::KeyH)),
        (
            INSPECT,
            ui::CatalogEntry {
                name: "Right-click".to_owned(),
                clicks: vec![click(MouseButton::Right)],
                ..default()
            },
        ),
        (
            CARRY,
            ui::CatalogEntry {
                name: "Drag".to_owned(),
                drags: vec![ui::DragGesture {
                    button: MouseButton::Left,
                    modifiers: ui::KeyModifiers::NONE,
                }],
                ..default()
            },
        ),
    ]);
    entries.extend(
        CHOICES
            .into_iter()
            .zip(CHOICE_KEYS)
            .enumerate()
            .map(|(index, (input, code))| (input, key(&(index + 1).to_string(), code))),
    );
    ui::InputCatalog(entries)
}

fn choice_hint() -> Vec<RichPiece> {
    vec![
        RichPiece::Input(UP),
        RichPiece::Input(DOWN),
        RichPiece::text(" choose · "),
        RichPiece::Input(PICK),
        RichPiece::text(" pick · "),
        RichPiece::Input(CHOICES[0]),
        RichPiece::text("–"),
        RichPiece::Input(CHOICES[3]),
        RichPiece::text(" shortcut · "),
        RichPiece::Input(LEAVE),
        RichPiece::text(" leave"),
    ]
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
    commands.insert_resource(sample_inputs());
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
            {toast(Duration::from_secs(4))}
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
    ("Speech bubbles", bubbles_scene),
    ("Captions", captions_scene),
    ("Alert bar", alert_bar_scene),
    ("Milestones", milestones_scene),
    ("Intro", intro_scene),
    ("Notification toasts", notification_toasts_scene),
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
                    badge: Some(INVENTORY),
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
                    submit: SUBMIT,
                    blur: LEAVE,
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
                    tab: 0,
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
                    submit: SUBMIT,
                    blur: LEAVE,
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
            "input",
            vec![
                plain("press "),
                RichPiece::Input(INVENTORY),
                plain(" for your bag, "),
                RichPiece::Input(INSPECT),
                plain(" to look"),
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
        inspect: None,
    }
}

fn reward(assets: &AssetServer) -> ChipOptions {
    ChipOptions {
        label: "+1".to_owned(),
        icon: Some(assets.load("icons/misc/scroll.png")),
        family: ui::Family::outline(palette::EMERALD_70),
        inspect: Some(ui::InspectableOptions {
            tooltip: ui::TooltipText {
                title: "Road Pass".to_owned(),
                lines: vec![vec![RichPiece::text("Material · bound")]],
                hint: Some(vec![
                    RichPiece::Input(INSPECT),
                    RichPiece::text(" for more information"),
                ]),
            },
            input: INSPECT,
            on_inspect: ui::OnTap::new(|_| {}),
        }),
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
        inspect: None,
    }
}

fn sample_choices(assets: &AssetServer) -> Vec<ChoiceOptions> {
    let choice = |label: &'static str, chips: Vec<ChipOptions>, locked: bool| ChoiceOptions {
        label: RichText::new(vec![plain(label)]),
        icon: None,
        chips,
        locked,
        shortcut: None,
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
            vec![
                cost(assets, 20, true),
                reward(assets),
                tag("DIALOGUE", palette::AZURE_70),
            ],
            false,
        ),
        choice(
            "Never mind.",
            vec![tag("DIALOGUE", palette::AZURE_70)],
            false,
        ),
    ]
    .into_iter()
    .zip(CHOICES)
    .map(|(choice, shortcut)| ChoiceOptions {
        shortcut: Some(shortcut),
        ..choice
    })
    .collect()
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
        reward(assets),
    ]
    .into_iter()
    .map(|options| boxed(chip(options)))
    .chain(std::iter::once(boxed(key_hint(
        choice_hint(),
        theme().surface_trough,
    ))))
    .collect();
    wrap(chips)
}

fn choices_scene() -> Box<dyn Scene> {
    col(
        720.0,
        vec![boxed(choice_list(sample_choices(assets()), PICK))],
    )
}

fn dialogue_box_scene() -> Box<dyn Scene> {
    let line = RichText::new(vec![
        plain("The forest road is "),
        ink("closed", palette::CRIMSON_80),
        plain(". Orders from the harbour master."),
    ]);
    let panel = dialogue_panel(DialoguePanelOptions {
        title: "Road pass".to_owned(),
        width: Val::Px(420.0),
        height: Val::Auto,
        content: boxed(bsn! {
            Node { padding: {UiRect::all(Val::Px(12.0))} }
            Children [ {EntityScene(text_colored("Panels hold any content, centered above the box", theme().surface_floating.on))} ]
        }),
    });
    boxed(bsn! {
        Node { width: Val::Percent(100.0), height: Val::Percent(100.0), flex_direction: FlexDirection::Column, align_items: AlignItems::Center, justify_content: JustifyContent::End, row_gap: Val::Px(24.0), padding: {UiRect::bottom(Val::Px(40.0))} }
        Children [
            {EntityScene(panel)},
            {EntityScene(dialogue_box(DialogueBoxOptions {
                speaker: Some(("Ilsa".to_owned(), Side::Right)),
                line,
                typed: true,
                choices: sample_choices(assets()),
                hint: choice_hint(),
                advance: NEXT,
                pick: PICK,
            }))},
        ]
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
                hint: vec![RichPiece::Input(NEXT), RichPiece::text(" next")],
                advance: NEXT,
                pick: PICK,
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
        member(0, "busts/tobb/thinking.png", Side::Left, !mara_speaks),
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

const NOTIFICATION_SECONDS: f32 = 2.5;

#[derive(Component, Default, Clone)]
struct GalleryBubble(usize);

fn notification_round(time: &Time, rounds: u64) -> u64 {
    (time.elapsed_secs() / NOTIFICATION_SECONDS) as u64 % rounds
}

fn bubbles_scene() -> Box<dyn Scene> {
    let anchors: Vec<Box<dyn Scene>> = bubble_round(0)
        .into_iter()
        .enumerate()
        .map(|(index, (left, top, bubble))| -> Box<dyn Scene> {
            boxed(bsn! {
                {speech_bubble(bubble)}
                Node { left: Val::Percent({left}), top: Val::Percent({top}) }
                component(GalleryBubble(index))
            })
        })
        .collect();
    boxed(bsn! {
        GalleryBubbles
        Node { width: Val::Percent(100.0), height: Val::Percent(100.0) }
        Children [ {anchors} ]
    })
}

#[derive(Component, Default, Clone)]
struct GalleryBubbles;

#[derive(Component, Default, Clone)]
struct GalleryVisitor;

fn visit_bubbles(
    time: Res<Time>,
    stages: Query<Entity, With<GalleryBubbles>>,
    visitors: Query<Entity, (With<GalleryVisitor>, Without<Leaving>)>,
    mut commands: Commands,
) {
    let Ok(stage) = stages.single() else {
        return;
    };
    let here = notification_round(&time, 3) < 2;
    match (here, visitors.iter().next()) {
        (true, None) => {
            let bubble = SpeechBubble {
                speaker: "Pell".to_owned(),
                replaced: 0,
                lines: vec![BubbleLine {
                    key: 30,
                    text: RichText::new(vec![plain("Guards! Seize that thief!")]),
                    read: false,
                }],
                tail: Some(BubbleTail::Down),
                tail_offset: 0.0,
                folded_speakers: None,
            };
            commands
                .spawn_scene(bsn! {
                    {speech_bubble(bubble)}
                    Node { left: Val::Percent(50.0), top: Val::Percent(88.0) }
                    GalleryVisitor
                })
                .insert(ChildOf(stage));
        }
        (false, Some(visitor)) => {
            commands.entity(visitor).insert(Leaving);
        }
        _ => {}
    }
}

fn bubble_round(round: u64) -> Vec<(f32, f32, SpeechBubble)> {
    let line = |key: u64, text: RichText, read: bool| BubbleLine { key, text, read };
    let shout = RichText::new(vec![
        RichSpan::plain("WHO DARES STEAL TUSKS FROM MY CLAN?!")
            .voice(TextVoice::Shout)
            .motion(TextMotion::Shake)
            .into(),
    ]);
    let threat = RichText::new(vec![plain("I'll grind your bones for soup!")]);
    let whisper = RichText::new(vec![
        RichSpan::plain("Psst. The chief sleeps at noon.")
            .voice(TextVoice::Whisper)
            .into(),
    ]);
    let chief_lines = match round {
        0 => vec![line(0, shout, false)],
        1 => vec![line(0, shout, true), line(1, threat, false)],
        _ => vec![line(1, threat, true)],
    };
    let mara_text = |price: u32| {
        RichText::new(vec![plain(match price {
            0 => "Silk! Fresh off the boat!",
            1 => "Silk for twenty gold!",
            _ => "Silk for fifteen, final offer!",
        })])
    };
    vec![
        (
            28.0,
            48.0,
            SpeechBubble {
                speaker: "Orc Chief".to_owned(),
                replaced: 0,
                lines: chief_lines,
                tail: Some(BubbleTail::Down),
                tail_offset: 0.0,
                folded_speakers: None,
            },
        ),
        (
            62.0,
            36.0,
            SpeechBubble {
                speaker: "Mara".to_owned(),
                replaced: round as u32,
                lines: vec![line(10 + round, mara_text(round as u32), false)],
                tail: Some(BubbleTail::Up),
                tail_offset: -40.0,
                folded_speakers: None,
            },
        ),
        (
            96.0,
            62.0,
            SpeechBubble {
                speaker: "Grisha".to_owned(),
                replaced: 0,
                lines: vec![line(20, whisper, false)],
                tail: Some(BubbleTail::Right),
                tail_offset: 0.0,
                folded_speakers: Some(FoldedSpeakers(3)),
            },
        ),
    ]
}

fn cycle_bubbles(time: Res<Time>, mut bubbles: Query<(&GalleryBubble, &mut SpeechBubble)>) {
    let round = bubble_round(notification_round(&time, 3));
    for (index, mut bubble) in &mut bubbles {
        let next = &round[index.0].2;
        if *bubble != *next {
            *bubble = next.clone();
        }
    }
}

fn notification_column(place: Box<dyn Scene>) -> Box<dyn Scene> {
    boxed(bsn! {
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            padding: {UiRect::top(Val::Px(ui::tokens::spacing::XL))},
        }
        Children [ {EntityScene(place)} ]
    })
}

fn labelled(key: u64, label: &str, replaced: u32, text: &str) -> LabelledLine {
    LabelledLine {
        key,
        label: label.to_owned(),
        replaced,
        text: RichText::new(vec![plain_owned(text)]),
    }
}

fn plain_owned(text: &str) -> RichPiece {
    RichPiece::text(text.to_owned())
}

fn more(count: u32) -> Option<RichText> {
    Some(RichText::new(vec![
        RichPiece::text(format!("+{count} more \u{b7} ")),
        RichPiece::Input(HISTORY),
        plain(" history"),
    ]))
}

fn captions_scene() -> Box<dyn Scene> {
    notification_column(boxed(captions()))
}

fn caption_round(round: u64) -> Captions {
    let tide = labelled(
        0,
        "Tide",
        0,
        "The tide turns. Moored boats knock against the pier.",
    );
    let gulls = labelled(
        1,
        "Gulls",
        0,
        "Gulls squabble over a fish head at the end of the pier.",
    );
    let sails = labelled(
        2,
        "The Gull",
        0,
        "The Gull slips her moorings and leans into the wind.",
    );
    match round {
        0 => Captions {
            rows: vec![tide],
            more: None,
        },
        1 => Captions {
            rows: vec![tide, gulls],
            more: None,
        },
        2 => Captions {
            rows: vec![tide, gulls, sails],
            more: None,
        },
        3 => Captions {
            rows: vec![
                labelled(1, "Gulls", 1, "A gull makes off with the whole fish."),
                sails,
            ],
            more: more(2),
        },
        _ => Captions::default(),
    }
}

fn cycle_captions(time: Res<Time>, mut captions: Query<&mut Captions>) {
    let next = caption_round(notification_round(&time, 5));
    for mut shown in &mut captions {
        if *shown != next {
            *shown = next.clone();
        }
    }
}

fn alert_bar_scene() -> Box<dyn Scene> {
    notification_column(boxed(alert_bar()))
}

fn alert_round(round: u64) -> AlertBar {
    let restart = labelled(
        0,
        "Server",
        0,
        "The server restarts in five minutes, so finish your fights.",
    );
    let watch = labelled(
        1,
        "Harbour watch",
        0,
        "The watch is looking for whoever stole from Pell.",
    );
    match round {
        0 => AlertBar {
            alerts: vec![restart],
            more: None,
        },
        1 => AlertBar {
            alerts: vec![restart, watch],
            more: None,
        },
        2 => AlertBar {
            alerts: vec![watch],
            more: more(1),
        },
        _ => AlertBar::default(),
    }
}

fn cycle_alerts(time: Res<Time>, mut bars: Query<&mut AlertBar>) {
    let next = alert_round(notification_round(&time, 4));
    for mut bar in &mut bars {
        if *bar != next {
            *bar = next.clone();
        }
    }
}

fn milestones_scene() -> Box<dyn Scene> {
    notification_column(boxed(milestones()))
}

fn milestone_round(round: u64) -> Milestones {
    let quest = labelled(0, "Quest complete", 0, "Tusks for the Chief");
    match round {
        0 => Milestones {
            rows: vec![quest],
            more: None,
        },
        1 => Milestones {
            rows: vec![quest, labelled(1, "Level up", 0, "Level 5")],
            more: None,
        },
        2 => Milestones {
            rows: vec![labelled(1, "Level up", 1, "Level 6")],
            more: None,
        },
        _ => Milestones::default(),
    }
}

fn cycle_milestones(time: Res<Time>, mut milestones: Query<&mut Milestones>) {
    let next = milestone_round(notification_round(&time, 4));
    for mut shown in &mut milestones {
        if *shown != next {
            *shown = next.clone();
        }
    }
}

fn intro_scene() -> Box<dyn Scene> {
    notification_column(boxed(bsn! {
        GalleryIntroHost
        Node { display: Display::Grid, justify_items: JustifyItems::Center }
    }))
}

#[derive(Component, Default, Clone)]
struct GalleryIntroHost;

fn cycle_intro(
    time: Res<Time>,
    hosts: Query<Entity, With<GalleryIntroHost>>,
    intros: Query<Entity, (With<Intro>, Without<Leaving>)>,
    mut commands: Commands,
) {
    let Ok(host) = hosts.single() else {
        return;
    };
    let here = notification_round(&time, 2) == 0;
    match (here, intros.iter().next()) {
        (true, None) => {
            let shown = Intro {
                title: "The forest".to_owned(),
                text: RichText::new(vec![plain(
                    "Old trees, older drums. Something is awake out here.",
                )]),
                tint: Some(Color::srgb_u8(0x8f, 0xd1, 0x8b)),
                motion: Some(TextMotion::Pulse),
                emblem: None,
            };
            commands
                .spawn_scene(bsn! {
                    {intro(shown)}
                    Node { grid_row: {GridPlacement::start(1)}, grid_column: {GridPlacement::start(1)} }
                })
                .insert(ChildOf(host));
        }
        (false, Some(shown)) => {
            commands.entity(shown).insert(Leaving);
        }
        _ => {}
    }
}

fn notification_toasts_scene() -> Box<dyn Scene> {
    notification_column(boxed(bsn! {
        Node { width: Val::Percent(100.0), height: Val::Percent(100.0), justify_content: JustifyContent::Center }
        Children [
            (
                {toaster(SonnerPosition::TopCenter)}
                GalleryCardToaster
                Node { position_type: PositionType::Relative, top: Val::Auto, left: Val::Auto, margin: UiRect::ZERO }
            ),
            ( {compact_toaster(320.0)} GalleryFeedToaster ),
        ]
    }))
}

#[derive(Component, Default, Clone)]
struct GalleryCardToaster;

#[derive(Component, Default, Clone)]
struct GalleryFeedToaster;

const FEED_EVERY: f32 = 1.1;
const FEED_LINES: &[&str] = &[
    "+3 Fish",
    "Quest accepted · Low Tide",
    "+40 XP",
    "Equipped Rusty Sword",
];
const ERROR_LINES: &[&str] = &["You need a helmet to enter.", "The door is locked."];

fn feed_toasts(
    time: Res<Time>,
    cards: Query<(Entity, Option<&Children>), With<GalleryCardToaster>>,
    feeds: Query<Entity, With<GalleryFeedToaster>>,
    mut shown: Query<&mut Toast>,
    mut fed: Local<Option<u64>>,
    mut commands: Commands,
) {
    let (Ok((card, errors)), Ok(feed)) = (cards.single(), feeds.single()) else {
        *fed = None;
        return;
    };
    let tick = (time.elapsed_secs() / FEED_EVERY) as u64;
    if *fed == Some(tick) {
        return;
    }
    *fed = Some(tick);
    let line = FEED_LINES[tick as usize % FEED_LINES.len()];
    commands
        .spawn_scene(bsn! {
            {compact_toast(Duration::from_secs(3))}
            Children [ {EntityScene(text(line))} ]
        })
        .insert(ChildOf(feed));
    if tick.is_multiple_of(2) {
        let mut replaced = shown.iter_many_mut(errors.into_iter().flatten());
        while let Some(mut toast) = replaced.fetch_next() {
            toast.leaving = true;
        }
        let error = ERROR_LINES[(tick / 2) as usize % ERROR_LINES.len()];
        commands
            .spawn_scene(bsn! {
                {toast(Duration::from_secs(4))}
                Children [ {EntityScene(text_colored(error, palette::CRIMSON_80))} ]
            })
            .insert(ChildOf(card));
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
        Children [ {EntityScene(split_view(list, boxed(ui::scrolled(detail))))} ]
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
                        keep: KEEP,
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
            component(Carriable { image: image.clone(), payload, input: CARRY })
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

fn stage_keys(
    catalog: Res<ui::InputCatalog>,
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
) {
    let pressed = |input| catalog.just_pressed(input, &keys);
    let leave = pressed(LEAVE);
    let step = if pressed(UP) {
        Some(-1)
    } else if pressed(DOWN) {
        Some(1)
    } else {
        None
    };
    let pick = pressed(PICK);
    let shortcut = CHOICES.into_iter().position(pressed);
    if !leave && step.is_none() && !pick && shortcut.is_none() {
        return;
    }
    commands.queue(move |world: &mut World| {
        if leave {
            ui::dismiss_topmost(world);
        }
        let Some(list) = world
            .query_filtered::<Entity, With<ui::ChoiceList>>()
            .iter(world)
            .next()
        else {
            return;
        };
        if let Some(delta) = step {
            ui::step_choice(world, list, delta);
        }
        if pick && !ui::modal_open(world) {
            ui::pick_choice(world, list);
        }
        if let Some(index) = shortcut {
            ui::pick_choice_at(world, list, index);
        }
    });
}
