use engine::core::content::ContentNames;
use engine::core::script::{Expr, ExprKind, ValueType};
use engine::systems::script::{Condition, RiftQueryKind, condition as script_condition};
use serde_json::Value;
use strum::VariantArray;
use ui::chip;
use ui::controls;
use ui::egui;
use ui::egui::{RichText, Stroke, Ui};
use ui::glyph::{self, Mark};
use ui::input;
use ui::paint;
use ui::sheet;
use ui::theme;

use crate::forms::{self, FormChange, FormCx};
use crate::schema::{self, SchemaVariant};
use ui::input::InputField;

#[derive(Clone, Copy, PartialEq, Eq, VariantArray)]
enum QueryGroup {
    Character,
    Gear,
    Quests,
    Memories,
    Places,
    Actors,
    Ground,
    Skill,
}

impl QueryGroup {
    fn of(kind: RiftQueryKind) -> QueryGroup {
        match kind {
            RiftQueryKind::Level
            | RiftQueryKind::IsClass
            | RiftQueryKind::Stat
            | RiftQueryKind::HasStatus
            | RiftQueryKind::StatusCount
            | RiftQueryKind::StatusPower
            | RiftQueryKind::SkillRank
            | RiftQueryKind::SkillStat
            | RiftQueryKind::SpentSkillPoints => QueryGroup::Character,
            RiftQueryKind::Holding | RiftQueryKind::FreeSlots | RiftQueryKind::Equipped => {
                QueryGroup::Gear
            }
            RiftQueryKind::OnQuest
            | RiftQueryKind::QuestDone
            | RiftQueryKind::QuestCooling
            | RiftQueryKind::TaskDone
            | RiftQueryKind::QuestResetsIn => QueryGroup::Quests,
            RiftQueryKind::Memory | RiftQueryKind::SetWithin => QueryGroup::Memories,
            RiftQueryKind::Inside | RiftQueryKind::InArea | RiftQueryKind::KnowsArea => {
                QueryGroup::Places
            }
            RiftQueryKind::Nearest
            | RiftQueryKind::Count
            | RiftQueryKind::Distance
            | RiftQueryKind::Health
            | RiftQueryKind::Present
            | RiftQueryKind::IsNpc
            | RiftQueryKind::Walking
            | RiftQueryKind::Hostile
            | RiftQueryKind::Subject
            | RiftQueryKind::Owner
            | RiftQueryKind::Target => QueryGroup::Actors,
            RiftQueryKind::GroundItems => QueryGroup::Ground,
            RiftQueryKind::SkillTargets | RiftQueryKind::SkillArea | RiftQueryKind::AimedPoint => {
                QueryGroup::Skill
            }
        }
    }

    fn title(self) -> &'static str {
        match self {
            QueryGroup::Character => "The character",
            QueryGroup::Gear => "Bag and gear",
            QueryGroup::Quests => "Quests",
            QueryGroup::Memories => "Memories",
            QueryGroup::Places => "Places",
            QueryGroup::Actors => "Actors",
            QueryGroup::Ground => "Ground",
            QueryGroup::Skill => "The skill",
        }
    }
}

/// How wide the name of a kept value is typed.
const NAME_WIDTH: f32 = 120.0;
const NUMBER_WIDTH: f32 = 56.0;
const TEXT_WIDTH: f32 = 120.0;
const VARIABLE_WIDTH: f32 = 100.0;
const CARET_WIDTH: f32 = 16.0;

fn sign(kind: ExprKind) -> Option<&'static str> {
    Some(match kind {
        ExprKind::Equal => "=",
        ExprKind::NotEqual => "≠",
        ExprKind::Less => "<",
        ExprKind::AtMost => "≤",
        ExprKind::More => ">",
        ExprKind::AtLeast => "≥",
        ExprKind::Add => "+",
        ExprKind::Subtract => "−",
        ExprKind::Multiply => "×",
        ExprKind::Divide => "÷",
        ExprKind::Min => "min",
        ExprKind::Max => "max",
        ExprKind::Local
        | ExprKind::State
        | ExprKind::Event
        | ExprKind::All
        | ExprKind::Any
        | ExprKind::Join
        | ExprKind::Not
        | ExprKind::Round
        | ExprKind::Bool
        | ExprKind::Int
        | ExprKind::Num
        | ExprKind::Text
        | ExprKind::Var
        | ExprKind::Query => return None,
    })
}

