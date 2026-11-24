use bevy_ecs::prelude::*;
use engine::core::math::{Offset, Pos};
use engine::core::time::{self, UnixMillis};
use engine::core::units::Meters;
use engine::systems::area::{AreaDef, AreaTag};
use engine::systems::identity::{self, Names};
use engine::systems::item::plates::{self, ItemNaming, Landed, Named, NamingItems, PointedItem};
use engine::systems::item::rarity::RarityRank;
use engine::systems::item::{DroppedItem, ItemDef, ItemId, Reservation, render};
use engine::systems::movement::{Position, position};
use engine::systems::player::ClientId;
use engine::systems::player::session::{MyCharacter, MyClient};
use serde_json::json;
use testkit::{Game, Map, Sim, path};

const ME: ClientId = ClientId(1);
const THEM: ClientId = ClientId(2);

#[test]
fn an_item_on_the_ground_is_named_by_its_rarity_by_the_key_or_under_the_pointer() {
    let (mut sim, at) = field();
    let bone = lie(&mut sim, "bone", at + Offset::new(2.0, 0.0), None);
    let ring = lie(&mut sim, "ring", at + Offset::new(3.0, 0.0), None);
    let crown = lie(&mut sim, "crown", at + Offset::new(4.0, 0.0), None);
    let out_of_reach = lie(&mut sim, "crown", at + Offset::new(20.0, 0.0), None);
    let theirs = lie(&mut sim, "crown", at + Offset::new(5.0, 0.0), Some(THEM));

    assert_eq!(
        names(&mut sim),
        sorted(vec![(ring, Named::Unprompted), (crown, Named::Unprompted)])
    );
    sim.run(6.0);
    assert_eq!(names(&mut sim), vec![(crown, Named::Unprompted)]);

    sim.world.insert_resource(NamingItems(true));
    assert_eq!(
        names(&mut sim),
        sorted(vec![
            (bone, Named::ByKey),
            (ring, Named::ByKey),
            (crown, Named::ByKey)
        ])
    );
    sim.world.insert_resource(PointedItem {
        item: Some(bone),
        plate: Some(Named::ByKey),
    });
    assert_eq!(
        names(&mut sim),
        sorted(vec![
            (bone, Named::ByKey),
            (ring, Named::ByKey),
            (crown, Named::ByKey)
        ]),
        "a name asked for stays asked for under the pointer"
    );
    sim.world.insert_resource(PointedItem::default());
    sim.world.insert_resource(ItemNaming {
        from: RarityRank(2),
        ..ItemNaming::default()
    });
    assert_eq!(
        names(&mut sim),
        sorted(vec![(ring, Named::ByKey), (crown, Named::ByKey)])
    );

    sim.world.insert_resource(NamingItems(false));
    sim.world.insert_resource(ItemNaming {
        as_dropped: false,
        ..ItemNaming::default()
    });
    assert_eq!(names(&mut sim), vec![]);
    for pointed in [out_of_reach, theirs] {
        sim.world.insert_resource(PointedItem {
            item: Some(pointed),
            plate: None,
        });
        assert_eq!(names(&mut sim), vec![(pointed, Named::ByPointer)]);
    }

    sim.world.insert_resource(PointedItem::default());
    sim.world.insert_resource(NamingItems(true));
    sim.world.insert_resource(ItemNaming::default());
    sim.world.add_observer(render::leave_its_look);
    identity::vanish(sim.world.entity_mut(crown));
    sim.world.flush();
    assert_eq!(
        names(&mut sim),
        sorted(vec![(bone, Named::ByKey), (ring, Named::ByKey)]),
        "an item taken is named no more, though its look lingers as it fades"
    );
}

#[test]
fn the_names_of_a_pile_stack_by_rank_and_items_of_a_kind_share_a_line() {
    let (mut sim, at) = field();
    sim.world.insert_resource(NamingItems(true));
    let pile = at + Offset::new(3.0, 0.0);
    let nearer = lie(&mut sim, "bone", pile, None);
    let farther = lie(&mut sim, "bone", pile + Offset::new(0.5, 0.0), None);
    let crown = lie(&mut sim, "crown", pile + Offset::new(0.0, 0.5), None);
    let chained = lie(&mut sim, "bone", pile + Offset::new(2.3, 0.0), None);
    let apart = lie(&mut sim, "bone", at + Offset::new(8.0, 0.0), None);

    let named = plates::named(&mut sim.world);
    let mut lines: Vec<Vec<(ItemId, Vec<Entity>, u32)>> = plates::columns(&sim.world, &named)
        .into_iter()
        .map(|column| {
            column
                .lines
                .into_iter()
                .map(|line| (line.item, line.items, line.count))
                .collect()
        })
        .collect();
    lines.sort_by_key(|pile: &Vec<_>| std::cmp::Reverse(pile.len()));

    let (bones, crowns) = (sim.id::<ItemDef>("bone"), sim.id::<ItemDef>("crown"));
    assert_eq!(
        lines,
        vec![
            vec![
                (crowns, vec![crown], 1),
                (bones, vec![nearer, farther, chained], 3)
            ],
            vec![(bones, vec![apart], 1)],
        ]
    );
}

