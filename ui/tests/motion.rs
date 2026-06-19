//! Contract for the motion system: an interactive component's paint follows its [`PointerState`] and
//! eases between states over time rather than snapping. Asserted through [`Button`] — render it, drive
//! its pointer state, step the clock, and watch its `BackgroundColor` travel from the resting color
//! toward the hover color.

mod harness;

use std::time::Duration;

use bevy_color::Color;
use bevy_ui::BackgroundColor;
use ui::theme::color;
use ui::themes::dark;
use ui::{Button, PointerState};

use harness::Ui;

fn background(ui: &Ui, entity: bevy_ecs::entity::Entity) -> Color {
    ui.get::<BackgroundColor>(entity).expect("a background").0
}

/// Distance between two colors in linear RGB — small means visually equal.
fn distance(a: Color, b: Color) -> f32 {
    let a = a.to_linear();
    let b = b.to_linear();
    ((a.red - b.red).powi(2) + (a.green - b.green).powi(2) + (a.blue - b.blue).powi(2)).sqrt()
}

#[test]
fn a_button_eases_from_resting_to_hover_when_pointed_at() {
    let theme = dark::theme();
    let resting = color::primary_base.resolve(&theme);
    let hover = color::primary_hover.resolve(&theme);

    let mut ui = Ui::new().with_clock();
    ui.render(Button::default().label("Play"));
    let button = ui.children()[0];

    // At rest it sits exactly on the resting color, with no fade-in on mount.
    assert!(
        distance(background(&ui, button), resting) < 1e-3,
        "a freshly mounted button shows its resting color immediately"
    );

    // Point at it and re-render so the recipe aims at the hover color.
    ui.world().entity_mut(button).insert(PointerState {
        hovered: true,
        pressed: false,
    });
    ui.render(Button::default().label("Play"));

    // It has not jumped — it is still essentially at rest the instant the hover begins.
    assert!(
        distance(background(&ui, button), resting) < 0.05,
        "the hover does not snap; it begins from the resting color"
    );

    // Part way through the transition it sits between the two colors.
    ui.tick(Duration::from_millis(120));
    let midway = background(&ui, button);
    assert!(
        distance(midway, resting) > 0.01 && distance(midway, hover) > 0.01,
        "midway through, the color is between resting and hover (got {midway:?})"
    );

    // Once the transition has run its course it rests on the hover color.
    ui.tick(Duration::from_millis(300));
    assert!(
        distance(background(&ui, button), hover) < 1e-2,
        "after the transition completes it settles on the hover color"
    );
}

#[test]
fn releasing_the_pointer_eases_back_to_resting() {
    let theme = dark::theme();
    let resting = color::primary_base.resolve(&theme);

    let mut ui = Ui::new().with_clock();
    ui.render(Button::default().label("Play"));
    let button = ui.children()[0];

    ui.world().entity_mut(button).insert(PointerState {
        hovered: true,
        pressed: false,
    });
    ui.render(Button::default().label("Play"));
    ui.tick(Duration::from_millis(400));

    // Pointer leaves: aim back at the resting color and let it settle.
    ui.world()
        .entity_mut(button)
        .insert(PointerState::default());
    ui.render(Button::default().label("Play"));
    ui.tick(Duration::from_millis(400));

    assert!(
        distance(background(&ui, button), resting) < 1e-2,
        "after the pointer leaves it eases back to the resting color"
    );
}