/// What a block of a condition is: one of the expression's own kinds, or a query of the world.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Expr(ExprKind),
    Query(RiftQueryKind),
}

impl BlockKind {
    fn of(condition: &Condition) -> BlockKind {
        match condition {
            Expr::Query(query) => BlockKind::Query(query.into()),
            other => BlockKind::Expr(other.kind()),
        }
    }

    /// What the block yields when its kind alone says; a kept value's or a memory's comes from
    /// where it is kept.
    fn yields(self) -> Option<ValueType> {
        let kind = match self {
            BlockKind::Query(query) => return query.value_type(),
            BlockKind::Expr(kind) => kind,
        };
        match kind {
            ExprKind::All
            | ExprKind::Any
            | ExprKind::Not
            | ExprKind::Equal
            | ExprKind::NotEqual
            | ExprKind::Less
            | ExprKind::AtMost
            | ExprKind::More
            | ExprKind::AtLeast
            | ExprKind::Bool => Some(ValueType::Boolean),
            ExprKind::Add
            | ExprKind::Subtract
            | ExprKind::Multiply
            | ExprKind::Divide
            | ExprKind::Min
            | ExprKind::Max
            | ExprKind::Num => Some(ValueType::Number),
            ExprKind::Round | ExprKind::Int => Some(ValueType::Integer),
            ExprKind::Join | ExprKind::Text => Some(ValueType::Text),
            ExprKind::Local
            | ExprKind::State
            | ExprKind::Event
            | ExprKind::Var
            | ExprKind::Query => None,
        }
    }

    fn title(self) -> String {
        match self {
            BlockKind::Expr(ExprKind::All) => "all of".to_owned(),
            BlockKind::Expr(ExprKind::Any) => "any of".to_owned(),
            BlockKind::Expr(kind) => kind.name().replace('_', " "),
            BlockKind::Query(query) => <&str>::from(query).replace('_', " "),
        }
    }

    fn fits(self, slot: Slot) -> bool {
        slot == Slot::Any || matches!(self.yields(), None | Some(ValueType::Boolean))
    }
}

fn type_word(yields: Option<ValueType>) -> &'static str {
    match yields {
        Some(ValueType::Boolean) => "yes or no",
        Some(ValueType::Integer | ValueType::Number) => "number",
        Some(ValueType::Text) => "text",
        Some(ValueType::Actor) => "actor",
        Some(ValueType::Actors) => "actors",
        Some(ValueType::Point) => "point",
        Some(ValueType::Area) => "area",
        None => "its type",
    }
}

/// A condition as blocks to click together: each block a chip that swaps it for another, holding
/// the blocks it is made of.
pub fn edit(ui: &mut Ui, cx: &mut FormCx, condition: &mut Condition, path: &str) -> FormChange {
    let palette = Palette::new();
    let mut changed = FormChange::default();
    ui.vertical(|ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
        ui.horizontal_wrapped(|ui| {
            changed = block(ui, cx, &palette, condition, path, Slot::Any);
        });
    });
    changed
}

/// The queries a block may be, as the schema of a condition states them.
struct Palette {
    root: &'static Value,
    queries: Vec<SchemaVariant<'static>>,
}

impl Palette {
    fn new() -> Palette {
        let root = schema::of_type::<Condition>();
        Palette {
            root,
            queries: schema::tagged_variants(root, root)
                .into_iter()
                .filter(|variant| variant.tag.parse::<RiftQueryKind>().is_ok())
                .collect(),
        }
    }

