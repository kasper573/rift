use std::collections::BTreeMap;
use ui::controls;
use ui::drag;
use ui::egui;
use ui::face;
use ui::form;
use ui::menu;
use ui::paint;
use ui::panel;
use ui::sheet;
use ui::theme;

use crate::widgets::Badge;
use engine::core::content::serial::TypeTag;
use engine::core::content::{ContentFile, FolderReader};
use engine::core::script::Expr;
use engine::field;
use engine::systems::class::{AccessRequirement, ClassDef, NodeKey, SkillNode, SkillTree};
use engine::systems::skill::{Rank, SkillDef};
use ui::egui::{Sense, Stroke, Ui, Vec2};
use ui::glyph::{self, Mark};

use super::tree::{self, CarriedNode, NodeSkill};
use super::{edit, node_of, select_node, write};
use crate::app::Editor;
use crate::forms::{self, FormChange, FormCx};
use crate::project::RowSelection;
use crate::tabs::shown_path;
use crate::widgets;
use crate::{picker, schema, views};
use ui::input::NumberField;

const DROP_HEIGHT: f32 = 30.0;

pub fn show(editor: &mut Editor, ui: &mut Ui, selection: &RowSelection) {
    let Some(node) = node_of(selection).map(NodeKey::from) else {
        return;
    };
    let key = selection.keys[0].clone();
    let Some(read) = views::read_shown::<ClassDef>(editor, ui, &key) else {
        return;
    };
    let (Some(project), Some(reader)) = (&editor.project, editor.reader()) else {
        return;
    };
    let tree = read.tree.clone();
    let Some(mut held) = tree.node(&node).cloned() else {
        ui.label(theme::muted(format!("node {node} is gone")));
        return;
    };
    let skills = tree::node_skills(project, &tree);
    let level_count = read.levels.len().max(1) as u32;
    let shapes = editor.shapes.clone();
    let mut changed = FormChange::default();
    let mut removed = false;
    editor.with_form(
        ClassDef::TABLE,
        Some(&key),
        shapes.shape_of(ClassDef::TABLE),
        |cx| {
            egui::ScrollArea::vertical()
                .id_salt(("class-node", &key, &node))
                .show(ui, |ui| {
                    ui.label(
                        face::mono(format!("{} › node {node}", shown_path(&key)))
                            .color(theme::CODE)
                            .size(12.0),
                    );
                    ui.add_space(6.0);
                    let path = format!("{}{node}", super::TREE_PART);
                    changed |= fields(ui, cx, &reader, &mut held, &path);
                    sheet::section(ui, "access requirements");
                    let around = Around {
                        tree: &tree,
                        skills: &skills,
                        node: &node,
                        level_count,
                    };
                    changed |= requirements(ui, cx, &around, &mut held.requires, &path);
                    ui.add_space(10.0);
                    if controls::danger(ui, "Remove node").clicked() {
                        removed = true;
                    }
                });
        },
    );
    let mut class = (*read).clone();
    if removed {
        tree::remove_node(&mut class.tree, &node);
        write(editor, &key, edit(&key, "Remove node"), &class);
        select_node(editor, &key, None);
    } else if changed.is_some() {
        class.tree.0.insert(node, held);
        write(editor, &key, edit(&key, "Edit node").merging(), &class);
    }
}

/// What the requirements of `node` are picked among.
struct Around<'a> {
    tree: &'a SkillTree,
    skills: &'a BTreeMap<NodeKey, NodeSkill>,
    node: &'a NodeKey,
    level_count: u32,
}

fn fields(
    ui: &mut Ui,
    cx: &mut FormCx,
    reader: &FolderReader,
    node: &mut SkillNode,
    path: &str,
) -> FormChange {
    let mut changed = FormChange::default();
    form::kv(ui, "skill", |ui| {
        let mut skill = reader.path_of(node.skill).unwrap_or_default().to_owned();
        let skill_path = format!("{path}{}", field!(SkillNode, skill));
        if picker::row(ui, cx, SkillDef::TABLE, &mut skill, &skill_path)
            && let Some(picked) = reader.id_of::<SkillDef>(&skill)
        {
            node.skill = picked;
            changed = FormChange::at(path);
        }
    });
    for (axis, at) in [("x", &mut node.at.x), ("y", &mut node.at.y)] {
        form::kv(ui, axis, |ui| {
            if NumberField::new(at).range(0.0..=f32::MAX).show(ui) {
                changed = FormChange::at(path);
            }
        });
    }
    changed
}

