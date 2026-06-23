//! `me` resolves the local client's character from its [`Owner`] and reads its components like any
//! other entity — no bespoke per-field accessors, no special "me" character.

use world::protocol::session::{self, MyClient};
use world::{ClientId, Owner, Vitals, World};

#[test]
fn me_is_none_before_the_welcome_assigns_a_client_id() {
    let mut world = World::new();
    world.insert_resource(MyClient(None));
    world.spawn((
        Owner {
            client: ClientId(1),
        },
        Vitals {
            health: 5.0,
            max: 10.0,
        },
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
        Vitals {
            health: 1.0,
            max: 1.0,
        },
    ));
    let mine = world
        .spawn((
            Owner {
                client: ClientId(1),
            },
            Vitals {
                health: 7.0,
                max: 10.0,
            },
        ))
        .id();

    let me = session::me(&world).expect("our character exists once the id is known");
    assert_eq!(me.id(), mine);
    assert_eq!(me.get::<Vitals>().map(|vitals| vitals.health), Some(7.0));
}
