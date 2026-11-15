//! The Social tab of the menu: a page for each thing players share that the game has on.

use bevy::prelude::*;
use ui::egui::Ui;

use super::SocietyMember;
use crate::systems::friend::page::FriendsPage;
use crate::systems::hud::menu::{MenuTab, MenuTabName, SubTabs};
use crate::systems::input::map::InputAction;
use crate::systems::interface::InterfaceIcon;

/// A page of the Social tab, brought by the system it shows.
pub trait SocialPage: Send + Sync {
    fn title(&self) -> &'static str;
    /// Whether the game has the page's system on.
    fn on(&self, world: &World) -> bool;
    /// How much waits on the page for the player to see or answer.
    fn waiting(&self, world: &World) -> u32;
    /// The inputs that do something on the page as it stands, and what each does.
    fn hints(&self, _world: &World) -> Vec<(InputAction, &'static str)> {
        Vec::new()
    }
    fn show(&self, ui: &mut Ui, world: &mut World);
}

/// Something a page offers to do with a player it lists, brought by the system that does it.
pub trait PlayerAction: Send + Sync {
    fn label(&self) -> &'static str;
    /// Whether the game has the action's system on.
    fn on(&self, world: &World) -> bool;
    fn act(&self, world: &mut World, player: &SocietyMember);
}

/// What the systems the game has on offer to do with a player.
pub fn player_actions(world: &World) -> Vec<&'static dyn PlayerAction> {
    PLAYER_ACTIONS
        .iter()
        .copied()
        .filter(|action| action.on(world))
        .collect()
}

pub struct SocialMenuTab;

impl MenuTab for SocialMenuTab {
    fn name(&self) -> MenuTabName {
        MenuTabName::Social
    }
    fn icon(&self) -> InterfaceIcon {
        InterfaceIcon::Social
    }
    fn opens(&self) -> InputAction {
        InputAction::OpenSocial
    }
    fn shown(&self, world: &World) -> bool {
        !pages(world).is_empty()
    }
    fn badge(&self, world: &World) -> Option<u32> {
        let waiting: u32 = pages(world).iter().map(|page| page.waiting(world)).sum();
        (waiting > 0).then_some(waiting)
    }
    fn sub_tabs(&self, world: &World) -> Option<SubTabs> {
        let pages = pages(world);
        let open = open_page(world)?;
        Some(SubTabs {
            labels: pages
                .iter()
                .map(|page| match page.waiting(world) {
                    0 => page.title().to_owned(),
                    waiting => format!("{} · {waiting}", page.title()),
                })
                .collect(),
            chosen: pages
                .iter()
                .position(|page| page.title() == open.title())
                .unwrap_or_default(),
        })
    }
    fn choose_sub_tab(&self, world: &mut World, chosen: usize) {
        if let Some(page) = pages(world).get(chosen) {
            world.resource_mut::<LastPage>().0 = Some(page.title());
        }
    }
    fn hints(&self, world: &World) -> Vec<(InputAction, &'static str)> {
        open_page(world).map_or_else(Vec::new, |page| page.hints(world))
    }
    fn show(&self, ui: &mut Ui, world: &mut World) {
        if let Some(page) = open_page(world) {
            page.show(ui, world);
        }
    }
}

pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<LastPage>();
}

/// The page the player used last, by its title.
#[derive(Resource, Default)]
struct LastPage(Option<&'static str>);

// Their sub-tabs line up in this order.
static PAGES: &[&dyn SocialPage] = &[&FriendsPage];

static PLAYER_ACTIONS: &[&dyn PlayerAction] = &[];

/// The pages of the systems the game has on.
fn pages(world: &World) -> Vec<&'static dyn SocialPage> {
    PAGES
        .iter()
        .copied()
        .filter(|page| page.on(world))
        .collect()
}

/// The page the tab shows: the one used last, or the first.
fn open_page(world: &World) -> Option<&'static dyn SocialPage> {
    let pages = pages(world);
    let last = world.resource::<LastPage>().0;
    pages
        .iter()
        .copied()
        .find(|page| Some(page.title()) == last)
        .or_else(|| pages.first().copied())
}
