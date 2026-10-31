use bevy::prelude::*;
use engine::core::math::Pos;
use engine::core::units::Meters;
use engine::systems::identity::{self, ClientIdentityPlugin, Lingering, Names};
use engine::systems::movement::RenderPosition;
use engine::systems::player::session::MyClient;
use engine::systems::player::{ClientId, PlayedBy};

/// While a client switches worlds (its own character gone from the one it was in, not yet come
/// in the next), what the server despawns keeps being shown as it was, and what the next world
/// replicates under the same name takes it over, figure and all, so the view never gaps.
#[test]
fn what_comes_back_under_its_name_while_the_client_switches_worlds_keeps_its_depiction() {
    let mut app = client();
    let mut names = Names::for_world(0);
    let (me, brute) = (names.give(), names.give());
    let world = app.world_mut();
    let hero = world
        .spawn((
            me,
            PlayedBy { client: MINE },
            RenderPosition(Pos::new(1.0, 2.0)),
        ))
        .id();
    let figure = world.spawn(ChildOf(hero)).id();
    let other = world
        .spawn((brute, RenderPosition(Pos::new(3.0, 4.0))))
        .id();
    vanish(world, other);
    vanish(world, hero);
    app.update();
    let world = app.world_mut();
    assert_eq!(
        shown(world),
        2,
        "both keep being shown while the character is away"
    );

    let hero_back = world.spawn((me, PlayedBy { client: MINE })).id();
    let brute_back = world.spawn(brute).id();
    app.update();
    let world = app.world_mut();
    assert_eq!(played_back(world, hero_back), Some(Pos::new(1.0, 2.0)));
    assert_eq!(
        world.get::<ChildOf>(figure).map(ChildOf::parent),
        Some(hero_back),
        "the figure follows its character"
    );
    assert_eq!(played_back(world, brute_back), Some(Pos::new(3.0, 4.0)));
    assert_eq!(shown(world), 2, "nothing stands in for anything any more");
    assert!(world.get_entity(hero).is_err() && world.get_entity(other).is_err());
}

/// What the next world doesn't replicate goes once the character is back; and outside a switch,
/// a despawn is a despawn, that frame.
#[test]
fn what_doesnt_come_back_goes_with_the_switch_and_a_despawn_outside_one_goes_at_once() {
    let mut app = client();
    let mut names = Names::for_world(0);
    let (me, brute, stray) = (names.give(), names.give(), names.give());
    let world = app.world_mut();
    let hero = world
        .spawn((
            me,
            PlayedBy { client: MINE },
            RenderPosition(Pos::new(1.0, 2.0)),
        ))
        .id();
    let other = world
        .spawn((brute, RenderPosition(Pos::new(3.0, 4.0))))
        .id();
    vanish(world, other);
    vanish(world, hero);
    world.spawn((me, PlayedBy { client: MINE }));
    app.update();
    let world = app.world_mut();
    assert_eq!(shown(world), 1, "the brute went with the old world");

    let passerby = world
        .spawn((stray, RenderPosition(Pos::new(5.0, 6.0))))
        .id();
    vanish(world, passerby);
    app.update();
    assert_eq!(
        shown(app.world_mut()),
        1,
        "a despawn outside a switch shows nothing"
    );
}

const MINE: ClientId = ClientId(1);

fn client() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, ClientIdentityPlugin));
    app.insert_resource(MyClient(Some(MINE)));
    app
}

/// A server's despawn, as replicon lands it on the client.
fn vanish(world: &mut World, entity: Entity) {
    identity::vanish(world.entity_mut(entity));
}

fn played_back(world: &World, entity: Entity) -> Option<Pos<Meters>> {
    world
        .get::<RenderPosition>(entity)
        .map(|position| position.0)
}

/// How many depictions the client shows: entities with a played-back position, stand-ins among
/// them.
fn shown(world: &mut World) -> usize {
    world
        .query_filtered::<(), Or<(With<RenderPosition>, With<Lingering>)>>()
        .iter(world)
        .count()
}
