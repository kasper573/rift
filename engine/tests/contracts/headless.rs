use std::collections::BTreeMap;

use bevy::app::App;
use bevy::ecs::message::Messages;
use bevy_replication::ClientMessageRefusal;
use engine::core::content::{ContentScope, with_content};
use engine::systems::item::{ItemDef, ItemStack};
use engine::systems::player::headless::write_request;
use engine::systems::player::session::ClientSessionPlugin;
use engine::systems::trade::TradeRequest;
use serde_json::json;
use testkit::{Game, Map, path};

#[test]
fn a_client_run_from_outside_names_content_by_its_path() {
    let content = Game::new()
        .area("yard", &Map::open(4, 4), json!([]))
        .hero("yard", 1.0, 1.0)
        .sword()
        .engine_rows()
        .load();
    let sword = content
        .by_path::<ItemDef>(&path("item", "sword"))
        .expect("the game has a sword");
    let give = TradeRequest::Give(ItemStack {
        item: sword,
        count: 2,
    });
    let (asked, _) = with_content(ContentScope::of(&content), || {
        serde_json::to_string(&BTreeMap::from([("TradeRequest", &give)])).expect("a request")
    });
    assert!(
        asked.contains(&path("item", "sword")),
        "{asked} names the sword by its path"
    );
    let mut client = App::new();
    client
        .add_plugins(ClientSessionPlugin)
        .insert_resource(content);

    let spoon = asked.replace("sword", "spoon");
    let refused = write_request(
        client.world_mut(),
        serde_json::from_str(&spoon).expect("JSON"),
    );
    assert!(
        matches!(refused, Err(ClientMessageRefusal::Unreadable(_))),
        "a path the game does not hold names nothing, yet {spoon} was taken as {refused:?}"
    );
    write_request(
        client.world_mut(),
        serde_json::from_str(&asked).expect("JSON"),
    )
    .expect("a request naming the game's own content is written");

    let written: Vec<TradeRequest> = client
        .world_mut()
        .resource_mut::<Messages<TradeRequest>>()
        .drain()
        .collect();
    assert_eq!(written, [give]);
}
