//! The faces of a bust: drawn as cards, added, replaced, renamed and removed, with every line that
//! asks for one.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use ui::egui;
use ui::paint;
use ui::panel;
use ui::sheet;
use ui::theme;

use engine::core::content::files::{NAME_RULE, valid_name};
use engine::core::content::{ContentFile, bundle_of};
use engine::core::script::Node;
use engine::systems::dialogue::bust::{BUST_FACES, BustDef, Expression, GenericExpression};
use engine::systems::npc::NpcDef;
use engine::systems::script::{Line, ScriptDef, Speaker};
use strum::VariantArray;
use ui::egui::{Color32, Rect as ERect, Sense, Ui, Vec2};
use ui::glyph::Mark;

use crate::app::Editor;
use crate::project::{ContentEdit, Project, RowSelection};
use crate::rows;
use crate::tabs::shown_path;
use crate::thumbs;

const CARD: Vec2 = Vec2::new(178.0, 165.0);

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
        bundle_of(&self.file).map_or_else(|| self.file.clone(), |(_, part)| part.to_owned())
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
pub fn face_names(bust: Option<&BustDef>) -> Vec<String> {
    let generic = GenericExpression::VARIANTS
        .iter()
        .map(|expression| expression.as_ref().to_owned());
    let own = bust
        .into_iter()
        .flat_map(|bust| bust.own.iter().map(|face| face.name.clone()));
    generic.chain(own).collect()
}

/// The bust the npc at `npc` is shown in, when both read.
pub fn bust_of_npc(project: &Project, npc: &str) -> Option<Arc<BustDef>> {
    let npc = project.read::<NpcDef>(npc).ok()?;
    project.read_id(npc.bust?)
}

/// The part of the bust at `key` its face `name` is drawn in.
pub fn face_file(key: &str, name: &str) -> String {
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
    let selected = editor.views.bust.view(key).face.clone();
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
pub fn browse_faces(key: &str) -> Vec<PathBuf> {
    rfd::FileDialog::new()
        .set_title(format!("The faces of {}", shown_path(key)))
        .add_filter("PNG", &["png"])
        .pick_files()
        .unwrap_or_default()
}

fn drop_slot(ui: &mut Ui, size: Vec2, text: &str) -> egui::Response {
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

fn card(
    ui: &mut Ui,
    face: &BustFace,
    thumb: Option<&thumbs::Thumbnail>,
    selected: bool,
    key: &str,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(CARD, Sense::click());
    let painter = ui.painter_at(rect);
    panel::paint_card(&painter, rect, selected);
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

fn png_bytes(path: &Path) -> Result<Vec<u8>, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if !bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Err(format!("{} is not a PNG", path.display()));
    }
    Ok(bytes)
}

fn replace_file(editor: &mut Editor, key: &str, face: &BustFace, path: &Path) {
    let bytes = match png_bytes(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            editor.toast(error);
            return;
        }
    };
    let mut edit = ContentEdit::new(format!(
        "Replace {} of {}'s bust",
        face.name,
        shown_path(key)
    ));
    edit.write(face.file.to_owned(), bytes.into());
    editor.apply(edit);
}

/// Takes the PNGs at `paths` into the bust as one step: one named after a face the bust has
/// replaces its picture, any other becomes an expression of its own.
fn add_faces(editor: &mut Editor, key: &str, faces: &[BustFace], paths: &[PathBuf]) {
    let mut edit = ContentEdit::new(match paths {
        [path] => format!(
            "Add {} to {}'s bust",
            path.file_name().unwrap_or_default().to_string_lossy(),
            shown_path(key)
        ),
        _ => format!("Add {} faces to {}'s bust", paths.len(), shown_path(key)),
    });
    let named = match face_writes(&mut edit, key, faces, paths) {
        Ok(named) => named,
        Err(error) => {
            editor.toast(error);
            return;
        }
    };
    if editor.apply(edit)
        && let Some(face) = named.last()
    {
        editor.views.bust.view_mut(key).face = Some(face.clone());
        let part = BUST_FACES.named(face, "png");
        editor.select(RowSelection::parts(BustDef::TABLE, key, vec![part]));
    }
}

