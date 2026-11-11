//! The recipe book view: a book's recipes as a plain list, a row each, saying everything it makes
//! and everything it takes. What joins the recipes to each other is followed in the inspector: the other
//! ways to make a thing, and the recipes it goes into. Every change is the whole file written
//! again, so the loader's rules refuse a loop or an empty recipe as it is made.

use engine::core::content::ContentFile;
use engine::systems::crafting::{Recipe, RecipeBookDef};
use engine::systems::item::{ItemDef, ItemId, ItemStack};
use ui::egui::{self, Align, Layout, Rect, Sense, Ui, Vec2};
use ui::face;
use ui::{chip, controls, form, sheet, theme};

use crate::app::Editor;
use crate::forms::FormCx;
use crate::icons::{self, IconTone};
use crate::picker::{self, Dragged};
use crate::project::{ContentEdit, RowSelection};
use crate::row_form;
use crate::tabs::shown_path;

const ROW: f32 = 34.0;
const NUMBER: f32 = 64.0;
const MAKES: f32 = 330.0;
const ADD: f32 = 150.0;
const ADD_MADE: f32 = 110.0;
const FACE: f32 = 18.0;

/// What a recipe book's window keeps besides its file: what the list is narrowed to, and the item
/// a recipe being added makes until it has an ingredient.
#[derive(Default)]
pub struct RecipeBookView {
    pub filter: String,
    pub draft: Option<String>,
}

/// What the inspector follows: a recipe, by its place in the book, or an item by its file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Picked {
    Recipe(usize),
    Item(String),
}

const RECIPE_PART: &str = "recipe:";
const ITEM_PART: &str = "item:";

impl Picked {
    /// The part of the book's row a selection names it by.
    pub fn part(&self) -> String {
        match self {
            Picked::Recipe(index) => format!("{RECIPE_PART}{index}"),
            Picked::Item(path) => format!("{ITEM_PART}{path}"),
        }
    }

    fn of_part(part: &str) -> Option<Picked> {
        match part.strip_prefix(RECIPE_PART) {
            Some(index) => index.parse().ok().map(Picked::Recipe),
            None => part
                .strip_prefix(ITEM_PART)
                .map(|path| Picked::Item(path.to_owned())),
        }
    }
}

/// Selects `picked` in the book at `key`, for its inspector to follow.
pub fn select(editor: &mut Editor, key: &str, picked: Option<Picked>) {
    editor.select(match picked {
        Some(picked) => RowSelection::parts(RecipeBookDef::TABLE, key, vec![picked.part()]),
        None => RowSelection::row(RecipeBookDef::TABLE, key),
    });
}

/// Adds to the book at `key` a recipe making one of the item at `makes` from one of the item at
/// `from`.
pub fn add_recipe(editor: &mut Editor, key: &str, makes: &str, from: &str) -> bool {
    let (Some(made), Some(taken)) = (item_id(editor, makes), item_id(editor, from)) else {
        editor.toast("a recipe names two item files");
        return false;
    };
    let label = format!("Add a recipe for {}", name_of(editor, makes));
    editor.change(ContentEdit::new(label), key, |book: &mut RecipeBookDef| {
        book.recipes.push(Recipe {
            makes: vec![ItemStack::new(made, 1)],
            from: vec![ItemStack::new(taken, 1)],
        });
        Ok(())
    })
}

/// The two lists of a recipe: what one craft gives, and what it takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecipeSide {
    Makes,
    From,
}

impl RecipeSide {
    fn of(self, recipe: &mut Recipe) -> &mut Vec<ItemStack> {
        match self {
            RecipeSide::Makes => &mut recipe.makes,
            RecipeSide::From => &mut recipe.from,
        }
    }

    fn verb(self) -> &'static str {
        match self {
            RecipeSide::Makes => "makes",
            RecipeSide::From => "takes",
        }
    }

    fn field(self) -> &'static str {
        match self {
            RecipeSide::Makes => "makes",
            RecipeSide::From => "from",
        }
    }
}

