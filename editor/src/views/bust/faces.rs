//! The faces of a bust: drawn as cards, added, replaced, renamed and removed, with every line that
//! asks for one.

use super::face_files::{add_faces, replace_file};

use std::path::PathBuf;
use std::sync::Arc;
use ui::egui;
use ui::paint;
use ui::panel;
use ui::sheet;
use ui::theme;

use engine::core::content::ContentFile;
use engine::core::script::Node;
use engine::systems::dialogue::bust::{BUST_FACES, BustDef, GenericExpression};
use engine::systems::npc::NpcDef;
use engine::systems::script::{ScriptDef, ScriptLine, Speaker};
use strum::VariantArray;
use ui::egui::{Color32, Rect as ERect, Sense, Ui, Vec2};
use ui::glyph::Mark;

use crate::app::Editor;
use crate::project::Project;
use crate::tabs::shown_path;
use crate::thumbs;

pub(crate) const CARD: Vec2 = Vec2::new(178.0, 165.0);

/// A face of a bust, drawn in a part of its bundle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BustFace {
    pub name: String,
    pub file: String,
    /// Whether the face is the bust's own rather than one every bust has.
    pub own: bool,
}

impl BustFace {
    /// What a selection holds while the face is picked: its part of the bundle.
    pub fn part(&self) -> String {
        crate::project::files::bundle_of(&self.file)
            .map_or_else(|| self.file.clone(), |(_, part)| part.to_owned())
    }
}

/// Every face of the bust: the generic ones, then its own.
pub fn faces(busts: &BustDef) -> Vec<BustFace> {
    let generic = GenericExpression::VARIANTS
        .iter()
        .map(|&expression| BustFace {
            name: expression.as_ref().to_owned(),
            file: busts.generic.of(expression).as_str().to_owned(),
            own: false,
        });
    let own = busts.own.iter().map(|bust| BustFace {
        name: bust.name.clone(),
        file: bust.file.as_str().to_owned(),
        own: true,
    });
    generic.chain(own).collect()
}

/// The name of every face `bust` has: the generic ones, then its own.
pub(crate) fn face_names(bust: Option<&BustDef>) -> Vec<String> {
    let generic = GenericExpression::VARIANTS
        .iter()
        .map(|expression| expression.as_ref().to_owned());
    let own = bust
        .into_iter()
        .flat_map(|bust| bust.own.iter().map(|face| face.name.clone()));
    generic.chain(own).collect()
}

/// The bust the npc at `npc` is shown in, when both read.
pub(crate) fn bust_of_npc(project: &Project, npc: &str) -> Option<Arc<BustDef>> {
    let npc = project.read::<NpcDef>(npc).ok()?;
    project.read_id(npc.bust?)
}

/// The part of the bust at `key` its face `name` is drawn in.
pub(crate) fn face_file(key: &str, name: &str) -> String {
    format!("{key}/{}", BUST_FACES.named(name, "png"))
}

/// What the npcs shown in the bust at `key` are called, its path when nobody is.
pub fn speaker_name(editor: &Editor, key: &str) -> String {
    super::users(editor, key)
        .iter()
        .find_map(|(_, user)| Some(editor.read::<NpcDef>(user)?.ok()?.name.clone()))
        .unwrap_or_else(|| shown_path(key).to_owned())
}

/// The faces of the bust at `key` as cards, each picked to inspect it or replaced by a PNG dropped
/// on it, and a slot taking more.
pub fn grid(editor: &mut Editor, ui: &mut Ui, key: &str, row: &BustDef) {
    let ctx = ui.ctx().clone();
    let faces = faces(row);
    let title = speaker_name(editor, key);
    sheet::section(ui, &format!("Bust · {title}"));
    let drops = std::mem::take(&mut editor.views.bust.drops);
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let selected = editor.views.bust.views.view(key).face.clone();
    let mut pick = None;
    let mut replace: Option<(BustFace, PathBuf)> = None;
    let mut add: Vec<PathBuf> = Vec::new();
    let mut browse = false;
    let per_row = ((ui.available_width() + 12.0) / (CARD.x + 12.0))
        .floor()
        .max(1.0) as usize;
    ui.add_space(4.0);
    egui::Grid::new(("busts", key))
        .spacing(Vec2::new(12.0, 12.0))
        .show(ui, |ui| {
            for (index, face) in faces.iter().enumerate() {
                let thumb = editor.thumb(&ctx, &face.file);
                let is_selected = selected.as_deref() == Some(&face.name);
                let response = card(ui, face, thumb.as_ref(), is_selected, key);
                if response.clicked() {
                    pick = Some(face.clone());
                }
                if !drops.is_empty()
                    && let Some(at) = pointer
                    && response.rect.contains(at)
                    && let Some(path) = drops.first()
                {
                    replace = Some((face.clone(), path.clone()));
                }
                if (index + 1) % per_row == 0 {
                    ui.end_row();
                }
            }
            if drop_slot(ui, CARD, "drop to add an expression")
                .on_hover_text("Drop PNGs here, or click to browse for them")
                .clicked()
            {
                browse = true;
            }
            ui.end_row();
        });
    if replace.is_none() && !drops.is_empty() {
        add = drops.clone();
    }
    if browse {
        add.extend(browse_faces(key));
    }
    if let Some(face) = pick {
        super::select_face(editor, key, &face);
    }
    if let Some((face, path)) = replace {
        replace_file(editor, key, &face, &path);
    }
    if !add.is_empty() {
        add_faces(editor, key, &faces, &add);
    }
    editor.status.left.push(format!(
        "{} · bust · {} expressions",
        shown_path(key),
        faces.len()
    ));
    editor
        .status
        .left
        .push("drop PNG files onto slots to add or replace".to_owned());
}