/// Writes each PNG at `paths` as the face its file is named after, numbering a name `faces`
/// or an earlier path already took. Gives the faces' names in order.
fn face_writes(
    edit: &mut ContentEdit,
    key: &str,
    faces: &[BustFace],
    paths: &[PathBuf],
) -> Result<Vec<String>, String> {
    let mut written: Vec<String> = Vec::new();
    for path in paths {
        let bytes = png_bytes(path)?;
        let stem = path
            .file_stem()
            .map(|stem| rows::name_from(&stem.to_string_lossy().replace('-', "_")))
            .filter(|stem| valid_name(stem))
            .unwrap_or_else(|| "expression".to_owned());
        let name = match faces.iter().find(|face| face.name == stem) {
            Some(face) if !written.contains(&face.name) => face.name.clone(),
            _ => {
                let taken: HashSet<String> = faces
                    .iter()
                    .map(|face| face.name.clone())
                    .chain(written.iter().cloned())
                    .collect();
                rows::free_name(&stem, &taken)
            }
        };
        edit.write(face_file(key, &name), bytes.into());
        written.push(name);
    }
    Ok(written)
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

fn face_lines(script: &ScriptDef) -> impl Iterator<Item = (&String, &Line)> {
    script.nodes.iter().flat_map(|(name, node)| {
        let lines = match node {
            Node::Dialogue(dialogue) => dialogue.lines.as_slice(),
            _ => &[],
        };
        lines.iter().map(move |line| (name, line))
    })
}

/// Whether a line is spoken by an npc shown in the bust at `key`.
fn speaks_with(editor: &Editor, key: &str) -> Option<impl Fn(&Line) -> bool + use<>> {
    let reader = editor.reader()?;
    let speakers: Vec<String> = super::users(editor, key)
        .into_iter()
        .filter(|(table, _)| table == NpcDef::TABLE)
        .map(|(_, key)| key)
        .collect();
    Some(move |line: &Line| match line.by {
        Speaker::Npc(npc) => reader
            .path_of(npc)
            .is_some_and(|npc| speakers.iter().any(|speaker| speaker == npc)),
        _ => false,
    })
}

pub fn rename(editor: &mut Editor, key: &str, face: &BustFace, new_name: &str) -> bool {
    let Some(Ok(bust)) = editor.read::<BustDef>(key) else {
        return false;
    };
    if !face.own {
        return false;
    }
    if !valid_name(new_name) {
        editor.toast(format!("A face's name is {NAME_RULE}"));
        return false;
    }
    if faces(&bust).iter().any(|face| face.name == new_name) {
        editor.toast(format!("{new_name} is already a face of this bust"));
        return false;
    }
    let Some(bytes) = editor
        .project
        .as_ref()
        .and_then(|project| project.files.get(&face.file).cloned())
    else {
        return false;
    };
    let Some(mut edit) = retarget_face(
        editor,
        key,
        &face.name,
        new_name,
        format!(
            "Rename face {} of {} to {new_name}",
            face.name,
            shown_path(key)
        ),
    ) else {
        return false;
    };
    edit.remove(face.file.clone());
    edit.write(face_file(key, new_name), bytes);
    editor.apply(edit)
}

pub fn remove(editor: &mut Editor, key: &str, face: &BustFace, replacement: &str) -> bool {
    if !face.own {
        return false;
    }
    let Some(mut edit) = retarget_face(
        editor,
        key,
        &face.name,
        replacement,
        format!("Remove face {} of {}", face.name, shown_path(key)),
    ) else {
        return false;
    };
    edit.remove(face.file.clone());
    editor.apply(edit)
}

/// An edit giving every line that asks the bust at `key` for face `from` face `to` instead.
fn retarget_face(
    editor: &mut Editor,
    key: &str,
    from: &str,
    to: &str,
    label: String,
) -> Option<ContentEdit> {
    let mut scripts: Vec<String> = lines(editor, key)
        .into_iter()
        .filter(|line| line.face == from)
        .map(|line| line.script)
        .collect();
    scripts.sort();
    scripts.dedup();
    let speaks = speaks_with(editor, key)?;
    let (from, to) = (
        Expression::from(from.to_owned()),
        Expression::from(to.to_owned()),
    );
    let mut edit = ContentEdit::new(label);
    for path in scripts {
        let mut script = (*editor.read::<ScriptDef>(&path)?.ok()?).clone();
        for node in script.nodes.values_mut() {
            if let Node::Dialogue(dialogue) = node {
                for line in &mut dialogue.lines {
                    if line.face.as_ref() == Some(&from) && speaks(line) {
                        line.face = Some(to.clone());
                    }
                }
            }
        }
        if let Err(reason) = edit.write_content(editor.project.as_ref()?, &path, &script) {
            editor.toast(reason);
            return None;
        }
    }
    Some(edit)
}
