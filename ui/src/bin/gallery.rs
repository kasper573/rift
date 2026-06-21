use std::collections::HashSet;

use bevy::prelude::*;
use ui::theme::color;
use ui::themes;
use ui::{
    Align, ButtonIntent, ButtonSize, CardIntent, CardOpts, Check, Orientation, Side,
    SonnerPosition, accordion, accordion_body, accordion_content, accordion_header, accordion_item,
    accordion_trigger, alert_dialog, alert_dialog_action, alert_dialog_cancel, avatar,
    avatar_fallback, button, button_styled, card, checkbox, checkbox_indicator, collapsible,
    collapsible_content, collapsible_trigger, dialog, dialog_close, popover, popover_content,
    popover_trigger, progress, progress_indicator, radio_circle, radio_group, radio_indicator,
    radio_item, scroll_area, scroll_bar, scroll_thumb, scroll_viewport, separator, slider,
    slider_range, slider_thumb, slider_track, sonner_close, switch, switch_thumb, tabs, tabs_list,
    tabs_trigger, text, text_colored, toast, toaster, tooltip, tooltip_content,
};

const WINDOW: Vec2 = Vec2::new(1600.0, 900.0);

const TOAST_MESSAGES: &[(&str, &str)] = &[
    ("Event created", "Monday, January 6 at 9:00 AM"),
    ("Changes saved", "Your project is up to date."),
    ("Copied to clipboard", "The share link is ready."),
    ("Upload complete", "report-q3.pdf finished uploading."),
];

#[derive(Resource, Default)]
struct CurrentScene(usize);

#[derive(Component)]
struct GalleryRoot;

#[derive(Component)]
struct SceneRoot;

#[derive(Component)]
struct SceneTab(usize);

#[derive(Component)]
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
        .insert_resource(ClearColor(
            color::surface_inset.base.resolve(&themes::light::THEME),
        ))
        .insert_resource(themes::light::THEME)
        .init_resource::<CurrentScene>()
        .add_plugins(ui::UiPlugin)
        .add_systems(Startup, setup)
        .add_systems(Update, (rebuild_scene, animate_progress))
        .run();
}

fn setup(mut commands: Commands) {
    commands.spawn((Camera2d, IsDefaultUiCamera));
    commands
        .spawn((
            GalleryRoot,
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                ..default()
            },
        ))
        .with_children(|root| {
            root.spawn(tabs(Some("0".to_owned())))
                .with_children(|group| {
                    group
                        .spawn(Node {
                            flex_direction: FlexDirection::Row,
                            flex_wrap: FlexWrap::Wrap,
                            width: Val::Percent(100.0),
                            column_gap: Val::Px(4.0),
                            row_gap: Val::Px(4.0),
                            padding: UiRect::all(Val::Px(12.0)),
                            ..default()
                        })
                        .with_children(|list| {
                            for (index, (name, _)) in SCENES.iter().enumerate() {
                                list.spawn((tabs_trigger(index.to_string()), SceneTab(index)))
                                    .observe(on_tab)
                                    .with_children(|trigger| {
                                        trigger.spawn(text(*name));
                                    });
                            }
                        });
                });
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
    commands.entity(root).with_children(|parent| {
        let mut scene = parent.spawn((
            SceneRoot,
            Node {
                flex_grow: 1.0,
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
        ));
        let (_name, builder) = SCENES[current.0];
        builder(&mut scene);
    });
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
    commands.entity(toaster).with_children(|parent| {
        parent.spawn(toast()).with_children(|toast| {
            toast
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(12.0),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn(text(title));
                    row.spawn(sonner_close()).with_children(|close| {
                        close.spawn(button_styled(
                            ButtonIntent::Secondary,
                            ButtonSize::Sm,
                            "close",
                        ));
                    });
                });
            toast.spawn(text_colored(body, color::surface_canvas.on));
        });
    });
}

type SceneBuilder = fn(&mut EntityCommands);

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
];

fn button_intents_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent
            .spawn((Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                column_gap: Val::Px(18.0),
                row_gap: Val::Px(18.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                max_width: Val::Px(1360.0),
                ..default()
            },))
            .with_children(|parent| {
                for intent in ButtonIntent::ALL {
                    parent.spawn((button_styled(intent, ButtonSize::Md, intent.label()),));
                }
            });
    });
}

