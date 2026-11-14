//! A rarity and a card style as the game draws them, beside their forms: on a real item, quest
//! or skill that uses them, or on a plain stand-in while nothing does.

use engine::core::card::{CardMaterial, CardStyleDef, CardStyleFixture, CardStyleId};
use engine::core::content::{Content, Fixture};
use engine::core::gui::ImageSource;
use engine::core::math::ColorGradient;
use engine::core::stage::card::StagedCard;
use engine::systems::GameSettings;
use engine::systems::item::card::{self as item_card, CardLook};
use engine::systems::item::rarity::ItemRarityDef;
use engine::systems::item::{ItemDef, ItemId, widget};
use engine::systems::quest::QuestDef;
use engine::systems::skill::{self, SkillDef};
use ui::card::{self, Card};
use ui::egui::{LayerId, Ui};
use ui::glyph::CardKind;
use ui::motion;
use ui::theme;

use super::preview::quest_card;
use crate::app::Editor;
use crate::forms::FormState;
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
    let FormState {
        thumbnails, staged, ..
    } = form;
    let mut images = ProjectImages {
        ctx: &ctx,
        files: &project.as_ref().expect("a project").files,
        thumbnails,
    };
    let item = content
        .ids::<ItemDef>()
        .find(|item| item.get(&content).rarity == rarity);
    widget::shown_slot(ui, &mut images, &content, rarity, item);
    ui.add_space(12.0);
    let material = &CardStyleDef::of(&content, rarity.get(&content).card_style).material;
    on_stage(staged, ui, material, |ui| match item {
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
    });
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
    let FormState {
        thumbnails, staged, ..
    } = form;
    let mut images = ProjectImages {
        ctx: &ctx,
        files: &project.as_ref().expect("a project").files,
        thumbnails,
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
    on_stage(staged, ui, &style.get(&content).material, |ui| {
        if let Some(item) = item {
            show_item(ui, &mut images, &content, item);
        } else if let Some(quest) = quest {
            quest_card(ui, &mut images, &content, quest.path(&content));
        } else if let Some(skill) = skill {
            let def = skill.get(&content);
            let mut face = Card::new(skill::card::kind(def), &def.name);
            face.dress = style.get(&content).dress(&mut images, None);
            face.picture = images.image(&def.icon).map(|image| image.texture);
            face.show(ui, |_, _| {});
        } else {
            stand_in(ui, &mut images, style.get(&content), None, "Card");
        }
    });
}

/// Paints a card and hands it to the game's card stage, to lie under the game's lamp in
/// `material` and turn toward the pointer as it will in the game.
pub(super) fn on_stage(
    staged: &mut Vec<(StagedCard, LayerId)>,
    ui: &mut Ui,
    material: &CardMaterial,
    paint: impl FnOnce(&mut Ui),
) {
    let layer = ui.layer_id();
    let since = motion::mark(ui.ctx(), layer);
    let flat = ui.scope(paint).response.rect;
    let card = StagedCard {
        key: ui.id().with("staged card"),
        runs: motion::lifted(ui.ctx(), layer, since),
        flat,
        pose: None,
        turns: true,
        material: material.clone(),
    };
    staged.push((card, layer));
}

fn show_item(ui: &mut Ui, images: &mut ProjectImages, content: &Content, item: ItemId) {
    let look = CardLook {
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
    face.show(ui, |ui, plate| {
        card::overview(ui, "A card with nothing on it yet");
        card::heading(ui, plate, "a section");
        card::fact(ui, "What it says goes here");
    });
}