    fn query(&self, query: RiftQueryKind) -> Option<&SchemaVariant<'static>> {
        let tag: &str = query.into();
        self.queries.iter().find(|variant| variant.tag == tag)
    }

    /// A query of `kind` as the schema starts one, its references among the files nearby.
    fn fresh(&self, cx: &FormCx, query: RiftQueryKind) -> Option<Condition> {
        let template = schema::variant_template(self.root, self.query(query)?, cx.nearby());
        cx.project.reader().from_json(&template).ok()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Any,
    YesOrNo,
}

fn block(
    ui: &mut Ui,
    cx: &mut FormCx,
    palette: &Palette,
    condition: &mut Condition,
    path: &str,
    slot: Slot,
) -> FormChange {
    let mut changed = FormChange::default();
    let picked =
        |ui: &mut Ui, cx: &mut FormCx, response: &egui::Response, condition: &mut Condition| {
            picker_popup(ui, cx, response, palette, slot, condition, path)
        };
    match condition {
        Expr::Bool(flag) => {
            let text = if *flag { "true" } else { "false" };
            let response = chip::colored(ui, text, theme::INPUT, theme::TEXT);
            if response.clicked() {
                *flag = !*flag;
                changed = FormChange::at(path);
            }
            changed |= picked(ui, cx, &response, condition).unwrap_or_default();
        }
        Expr::Int(_) | Expr::Num(_) => {
            make_room(ui, NUMBER_WIDTH + CARET_WIDTH);
            ui.scope(|ui| {
                ui.set_width(NUMBER_WIDTH);
                changed |= number(ui, cx, condition, &format!("{path}~n"));
            });
            let response = glyph::button(ui, Mark::CaretDown, 12.0, theme::FAINT, "Pick");
            changed |= picked(ui, cx, &response, condition).unwrap_or_default();
        }
        Expr::Text(text) => {
            make_room(ui, TEXT_WIDTH + CARET_WIDTH);
            ui.scope(|ui| {
                ui.set_width(TEXT_WIDTH);
                changed |= forms::text_of(ui, cx, text, &format!("{path}~s"));
            });
            let response = glyph::button(ui, Mark::CaretDown, 12.0, theme::FAINT, "Pick");
            changed |= picked(ui, cx, &response, condition).unwrap_or_default();
        }
        Expr::Var { var } => {
            let declared: Vec<(String, &str)> = cx
                .kept
                .iter()
                .flat_map(|kept| &kept.variables)
                .map(|(name, ty)| (name.clone(), ty.name()))
                .collect();
            make_room(ui, VARIABLE_WIDTH);
            if declared.is_empty() {
                ui.scope(|ui| {
                    ui.set_width(VARIABLE_WIDTH);
                    changed |= forms::text_of(ui, cx, var, &format!("{path}/var"));
                });
            } else {
                changed |= declared_name(ui, &declared, var, path);
            }
            let response = chip::colored(ui, "◇ variable", theme::INPUT, forms::VARIABLE);
            changed |= picked(ui, cx, &response, condition).unwrap_or_default();
        }
        Expr::All { .. } | Expr::Any { .. } | Expr::Join { .. } => {
            changed = list_block(ui, cx, palette, condition, path, slot);
        }
        Expr::Not { .. } | Expr::Round { .. } => {
            let response = tag_chip(ui, BlockKind::of(condition));
            if let Some(picked) = picked(ui, cx, &response, condition) {
                return picked;
            }
            let inner_slot = if matches!(condition, Expr::Not { .. }) {
                Slot::YesOrNo
            } else {
                Slot::Any
            };
            if let Expr::Not { of } | Expr::Round { of } = condition
                && block(ui, cx, palette, of, &format!("{path}/of"), inner_slot).is_some()
            {
                changed = FormChange::at(path);
            }
        }
        Expr::Local { .. } | Expr::State { .. } | Expr::Event { .. } => {
            let response = tag_chip(ui, BlockKind::of(condition));
            if let Some(picked) = picked(ui, cx, &response, condition) {
                return picked;
            }
            let kept = cx.kept.clone().unwrap_or_default();
            let declared = match condition {
                Expr::Local { .. } => Some(kept.locals),
                Expr::State { .. } => Some(kept.state),
                _ => None,
            };
            if let Expr::Local { local: name }
            | Expr::State { state: name }
            | Expr::Event { event: name } = condition
            {
                make_room(ui, NAME_WIDTH);
                match declared {
                    Some(declared) => {
                        let declared: Vec<(String, &str)> = declared
                            .into_iter()
                            .map(|(name, ty)| (name, type_word(Some(ty))))
                            .collect();
                        changed |= declared_name(ui, &declared, name, path);
                    }
                    None => {
                        ui.scope(|ui| {
                            ui.set_width(NAME_WIDTH.min(ui.available_size_before_wrap().x));
                            changed |= forms::text_of(ui, cx, name, &format!("{path}/name"));
                        });
                    }
                }
            }
        }
        Expr::Query(_) => changed = query_block(ui, cx, palette, condition, path, slot),
        binary => {
            let kind = binary.kind();
            let Some([a, b]) = binary.operands_mut() else {
                return changed;
            };
            if block(ui, cx, palette, a, &format!("{path}/a"), Slot::Any).is_some() {
                changed = FormChange::at(path);
            }
            let response =
                chip::dropdown_colored(ui, sign(kind).unwrap_or("?"), theme::INPUT, theme::CODE);
            let mut swapped = None;
            egui::Popup::from_toggle_button_response(&response)
                .id(ui.id().with(path).with("op"))
                .show(|ui| {
                    for &op in ExprKind::VARIANTS {
                        let Some(sign) = sign(op) else { continue };
                        if ui
                            .button(format!("{sign}  {}", schema::title(op.name())))
                            .clicked()
                        {
                            swapped = Some(op);
                            ui.close();
                        }
                    }
                });
            if block(ui, cx, palette, b, &format!("{path}/b"), Slot::Any).is_some() {
                changed = FormChange::at(path);
            }
            if let Some(op) = swapped
                && let Some([a, b]) = binary.operands_mut()
                && let Some(swapped) = Expr::binary(op, taken(a), taken(b))
            {
                *binary = swapped;
                changed = FormChange::at(path);
            }
        }
    }
    changed
}

