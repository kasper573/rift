//! The rigs page: our rig, read from the glTF files that hold its joints, each top but the body's
//! hung on a joint of ours; the source rigs whose clips play on ours, each read from a file made on
//! it, with the joint of ours each of his moves; and the clips of the game folder that play. The
//! stage plays any of them on ours, and a source rig's on his beside it, so a wrong pair shows as a
//! limb moving wrong; while a joint of his is chosen, clicking a joint of ours pairs them.

use std::collections::{BTreeMap, BTreeSet};

use bevy::math::Vec3;
use engine::core::assets::ModelRef;
use engine::core::content::gltf::NodeTree;
use engine::systems::actor::skeleton::{JointName, SkeletonDef, SourceRig};
use ui::egui::{self, RichText, Ui, Vec2};
use ui::{controls, form, panel, theme, words};

use super::rigging::{self, JointFile, Rigging};
use crate::app::Editor;

#[derive(Default)]
pub struct RigsState {
    pub picker: Option<FilePicker>,
    /// The source rig whose pairs the page lists, by its place among them.
    pub source: usize,
    /// The joint of his the next joint of ours clicked on the stage is paired with.
    pub pairing: Option<String>,
    pub new_source: String,
}

/// The files offered to read joints from, as narrowed and ticked so far, and what they are read
/// for.
pub struct FilePicker {
    pub reading: Reading,
    pub query: String,
    pub ticked: BTreeSet<String>,
}

#[derive(Clone, PartialEq)]
pub enum Reading {
    OurRig,
    NewSource(String),
    Source(usize),
}

impl Reading {
    fn title(&self) -> String {
        match self {
            Reading::OurRig => "Read our rig's joints from files".to_owned(),
            Reading::NewSource(name) => format!("Read {name}'s joints from a file"),
            Reading::Source(_) => "Read more joints from a file".to_owned(),
        }
    }

    fn many(&self) -> bool {
        *self == Reading::OurRig
    }
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str, def: &SkeletonDef, rigging: &Rigging) {
    picker_window(editor, ui, key, def);
    let width = ui.available_width();
    let [left, right] = panel::split(ui, (width * 0.42).clamp(300.0, 520.0));
    panel::body(ui, left, |ui| {
        egui::ScrollArea::vertical()
            .id_salt(("rigs", key))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                our_rig(editor, ui, key, def, rigging);
                ui.add_space(10.0);
                form::rule(ui);
                sources(editor, ui, key, def, rigging);
                ui.add_space(10.0);
                form::rule(ui);
                clips(ui, def, rigging);
            });
    });
    panel::body(ui, right, |ui| stage(editor, ui, key, def, rigging));
}

fn our_rig(editor: &mut Editor, ui: &mut Ui, key: &str, def: &SkeletonDef, rigging: &Rigging) {
    form::section_count(ui, "Our rig", def.rig.files.len());
    let joints = rigging
        .joints
        .as_ref()
        .map_or(0, |tree| tree.joints().count());
    ui.horizontal_wrapped(|ui| {
        ui.label(format!(
            "{} · {}",
            words::counted(def.rig.files.len(), "file"),
            words::counted(joints, "joint")
        ));
        if ui.button("Read joints from files…").clicked() {
            editor.views.skeleton.view_mut(key).rigs.picker = Some(FilePicker {
                reading: Reading::OurRig,
                query: String::new(),
                ticked: def
                    .rig
                    .files
                    .iter()
                    .map(|file| file.as_str().to_owned())
                    .collect(),
            });
        }
    });
    if def.rig.files.is_empty() {
        form::hint(
            ui,
            "Our rig is every joint the files picked hold, each with its parent and its rest as \
             they place it: a rig file, or a seller's part files that each hold some of its joints.",
        );
    }
    for problem in &rigging.problems {
        ui.colored_label(theme::DANGER, problem.problem.to_string());
    }
    let Some(tree) = &rigging.joints else {
        return;
    };
    let tops: Vec<usize> = (0..tree.nodes.len())
        .filter(|&node| tree.nodes[node].parent.is_none())
        .collect();
    let Some(&body) = tops.iter().max_by_key(|&&top| {
        tree.joints()
            .filter(|&joint| tree.descends(joint, top))
            .count()
    }) else {
        return;
    };
    ui.label(theme::muted(format!(
        "{} carries the body",
        tree.nodes[body].name
    )));
    let mut hangs = def.rig.hangs.clone();
    for &top in tops.iter().filter(|&&top| top != body) {
        let name = JointName::new(tree.nodes[top].name.clone());
        let mut on = hangs.get(&name).cloned();
        ui.horizontal(|ui| {
            ui.label(theme::mono(name.as_str()));
            ui.label(theme::muted("hangs on"));
            controls::combo(("hang", key, name.as_str()))
                .selected_text(on.as_ref().map_or("nothing", JointName::as_str))
                .width(160.0)
                .show_ui(ui, |ui| {
                    for joint in tree.joints().filter(|&joint| !tree.descends(joint, top)) {
                        let joint = JointName::new(tree.nodes[joint].name.clone());
                        let label = joint.0.clone();
                        ui.selectable_value(&mut on, Some(joint), label);
                    }
                });
        });
        match on {
            Some(on) => hangs.insert(name, on),
            None => hangs.remove(&name),
        };
    }
    if tops.len() > 1 {
        ui.horizontal(|ui| {
            if ui
                .button("Guess where they hang")
                .on_hover_text(
                    "the joint that stands what hangs closest to where its skins bind it",
                )
                .clicked()
            {
                let Editor { project, views, .. } = &mut *editor;
                if let Some(project) = project.as_ref() {
                    hangs = rigging::guess_hangs(
                        &def.rig.files,
                        &project.files,
                        &mut views.skeleton.gltfs,
                    );
                }
            }
        });
        form::hint(
            ui,
            "A top hung on a joint stands where that joint stands, turned as its files rest it, and \
             moves with it.",
        );
    }
    if hangs != def.rig.hangs {
        super::change(editor, key, "Hang the rig's tops", |skeleton| {
            skeleton.rig.hangs = hangs;
            Ok(())
        });
    }
}