/// Adds one of the item at `item` to a `side` of the recipe at `recipe`: to its amount when the
/// side lists it already.
pub fn add_stack(
    editor: &mut Editor,
    key: &str,
    recipe: usize,
    side: RecipeSide,
    item: &str,
) -> bool {
    let Some(id) = item_id(editor, item) else {
        editor.toast("a recipe lists item files");
        return false;
    };
    let label = format!(
        "Recipe {} {} {}",
        recipe + 1,
        side.verb(),
        name_of(editor, item)
    );
    change_recipe(editor, key, label, recipe, |recipe| {
        let stacks = side.of(recipe);
        match stacks.iter_mut().find(|stack| stack.item == id) {
            Some(stack) => stack.count = stack.count.saturating_add(1),
            None => stacks.push(ItemStack::new(id, 1)),
        }
    })
}

/// Sets how many of the `nth` item of a `side` one craft of the recipe at `recipe` gives or takes.
pub fn set_amount(
    editor: &mut Editor,
    key: &str,
    recipe: usize,
    (side, nth): (RecipeSide, usize),
    amount: u32,
) -> bool {
    let label = format!("Change an amount recipe {} {}", recipe + 1, side.verb());
    change_recipe(editor, key, label, recipe, |recipe| {
        if let Some(stack) = side.of(recipe).get_mut(nth) {
            stack.count = amount;
        }
    })
}

/// Makes the `nth` item of a `side` of the recipe at `recipe` the item at `item` instead.
pub fn set_item(
    editor: &mut Editor,
    key: &str,
    recipe: usize,
    (side, nth): (RecipeSide, usize),
    item: &str,
) -> bool {
    let Some(id) = item_id(editor, item) else {
        return false;
    };
    let label = format!(
        "Recipe {} {} {}",
        recipe + 1,
        side.verb(),
        name_of(editor, item)
    );
    change_recipe(editor, key, label, recipe, |recipe| {
        if let Some(stack) = side.of(recipe).get_mut(nth) {
            stack.item = id;
        }
    })
}

pub fn remove_stack(
    editor: &mut Editor,
    key: &str,
    recipe: usize,
    (side, nth): (RecipeSide, usize),
) -> bool {
    let label = format!("Remove what recipe {} {}", recipe + 1, side.verb());
    change_recipe(editor, key, label, recipe, |recipe| {
        let stacks = side.of(recipe);
        if nth < stacks.len() {
            stacks.remove(nth);
        }
    })
}

pub fn remove_recipe(editor: &mut Editor, key: &str, recipe: usize) -> bool {
    editor.change(
        ContentEdit::new(format!("Remove recipe {}", recipe + 1)),
        key,
        |book: &mut RecipeBookDef| {
            if recipe < book.recipes.len() {
                book.recipes.remove(recipe);
            }
            Ok(())
        },
    )
}

/// Moves the recipe at `from` to stand at `to` in the book, which is the order a prompt lists it.
pub fn move_recipe(editor: &mut Editor, key: &str, from: usize, to: usize) -> bool {
    editor.change(
        ContentEdit::new(format!("Move recipe {}", from + 1)),
        key,
        |book: &mut RecipeBookDef| {
            if from < book.recipes.len() && to < book.recipes.len() {
                let moved = book.recipes.remove(from);
                book.recipes.insert(to, moved);
            }
            Ok(())
        },
    )
}