fn button_sizes_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent
            .spawn((Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                column_gap: Val::Px(18.0),
                row_gap: Val::Px(18.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                max_width: Val::Px(1360.0),
                ..default()
            },))
            .with_children(|parent| {
                for size in ButtonSize::ALL {
                    parent.spawn((button_styled(ButtonIntent::Primary, size, size.label()),));
                }
            });
    });
}

fn tabs_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent
            .spawn((Node {
                width: Val::Px(520.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },))
            .with_children(|parent| {
                parent
                    .spawn(tabs(Some("overview".to_owned())))
                    .with_children(|parent| {
                        parent.spawn(tabs_list()).with_children(|parent| {
                            parent
                                .spawn((tabs_trigger("overview"),))
                                .with_children(|parent| {
                                    parent.spawn(text("Overview"));
                                });
                            parent
                                .spawn((tabs_trigger("activity"),))
                                .with_children(|parent| {
                                    parent.spawn(text("Activity"));
                                });
                            parent
                                .spawn((tabs_trigger("settings"),))
                                .with_children(|parent| {
                                    parent.spawn(text("Settings"));
                                });
                        });
                    });
            });
    });
}

fn checkbox_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent
            .spawn((Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                column_gap: Val::Px(18.0),
                row_gap: Val::Px(18.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                max_width: Val::Px(1360.0),
                ..default()
            },))
            .with_children(|parent| {
                parent
                    .spawn((checkbox(Check::Off),))
                    .with_children(|parent| {
                        parent.spawn(checkbox_indicator());
                    });
                parent.spawn(checkbox(Check::On)).with_children(|parent| {
                    parent.spawn(checkbox_indicator());
                });
                parent
                    .spawn(checkbox(Check::Indeterminate))
                    .with_children(|parent| {
                        parent.spawn((
                            Node {
                                width: Val::Px(10.0),
                                height: Val::Px(2.0),
                                border_radius: BorderRadius::all(Val::Px(999.0)),
                                ..default()
                            },
                            ui::Style::new().background(color::primary.on),
                        ));
                    });
            });
    });
}

fn switch_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent
            .spawn((Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                column_gap: Val::Px(18.0),
                row_gap: Val::Px(18.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                max_width: Val::Px(1360.0),
                ..default()
            },))
            .with_children(|parent| {
                parent.spawn((switch(false),)).with_children(|parent| {
                    parent.spawn(switch_thumb());
                });
                parent.spawn(switch(true)).with_children(|parent| {
                    parent.spawn(switch_thumb());
                });
            });
    });
}

fn radio_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent
            .spawn((Node {
                width: Val::Px(240.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },))
            .with_children(|parent| {
                parent
                    .spawn(radio_group(Some("apple".to_owned())))
                    .with_children(|parent| {
                        parent
                            .spawn((radio_item("apple"),))
                            .with_children(|parent| {
                                parent.spawn(radio_circle()).with_children(|parent| {
                                    parent.spawn(radio_indicator());
                                });
                                parent.spawn(text("Apple"));
                            });
                        parent
                            .spawn((radio_item("banana"),))
                            .with_children(|parent| {
                                parent.spawn(radio_circle()).with_children(|parent| {
                                    parent.spawn(radio_indicator());
                                });
                                parent.spawn(text("Banana"));
                            });
                        parent
                            .spawn((radio_item("cherry"),))
                            .with_children(|parent| {
                                parent.spawn(radio_circle()).with_children(|parent| {
                                    parent.spawn(radio_indicator());
                                });
                                parent.spawn(text("Cherry"));
                            });
                    });
            });
    });
}

fn slider_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent
            .spawn((Node {
                width: Val::Px(360.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },))
            .with_children(|parent| {
                parent
                    .spawn((slider(35.0, 0.0, 100.0),))
                    .with_children(|parent| {
                        parent.spawn(slider_track()).with_children(|parent| {
                            parent.spawn(slider_range());
                            parent.spawn(slider_thumb());
                        });
                    });
            });
    });
}

fn progress_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent
            .spawn((Node {
                width: Val::Px(360.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },))
            .with_children(|parent| {
                parent
                    .spawn((progress(0.0, 100.0),))
                    .with_children(|parent| {
                        parent.spawn(progress_indicator());
                    });
            });
    });
}