fn taken(operand: &mut Box<Condition>) -> Box<Condition> {
    std::mem::replace(operand, Box::new(Expr::Bool(false)))
}

/// A number typed in place: whole numbers stay whole.
fn number(ui: &mut Ui, cx: &mut FormCx, condition: &mut Condition, path: &str) -> FormChange {
    let current = match condition {
        Expr::Int(number) => number.to_string(),
        Expr::Num(number) => number.to_string(),
        _ => return FormChange::default(),
    };
    let id = ui.id().with(path).with("number");
    let typed = input::drafted(ui, id, &current, |ui, draft| {
        InputField::new(draft).show(ui)
    });
    let Some(typed) = typed else {
        forms::error_line(ui, cx, path);
        return FormChange::default();
    };
    let typed = typed.trim();
    if let Ok(whole) = typed.parse::<i64>() {
        *condition = Expr::Int(whole);
    } else if let Ok(number) = typed.parse::<f64>() {
        *condition = Expr::Num(number);
    } else {
        cx.state
            .errors
            .insert(path.to_owned(), format!("not a number: {typed}"));
        forms::error_line(ui, cx, path);
        return FormChange::default();
    }
    cx.state.errors.remove(path);
    FormChange::at(path)
}

fn list_block(
    ui: &mut Ui,
    cx: &mut FormCx,
    palette: &Palette,
    condition: &mut Condition,
    path: &str,
    slot: Slot,
) -> FormChange {
    let mut changed = FormChange::default();
    let response = tag_chip(ui, BlockKind::of(condition));
    if let Some(picked) = picker_popup(ui, cx, &response, palette, slot, condition, path) {
        return picked;
    }
    let item_slot = match condition {
        Expr::Join { .. } => Slot::Any,
        _ => Slot::YesOrNo,
    };
    let (Expr::All { of: items } | Expr::Any { of: items } | Expr::Join { of: items }) = condition
    else {
        return changed;
    };
    let mut remove = None;
    let mut added: Option<Condition> = None;
    ui.horizontal_top(|ui| {
        ui.add_space(6.0);
        let guide_x = ui.cursor().min.x;
        let top = ui.cursor().min.y;
        ui.add_space(8.0);
        let bottom = ui
            .vertical(|ui| {
                for (index, item) in items.iter_mut().enumerate() {
                    let row = ui.horizontal_wrapped(|ui| {
                        if block(
                            ui,
                            cx,
                            palette,
                            item,
                            &format!("{path}/of/{index}"),
                            item_slot,
                        )
                        .is_some()
                        {
                            changed = FormChange::at(path);
                        }
                    });
                    let hovered =
                        row.response.hovered() || ui.rect_contains_pointer(row.response.rect);
                    if hovered {
                        let at = egui::pos2(
                            row.response.rect.right() + 8.0,
                            row.response.rect.center().y,
                        );
                        let hit = egui::Rect::from_center_size(at, egui::vec2(14.0, 14.0));
                        let cross = ui.interact(
                            hit,
                            ui.id().with(path).with(("remove", index)),
                            egui::Sense::click(),
                        );
                        Mark::Cross.paint(
                            ui.painter(),
                            egui::Rect::from_center_size(at, egui::vec2(9.0, 9.0)),
                            theme::FAINT,
                        );
                        if cross.clicked() {
                            remove = Some(index);
                        }
                    }
                }
                let add = chip::dropdown_colored(ui, "+ condition", theme::INPUT, theme::TEXT);
                let id = ui.id().with(path).with("add");
                egui::Popup::from_toggle_button_response(&add)
                    .id(id)
                    .width(300.0)
                    .show(|ui| {
                        if let Some(fresh) = picker_body(ui, cx, palette, item_slot) {
                            added = Some(fresh);
                            ui.close();
                        }
                    });
                ui.cursor().min.y
            })
            .inner;
        ui.painter().line_segment(
            [
                egui::pos2(guide_x, top + 2.0),
                egui::pos2(guide_x, bottom - 4.0),
            ],
            Stroke::new(1.0, theme::BORDER),
        );
    });
    if let Some(index) = remove {
        items.remove(index);
        changed = FormChange::at(path);
    }
    if let Some(fresh) = added {
        items.push(fresh);
        changed = FormChange::at(path);
    }
    changed
}