/// The items the recipes of `book` take and none of them makes, each once, in the order they are
/// first taken.
pub fn not_made(book: &RecipeBookDef) -> Vec<ItemId> {
    let mut raw: Vec<ItemId> = Vec::new();
    for stack in book.recipes.iter().flat_map(|recipe| &recipe.from) {
        let made = book
            .recipes
            .iter()
            .any(|recipe| recipe.makes_item(stack.item));
        if !made && !raw.contains(&stack.item) {
            raw.push(stack.item);
        }
    }
    raw
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let Some(book) = super::read_shown::<RecipeBookDef>(editor, ui, key) else {
        return;
    };
    editor.views.recipe_book.view_or_default(key);
    let rows = shown_rows(editor, &book);
    ui::panel::inspector(ui, editor, "recipe_book", |editor, ui| {
        inspect(editor, ui, key, &rows);
    });
    egui::Frame::new()
        .inner_margin(crate::widgets::VIEW_PADDING)
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            bar(editor, ui, key, &book, &rows);
            ui.add_space(6.0);
            let listed = egui::ScrollArea::vertical()
                .id_salt(("recipes", key))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    list(editor, ui, key, &rows);
                    draft_row(editor, ui, key, rows.len());
                    foot(editor, ui, key, &book, &rows);
                });
            dropped_item(editor, ui, key, listed.inner_rect);
        });
    editor.status.left.push(shown_path(key).to_owned());
}

/// An item as a row shows it: its file and its name.
#[derive(Clone)]
struct ShownItem {
    path: String,
    name: String,
}

/// A recipe as the list shows it.
struct ShownRecipe {
    makes: Vec<(ShownItem, u32)>,
    from: Vec<(ShownItem, u32)>,
}

impl ShownRecipe {
    fn side(&self, side: RecipeSide) -> &[(ShownItem, u32)] {
        match side {
            RecipeSide::Makes => &self.makes,
            RecipeSide::From => &self.from,
        }
    }

    fn amount_of(&self, side: RecipeSide, path: &str) -> Option<u32> {
        self.side(side)
            .iter()
            .find(|(item, _)| item.path == path)
            .map(|(_, amount)| *amount)
    }

    fn holds(&self, filter: &str) -> bool {
        let filter = filter.trim().to_lowercase();
        filter.is_empty()
            || self
                .makes
                .iter()
                .chain(&self.from)
                .any(|(item, _)| item.name.to_lowercase().contains(&filter))
    }

    fn listed(&self, side: RecipeSide) -> String {
        self.side(side)
            .iter()
            .map(|(item, amount)| format!("{amount} {}", item.name))
            .collect::<Vec<_>>()
            .join(" + ")
    }
}

fn shown_rows(editor: &Editor, book: &RecipeBookDef) -> Vec<ShownRecipe> {
    let shown = |stacks: &[ItemStack]| {
        stacks
            .iter()
            .map(|stack| (shown_item(editor, stack.item), stack.count))
            .collect()
    };
    book.recipes
        .iter()
        .map(|recipe| ShownRecipe {
            makes: shown(&recipe.makes),
            from: shown(&recipe.from),
        })
        .collect()
}

fn shown_item(editor: &Editor, item: ItemId) -> ShownItem {
    let path = editor
        .reader()
        .and_then(|reader| reader.path_of(item).map(str::to_owned))
        .unwrap_or_default();
    ShownItem {
        name: name_of(editor, &path),
        path,
    }
}

fn item_id(editor: &Editor, path: &str) -> Option<ItemId> {
    editor.reader()?.id_of::<ItemDef>(path)
}

/// The name of the item at `path` as the game shows it, or its file's while it does not read.
fn name_of(editor: &Editor, path: &str) -> String {
    editor
        .read::<ItemDef>(path)
        .and_then(Result::ok)
        .map_or_else(|| shown_path(path).to_owned(), |item| item.name.clone())
}

fn change_recipe(
    editor: &mut Editor,
    key: &str,
    label: String,
    recipe: usize,
    change: impl FnOnce(&mut Recipe),
) -> bool {
    editor.change(
        ContentEdit::new(label),
        key,
        |book: &mut RecipeBookDef| match book.recipes.get_mut(recipe) {
            Some(recipe) => {
                change(recipe);
                Ok(())
            }
            None => Err(format!("there is no recipe {}", recipe + 1)),
        },
    )
}

fn selected(editor: &Editor, key: &str) -> Option<Picked> {
    let selection = &editor.selection;
    let here = selection.table == RecipeBookDef::TABLE && selection.keys == [key];
    selection
        .parts
        .first()
        .filter(|_| here)
        .and_then(|part| Picked::of_part(part))
}

