pub mod area;
pub mod attention;
pub mod babble;
pub mod babble_bank;
pub mod dialogue;
pub mod expression;
pub mod input;
pub mod item;
pub mod job;
pub mod memory;
pub mod model;
pub mod notification;
pub mod npc;
pub mod player;
pub mod prop;
pub mod quest;
pub mod sfx;
pub mod shop;

use crate::core::content::{Content, ContentBuilder};

pub fn build() -> Content {
    let mut tables = ContentBuilder::default();
    tables
        .table(area::rows())
        .table(attention::rows())
        .table(babble::rows())
        .table(babble_bank::rows())
        .table(dialogue::rows())
        .table(expression::rows())
        .table(input::rows())
        .table(item::rows())
        .table(job::rows())
        .table(memory::rows())
        .table(model::rows())
        .table(notification::rows())
        .table(npc::rows())
        .table(player::rows())
        .table(prop::rows())
        .table(quest::rows())
        .table(sfx::rows())
        .table(shop::rows());
    tables.build()
}