fn query_block(
    ui: &mut Ui,
    cx: &mut FormCx,
    palette: &Palette,
    condition: &mut Condition,
    path: &str,
    slot: Slot,
) -> FormChange {
    let Expr::Query(query) = condition else {
        return FormChange::default();
    };
    let kind = RiftQueryKind::from(&*query);
    let response = tag_chip(ui, BlockKind::Query(kind));
    if let Some(picked) = picker_popup(ui, cx, &response, palette, slot, condition, path) {
        return picked;
    }
    let (Expr::Query(query), Some(variant)) = (&mut *condition, palette.query(kind)) else {
        return FormChange::default();
    };
    forms::inline_fields_of(
        ui,
        cx,
        palette.root,
        variant.schema,
        query,
        path,
        &mut |ui, cx, nested: &mut Condition, nested_path| {
            block(ui, cx, palette, nested, nested_path, Slot::Any)
        },
    )
}

fn picker_popup(
    ui: &mut Ui,
    cx: &mut FormCx,
    response: &egui::Response,
    palette: &Palette,
    slot: Slot,
    condition: &mut Condition,
    path: &str,
) -> Option<FormChange> {
    let id = ui.id().with(path).with("picker");
    let mut changed = None;
    egui::Popup::from_toggle_button_response(response)
        .id(id)
        .width(300.0)
        .show(|ui| {
            if let Some(fresh) = picker_body(ui, cx, palette, slot) {
                *condition = fresh;
                changed = Some(FormChange::at(path));
                ui.close();
            }
        });
    changed
}