#[test]
fn names_past_what_the_game_shows_at_once_fold_the_lowest_ranks_and_the_farthest_first() {
    let (mut sim, at) = field();
    sim.world.insert_resource(NamingItems(true));
    let crown = lie(&mut sim, "crown", at + Offset::new(11.0, 0.0), None);
    let bones: Vec<Entity> = (0..28)
        .map(|step| {
            let along = 2.0 + 0.25 * step as f32;
            lie(&mut sim, "bone", at + Offset::new(along, 1.0), None)
        })
        .collect();

    let named = plates::named(&mut sim.world);

    let mut folded = named.folded.clone();
    folded.sort();
    let mut farthest = bones[23..].to_vec();
    farthest.sort();
    assert_eq!(folded, farthest);
    assert_eq!(named.shown.first(), Some(&(crown, Named::ByKey)));
}

#[test]
fn a_click_on_a_plate_takes_its_item_when_it_is_the_players_to_take() {
    let (mut sim, at) = field();
    let mine = lie(&mut sim, "bone", at + Offset::new(2.0, 0.0), None);
    let theirs = lie(&mut sim, "bone", at + Offset::new(3.0, 0.0), Some(THEM));
    let cases = [
        (mine, Some(Named::ByKey), Some(mine)),
        (mine, Some(Named::Unprompted), None),
        (mine, None, None),
        (theirs, Some(Named::ByKey), None),
    ];
    for (item, plate, taken) in cases {
        sim.world.insert_resource(PointedItem {
            item: Some(item),
            plate,
        });
        assert_eq!(
            plates::taken_by_plate(&sim.world, &[Named::ByKey]),
            taken,
            "{plate:?}"
        );
    }
}

/// Why each item on the ground is named, in an order of the items that holds from call to call.
fn names(sim: &mut Sim) -> Vec<(Entity, Named)> {
    sorted(plates::named(&mut sim.world).shown)
}

fn sorted(mut names: Vec<(Entity, Named)>) -> Vec<(Entity, Named)> {
    names.sort();
    names
}

/// A field the player stands at one end of, and where they stand: bones are named only when
/// asked, a ring for five seconds from landing, and a crown for as long as it lies in reach.
fn field() -> (Sim, Pos<Meters>) {
    let rarity =
        |name: &str| json!({ "rarity": path("item_rarity", name), "kind": { "type": "resource" } });
    let content = Game::new()
        .area("field", &Map::open(30, 5), json!([]))
        .hero("field", 2.0, 2.0)
        .item("bone", json!({ "kind": { "type": "resource" } }))
        .rarity(
            "fine",
            2,
            json!({ "dropped": { "named": { "type": "for", "seconds": 5.0 } } }),
        )
        .rarity(
            "prized",
            3,
            json!({ "dropped": { "named": { "type": "always" } } }),
        )
        .item("ring", rarity("fine"))
        .item("crown", rarity("prized"))
        .load();
    let mut sim = Sim::new(content);
    sim.serve("field");
    let player = sim.join(ME.0);
    sim.world.insert_resource(MyClient(Some(ME)));
    sim.world.insert_resource(MyCharacter(Some(player)));
    sim.world.init_resource::<ItemNaming>();
    sim.world.init_resource::<NamingItems>();
    sim.world.init_resource::<PointedItem>();
    let at = position(&sim.world, player).expect("position");
    (sim, at)
}

/// An item of the kind named `item` lying at `at` since now, reserved for `by` or for nobody.
fn lie(sim: &mut Sim, item: &str, at: Pos<Meters>, by: Option<ClientId>) -> Entity {
    let area = sim.id::<AreaDef>("field");
    let item = sim.id::<ItemDef>(item);
    let now = time::uptime(&sim.world);
    let name = sim
        .world
        .get_resource_or_insert_with(|| Names::for_world(0))
        .give();
    sim.world
        .spawn((
            Position::at(at),
            AreaTag { area },
            DroppedItem {
                item,
                count: 1,
                until: UnixMillis(u64::MAX),
            },
            Reservation { by, at: now },
            Landed(now),
            name,
        ))
        .id()
}
