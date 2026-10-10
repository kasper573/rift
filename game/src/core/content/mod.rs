pub mod fixture;
pub mod id;
pub mod key;
pub mod table;

pub use fixture::Fixture;
pub use id::{ContentId, ContentScope, UnknownRow, with_content};
pub use key::{KeyBuf, key_of, sorted_index};
pub use table::{Content, ContentBuilder, ContentRow, ModuleRule, ModuleSet, Table};

pub fn named<Row: ContentRow>(
    world: &bevy_ecs::world::World,
    key: &str,
) -> Result<ContentId<Row>, String> {
    world
        .resource::<Content>()
        .by_key::<Row>(key)
        .ok_or_else(|| format!("`{key}` is not a known {}", Row::TABLE))
}