fn avatar_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent
            .spawn((Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                column_gap: Val::Px(18.0),
                row_gap: Val::Px(18.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                max_width: Val::Px(1360.0),
                ..default()
            },))
            .with_children(|parent| {
                parent.spawn(avatar()).with_children(|parent| {
                    parent.spawn(avatar_fallback()).with_children(|parent| {
                        parent.spawn(text_colored("KS", color::primary.on));
                    });
                });
                parent.spawn(avatar()).with_children(|parent| {
                    parent.spawn(avatar_fallback()).with_children(|parent| {
                        parent.spawn(text_colored("AB", color::primary.on));
                    });
                });
            });
    });
}

fn separator_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent
            .spawn((Node {
                width: Val::Px(360.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },))
            .with_children(|parent| {
                parent.spawn(text("Above"));
                parent.spawn(separator(Orientation::Horizontal));
                parent.spawn(text("Below"));
            });
    });
}

fn accordion_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent
            .spawn((Node {
                width: Val::Px(440.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },))
            .with_children(|parent| {
                parent
                    .spawn(accordion(HashSet::from(["shipping".to_owned()]), false))
                    .with_children(|parent| {
                        parent.spawn(accordion_item()).with_children(|parent| {
                            parent.spawn(accordion_header()).with_children(|parent| {
                                parent
                                    .spawn((accordion_trigger("shipping"),))
                                    .with_children(|parent| {
                                        parent.spawn(text("Is shipping free?"));
                                    });
                            });
                            parent
                                .spawn(accordion_content("shipping"))
                                .with_children(|parent| {
                                    parent.spawn(accordion_body()).with_children(|parent| {
                                        parent.spawn(text_colored(
                                            "Yes, on orders over $50.",
                                            color::surface_canvas.on,
                                        ));
                                    });
                                });
                        });
                        parent.spawn(accordion_item()).with_children(|parent| {
                            parent.spawn(accordion_header()).with_children(|parent| {
                                parent.spawn((accordion_trigger("returns"),)).with_children(
                                    |parent| {
                                        parent.spawn(text("Can I return it?"));
                                    },
                                );
                            });
                            parent
                                .spawn(accordion_content("returns"))
                                .with_children(|parent| {
                                    parent.spawn(accordion_body()).with_children(|parent| {
                                        parent.spawn(text_colored(
                                            "Within 30 days, no questions.",
                                            color::surface_canvas.on,
                                        ));
                                    });
                                });
                        });
                        parent.spawn(accordion_item()).with_children(|parent| {
                            parent.spawn(accordion_header()).with_children(|parent| {
                                parent.spawn((accordion_trigger("styled"),)).with_children(
                                    |parent| {
                                        parent.spawn(text("Is it themed?"));
                                    },
                                );
                            });
                            parent
                                .spawn(accordion_content("styled"))
                                .with_children(|parent| {
                                    parent.spawn(accordion_body()).with_children(|parent| {
                                        parent.spawn(text_colored(
                                            "Every color comes from the theme.",
                                            color::surface_canvas.on,
                                        ));
                                    });
                                });
                        });
                    });
            });
    });
}

fn collapsible_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent
            .spawn((Node {
                width: Val::Px(360.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },))
            .with_children(|parent| {
                parent.spawn(collapsible(false)).with_children(|parent| {
                    parent
                        .spawn((collapsible_trigger(),))
                        .with_children(|parent| {
                            parent.spawn(text("Notification settings"));
                        });
                    parent.spawn(collapsible_content()).with_children(|parent| {
                        parent.spawn(text_colored(
                            "Email me about replies and mentions.",
                            color::surface_canvas.on,
                        ));
                    });
                });
            });
    });
}

fn dialog_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent.spawn(dialog(
            false,
            button_styled(ButtonIntent::Primary, ButtonSize::Md, "Delete project"),
            children![
                text("Delete project?"),
                text_colored(
                    "This permanently removes the project and its data.",
                    color::surface_canvas.on,
                ),
                (
                    Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(12.0),
                        row_gap: Val::Px(12.0),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::FlexEnd,
                        flex_wrap: FlexWrap::Wrap,
                        max_width: Val::Px(1360.0),
                        ..default()
                    },
                    children![
                        (
                            dialog_close(),
                            children![button_styled(ButtonIntent::Plain, ButtonSize::Md, "Cancel")],
                        ),
                        button_styled(ButtonIntent::Danger, ButtonSize::Md, "Delete"),
                    ],
                ),
            ],
        ));
    });
}

