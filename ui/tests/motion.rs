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

    assert!(
        distance(background(&ui, button), resting) < 1e-3,
        "a freshly mounted button shows its resting color immediately"
    );

    ui.world().entity_mut(button).insert(PointerState {
        hovered: true,
        pressed: false,
    });
    ui.render(Button::default().label("Play"));

    assert!(
        distance(background(&ui, button), resting) < 0.05,
        "the hover does not snap; it begins from the resting color"
    );

    ui.tick(Duration::from_millis(120));
    let midway = background(&ui, button);
    assert!(
        distance(midway, resting) > 0.01 && distance(midway, hover) > 0.01,
        "midway through, the color is between resting and hover (got {midway:?})"
    );

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