/// The faces picked for the bust at `key`, each a PNG named after its expression.
pub(crate) fn browse_faces(key: &str) -> Vec<PathBuf> {
    rfd::FileDialog::new()
        .set_title(format!("The faces of {}", shown_path(key)))
        .add_filter("PNG", &["png"])
        .pick_files()
        .unwrap_or_default()
}

pub(crate) fn drop_slot(ui: &mut Ui, size: Vec2, text: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, theme::SELECTION.gamma_multiply(0.5));
    paint::dashed_rect(
        &painter,
        rect,
        egui::Stroke::new(1.0, theme::ACCENT),
        4.0,
        4.0,
    );
    Mark::Plus.paint(
        &painter,
        ERect::from_center_size(rect.center() - Vec2::new(0.0, 10.0), Vec2::splat(18.0)),
        theme::ACCENT,
    );
    painter.text(
        rect.center() + Vec2::new(0.0, 12.0),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(12.5),
        theme::ACCENT,
    );
    response
}

pub(crate) fn card(
    ui: &mut Ui,
    face: &BustFace,
    thumb: Option<&thumbs::Thumbnail>,
    selected: bool,
    key: &str,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(CARD, Sense::click());
    let painter = ui.painter_at(rect);
    panel::paint_tile(&painter, rect, selected);
    let art = ERect::from_min_max(
        rect.min + Vec2::new(4.0, 4.0),
        rect.right_bottom() - Vec2::new(4.0, 28.0),
    );
    match thumb {
        Some(thumb) => {
            let (w, h) = (thumb.size[0] as f32, thumb.size[1] as f32);
            let scale = (art.width() / w).min(art.height() / h);
            let shown = Vec2::new(w * scale, h * scale);
            painter.image(
                thumb.texture.id(),
                ERect::from_center_size(art.center(), shown),
                thumbs::FULL_UV,
                Color32::WHITE,
            );
        }
        None => {
            painter.text(
                art.center(),
                egui::Align2::CENTER_CENTER,
                "missing file",
                egui::FontId::proportional(12.0),
                theme::DANGER,
            );
        }
    }
    painter.text(
        egui::pos2(rect.min.x + 8.0, rect.max.y - 14.0),
        egui::Align2::LEFT_CENTER,
        &face.name,
        egui::FontId::proportional(13.0),
        theme::TEXT,
    );
    painter.text(
        egui::pos2(rect.max.x - 8.0, rect.max.y - 14.0),
        egui::Align2::RIGHT_CENTER,
        if face.own {
            format!("{} only", shown_path(key))
        } else {
            "generic".to_owned()
        },
        egui::FontId::proportional(11.0),
        theme::MUTED,
    );
    response
}

#[derive(Clone, Debug)]
pub struct FaceLine {
    pub script: String,
    pub node: String,
    pub face: String,
}

/// Every line of every script spoken by an npc shown in the bust at `key` that asks for a face.
pub fn lines(editor: &Editor, key: &str) -> Vec<FaceLine> {
    let (Some(reader), Some(speaks)) = (editor.reader(), speaks_with(editor, key)) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for path in reader.paths_of::<ScriptDef>() {
        let Some(Ok(script)) = editor.read::<ScriptDef>(path) else {
            continue;
        };
        for (node, line) in face_lines(&script) {
            if let Some(face) = &line.face
                && speaks(line)
            {
                found.push(FaceLine {
                    script: path.clone(),
                    node: node.clone(),
                    face: String::from(face.clone()),
                });
            }
        }
    }
    found
}

pub(crate) fn face_lines(script: &ScriptDef) -> impl Iterator<Item = (&String, &ScriptLine)> {
    script.nodes.iter().flat_map(|(name, node)| {
        let lines = match node {
            Node::Dialogue(dialogue) => dialogue.lines.as_slice(),
            _ => &[],
        };
        lines.iter().map(move |line| (name, line))
    })
}

/// Whether a line is spoken by an npc shown in the bust at `key`.
pub(crate) fn speaks_with(
    editor: &Editor,
    key: &str,
) -> Option<impl Fn(&ScriptLine) -> bool + use<>> {
    let reader = editor.reader()?;
    let speakers: Vec<String> = super::users(editor, key)
        .into_iter()
        .filter(|(table, _)| table == NpcDef::TABLE)
        .map(|(_, key)| key)
        .collect();
    Some(move |line: &ScriptLine| match line.by {
        Speaker::Npc(npc) => reader
            .path_of(npc)
            .is_some_and(|npc| speakers.iter().any(|speaker| speaker == npc)),
        _ => false,
    })
}
