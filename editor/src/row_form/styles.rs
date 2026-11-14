//! A rarity and a card style as the game draws them, beside their forms: on a real item, quest
//! or skill that uses them, or on a plain stand-in while nothing does.

use engine::core::card::{CardStyleDef, CardStyleFixture, CardStyleId};
use engine::core::content::{Content, Fixture};
use engine::core::gui::ImageSource;
use engine::core::math::ColorGradient;
use engine::systems::GameSettings;
use engine::systems::item::card::{self as item_card, CardLook};
use engine::systems::item::rarity::ItemRarityDef;
use engine::systems::item::{ItemDef, ItemId, widget};
use engine::systems::quest::QuestDef;
use engine::systems::skill::{self, SkillDef};
use ui::card::{self, Art, Card};
use ui::egui::Ui;
use ui::glyph::CardKind;
use ui::theme;

use super::preview::quest_card;
use crate::app::Editor;
use crate::thumbs::ProjectImages;

/// A rarity where it shows on an item of it: in a slot and on a card.
pub(super) fn show_rarity(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let ctx = ui.ctx().clone();
    let Some(content) = editor.loaded() else {
        ui.label(theme::muted("the preview shows once the content loads"));
        return;
    };
    let Some(rarity) = content.by_path::<ItemRarityDef>(key) else {
        ui.label(theme::muted("the preview shows once the row loads"));
        return;
    };
    let Editor { project, form, .. } = &mut *editor;
    let mut images = ProjectImages {
        ctx: &ctx,
        files: &project.as_ref().expect("a project").files,
        thumbnails: &mut form.thumbnails,
    };
    let item = content
        .ids::<ItemDef>()
        .find(|item| item.get(&content).rarity == rarity);
    widget::shown_slot(ui, &mut images, &content, rarity, item);
    ui.add_space(12.0);
    match item {
        Some(item) => show_item(ui, &mut images, &content, item),
        None => {
            let def = rarity.get(&content);
            let ground = content
                .built::<GameSettings>()
                .item_type_art_colors
                .resource;
            stand_in(
                ui,
                &mut images,
                CardStyleDef::of(&content, def.card_style),
                Some(ground),
                &def.name,
            );
        }
    }
}

/// A card style on a card that is drawn in it.
pub(super) fn show_card_style(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let ctx = ui.ctx().clone();
    let Some(content) = editor.loaded() else {
        ui.label(theme::muted("the preview shows once the content loads"));
        return;
    };
    let Some(style) = content.by_path::<CardStyleDef>(key) else {
        ui.label(theme::muted("the preview shows once the row loads"));
        return;
    };
    let Editor { project, form, .. } = &mut *editor;
    let mut images = ProjectImages {
        ctx: &ctx,
        files: &project.as_ref().expect("a project").files,
        thumbnails: &mut form.thumbnails,
    };
    let default = CardStyleFixture::Default.id(&content);
    let wears = |named: Option<CardStyleId>| named.unwrap_or(default) == style;
    let item = content
        .ids::<ItemDef>()
        .find(|item| wears(item.get(&content).rarity.get(&content).card_style));
    let quest = content
        .ids::<QuestDef>()
        .find(|quest| wears(quest.get(&content).card_style));
    let skill = content
        .ids::<SkillDef>()
        .find(|skill| wears(skill.get(&content).card_style));
    if let Some(item) = item {
        show_item(ui, &mut images, &content, item);
    } else if let Some(quest) = quest {
        quest_card(ui, &mut images, &content, quest.path(&content));
    } else if let Some(skill) = skill {
        let def = skill.get(&content);
        let mut face = Card::new(skill::card::kind(def), &def.name);
        face.dress = style.get(&content).dress(&mut images, None);
        face.art = Art {
            icon: images.image(&def.icon).map(|image| image.texture),
            height: card::ART,
        };
        face.width = ui.available_width().min(card::WIDTH);
        face.show(ui, |_, _| {});
    } else {
        stand_in(ui, &mut images, style.get(&content), None, "Card");
    }
}

fn show_item(ui: &mut Ui, images: &mut ProjectImages, content: &Content, item: ItemId) {
    let look = CardLook {
        width: ui.available_width().min(card::WIDTH),
        closable: false,
        ..CardLook::PINNED
    };
    let face = item_card::face_of(content, images, item, look);
    face.show(ui, |ui, plate| {
        item_card::summary(ui, content, item, plate);
        item_card::details(ui, content, item, |_| None);
        item_card::grant_lines(ui, images, content, item, plate);
    });
}

/// A card of nothing in particular, drawn in `style`.
fn stand_in(
    ui: &mut Ui,
    images: &mut ProjectImages,
    style: &CardStyleDef,
    ground: Option<ColorGradient>,
    name: &str,
) {
    let mut face = Card::new(CardKind::Resource, name);
    face.dress = style.dress(images, ground);
    face.width = ui.available_width().min(card::WIDTH);
    face.show(ui, |ui, plate| {
        card::overview(ui, "A card with nothing on it yet");
        card::heading(ui, plate, "a section");
        card::fact(ui, "What it says goes here");
    });
}
