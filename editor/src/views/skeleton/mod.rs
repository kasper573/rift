//! The skeleton view, on five pages: rigs, sets, hands, body and layers. Every page works from the
//! skeleton's file as it stands, with our rig put together from the files it names and every clip
//! of the game folder that plays on it, so a skeleton is set up from nothing. The stage shows the
//! rig as a model wearing it, or while none loads as its joints, posed as the game poses them.

pub mod body;
pub mod edits;
pub mod grips;
pub mod hands;
pub mod inspect;
pub mod layers;
pub mod pickers;
pub mod preview;
pub mod rigging;
pub mod rigs;
pub mod sets;
pub mod staging;

use std::collections::HashMap;
use std::sync::Arc;

use engine::core::content::Content;
use engine::core::content::gltf::{ClipKeys, NodeTree};
use engine::systems::actor::skeleton::{SkeletonDef, SkeletonId};
use serde::Deserialize;
use ui::chip;
use ui::egui::{self, Ui};

use self::rigging::{RiggedFrom, Rigging};
use super::model::hitbox::HitboxHandles;
use super::stage::{self, StageToggle};
use crate::app::Editor;
use crate::views::ViewMap;

#[derive(Default)]
pub struct SkeletonViews {
    pub views: ViewMap<SkeletonView>,
    rigged: HashMap<String, (RiggedFrom, Arc<Rigging>)>,
    posed: HashMap<(usize, StageKey), Posed>,
}

/// The keys a clip plays on a rig, with the rig, which the entry holds so no other stands where it
/// stood.
pub struct Posed {
    rig: Arc<NodeTree>,
    keys: Option<Arc<ClipKeys>>,
}

/// A stage clip as a key: what it plays and through which pairs.
type StageKey = (String, usize, Option<Vec<(String, String)>>);

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkeletonPage {
    #[default]
    Rigs,
    Sets,
    Hands,
    Body,
    Layers,
}

impl SkeletonPage {
    const ALL: [SkeletonPage; 5] = [
        SkeletonPage::Rigs,
        SkeletonPage::Sets,
        SkeletonPage::Hands,
        SkeletonPage::Body,
        SkeletonPage::Layers,
    ];

    fn label(self) -> &'static str {
        match self {
            SkeletonPage::Rigs => "rigs",
            SkeletonPage::Sets => "sets",
            SkeletonPage::Hands => "hands",
            SkeletonPage::Body => "body",
            SkeletonPage::Layers => "layers",
        }
    }
}

pub struct SkeletonView {
    pub page: SkeletonPage,
    pub preview: preview::SkeletonPreview,
    /// The hand the hands page edits, by its place in the list.
    pub hand: usize,
    pub rigs: rigs::RigsState,
    pub sets: sets::SetsState,
    pub hands: hands::HandsState,
    pub layers: layers::LayersState,
    /// What is picked on the stage, for the inspector to set by number.
    pub chosen: Option<Chosen>,
    pub hitbox: HitboxHandles,
    pub grips: grips::Grips,
}

impl SkeletonView {
    /// Whether the page in front takes the next click on a joint of the stage.
    fn picks_joints(&self) -> bool {
        match self.page {
            SkeletonPage::Rigs => self.rigs.pairing.is_some(),
            SkeletonPage::Hands => self.hands.picking.is_some(),
            SkeletonPage::Layers => self.layers.picking,
            SkeletonPage::Sets | SkeletonPage::Body => false,
        }
    }
}

/// A thing on the stage the inspector sets by number once it is clicked.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Chosen {
    /// The hitbox of the model wearing the rig.
    Hitbox,
    /// The grip frame of the hand at this place in the list.
    Grip(usize),
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let Some(def) = super::read_shown::<SkeletonDef>(editor, ui, key) else {
        return;
    };
    ensure_view(editor, key);
    let Some(rigging) = rigging(editor, key, &def) else {
        return;
    };
    let page = editor.views.skeleton.views.view(key).page;
    ui::panel::inspector(ui, editor, "skeleton", |editor, ui| {
        inspect::show(editor, ui, key, &def, &rigging, page);
    });
    egui::Frame::new()
        .inner_margin(crate::widgets::VIEW_PADDING)
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            page_bar(editor, ui, key, page);
            ui.add_space(4.0);
            match page {
                SkeletonPage::Rigs => rigs::show(editor, ui, key, &def, &rigging),
                SkeletonPage::Sets => sets::show(editor, ui, key, &def, &rigging),
                SkeletonPage::Hands => hands::show(editor, ui, key, &def, &rigging),
                SkeletonPage::Body => body::show(editor, ui, key, &def, &rigging),
                SkeletonPage::Layers => layers::show(editor, ui, key, &def, &rigging),
            }
        });
}

fn page_bar(editor: &mut Editor, ui: &mut Ui, key: &str, page: SkeletonPage) {
    let picked = stage::bar(
        ui,
        key,
        &mut editor.views.toggles,
        &[
            StageToggle::Turning,
            StageToggle::Skeleton,
            StageToggle::Grips,
            StageToggle::Hitbox,
        ],
        |ui| {
            SkeletonPage::ALL
                .into_iter()
                .filter(|other| chip::mode(ui, other.label(), page == *other).clicked())
                .last()
        },
    );
    if let Some(picked) = picked {
        editor.views.skeleton.views.view_mut(key).page = picked;
    }
}

pub fn ensure_view(editor: &mut Editor, key: &str) {
    if editor.views.skeleton.views.contains(key) {
        return;
    }
    editor.views.skeleton.views.insert(
        key,
        SkeletonView {
            page: SkeletonPage::default(),
            preview: preview::SkeletonPreview::default(),
            hand: 0,
            rigs: rigs::RigsState::default(),
            sets: sets::SetsState::default(),
            hands: hands::HandsState::default(),
            layers: layers::LayersState::default(),
            chosen: None,
            hitbox: HitboxHandles::default(),
            grips: grips::Grips::default(),
        },
    );
}

/// Our rig and the clips that play on it for the skeleton at `key` as its file reads `def`, worked
/// out again only when the files or what it names of them change.
pub fn rigging(editor: &mut Editor, key: &str, def: &SkeletonDef) -> Option<Arc<Rigging>> {
    let Editor { project, views, .. } = editor;
    let files = &project.as_ref()?.files;
    let from = RiggedFrom {
        revision: files.revision(),
        rig: def.rig.clone(),
        sources: def.sources.clone(),
    };
    let gltfs = &mut views.gltfs;
    let views = &mut views.skeleton;
    if let Some((held, rigged)) = views.rigged.get(key)
        && *held == from
    {
        return Some(rigged.clone());
    }
    let rigged = Arc::new(rigging::rig_up(def, files, gltfs));
    views.rigged.insert(key.to_owned(), (from, rigged.clone()));
    Some(rigged)
}

/// The skeleton at `key` as the game built it from the files as they last loaded, when it loaded.
pub(crate) fn skeleton_id(content: &Content, key: &str) -> Option<SkeletonId> {
    content.by_path::<SkeletonDef>(key)
}
