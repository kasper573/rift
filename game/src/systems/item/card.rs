use bevy::prelude::*;
use bevy::scene::EntityScene;
use ui::tokens::{palette, spacing, typography};
use ui::{
    ChipOptions, Family, InspectableOptions, OnTap, RichPiece, RichSpan, RichText, TextVoice,
};
use ui::{TooltipText, component};

use crate::core::sfx::SfxId;
use crate::core::sfx::playback::{PlaySfx, SfxPlace};
use crate::data::item::Id as ItemId;
use crate::systems::effect::Effect;
use crate::systems::hud;
use crate::systems::input::map::{InputAction, input};
use crate::systems::item::{ItemDef, ItemFlag, ItemKind};
use crate::systems::player::session::Viewpoint;
use crate::systems::scene::Scene as GameScene;
use crate::systems::stat::Stat;

const WINDOW_ID: &str = "Item";
const WINDOW_SIZE: Vec2 = Vec2::new(300.0, 360.0);
const WINDOW_TOP: f32 = 96.0;
const WINDOW_MARGIN: f32 = 72.0;
const ICON: f32 = 48.0;

pub struct ItemCardPlugin;

impl Plugin for ItemCardPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ItemCard>()
            .add_systems(Update, show_card.run_if(in_state(GameScene::Area)))
            .add_systems(
                OnExit(GameScene::Area),
                (crate::systems::scene::despawn_all::<ItemCardWindow>, forget),
            );
    }
}

#[derive(Component, Clone)]
pub struct ItemCardWindow {
    pub item: ItemId,
}

pub fn open(world: &mut World, item: ItemId) {
    world.resource_mut::<ItemCard>().0 = Some(item);
}

pub fn close(world: &mut World) -> bool {
    world.resource_mut::<ItemCard>().0.take().is_some()
}

pub fn inspectable(item: ItemId) -> InspectableOptions {
    let def = item.get();
    InspectableOptions {
        tooltip: TooltipText {
            title: def.display_name.to_owned(),
            lines: vec![vec![RichPiece::text(overview(def))]],
            hint: Some(inspect_hint()),
        },
        input: InputAction::InspectItem.into(),
        on_inspect: OnTap::new(move |world| open(world, item)),
    }
}

pub fn overview(def: &ItemDef) -> String {
    let category = def.category().label();
    match def.kind {
        ItemKind::Consumable { health_bonus, .. } if health_bonus > 0.0 => {
            format!("{category} · restores {health_bonus} health")
        }
        ItemKind::Equipment { slot, .. } => format!("{category} · {}", slot.label()),
        _ => category.to_owned(),
    }
}

pub fn inspect_hint() -> Vec<RichPiece> {
    vec![
        input(InputAction::InspectItem),
        RichPiece::text(" for more information"),
    ]
}

pub fn use_verb(def: &ItemDef) -> Option<&'static str> {
    match def.kind {
        ItemKind::Consumable { .. } | ItemKind::Usable { .. } => Some("use"),
        ItemKind::Equipment { .. } => Some("wear"),
        ItemKind::Resource => None,
    }
}

pub fn sheet(world: &World, item: ItemId) -> Box<dyn Scene> {
    let assets = world.resource::<AssetServer>();
    let def = item.get();
    let ink = ui::theme::theme().surface_floating.on;
    let tags: Vec<Box<dyn Scene>> = tags(def)
        .into_iter()
        .map(|(label, color)| {
            ui::chip(ChipOptions {
                label: label.to_owned(),
                icon: None,
                family: Family::outline(color),
                inspect: None,
            })
        })
        .collect();
    let facts: Vec<Box<dyn Scene>> = facts(world, def)
        .into_iter()
        .map(|(fact, color)| -> Box<dyn Scene> {
            Box::new(ui::styled_text(fact, color, typography::BODY))
        })
        .collect();
    let flavor = RichText {
        pieces: vec![RichPiece::Span(RichSpan {
            voice: TextVoice::Whisper,
            ..RichSpan::plain(def.flavor)
        })],
        size: typography::BODY.font_size,
        color: ink,
    };
    Box::new(bsn! {
        Node {
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px({spacing::L}),
            width: Val::Percent(100.0),
        }
        Children [
            (
                Node { column_gap: Val::Px({spacing::L}), align_items: AlignItems::Center }
                Children [
                    (
                        Node { width: Val::Px({ICON}), height: Val::Px({ICON}), flex_shrink: 0.0 }
                        component(ImageNode::new(assets.load(def.icon.0)))
                    ),
                    (
                        Node { flex_direction: FlexDirection::Column }
                        Children [
                            {EntityScene(ui::styled_text(def.display_name, ink, typography::NAME))},
                            {EntityScene(ui::styled_text(overview(def), ink.with_alpha(0.6), typography::CAPTION))},
                        ]
                    ),
                ]
            ),
            ( Node { column_gap: Val::Px({spacing::M}), flex_wrap: FlexWrap::Wrap } Children [ {tags} ] ),
            {EntityScene(ui::rich_text(flavor, false))},
            ( Node { flex_direction: FlexDirection::Column, row_gap: Val::Px({spacing::S}) } Children [ {facts} ] ),
        ]
    })
}

