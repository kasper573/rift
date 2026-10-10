use strum::VariantArray;

use super::id::ContentId;
use super::table::{Content, ContentRow};

pub trait Fixture: Copy + Into<&'static str> + VariantArray + 'static {
    type Row: ContentRow;

    fn key(self) -> &'static str {
        self.into()
    }

    fn id(self, content: &Content) -> ContentId<Self::Row> {
        content
            .table::<Self::Row>()
            .id_of(self.key())
            .unwrap_or_else(|| {
                panic!(
                    "fixture {}/{} is checked at load",
                    Self::Row::TABLE,
                    self.key()
                )
            })
    }

    fn get(self, content: &Content) -> &Self::Row {
        self.id(content).get(content)
    }

    fn missing(content: &Content) -> Vec<&'static str> {
        Self::VARIANTS
            .iter()
            .map(|fixture| fixture.key())
            .filter(|key| content.table::<Self::Row>().id_of(key).is_none())
            .collect()
    }
}