/// A field picking an item file, `wide` across: the file picked, when one just was.
fn pick_item(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    held: &str,
    pointer: &str,
    wide: f32,
) -> Option<String> {
    let mut actions = Vec::new();
    let mut picked = held.to_owned();
    let mut changed = false;
    {
        let Editor {
            project,
            form,
            shapes,
            ..
        } = editor;
        let project = project.as_ref()?;
        let mut cx = FormCx::schemaless(project, shapes, form, &mut actions)
            .editing(RecipeBookDef::TABLE, Some(key));
        ui.allocate_ui_with_layout(
            Vec2::new(wide, ROW - 8.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.set_width(wide);
                changed = picker::row(ui, &mut cx, ItemDef::TABLE, &mut picked, pointer);
            },
        );
    }
    for action in actions {
        row_form::act(editor, action, RecipeBookDef::TABLE, key);
    }
    (changed && !picked.is_empty()).then_some(picked)
}

/// The picture of the item at `path`, or the icon items have.
fn item_face(editor: &mut Editor, ui: &mut Ui, path: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(FACE), Sense::hover());
    let Editor { project, form, .. } = editor;
    let Some(project) = project.as_ref() else {
        return;
    };
    let mut budget = u32::MAX;
    let picture = icons::picture(ui.ctx(), &mut form.thumbnails, project, path, &mut budget);
    icons::paint_file(
        ui.painter(),
        rect,
        picture,
        icons::of_table(ItemDef::TABLE),
        IconTone::Normal,
    );
}

fn bar(editor: &mut Editor, ui: &mut Ui, key: &str, book: &RecipeBookDef, rows: &[ShownRecipe]) {
    ui.horizontal(|ui| {
        ui.label(face::mono(shown_path(key)).color(theme::CODE));
        ui.add_space(12.0);
        ui.label(theme::muted("new recipe makes"));
        if let Some(makes) = pick_item(editor, ui, key, "", "/recipes/new", 190.0) {
            editor.views.recipe_book.view_mut(key).draft = Some(makes);
            select(editor, key, None);
        }
        ui.add_space(12.0);
        let view = editor.views.recipe_book.view_mut(key);
        ui.add(
            egui::TextEdit::singleline(&mut view.filter)
                .hint_text("Filter…")
                .desired_width(160.0),
        );
        let mut made: Vec<ItemId> = book
            .recipes
            .iter()
            .flat_map(|recipe| &recipe.makes)
            .map(|stack| stack.item)
            .collect();
        made.sort_unstable();
        made.dedup();
        ui.label(theme::muted(format!(
            "{} recipes · {} items made",
            rows.len(),
            made.len()
        )));
    });
}

fn list(editor: &mut Editor, ui: &mut Ui, key: &str, rows: &[ShownRecipe]) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (title, wide) in [("", NUMBER), ("makes", MAKES)] {
            ui.allocate_ui_with_layout(
                Vec2::new(wide, 18.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.set_width(wide);
                    ui.label(form::section_heading(title));
                },
            );
        }
        ui.label(form::section_heading("from"));
    });
    let filter = editor.views.recipe_book.view(key).filter.clone();
    let chosen = selected(editor, key);
    for (index, row) in rows.iter().enumerate() {
        if !row.holds(&filter) {
            continue;
        }
        let on = chosen == Some(Picked::Recipe(index));
        let shown = ui
            .push_id(("recipe", index), |ui| {
                recipe_row(editor, ui, key, index, rows.len(), row, &chosen)
            })
            .response;
        if on {
            ui.painter().rect_stroke(
                shown.rect.expand(1.0),
                3.0,
                egui::Stroke::new(1.5, theme::ACCENT),
                egui::StrokeKind::Outside,
            );
        }
        form::rule(ui);
    }
    let hidden = rows.iter().filter(|row| !row.holds(&filter)).count();
    if hidden > 0 {
        ui.add_space(4.0);
        form::hint(
            ui,
            &format!("{hidden} recipes are hidden by the filter · clear it to bring them back"),
        );
    }
}