#[derive(Resource, Default)]
struct ItemCard(Option<ItemId>);

fn forget(mut card: ResMut<ItemCard>) {
    card.0 = None;
}

fn show_card(world: &mut World) {
    let wanted = world.resource::<ItemCard>().0;
    let shown = world
        .query::<(Entity, &ItemCardWindow)>()
        .iter(world)
        .next()
        .map(|(panel, card)| (panel, card.item));
    if shown.map(|(_, item)| item) == wanted {
        return;
    }
    if let Some((panel, _)) = shown {
        world.entity_mut(panel).despawn();
    }
    let Some(item) = wanted else {
        chime(world, SfxId::UiClose);
        return;
    };
    let screen = world
        .query::<&bevy::window::Window>()
        .single(world)
        .map_or(WINDOW_SIZE, |window| window.resolution.size());
    let fallback = Vec2::new(
        (screen.x - WINDOW_SIZE.x - WINDOW_MARGIN).max(8.0),
        WINDOW_TOP,
    );
    let scene = hud::placed_window(
        world,
        WINDOW_ID,
        (fallback, WINDOW_SIZE),
        OnTap::new(|world| {
            close(world);
        }),
        hud::single_tab(
            item.get().display_name,
            ui::scrolled(Box::new(padded(sheet(world, item)))),
        ),
    );
    if let Some(panel) = hud::spawn_in_hud(world, scene) {
        world.entity_mut(panel).insert(ItemCardWindow { item });
    }
    if shown.is_none() {
        chime(world, SfxId::UiOpen);
    }
}

fn padded(sheet: Box<dyn Scene>) -> impl Scene {
    bsn! {
        Node { padding: {UiRect::all(Val::Px(spacing::XL))}, width: Val::Percent(100.0) }
        Children [ {EntityScene(sheet)} ]
    }
}

fn tags(def: &ItemDef) -> Vec<(&'static str, Color)> {
    [
        (ItemFlag::Quest, "Quest item", palette::AMBER_70),
        (ItemFlag::Bound, "Bound", palette::CRIMSON_70),
    ]
    .into_iter()
    .filter(|(flag, ..)| def.has(*flag))
    .map(|(_, label, color)| (label, color))
    .collect()
}

fn facts(world: &World, def: &ItemDef) -> Vec<(String, Color)> {
    let ink = ui::theme::theme().surface_floating.on;
    let plain = |fact: String| (fact, ink);
    let mut facts = Vec::new();
    match def.kind {
        ItemKind::Consumable {
            health_bonus,
            duration,
        } => {
            if health_bonus > 0.0 {
                facts.push(plain(format!("Restores {health_bonus} health")));
            }
            facts.extend(
                modifiers(def).map(|stat| plain(format!("{} for {}s", modifier(stat), duration.0))),
            );
        }
        ItemKind::Equipment { slot, requirements } => {
            facts.push(plain(format!(
                "Worn as your {}",
                slot.label().to_lowercase()
            )));
            facts.extend(modifiers(def).map(|stat| plain(modifier(stat))));
            let viewer = world.resource::<Viewpoint>().0;
            facts.extend(requirements.iter().map(|requirement| {
                let color = match viewer.map(|viewer| requirement.met(world, viewer)) {
                    Some(true) => palette::EMERALD_80,
                    Some(false) => palette::CRIMSON_80,
                    None => ink,
                };
                (format!("Requires {}", requirement.describe()), color)
            }));
        }
        ItemKind::Usable { .. } => facts.push(plain("Can be used".to_owned())),
        ItemKind::Resource => facts
            .extend(modifiers(def).map(|stat| plain(format!("{} while carried", modifier(stat))))),
    }
    match def.stack_max() {
        1 => {}
        u32::MAX => facts.push(plain("Stacks without limit".to_owned())),
        max => facts.push(plain(format!("Stacks up to {max}"))),
    }
    if def.bound() {
        facts.push(plain("Can't be sold or dropped".to_owned()));
    }
    facts
}

fn modifiers(def: &ItemDef) -> impl Iterator<Item = Stat> {
    def.effects.iter().filter_map(|effect| match effect {
        Effect::StatModifier(stat) => Some(*stat),
        Effect::Chasing => None,
    })
}

fn modifier(stat: Stat) -> String {
    let words: String = stat
        .label()
        .chars()
        .enumerate()
        .flat_map(|(index, letter)| {
            let gap = (index > 0 && letter.is_uppercase()).then_some(' ');
            gap.into_iter().chain(std::iter::once(letter))
        })
        .collect();
    format!("{:+} {words}", stat.value)
}

fn chime(world: &mut World, id: SfxId) {
    world.write_message(PlaySfx {
        id,
        place: SfxPlace::Interface,
    });
}
