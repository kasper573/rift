mod harness;

use bevy_color::Color;
use bevy_ecs::prelude::World;
use bevy_ui::{BackgroundColor, BorderColor, Node, Val};
use ui::theme::color;
use ui::themes::dark;
use ui::{Button, Recipe, Style};

use harness::Ui;

const BLUE: Color = Color::srgb(0.0, 0.0, 1.0);
const RED: Color = Color::srgb(1.0, 0.0, 0.0);

fn swatch() -> Recipe {
    Recipe::new()
        .base(
            Style::new()
                .node(|node| node.width = Val::Px(10.0))
                .insert(BackgroundColor(Color::WHITE)),
        )
        .variant(
            "size",
            [
                ("small", Style::new().node(|n| n.height = Val::Px(1.0))),
                ("large", Style::new().node(|n| n.height = Val::Px(9.0))),
            ],
        )
        .variant(
            "color",
            [
                ("blue", Style::new().insert(BackgroundColor(BLUE))),
                ("red", Style::new().insert(BackgroundColor(RED))),
            ],
        )
        .compound(
            [("size", "large"), ("color", "blue")],
            Style::new().insert(BorderColor::all(Color::BLACK)),
        )
        .default_variant("size", "small")
        .default_variant("color", "blue")
}

/// Resolves `selection` against `recipe`, applies it to a fresh entity, and reports the styling. A
/// `Node` auto-requires `BackgroundColor`/`BorderColor` (transparent by default), so both are always
/// present — a missed variant leaves them transparent rather than absent.
fn resolve(recipe: &Recipe, selection: &[(&str, &str)]) -> (Node, Color, Color) {
    let style = recipe.resolve(selection);
    let mut world = World::new();
    let mut entity = world.spawn(Node::default());
    style.apply(&mut entity);
    let id = entity.id();
    (
        world.get::<Node>(id).cloned().unwrap(),
        world.get::<BackgroundColor>(id).unwrap().0,
        world.get::<BorderColor>(id).unwrap().left,
    )
}

#[test]
fn unselected_dimensions_fall_back_to_defaults() {
    let (node, background, border) = resolve(&swatch(), &[]);
    assert_eq!(node.width, Val::Px(10.0), "base survives");
    assert_eq!(node.height, Val::Px(1.0), "default size=small");
    assert_eq!(
        background, BLUE,
        "default color=blue, beating the base white"
    );
    assert_eq!(border, Color::NONE, "compound needs size=large");
}

#[test]
fn selection_overrides_a_default() {
    let (node, background, _) = resolve(&swatch(), &[("color", "red")]);
    assert_eq!(node.height, Val::Px(1.0), "size still defaults to small");
    assert_eq!(background, RED, "explicit color wins over the default");
}

#[test]
fn a_later_variant_beats_the_base() {
    // The base paints white; the chosen color variant must override it, not the other way round.
    let (_, background, _) = resolve(&swatch(), &[("color", "blue")]);
    assert_eq!(background, BLUE);
}

#[test]
fn a_compound_applies_only_on_a_full_match() {
    let matched = resolve(&swatch(), &[("size", "large"), ("color", "blue")]);
    assert_eq!(matched.2, Color::BLACK, "both pairs match the compound");

    let partial = resolve(&swatch(), &[("size", "large"), ("color", "red")]);
    assert_eq!(partial.2, Color::NONE, "color=red misses the compound");
}

#[test]
fn button_variant_props_drive_its_recipe() {
    let mut ui = Ui::new();
    ui.render(Button::default().variant("outline").label("Play"));
    let button = ui.children()[0];
    assert_eq!(
        ui.get::<BackgroundColor>(button).map(|paint| paint.0),
        Some(Color::NONE),
        "the outline variant is transparent"
    );
    assert_eq!(
        ui.texts(),
        vec!["Play".to_owned()],
        "the label renders as themed text"
    );
}

#[test]
fn button_defaults_to_the_primary_intent() {
    let mut ui = Ui::new();
    ui.render(Button::default().label("Play"));
    let button = ui.children()[0];
    assert_eq!(
        ui.get::<BackgroundColor>(button).map(|paint| paint.0),
        Some(color::primary_base.resolve(&dark::theme())),
        "the default variant is the primary intent, resolved against the default (dark) theme"
    );
}