fn recipe_row(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    index: usize,
    len: usize,
    row: &ShownRecipe,
    chosen: &Option<Picked>,
) {
    let on = chosen == &Some(Picked::Recipe(index));
    ui.horizontal(|ui| {
        ui.set_min_height(ROW);
        ui.allocate_ui_with_layout(
            Vec2::new(NUMBER, ROW),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.set_width(NUMBER);
                if ui
                    .selectable_label(on, face::mono(format!("{}", index + 1)))
                    .on_hover_text("Select this recipe")
                    .clicked()
                {
                    select(editor, key, Some(Picked::Recipe(index)));
                }
                if let Some(to) = controls::reorder(ui, index, len)
                    && move_recipe(editor, key, index, to)
                {
                    select(editor, key, Some(Picked::Recipe(to)));
                }
            },
        );
        ui.allocate_ui_with_layout(
            Vec2::new(MAKES, ROW),
            Layout::left_to_right(Align::Center).with_main_wrap(true),
            |ui| {
                ui.set_width(MAKES);
                ui.spacing_mut().item_spacing.x = 4.0;
                stack_chips(editor, ui, key, &row.makes, chosen);
                let pointer = format!("/recipes/{index}/makes/{}", row.makes.len());
                if let Some(item) = pick_item(editor, ui, key, "", &pointer, ADD_MADE) {
                    add_stack(editor, key, index, RecipeSide::Makes, &item);
                }
            },
        );
        ui.spacing_mut().item_spacing.x = 4.0;
        stack_chips(editor, ui, key, &row.from, chosen);
        let pointer = format!("/recipes/{index}/from/{}", row.from.len());
        if let Some(item) = pick_item(editor, ui, key, "", &pointer, ADD) {
            add_stack(editor, key, index, RecipeSide::From, &item);
        }
    });
}

/// Each item of one side of a recipe as a chip saying its amount, which selects the item.
fn stack_chips(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    stacks: &[(ShownItem, u32)],
    chosen: &Option<Picked>,
) {
    for (item, amount) in stacks {
        item_face(editor, ui, &item.path);
        let picked = chosen == &Some(Picked::Item(item.path.clone()));
        let label = format!("{} ×{amount}", item.name);
        if chip::toggle(ui, &label, picked)
            .on_hover_text(shown_path(&item.path))
            .clicked()
        {
            select(editor, key, Some(Picked::Item(item.path.clone())));
        }
        ui.add_space(4.0);
    }
}

/// The recipe being added, at the foot of the list until it has something to be made from.
fn draft_row(editor: &mut Editor, ui: &mut Ui, key: &str, len: usize) {
    let Some(makes) = editor.views.recipe_book.view(key).draft.clone() else {
        return;
    };
    let name = name_of(editor, &makes);
    ui.horizontal(|ui| {
        ui.set_min_height(ROW);
        ui.allocate_ui_with_layout(
            Vec2::new(NUMBER, ROW),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.set_width(NUMBER);
                ui.label(theme::faint(format!("{}", len + 1)));
            },
        );
        ui.allocate_ui_with_layout(
            Vec2::new(MAKES, ROW),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.set_width(MAKES);
                item_face(editor, ui, &makes);
                ui.add(egui::Label::new(face::bold(format!("{name} ×1"))).truncate());
            },
        );
        if let Some(from) = pick_item(editor, ui, key, "", "/recipes/draft/from", ADD)
            && add_recipe(editor, key, &makes, &from)
        {
            editor.views.recipe_book.view_mut(key).draft = None;
            select(editor, key, Some(Picked::Recipe(len)));
        }
        if controls::remove(ui, "Don't add this recipe").clicked() {
            editor.views.recipe_book.view_mut(key).draft = None;
        }
    });
    form::hint(
        ui,
        &format!(
            "A recipe for {name} once it has something to be made from: pick its first ingredient."
        ),
    );
    form::rule(ui);
}