fn picker_body(ui: &mut Ui, cx: &mut FormCx, palette: &Palette, slot: Slot) -> Option<Condition> {
    let mut picked: Option<Condition> = None;
    let zero = || Box::new(Expr::Int(0));
    ui.set_width(300.0);
    egui::ScrollArea::vertical()
        .max_height(420.0)
        .show(ui, |ui| {
            for &group in QueryGroup::VARIANTS {
                let members: Vec<RiftQueryKind> = RiftQueryKind::VARIANTS
                    .iter()
                    .copied()
                    .filter(|query| {
                        QueryGroup::of(*query) == group
                            && BlockKind::Query(*query).fits(slot)
                            && palette.query(*query).is_some()
                    })
                    .collect();
                if members.is_empty() {
                    continue;
                }
                sheet::section(ui, group.title());
                for query in members {
                    if entry(ui, query) {
                        picked = palette.fresh(cx, query);
                    }
                }
            }
            sheet::section(ui, "Kept values");
            ui.horizontal_wrapped(|ui| {
                let kept = cx.kept.clone().unwrap_or_default();
                let fitting = |declared: &[(String, ValueType)]| {
                    declared
                        .iter()
                        .find(|(_, ty)| slot == Slot::Any || *ty == ValueType::Boolean)
                        .map(|(name, _)| name.clone())
                };
                let offers = [
                    (
                        ExprKind::Local,
                        fitting(&kept.locals).map(|local| Expr::Local { local }),
                    ),
                    (
                        ExprKind::State,
                        fitting(&kept.state).map(|state| Expr::State { state }),
                    ),
                    (
                        ExprKind::Event,
                        Some(Expr::Event {
                            event: String::new(),
                        }),
                    ),
                ];
                for (kind, fresh) in offers {
                    let offered = fresh.is_some();
                    let chip = ui
                        .add_enabled_ui(offered, |ui| {
                            chip::colored(ui, kind.name(), theme::INPUT, theme::TEXT)
                        })
                        .inner
                        .on_disabled_hover_text(format!(
                            "None fits here: declare a {} in the script's header",
                            kind.name()
                        ));
                    if chip.clicked() {
                        picked = fresh;
                    }
                }
                let variable = kept
                    .variables
                    .first()
                    .map(|(name, _)| Expr::Var { var: name.clone() });
                let chip = ui
                    .add_enabled_ui(variable.is_some(), |ui| {
                        chip::colored(ui, "◇ variable", theme::INPUT, forms::VARIABLE)
                    })
                    .inner
                    .on_disabled_hover_text(
                        "The script has no variable yet: make one from a field's ◇ menu",
                    );
                if chip.clicked() {
                    picked = variable;
                }
            });
            ui.add_space(6.0);
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                let joins: [(ExprKind, fn() -> Condition); 3] = [
                    (ExprKind::All, || Expr::All { of: Vec::new() }),
                    (ExprKind::Any, || Expr::Any { of: Vec::new() }),
                    (ExprKind::Not, || Expr::Not {
                        of: Box::new(Expr::Bool(true)),
                    }),
                ];
                for (kind, fresh) in joins {
                    if chip::colored(ui, BlockKind::Expr(kind).title(), theme::INPUT, theme::TEXT)
                        .clicked()
                    {
                        picked = Some(fresh());
                    }
                }
                if chip::colored(ui, "compare", theme::INPUT, theme::TEXT).clicked() {
                    picked = Some(Expr::Equal {
                        a: zero(),
                        b: zero(),
                    });
                }
                if slot == Slot::Any {
                    if chip::colored(ui, "arithmetic", theme::INPUT, theme::TEXT).clicked() {
                        picked = Some(Expr::Add {
                            a: zero(),
                            b: zero(),
                        });
                    }
                    let constants = [
                        ("true", Expr::Bool(true)),
                        ("0", Expr::Int(0)),
                        ("text", Expr::Text(String::new())),
                    ];
                    for (label, constant) in constants {
                        if chip::colored(ui, label, theme::INPUT, theme::MUTED).clicked() {
                            picked = Some(constant);
                        }
                    }
                } else if chip::colored(ui, "true", theme::INPUT, theme::MUTED).clicked() {
                    picked = Some(Expr::Bool(true));
                }
            });
        });
    picked
}