fn alert_dialog_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent.spawn(alert_dialog(
            false,
            button_styled(ButtonIntent::Danger, ButtonSize::Md, "Reset everything"),
            children![
                text("Are you absolutely sure?"),
                text_colored("This action cannot be undone.", color::surface_canvas.on),
                (
                    Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(12.0),
                        row_gap: Val::Px(12.0),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::FlexEnd,
                        flex_wrap: FlexWrap::Wrap,
                        max_width: Val::Px(1360.0),
                        ..default()
                    },
                    children![
                        (
                            alert_dialog_cancel(),
                            children![button_styled(ButtonIntent::Plain, ButtonSize::Md, "Cancel")],
                        ),
                        (
                            alert_dialog_action(),
                            children![button_styled(
                                ButtonIntent::Primary,
                                ButtonSize::Md,
                                "Continue"
                            )],
                        ),
                    ],
                ),
            ],
        ));
    });
}

fn card_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent
            .spawn((Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                column_gap: Val::Px(18.0),
                row_gap: Val::Px(18.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                max_width: Val::Px(1360.0),
                ..default()
            },))
            .with_children(|parent| {
                let variants = vec![
                    (
                        "Surface",
                        "Default, bordered",
                        color::surface_elevated.on,
                        CardOpts::default(),
                    ),
                    (
                        "Floating",
                        "Elevation shadow",
                        color::surface_elevated.on,
                        CardOpts {
                            floating: true,
                            ..default()
                        },
                    ),
                    (
                        "Compact",
                        "Tighter padding",
                        color::surface_elevated.on,
                        CardOpts {
                            compact: true,
                            ..default()
                        },
                    ),
                    (
                        "Interactive",
                        "Hover & press me",
                        color::surface_elevated.on,
                        CardOpts {
                            interactive: true,
                            ..default()
                        },
                    ),
                    (
                        "Floating + interactive",
                        "Lifts higher on hover",
                        color::surface_elevated.on,
                        CardOpts {
                            floating: true,
                            interactive: true,
                            ..default()
                        },
                    ),
                    (
                        "Success",
                        "Intent palette",
                        color::success_soft.on,
                        CardOpts {
                            intent: CardIntent::Success,
                            ..default()
                        },
                    ),
                    (
                        "Error",
                        "Intent palette",
                        color::error_soft.on,
                        CardOpts {
                            intent: CardIntent::Error,
                            ..default()
                        },
                    ),
                    (
                        "Info",
                        "Intent palette",
                        color::info_soft.on,
                        CardOpts {
                            intent: CardIntent::Info,
                            ..default()
                        },
                    ),
                    (
                        "Utility",
                        "Intent palette",
                        color::neutral.on,
                        CardOpts {
                            intent: CardIntent::Muted,
                            ..default()
                        },
                    ),
                ];
                for (title, desc, on, opts) in variants {
                    parent.spawn(card(opts)).with_children(|parent| {
                        parent
                            .spawn((Node {
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(4.0),
                                width: Val::Px(150.0),
                                ..default()
                            },))
                            .with_children(|parent| {
                                parent.spawn(text_colored(title, on));
                                parent.spawn(text_colored(desc, on));
                            });
                    });
                }
            });
    });
}