fn sources(editor: &mut Editor, ui: &mut Ui, key: &str, def: &SkeletonDef, rigging: &Rigging) {
    form::section_count(ui, "Source rigs", def.sources.len());
    let state = &editor.views.skeleton.view(key).rigs;
    let chosen = state.source.min(def.sources.len().saturating_sub(1));
    let mut pick = None;
    let mut removed = None;
    for (index, source) in def.sources.iter().enumerate() {
        ui.horizontal(|ui| {
            let label = format!(
                "{} · {} · {} paired",
                source.name,
                words::counted(source.joints.len(), "joint"),
                source.pairs.len()
            );
            if ui
                .add(egui::Button::selectable(index == chosen, label))
                .clicked()
            {
                pick = Some(index);
            }
            if controls::remove(ui, "source rig").clicked() {
                removed = Some(index);
            }
        });
    }
    ui.horizontal(|ui| {
        let state = &mut editor.views.skeleton.view_mut(key).rigs;
        ui.add(
            egui::TextEdit::singleline(&mut state.new_source)
                .hint_text("its name")
                .desired_width(160.0),
        );
        let name = state.new_source.trim().to_owned();
        if ui
            .add_enabled(!name.is_empty(), egui::Button::new("+ Add source rig"))
            .clicked()
        {
            state.picker = Some(FilePicker {
                reading: Reading::NewSource(name),
                query: String::new(),
                ticked: BTreeSet::new(),
            });
            state.new_source.clear();
        }
    });
    if def.sources.is_empty() {
        form::hint(
            ui,
            "A source rig is a rig other than ours that clips are made on, such as a seller's: its \
             clips play on ours through pairs of his joints and ours.",
        );
    }
    if let Some(index) = removed {
        super::change(editor, key, "Remove a source rig", |skeleton| {
            skeleton.sources.remove(index);
            Ok(())
        });
        return;
    }
    if let Some(index) = pick {
        let state = &mut editor.views.skeleton.view_mut(key).rigs;
        state.source = index;
        state.pairing = None;
    }
    if let Some(source) = def.sources.get(chosen) {
        ui.add_space(6.0);
        pairs(editor, ui, key, chosen, source, rigging.rig.as_deref());
    }
}