fn tag_chip(ui: &mut Ui, block: BlockKind) -> egui::Response {
    let response = chip::colored(ui, block.title(), theme::RAISED, theme::TEXT);
    ui.label(
        RichText::new(type_word(block.yields()).to_uppercase())
            .size(10.0)
            .color(theme::FAINT),
    );
    response
}

fn entry(ui: &mut Ui, query: RiftQueryKind) -> bool {
    let block = BlockKind::Query(query);
    let response = ui
        .horizontal(|ui| {
            ui.label(RichText::new(block.title()).size(13.0).color(theme::TEXT));
            ui.label(theme::muted(type_word(block.yields())).size(11.5));
        })
        .response
        .interact(egui::Sense::click());
    if response.hovered() {
        paint::row_wash(ui.painter(), response.rect.expand2(egui::vec2(6.0, 1.0)));
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// A condition at a glance, as a writer reads it among the blocks of a node.
pub fn summary(names: &impl ContentNames, condition: &Condition) -> String {
    fn go(names: &impl ContentNames, condition: &Condition, out: &mut String, depth: usize) {
        if depth > 4 {
            out.push('…');
            return;
        }
        let joined = |list: &[Condition], word: &str, out: &mut String| {
            out.push('(');
            for (index, item) in list.iter().enumerate() {
                if index > 0 {
                    out.push_str(word);
                }
                go(names, item, out, depth + 1);
            }
            out.push(')');
        };
        match condition {
            Expr::Var { var } => out.push_str(&format!("◇{var}")),
            Expr::All { of } => joined(of, " and ", out),
            Expr::Any { of } => joined(of, " or ", out),
            Expr::Join { of } => joined(of, " + ", out),
            Expr::Not { of } => {
                out.push_str("not ");
                go(names, of, out, depth + 1);
            }
            Expr::Round { of } => {
                out.push_str("round ");
                go(names, of, out, depth + 1);
            }
            Expr::Local { local: name }
            | Expr::State { state: name }
            | Expr::Event { event: name } => out.push_str(name),
            Expr::Bool(flag) => out.push_str(&flag.to_string()),
            Expr::Int(number) => out.push_str(&number.to_string()),
            Expr::Num(number) => out.push_str(&number.to_string()),
            Expr::Text(text) => out.push_str(&format!("{text:?}")),
            Expr::Query(_) => out.push_str(&script_condition::describe(names, condition)),
            binary => {
                let shown = sign(binary.kind()).unwrap_or("?");
                let children = binary.children();
                if let [a, b] = children.as_slice() {
                    go(names, a, out, depth + 1);
                    out.push_str(&format!(" {shown} "));
                    go(names, b, out, depth + 1);
                }
            }
        }
    }
    let mut out = String::new();
    go(names, condition, &mut out, 0);
    out
}

/// Starts the next row of blocks unless `width` is left on this one, as a field of a set width
/// doesn't wrap by itself.
fn make_room(ui: &mut Ui, width: f32) {
    if ui.available_size_before_wrap().x < width {
        ui.end_row();
    }
}

/// The name of a value the script names, picked among those it declares, each shown with what
/// type it is.
fn declared_name(
    ui: &mut Ui,
    declared: &[(String, &str)],
    name: &mut String,
    path: &str,
) -> FormChange {
    let mut picked = name.clone();
    controls::combo(ui.id().with(path).with("name"))
        .selected_text(name.as_str())
        .show_ui(ui, |ui| {
            for (declared, ty) in declared {
                ui.selectable_value(&mut picked, declared.clone(), format!("{declared}  {ty}"));
            }
        });
    if picked == *name {
        return FormChange::default();
    }
    *name = picked;
    FormChange::at(path)
}
