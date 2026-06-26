//! `me` resolves the local client's character from its [`Owner`] and reads its components like any
//! other entity — no bespoke per-field accessors, no special "me" character.

use bevy_ecs::world::World;
use world::systems::player::session::{self, MyClient};
use world::systems::player::{ClientId, Owner};
use world::systems::stat::Health;

#[test]
fn me_is_none_before_the_welcome_assigns_a_client_id() {
    let mut world = World::new();
    world.insert_resource(MyClient(None));
    world.spawn((
        Owner {
            client: ClientId(1),
        },
        Health(5.0),
    ));
    assert!(session::me(&world).is_none());
}

#[test]
fn me_finds_the_owned_character_and_reads_its_components() {
    let mut world = World::new();
    world.insert_resource(MyClient(Some(ClientId(1))));
    world.spawn((
        Owner {
            client: ClientId(2),
        },
        Health(1.0),
    ));
    let mine = world
        .spawn((
            Owner {
                client: ClientId(1),
            },
            Health(7.0),
        ))
        .id();

    let me = session::me(&world).expect("our character exists once the id is known");
    assert_eq!(me.id(), mine);
    assert_eq!(me.get::<Health>().map(|health| health.0), Some(7.0));
}