fn tooltip_scene(scene: &mut EntityCommands) {
    const SLOTS: [(f32, f32, Side); 5] = [
        (0.5, 0.02, Side::Top),
        (0.94, 0.5, Side::Right),
        (0.5, 0.95, Side::Bottom),
        (0.03, 0.5, Side::Left),
        (0.5, 0.5, Side::Bottom),
    ];
    const FLIP_NOTE: &str = "This floating panel flips to the opposite side when its preferred side would overflow the viewport.";

    let open: Option<usize> = None;

    scene.with_children(|parent| {
        parent
            .spawn(Node {
                position_type: PositionType::Relative,
                ..default()
            })
            .with_children(|parent| {
                for (i, (fx, fy, side)) in SLOTS.iter().enumerate() {
                    let left = (fx - 0.5) * WINDOW.x;
                    let top = (fy - 0.5) * WINDOW.y;
                    parent
                        .spawn(Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(left),
                            top: Val::Px(top),
                            ..default()
                        })
                        .with_children(|parent| {
                            parent.spawn((Node::default(),)).with_children(|parent| {
                                parent
                                    .spawn((Node::default(), tooltip(open == Some(i))))
                                    .with_children(|parent| {
                                        parent.spawn(button_styled(
                                            ButtonIntent::Primary,
                                            ButtonSize::Md,
                                            "Hover me",
                                        ));
                                        parent
                                            .spawn(tooltip_content(*side, Align::Center, 8.0))
                                            .with_children(|parent| {
                                                parent
                                                    .spawn((Node {
                                                        width: Val::Px(220.0),
                                                        padding: UiRect::all(Val::Px(12.0)),
                                                        ..default()
                                                    },))
                                                    .with_children(|parent| {
                                                        parent.spawn(text(FLIP_NOTE));
                                                    });
                                            });
                                    });
                            });
                        });
                }
            });
    });
}

fn popover_scene(scene: &mut EntityCommands) {
    const SLOTS: [(f32, f32, Side); 5] = [
        (0.5, 0.02, Side::Top),
        (0.94, 0.5, Side::Right),
        (0.5, 0.95, Side::Bottom),
        (0.03, 0.5, Side::Left),
        (0.5, 0.5, Side::Bottom),
    ];
    const FLIP_NOTE: &str = "This floating panel flips to the opposite side when its preferred side would overflow the viewport.";

    let open: Option<usize> = None;

    scene.with_children(|parent| {
        parent
            .spawn(Node {
                position_type: PositionType::Relative,
                ..default()
            })
            .with_children(|parent| {
                for (i, (fx, fy, side)) in SLOTS.iter().enumerate() {
                    let left = (fx - 0.5) * WINDOW.x;
                    let top = (fy - 0.5) * WINDOW.y;
                    parent
                        .spawn(Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(left),
                            top: Val::Px(top),
                            ..default()
                        })
                        .with_children(|parent| {
                            parent.spawn((Node::default(),)).with_children(|parent| {
                                parent
                                    .spawn((Node::default(), popover(open == Some(i))))
                                    .with_children(|parent| {
                                        parent.spawn(popover_trigger()).with_children(|parent| {
                                            parent.spawn(button("Open"));
                                        });
                                        parent
                                            .spawn(popover_content(*side, Align::Center, 8.0))
                                            .with_children(|parent| {
                                                parent
                                                    .spawn((Node {
                                                        width: Val::Px(220.0),
                                                        flex_direction: FlexDirection::Column,
                                                        row_gap: Val::Px(8.0),
                                                        padding: UiRect::all(Val::Px(12.0)),
                                                        ..default()
                                                    },))
                                                    .with_children(|parent| {
                                                        parent.spawn(text("Dimensions"));
                                                        parent.spawn(text_colored(
                                                            FLIP_NOTE,
                                                            color::surface_canvas.on,
                                                        ));
                                                    });
                                            });
                                    });
                            });
                        });
                }
            });
    });
}

fn tooltip_card_scene(scene: &mut EntityCommands) {
    const SLOTS: [(f32, f32, Side); 5] = [
        (0.5, 0.02, Side::Top),
        (0.94, 0.5, Side::Right),
        (0.5, 0.95, Side::Bottom),
        (0.03, 0.5, Side::Left),
        (0.5, 0.5, Side::Bottom),
    ];
    const FLIP_NOTE: &str = "This floating panel flips to the opposite side when its preferred side would overflow the viewport.";

    let open: Option<usize> = None;

    scene.with_children(|parent| {
        parent
            .spawn(Node {
                position_type: PositionType::Relative,
                ..default()
            })
            .with_children(|parent| {
                for (i, (fx, fy, side)) in SLOTS.iter().enumerate() {
                    let left = (fx - 0.5) * WINDOW.x;
                    let top = (fy - 0.5) * WINDOW.y;
                    parent
                        .spawn(Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(left),
                            top: Val::Px(top),
                            ..default()
                        })
                        .with_children(|parent| {
                            parent.spawn((Node::default(),)).with_children(|parent| {
                                parent
                                    .spawn((Node::default(), tooltip(open == Some(i))))
                                    .with_children(|parent| {
                                        parent.spawn(button_styled(
                                            ButtonIntent::Primary,
                                            ButtonSize::Md,
                                            "Hover me",
                                        ));
                                        parent
                                            .spawn(tooltip_content(*side, Align::Center, 8.0))
                                            .with_children(|parent| {
                                                parent
                                                    .spawn(card(CardOpts {
                                                        floating: true,
                                                        ..default()
                                                    }))
                                                    .with_children(|parent| {
                                                        parent
                                                            .spawn((Node {
                                                                width: Val::Px(220.0),
                                                                padding: UiRect::all(Val::Px(12.0)),
                                                                ..default()
                                                            },))
                                                            .with_children(|parent| {
                                                                parent.spawn(text(FLIP_NOTE));
                                                            });
                                                    });
                                            });
                                    });
                            });
                        });
                }
            });
    });
}

