use bevy::prelude::*;
use bevy::scene::EntityScene;
use ui::button::intent;
use ui::component;
use ui::{
    Activate, ButtonSize, DragHandle, DragRoot, OnSettle, RichPiece, RichText, button_styled,
    text_colored,
};

use super::{SpectateRequest, Spectating};
use crate::systems::actor::Name;
use crate::systems::hud::{BORDER, HudAudience, PANEL_BG, Widget, persist_widget};
use crate::systems::input::map::{self, InputAction};
use crate::systems::player::session::{self, SpectateStatus, Viewpoint};
use crate::systems::scene::mode::Mode;

const BLANK_BG: Color = Color::srgb(0.07, 0.07, 0.07);
const HINT: Color = Color::srgb(0.6, 0.6, 0.6);

pub struct SpectatorPlugin;

impl Plugin for SpectatorPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (step_keys.run_if(not(ui::typing)), show_nobody_playing)
                .run_if(in_state(crate::systems::scene::Scene::Area))
                .run_if(resource_equals(Mode::Spectate)),
        )
        .add_systems(
            OnExit(crate::systems::scene::Scene::Area),
            crate::systems::scene::despawn_all::<NobodyPlaying>,
        );
    }
}

pub struct SpectatorWidget;

impl Widget for SpectatorWidget {
    fn audience(&self) -> HudAudience {
        HudAudience::Spectators
    }
    fn fallback(&self, _: f32) -> Vec2 {
        Vec2::new(8.0, 8.0)
    }
    fn build(&self, pos: Vec2, id: &'static str) -> Box<dyn Scene> {
        build(pos, id)
    }
    fn sync(&self, world: &mut World) {
        let (watched, place) = panel_text(world);
        set_text::<WatchedName>(world, watched);
        set_text::<WatchedPlace>(world, place);
    }
}

#[derive(Component, Default, Clone)]
struct WatchedName;

#[derive(Component, Default, Clone)]
struct WatchedPlace;

#[derive(Component, Default, Clone)]
struct NobodyPlaying;

fn build(pos: Vec2, id: &'static str) -> Box<dyn Scene> {
    let node = Node {
        position_type: PositionType::Absolute,
        left: Val::Px(pos.x),
        top: Val::Px(pos.y),
        min_width: Val::Px(180.0),
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(6.0),
        border: UiRect::all(Val::Px(1.0)),
        padding: UiRect::all(Val::Px(8.0)),
        ..default()
    };
    let controls = Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: Val::Px(8.0),
        ..default()
    };
    Box::new(bsn! {
        template_value(node)
        BackgroundColor({PANEL_BG})
        component(BorderColor::all(BORDER))
        DragRoot
        DragHandle
        component(OnSettle::new(move |world, geom| persist_widget(world, id, geom)))
        Children [
            ( {text_colored(String::new(), Color::WHITE)} WatchedName ),
            (
                template_value(controls)
                Children [
                    ( {button_styled(intent::PRIMARY, ButtonSize::Sm, "<")} on(step(SpectateRequest::Previous)) ),
                    ( {text_colored(String::new(), Color::WHITE)} WatchedPlace ),
                    ( {button_styled(intent::PRIMARY, ButtonSize::Sm, ">")} on(step(SpectateRequest::Next)) ),
                ]
            ),
            {EntityScene(ui::rich_text(switch_hint(), false))},
        ]
    })
}

fn step(request: SpectateRequest) -> impl Fn(On<Activate>, Commands) + Clone {
    move |_, mut commands| {
        commands.queue(move |world: &mut World| session::spectate(world, request));
    }
}

fn switch_hint() -> RichText {
    RichText {
        color: HINT,
        ..RichText::new(vec![
            map::input(InputAction::SpectatePrevious),
            map::input(InputAction::SpectateNext),
            RichPiece::text(" to switch"),
        ])
    }
}

fn step_keys(world: &mut World) {
    let request = if map::just_pressed(world, InputAction::SpectateNext) {
        SpectateRequest::Next
    } else if map::just_pressed(world, InputAction::SpectatePrevious) {
        SpectateRequest::Previous
    } else {
        return;
    };
    session::spectate(world, request);
}

fn panel_text(world: &World) -> (String, String) {
    match world.resource::<SpectateStatus>().0 {
        None => (String::new(), String::new()),
        Some(Spectating::Nobody) => ("Nobody is playing".to_owned(), String::new()),
        Some(Spectating::Player(watched)) => {
            let name = world
                .resource::<Viewpoint>()
                .0
                .and_then(|seen| world.get::<Name>(seen));
            (
                name.map_or_else(
                    || "Watching".to_owned(),
                    |name| format!("Watching {}", name.name),
                ),
                format!("{} of {}", watched.place, watched.online),
            )
        }
    }
}

fn set_text<M: Component>(world: &mut World, content: String) {
    let mut texts = world.query_filtered::<&mut Text, With<M>>();
    for mut text in texts.iter_mut(world) {
        if text.0 != content {
            text.0.clone_from(&content);
        }
    }
}

// Without a player to watch there is nothing of the world to see, so none of it shows.
fn show_nobody_playing(world: &mut World) {
    let nobody = world.resource::<SpectateStatus>().0 == Some(Spectating::Nobody);
    let shown = world
        .query_filtered::<Entity, With<NobodyPlaying>>()
        .iter(world)
        .next();
    match (nobody, shown) {
        (true, None) => {
            let _ = world.spawn_scene(nobody_playing());
        }
        (false, Some(screen)) => world.entity_mut(screen).despawn(),
        _ => {}
    }
}

fn nobody_playing() -> impl Scene {
    bsn! {
        NobodyPlaying
        template_value(crate::systems::scene::screen_node())
        BackgroundColor({BLANK_BG})
        GlobalZIndex({ui::tokens::layer::BACKDROP})
        Children [
            {EntityScene(text_colored("Nobody is playing right now", Color::WHITE))},
            {EntityScene(text_colored("Spectating starts as soon as someone joins", HINT))},
        ]
    }
}