/// The joints of source rig `index`, each with the joint of ours it moves.
fn pairs(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    index: usize,
    source: &SourceRig,
    rig: Option<&NodeTree>,
) {
    let ours: Vec<String> = rig.map_or_else(Vec::new, |rig| {
        rig.joints()
            .map(|joint| rig.nodes[joint].name.clone())
            .collect()
    });
    let mut changed = source.pairs.clone();
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{} · joint pairs", source.name)).strong());
        if ui.button("Read joints from a file…").clicked() {
            editor.views.skeleton.view_mut(key).rigs.picker = Some(FilePicker {
                reading: Reading::Source(index),
                query: String::new(),
                ticked: BTreeSet::new(),
            });
        }
        if ui
            .button("Pair by names")
            .on_hover_text("pairs a joint of his with the joint of ours of exactly its name")
            .clicked()
        {
            changed = by_names(&source.joints, &ours);
        }
    });
    let pairing = editor.views.skeleton.view(key).rigs.pairing.clone();
    let mut chosen = None;
    egui::Grid::new(("pairs", key, index))
        .num_columns(2)
        .spacing(Vec2::new(10.0, 3.0))
        .striped(true)
        .show(ui, |ui| {
            for his in &source.joints {
                let lit = pairing.as_deref() == Some(his.as_str());
                if ui
                    .add(egui::Button::selectable(
                        lit,
                        theme::mono(his.as_str()).size(12.5),
                    ))
                    .clicked()
                {
                    chosen = Some((!lit).then(|| his.0.clone()));
                }
                let mut pair = changed.get(his).cloned();
                let filter_id = ui.make_persistent_id(("pair-filter", key, index, his.as_str()));
                controls::combo(("pair", key, index, his.as_str()))
                    .selected_text(
                        pair.as_ref()
                            .map_or("none".to_owned(), |joint| joint.0.clone()),
                    )
                    .width(150.0)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show_ui(ui, |ui| {
                        let mut filter: String = ui
                            .data_mut(|data| data.get_temp(filter_id))
                            .unwrap_or_default();
                        ui.add(
                            egui::TextEdit::singleline(&mut filter)
                                .hint_text("filter")
                                .desired_width(140.0),
                        )
                        .request_focus();
                        ui.data_mut(|data| data.insert_temp(filter_id, filter.clone()));
                        let filter = filter.to_lowercase();
                        let mut picked = ui.selectable_value(&mut pair, None, "none").clicked();
                        for joint in ours
                            .iter()
                            .filter(|joint| joint.to_lowercase().contains(&filter))
                        {
                            picked |= ui
                                .selectable_value(
                                    &mut pair,
                                    Some(JointName::new(joint.clone())),
                                    joint,
                                )
                                .clicked();
                        }
                        if picked {
                            ui.close();
                        }
                    });
                match pair {
                    Some(pair) => changed.insert(his.clone(), pair),
                    None => changed.remove(his),
                };
                ui.end_row();
            }
        });
    form::hint(
        ui,
        "Choose a joint of his, then click the joint of ours it moves on the stage. A joint of his \
         paired with none moves nothing of ours.",
    );
    if let Some(chosen) = chosen {
        editor.views.skeleton.view_mut(key).rigs.pairing = chosen;
    }
    if changed != source.pairs {
        set_pairs(editor, key, index, changed);
    }
}

/// Every joint of his paired with the joint of ours of exactly its name.
fn by_names(his: &[JointName], ours: &[String]) -> BTreeMap<JointName, JointName> {
    his.iter()
        .filter(|joint| ours.contains(&joint.0))
        .map(|joint| (joint.clone(), joint.clone()))
        .collect()
}

/// Gives source rig `index` the pairs `pairs`.
pub fn set_pairs(
    editor: &mut Editor,
    key: &str,
    index: usize,
    pairs: BTreeMap<JointName, JointName>,
) -> bool {
    super::change(editor, key, "Pair joints", |skeleton| {
        let source = skeleton
            .sources
            .get_mut(index)
            .ok_or("no such source rig")?;
        source.pairs = pairs;
        Ok(())
    })
}

/// What plays on the rig, and what moves a joint no rig has.
fn clips(ui: &mut Ui, def: &SkeletonDef, rigging: &Rigging) {
    form::section_count(ui, "Clips that play", rigging.clips.len());
    let on_ours = rigging
        .clips
        .iter()
        .filter(|clip| clip.source.is_none())
        .count();
    ui.label(format!("{on_ours} on our rig"));
    for (index, source) in def.sources.iter().enumerate() {
        let count = rigging
            .clips
            .iter()
            .filter(|clip| clip.source == Some(index))
            .count();
        ui.label(format!("{count} on {}", source.name));
    }
    form::hint(
        ui,
        "A clip of any glTF file in the game folder plays when every joint it moves is a joint of \
         ours, or every one is a joint of one source rig.",
    );
    if !rigging.strays.is_empty() {
        ui.add_space(4.0);
        ui.label(theme::muted(format!(
            "{} move a joint no rig has:",
            words::counted(rigging.strays.len(), "clip")
        )));
        for (clip, joint) in rigging.strays.iter().take(super::MAX_LISTED) {
            ui.colored_label(theme::DANGER, format!("{clip} moves {joint}"));
        }
    }
}