fn foot(editor: &mut Editor, ui: &mut Ui, key: &str, book: &RecipeBookDef, rows: &[ShownRecipe]) {
    ui.add_space(14.0);
    let raw: Vec<ShownItem> = not_made(book)
        .into_iter()
        .map(|item| shown_item(editor, item))
        .collect();
    if rows.is_empty() {
        form::hint(
            ui,
            "No recipes yet. Pick what a new recipe makes above, or drag an item file in.",
        );
        return;
    }
    if raw.is_empty() {
        return;
    }
    sheet::section(
        ui,
        "not made here · taken by these recipes, made by none of them",
    );
    let chosen = selected(editor, key);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for item in &raw {
            item_face(editor, ui, &item.path);
            let picked = chosen == Some(Picked::Item(item.path.clone()));
            if chip::toggle(ui, &item.name, picked)
                .on_hover_text(shown_path(&item.path))
                .clicked()
            {
                select(editor, key, Some(Picked::Item(item.path.clone())));
            }
            ui.add_space(6.0);
        }
    });
}

/// An item file dragged from a browser onto the list starts a recipe that makes it.
fn dropped_item(editor: &mut Editor, ui: &mut Ui, key: &str, list: Rect) {
    let response = ui.interact(list, ui.id().with(("drop", key)), Sense::hover());
    let hovering = response.dnd_hover_payload::<Dragged>().is_some_and(
        |dragged| matches!(&*dragged, Dragged::Row { table, .. } if table == ItemDef::TABLE),
    );
    if hovering {
        ui.painter().rect_stroke(
            list,
            4.0,
            egui::Stroke::new(2.0, theme::ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    if let Some(dragged) = response.dnd_release_payload::<Dragged>()
        && let Dragged::Row { table, key: item } = &*dragged
        && table == ItemDef::TABLE
    {
        editor.views.recipe_book.view_mut(key).draft = Some(item.clone());
        select(editor, key, None);
    }
}

fn inspect(editor: &mut Editor, ui: &mut Ui, key: &str, rows: &[ShownRecipe]) {
    match selected(editor, key) {
        Some(Picked::Recipe(index)) => match rows.get(index) {
            Some(row) => inspect_recipe(editor, ui, key, index, row, rows),
            None => select(editor, key, None),
        },
        Some(Picked::Item(path)) => inspect_item(editor, ui, key, &path, rows),
        None => form::nothing_selected(
            ui,
            "Select a recipe by its number to change it, or an item to follow it through the book.",
        ),
    }
}

fn inspect_recipe(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    index: usize,
    row: &ShownRecipe,
    rows: &[ShownRecipe],
) {
    sheet::section(ui, &format!("recipe {}", index + 1));
    for side in [RecipeSide::Makes, RecipeSide::From] {
        ui.push_id(side.field(), |ui| {
            inspect_side(editor, ui, key, index, side, row.side(side));
        });
        ui.add_space(8.0);
    }
    form::hint(
        ui,
        &format!(
            "One craft takes {} and makes {}.",
            row.listed(RecipeSide::From),
            row.listed(RecipeSide::Makes)
        ),
    );
    ui.add_space(4.0);
    if controls::danger(ui, "Remove recipe").clicked() && remove_recipe(editor, key, index) {
        select(editor, key, None);
        return;
    }
    ui.add_space(10.0);
    for (item, _) in &row.makes {
        made_by(editor, ui, key, item, rows, Some(index));
        goes_into(editor, ui, key, item, rows);
        open_item(editor, ui, &item.path);
        ui.add_space(8.0);
    }
}

/// One side of the recipe at `index` as a form: each item with its amount, and a field adding
/// another.
fn inspect_side(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    index: usize,
    side: RecipeSide,
    stacks: &[(ShownItem, u32)],
) {
    sheet::section(ui, side.field());
    let field = side.field();
    for (nth, (item, amount)) in stacks.iter().enumerate() {
        ui.push_id(nth, |ui| {
            ui.horizontal(|ui| {
                let pointer = format!("/recipes/{index}/{field}/{nth}/item");
                let wide = (ui.available_width() - 96.0).max(100.0);
                if let Some(picked) = pick_item(editor, ui, key, &item.path, &pointer, wide) {
                    set_item(editor, key, index, (side, nth), &picked);
                }
                let mut count = *amount;
                let changed = ui
                    .add(egui::DragValue::new(&mut count).range(1..=u32::MAX))
                    .on_hover_text(format!("How many one craft {}", side.verb()))
                    .changed();
                if changed {
                    set_amount(editor, key, index, (side, nth), count);
                }
                if controls::remove(ui, "Take this out of the recipe").clicked() {
                    remove_stack(editor, key, index, (side, nth));
                }
            });
        });
    }
    let wide = (ui.available_width() - 90.0).max(120.0);
    form::kv(ui, "+ item", |ui| {
        let pointer = format!("/recipes/{index}/{field}/{}", stacks.len());
        if let Some(item) = pick_item(editor, ui, key, "", &pointer, wide) {
            add_stack(editor, key, index, side, &item);
        }
    });
}

fn inspect_item(editor: &mut Editor, ui: &mut Ui, key: &str, path: &str, rows: &[ShownRecipe]) {
    let item = ShownItem {
        path: path.to_owned(),
        name: name_of(editor, path),
    };
    sheet::section(ui, &item.name);
    form::kv_code(ui, "item", shown_path(path));
    ui.add_space(8.0);
    made_by(editor, ui, key, &item, rows, None);
    goes_into(editor, ui, key, &item, rows);
    open_item(editor, ui, path);
}

/// The recipes that make `item`, other than the one at `besides`, each a line that jumps to its
/// row.
fn made_by(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    item: &ShownItem,
    rows: &[ShownRecipe],
    besides: Option<usize>,
) {
    let made: Vec<usize> = (0..rows.len())
        .filter(|index| Some(*index) != besides)
        .filter(|index| {
            rows[*index]
                .amount_of(RecipeSide::Makes, &item.path)
                .is_some()
        })
        .collect();
    let title = match besides {
        Some(_) if made.is_empty() => return,
        Some(_) => format!("{} is also made by", item.name),
        None => format!("made by · {} recipes here", made.len()),
    };
    sheet::section(ui, &title);
    for index in made {
        jump(
            editor,
            ui,
            key,
            index,
            &rows[index].listed(RecipeSide::From),
        );
    }
    ui.add_space(8.0);
}

/// The recipes that take `item`, each a line that jumps to its row.
fn goes_into(editor: &mut Editor, ui: &mut Ui, key: &str, item: &ShownItem, rows: &[ShownRecipe]) {
    let taken: Vec<(usize, u32)> = rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| Some((index, row.amount_of(RecipeSide::From, &item.path)?)))
        .collect();
    sheet::section(
        ui,
        &format!("{} goes into · {} recipes here", item.name, taken.len()),
    );
    for (index, amount) in taken {
        let line = format!("{amount} into {}", rows[index].listed(RecipeSide::Makes));
        jump(editor, ui, key, index, &line);
    }
    ui.add_space(8.0);
}

/// A line naming the recipe at `index`, which selects it.
fn jump(editor: &mut Editor, ui: &mut Ui, key: &str, index: usize, line: &str) {
    ui.horizontal(|ui| {
        ui.label(theme::muted(format!("{}", index + 1)));
        if ui
            .link(line)
            .on_hover_text(format!("Go to recipe {}", index + 1))
            .clicked()
        {
            select(editor, key, Some(Picked::Recipe(index)));
        }
    });
}

fn open_item(editor: &mut Editor, ui: &mut Ui, path: &str) {
    if ui
        .button(format!("{} →", shown_path(path)))
        .on_hover_text("Open the item's file")
        .clicked()
    {
        editor.open_row(ItemDef::TABLE, path);
    }
}