fn requirements(
    ui: &mut Ui,
    cx: &mut FormCx,
    around: &Around,
    requires: &mut Vec<AccessRequirement>,
    node_path: &str,
) -> FormChange {
    let mut changed = FormChange::default();
    let path = format!("{node_path}{}", field!(SkillNode, requires));
    let mut remove = None;
    for (index, requirement) in requires.iter_mut().enumerate() {
        let item_path = format!("{path}/{index}");
        let kind = TypeTag::of(requirement).unwrap_or_default();
        panel::tile_across(ui, false, |ui| {
            ui.horizontal(|ui| {
                widgets::badge(ui, Badge::of_tag(&kind, Some(ClassDef::TABLE)));
                ui.label(theme::strong(schema::title(&kind)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if glyph::button(ui, Mark::Cross, 11.0, theme::FAINT, "Remove")
                        .on_hover_text("Remove this requirement")
                        .clicked()
                    {
                        remove = Some(index);
                    }
                });
            });
            if requirement_body(ui, cx, around, requirement, index, &item_path) {
                changed = FormChange::at(&item_path);
            }
        });
        ui.add_space(4.0);
    }
    if requires.is_empty() {
        ui.label(theme::faint("a root: available from the start").size(12.5));
    }
    if let Some(index) = remove {
        requires.remove(index);
        changed = FormChange::at(&path);
    }
    ui.add_space(4.0);
    let fresh = templates(around, requires);
    menu::button(ui, "+ add requirement", |ui| {
        for (kind, template) in fresh {
            let button = ui
                .add_enabled(template.is_some(), egui::Button::new(schema::title(kind)))
                .on_disabled_hover_text("No other node of the tree can be required here");
            if button.clicked()
                && let Some(template) = template
            {
                requires.push(template);
                changed = FormChange::at(&path);
                ui.close();
            }
        }
    });
    if let Some(dropped) = drop_slot(ui) {
        let node = around.node;
        let listed = requires.iter().any(|requirement| {
            matches!(requirement, AccessRequirement::Skill { node: needed, .. } if *needed == dropped)
        });
        if dropped == *node {
            cx.state.status = vec!["a node can't require itself".to_owned()];
        } else if listed {
            cx.state.status = vec![format!("{dropped} is already required")];
        } else if around.tree.depends_on(&dropped, node) {
            cx.state.status = vec![format!("{dropped} already needs {node}")];
        } else {
            requires.push(AccessRequirement::Skill {
                node: dropped,
                rank: Rank::FIRST,
            });
            changed = FormChange::at(&path);
        }
    }
    changed
}

/// The requirement of each kind a node starts with when one is added: a skill requirement names
/// the first node it may require, and there is none when no node is left to.
fn templates(
    around: &Around,
    requires: &[AccessRequirement],
) -> [(&'static str, Option<AccessRequirement>); 3] {
    let node = around.node;
    let requirable = around.tree.0.keys().find(|other| {
        *other != node
            && !around.tree.depends_on(*other, node)
            && !requires.iter().any(|requirement| {
                matches!(requirement, AccessRequirement::Skill { node: needed, .. } if needed == *other)
            })
    });
    [
        (
            "skill",
            requirable.map(|other| AccessRequirement::Skill {
                node: other.clone(),
                rank: Rank::FIRST,
            }),
        ),
        ("level", Some(AccessRequirement::Level { level: 2 })),
        (
            "condition",
            Some(AccessRequirement::Condition {
                condition: Expr::Bool(true),
            }),
        ),
    ]
}

/// The fields of one requirement; whether any changed.
fn requirement_body(
    ui: &mut Ui,
    cx: &mut FormCx,
    around: &Around,
    requirement: &mut AccessRequirement,
    index: usize,
    path: &str,
) -> bool {
    let node = around.node;
    match requirement {
        AccessRequirement::Skill { node: needed, rank } => {
            let mut changed = false;
            form::kv(ui, "node", |ui| {
                controls::combo(("requirement-node", index))
                    .width(ui.available_width() - 8.0)
                    .selected_text(if needed.as_str().is_empty() {
                        theme::muted("pick a node…")
                    } else {
                        face::mono(needed.as_str()).size(13.0)
                    })
                    .show_ui(ui, |ui| {
                        for other in around.tree.0.keys().filter(|other| *other != node) {
                            let loops = around.tree.depends_on(other, node);
                            let response = ui.add_enabled(
                                !loops,
                                egui::Button::selectable(needed == other, other.as_str()),
                            );
                            if loops {
                                response.on_disabled_hover_text(format!(
                                    "{other} already needs {node}: picking it would close a loop"
                                ));
                            } else if response.clicked() && needed != other {
                                *needed = other.clone();
                                changed = true;
                            }
                        }
                    });
            });
            let ranks = around.skills.get(needed).map_or(1, NodeSkill::ranks);
            form::kv(ui, "rank", |ui| {
                let held = *rank;
                let mut draft = held.0.min(ranks);
                let of = format!("of {ranks}");
                NumberField::new(&mut draft)
                    .range(1..=ranks)
                    .unit(&of)
                    .show(ui);
                if draft != held.0 {
                    *rank = Rank(draft);
                    changed = true;
                }
            });
            changed
        }
        AccessRequirement::Level { level } => {
            let mut changed = false;
            let level_count = around.level_count;
            form::kv(ui, "level", |ui| {
                let of = format!("of {level_count}");
                changed = NumberField::new(level)
                    .range(1..=level_count)
                    .unit(&of)
                    .show(ui);
            });
            changed
        }
        AccessRequirement::Condition { condition } => {
            forms::of_type(ui, cx, condition, &format!("{path}/condition")).is_some()
        }
    }
}

/// While a node is carried from the canvas, a slot to drop it on; the node let go of over it.
fn drop_slot(ui: &mut Ui) -> Option<NodeKey> {
    let carried = drag::carried::<CarriedNode>(ui.ctx())?;
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), DROP_HEIGHT), Sense::hover());
    let lit = response.contains_pointer();
    paint::dashed_rect(
        ui.painter(),
        rect,
        Stroke::new(1.0, if lit { theme::ACCENT } else { theme::BORDER }),
        5.0,
        3.0,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        format!("drop to require {} at rank 1", carried.0),
        egui::FontId::proportional(12.5),
        if lit { theme::TEXT } else { theme::MUTED },
    );
    drag::dropped::<CarriedNode>(&response).map(|dropped| dropped.0)
}