/// The clip picked playing on our rig, and a source rig's clip on his beside it.
fn stage(editor: &mut Editor, ui: &mut Ui, key: &str, def: &SkeletonDef, rigging: &Rigging) {
    let Some(content) = editor.loaded() else {
        return;
    };
    if rigging.rig.is_none() {
        form::hint(ui, "Pick our rig's files to see it here.");
        return;
    }
    super::stage_pickers(editor, ui, key, &content, rigging, true);
    let size = Vec2::new(ui.available_width(), ui.available_height() - 24.0);
    let Some(staged) = super::stage_rig(editor, ui, key, def, rigging, size, Vec::new()) else {
        return;
    };
    let chosen = editor.views.skeleton.view(key).clip.clone();
    let his = chosen
        .as_ref()
        .and_then(|clip| rigging.playable(clip))
        .filter(|found| found.source.is_some())
        .cloned();
    if let (Some(found), Some(ours)) = (his, rigging.rig.as_ref()) {
        let playhead = editor.views.skeleton.view(key).scrub.playhead;
        if let Some((tree, pose)) = super::his_pose(editor, &found, playhead.seconds().0) {
            let scale = height_of(ours) / height_of(&tree).max(1.0e-3);
            let beside = staged.beside(&tree, Some(&pose), Vec3::new(1.2, 0.0, 0.0), scale);
            beside.draw(ui, theme::ACCENT);
        }
    }
    let pairing = editor.views.skeleton.view(key).rigs.pairing.clone();
    let lit: Vec<&str> = pairing
        .as_deref()
        .and_then(|his| {
            let source = def
                .sources
                .get(editor.views.skeleton.view(key).rigs.source)?;
            source
                .pairs
                .get(&JointName::new(his))
                .map(|ours| ours.as_str())
        })
        .into_iter()
        .collect();
    if let Some(clicked) = staged.pick(ui, &lit)
        && let Some(his) = pairing
    {
        let index = editor.views.skeleton.view(key).rigs.source;
        if let Some(source) = def.sources.get(index) {
            let mut pairs = source.pairs.clone();
            pairs.insert(JointName::new(his), JointName::new(clicked));
            set_pairs(editor, key, index, pairs);
        }
    }
    form::hint(
        ui,
        "A source rig's clip plays on his rig beside ours. Both must rest in the same pose.",
    );
}

/// How tall `tree` stands at rest, from its lowest joint to its highest.
fn height_of(tree: &NodeTree) -> f32 {
    let worlds = tree.worlds();
    let heights = tree.joints().map(|joint| worlds[joint].w_axis.y);
    let (low, high) = heights.fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), y| {
        (low.min(y), high.max(y))
    });
    (high - low).max(0.0)
}

/// The window listing the files to read joints from, while one is open.
fn picker_window(editor: &mut Editor, ui: &mut Ui, key: &str, def: &SkeletonDef) {
    let Some(reading) = editor
        .views
        .skeleton
        .view(key)
        .rigs
        .picker
        .as_ref()
        .map(|picker| picker.reading.clone())
    else {
        return;
    };
    let files = {
        let Editor { project, views, .. } = &mut *editor;
        let Some(project) = project.as_ref() else {
            return;
        };
        rigging::joint_files(&project.files, &mut views.skeleton.gltfs)
    };
    let mut open = true;
    let mut read = false;
    egui::Window::new(reading.title())
        .id(egui::Id::new(("joint-files", key)))
        .collapsible(false)
        .resizable(true)
        .default_size(Vec2::new(720.0, 520.0))
        .open(&mut open)
        .show(ui.ctx(), |ui| {
            let picker = editor
                .views
                .skeleton
                .view_mut(key)
                .rigs
                .picker
                .as_mut()
                .expect("an open picker");
            read = picker_list(ui, picker, &files);
        });
    if read {
        let picker = editor
            .views
            .skeleton
            .view_mut(key)
            .rigs
            .picker
            .take()
            .expect("an open picker");
        apply(editor, key, def, picker, &files);
    } else if !open {
        editor.views.skeleton.view_mut(key).rigs.picker = None;
    }
}