fn popover_card_scene(scene: &mut EntityCommands) {
    const SLOTS: [(f32, f32, Side); 5] = [
        (0.5, 0.02, Side::Top),
        (0.94, 0.5, Side::Right),
        (0.5, 0.95, Side::Bottom),
        (0.03, 0.5, Side::Left),
        (0.5, 0.5, Side::Bottom),
    ];
    const FLIP_NOTE: &str = "This floating panel flips to the opposite side when its preferred side would overflow the viewport.";

    let open: Option<usize> = None;

    scene.with_children(|parent| {
        parent
            .spawn(Node {
                position_type: PositionType::Relative,
                ..default()
            })
            .with_children(|parent| {
                for (i, (fx, fy, side)) in SLOTS.iter().enumerate() {
                    let left = (fx - 0.5) * WINDOW.x;
                    let top = (fy - 0.5) * WINDOW.y;
                    parent
                        .spawn(Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(left),
                            top: Val::Px(top),
                            ..default()
                        })
                        .with_children(|parent| {
                            parent.spawn((Node::default(),)).with_children(|parent| {
                                parent
                                    .spawn((Node::default(), popover(open == Some(i))))
                                    .with_children(|parent| {
                                        parent.spawn(popover_trigger()).with_children(|parent| {
                                            parent.spawn(button("Open"));
                                        });
                                        parent
                                            .spawn(popover_content(*side, Align::Center, 8.0))
                                            .with_children(|parent| {
                                                parent
                                                    .spawn(card(CardOpts {
                                                        floating: true,
                                                        ..default()
                                                    }))
                                                    .with_children(|parent| {
                                                        parent
                                                            .spawn((Node {
                                                                width: Val::Px(220.0),
                                                                flex_direction:
                                                                    FlexDirection::Column,
                                                                row_gap: Val::Px(8.0),
                                                                padding: UiRect::all(Val::Px(12.0)),
                                                                ..default()
                                                            },))
                                                            .with_children(|parent| {
                                                                parent.spawn(text("Dimensions"));
                                                                parent.spawn(text_colored(
                                                                    FLIP_NOTE,
                                                                    color::surface_canvas.on,
                                                                ));
                                                            });
                                                    });
                                            });
                                    });
                            });
                        });
                }
            });
    });
}

fn toasts_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent.spawn(button("Show toast")).observe(show_toast);
        parent.spawn((ToasterEntity, toaster(SonnerPosition::BottomRight)));
    });
}

fn scroll_area_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent
            .spawn((Node {
                width: Val::Px(320.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },))
            .with_children(|parent| {
                parent
                    .spawn((Node {
                        width: Val::Px(300.0),
                        height: Val::Px(220.0),
                        ..default()
                    },))
                    .with_children(|parent| {
                        parent.spawn(scroll_area()).with_children(|parent| {
                            parent.spawn(scroll_viewport()).with_children(|parent| {
                                parent
                                    .spawn((Node {
                                        flex_direction: FlexDirection::Column,
                                        row_gap: Val::Px(10.0),
                                        width: Val::Percent(100.0),
                                        padding: UiRect::all(Val::Px(8.0)),
                                        ..default()
                                    },))
                                    .with_children(|parent| {
                                        for n in 1..=16 {
                                            parent.spawn(text_colored(
                                                format!("Item {n}"),
                                                color::surface_canvas.on,
                                            ));
                                        }
                                    });
                            });
                            parent.spawn(scroll_bar()).with_children(|parent| {
                                parent.spawn(scroll_thumb());
                            });
                        });
                    });
            });
    });
}
