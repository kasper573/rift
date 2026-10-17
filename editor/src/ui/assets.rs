use std::convert::Infallible;

use egui::Ui;

use super::tree::{self, IconImage, TreeIcon, TreeNode, TreeState};
use super::{theme, widgets};
use crate::app::{Editor, RightTab};
use crate::picker::Dragged;
use crate::project::RowSelection;
use crate::thumbs;

#[derive(Default)]
pub struct AssetsState {
    pub search: String,
    pub tree: TreeState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetKind {
    Image,
    Audio,
    Font,
}

impl AssetKind {
    pub const ALL: &[AssetKind] = &[AssetKind::Image, AssetKind::Audio, AssetKind::Font];

    pub fn id(self) -> &'static str {
        match self {
            AssetKind::Image => "image",
            AssetKind::Audio => "audio",
            AssetKind::Font => "font",
        }
    }

    pub fn named(id: &str) -> Option<AssetKind> {
        AssetKind::ALL.iter().copied().find(|kind| kind.id() == id)
    }
}

pub fn kind(path: &str) -> Option<AssetKind> {
    match path
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_lowercase())
        .as_deref()
    {
        Some("png") => Some(AssetKind::Image),
        Some("wav") | Some("flac") | Some("ogg") => Some(AssetKind::Audio),
        Some("ttf") | Some("otf") => Some(AssetKind::Font),
        _ => None,
    }
}

pub const ICON_DECODES_PER_FRAME: u32 = 4;

pub fn show(editor: &mut Editor, ui: &mut Ui) {
    let Some(project) = &editor.project else {
        return;
    };
    let state = &mut editor.views.assets;
    widgets::search_box(ui, &mut state.search, "Search files", None);
    let needle = state.search.to_lowercase();
    let paths: Vec<&str> = project
        .files
        .paths_under("assets/")
        .map(|path| path.trim_start_matches("assets/"))
        .filter(|path| kind(path).is_some())
        .filter(|path| needle.is_empty() || path.to_lowercase().contains(&needle))
        .collect();
    let total = paths.len();
    let selected = (editor.selection.table == "asset")
        .then(|| editor.selection.keys.first().cloned())
        .flatten();
    let nodes: Vec<TreeNode<Infallible>> = tree::from_paths(
        paths.iter().copied(),
        &mut |path| {
            let name = path.rsplit('/').next().unwrap_or(path);
            let mut node = TreeNode::leaf(path, name);
            node.selected = selected.as_deref() == Some(path);
            node.unsaved = project.is_unsaved(&format!("assets/{path}"));
            node.drag = Some(Dragged::Asset(path.to_owned()));
            node.icon = match kind(path)? {
                AssetKind::Image => TreeIcon::Thumb(path.to_owned()),
                AssetKind::Audio => TreeIcon::Glyph("♪"),
                AssetKind::Font => TreeIcon::Glyph("A"),
            };
            Some(node)
        },
        &mut |_| {},
    );
    let mut budget = ICON_DECODES_PER_FRAME;
    let out = egui::ScrollArea::vertical()
        .id_salt("assets")
        .show(ui, |ui| {
            if nodes.is_empty() {
                ui.label(theme::muted("no files match"));
            }
            let Editor {
                project,
                form,
                views,
                ..
            } = editor;
            let files = &project.as_ref().expect("a project").files;
            tree::show(
                ui,
                &mut views.assets.tree,
                &nodes,
                !needle.is_empty(),
                tree::TreeHooks {
                    menu: None,
                    thumbs: Some(&mut |ctx, path| {
                        let thumb = form.thumbnails.icon(ctx, files, path, &mut budget)?;
                        Some(icon_image(&thumb))
                    }),
                },
            )
        })
        .inner;
    if let Some((path, _)) = out.clicked {
        editor.select(RowSelection::row("asset", &path));
        editor.panels.right = RightTab::Inspector;
    }
    if !needle.is_empty() {
        editor.status.left.push(format!(
            "{total} files for \"{}\"",
            editor.views.assets.search
        ));
    }
}

pub fn icon_image(thumb: &thumbs::Thumbnail) -> IconImage {
    let [w, h] = thumb.size;
    let scale = (16.0 / h.max(1) as f32).min(16.0 / w.max(1) as f32);
    IconImage {
        texture: thumb.texture.id(),
        uv: egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        size: egui::vec2(w as f32 * scale, h as f32 * scale),
    }
}