/// The files holding joints whose path holds the query, each ticked or not; gives whether Read was
/// pressed.
fn picker_list(ui: &mut Ui, picker: &mut FilePicker, files: &[JointFile]) -> bool {
    ui.add(
        egui::TextEdit::singleline(&mut picker.query)
            .hint_text("any part of the path")
            .desired_width(f32::INFINITY),
    );
    let query = picker.query.trim().to_lowercase();
    let shown: Vec<&JointFile> = files
        .iter()
        .filter(|file| query.is_empty() || file.path.to_lowercase().contains(&query))
        .collect();
    let many = picker.reading.many();
    if many {
        ui.horizontal(|ui| {
            if ui
                .button(format!("Select all {} shown", shown.len()))
                .clicked()
            {
                picker
                    .ticked
                    .extend(shown.iter().map(|file| file.path.clone()));
            }
            if ui.button("Clear shown").clicked() {
                for file in &shown {
                    picker.ticked.remove(&file.path);
                }
            }
        });
    }
    egui::ScrollArea::vertical()
        .max_height(ui.available_height() - 70.0)
        .auto_shrink([false, false])
        .show_rows(ui, 20.0, shown.len(), |ui, rows| {
            for file in &shown[rows] {
                ui.horizontal(|ui| {
                    let mut on = picker.ticked.contains(&file.path);
                    if ui
                        .checkbox(&mut on, theme::mono(&file.path).size(12.0))
                        .changed()
                    {
                        if !many {
                            picker.ticked.clear();
                        }
                        if on {
                            picker.ticked.insert(file.path.clone());
                        } else {
                            picker.ticked.remove(&file.path);
                        }
                    }
                    ui.label(theme::faint(words::counted(file.joints.len(), "joint")));
                });
            }
        });
    let joints: BTreeSet<&str> = files
        .iter()
        .filter(|file| picker.ticked.contains(&file.path))
        .flat_map(|file| file.joints.iter().map(String::as_str))
        .collect();
    ui.label(
        RichText::new(format!(
            "{} picked → {}",
            words::counted(picker.ticked.len(), "file"),
            words::counted(joints.len(), "joint")
        ))
        .color(theme::OK),
    );
    ui.add_enabled(!picker.ticked.is_empty(), egui::Button::new("Read"))
        .clicked()
}

/// Reads what the picker was opened for from the files ticked in it.
fn apply(
    editor: &mut Editor,
    key: &str,
    def: &SkeletonDef,
    picker: FilePicker,
    files: &[JointFile],
) {
    let joints_of = |path: &str| -> Vec<JointName> {
        files
            .iter()
            .find(|file| file.path == path)
            .map_or_else(Vec::new, |file| {
                file.joints.iter().cloned().map(JointName::new).collect()
            })
    };
    match picker.reading {
        Reading::OurRig => {
            read_our_rig(
                editor,
                key,
                picker.ticked.iter().cloned().map(ModelRef::new).collect(),
            );
        }
        Reading::NewSource(name) => {
            let Some(path) = picker.ticked.iter().next() else {
                return;
            };
            if add_source(editor, key, name, joints_of(path)) {
                editor.views.skeleton.view_mut(key).rigs.source = def.sources.len();
            }
        }
        Reading::Source(index) => {
            let Some(path) = picker.ticked.iter().next() else {
                return;
            };
            let joints = joints_of(path);
            super::change(editor, key, "Read a source rig's joints", |skeleton| {
                let source = skeleton
                    .sources
                    .get_mut(index)
                    .ok_or("no such source rig")?;
                for joint in joints {
                    if !source.joints.contains(&joint) {
                        source.joints.push(joint);
                    }
                }
                Ok(())
            });
        }
    }
}

/// Puts our rig together from `files`, each top but the body's hung where its skins say.
pub fn read_our_rig(editor: &mut Editor, key: &str, files: Vec<ModelRef>) -> bool {
    let hangs = {
        let Editor { project, views, .. } = &mut *editor;
        let Some(project) = project.as_ref() else {
            return false;
        };
        rigging::guess_hangs(&files, &project.files, &mut views.skeleton.gltfs)
    };
    super::change(editor, key, "Read our rig", |skeleton| {
        skeleton.rig.files = files;
        skeleton.rig.hangs = hangs;
        Ok(())
    })
}

/// Adds a source rig named `name` of `joints`, none of them paired yet.
pub fn add_source(editor: &mut Editor, key: &str, name: String, joints: Vec<JointName>) -> bool {
    super::change(editor, key, &format!("Add source rig {name}"), |skeleton| {
        if skeleton.sources.iter().any(|source| source.name == name) {
            return Err(format!("there is a source rig named {name} already"));
        }
        skeleton.sources.push(SourceRig {
            name,
            joints,
            pairs: BTreeMap::new(),
        });
        Ok(())
    })
}
