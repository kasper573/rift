//! The recipe book view: a book's recipes as a plain list, a row each, saying everything it makes
//! and everything it takes. What joins the recipes to each other is followed in the inspector: the other
//! ways to make a thing, and the recipes it goes into. Every change is the whole file written
//! again, so the loader's rules refuse a loop or an empty recipe as it is made.

use engine::core::content::ContentFile;
use engine::systems::crafting::{Recipe, RecipeBookDef};
use engine::systems::item::{ItemDef, ItemId, ItemStack};
use ui::egui::{self, Align, Layout, Rangef, Rect, Sense, Ui, Vec2};
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
const HANDLE: f32 = 30.0;
/// The share of a row's room that what it makes takes, what it is made from taking the rest.
const MAKES_SHARE: f32 = 0.4;
const COLUMN_GAP: f32 = 16.0;
const HEADING: f32 = 18.0;
const PLUS: f32 = 22.0;
const FACE: f32 = 18.0;
/// The room an item's amount and its cross take after its field in the inspector.
const ITEM_TRAIL: f32 = 96.0;

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
        "Add {} to what {} {}",
        name_of(editor, item),
        called(editor, key, recipe),
        side.verb()
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
    let label = format!(
        "Change an amount {} {}",
        called(editor, key, recipe),
        side.verb()
    );
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
        "Change what {} {} to {}",
        called(editor, key, recipe),
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
    let label = format!(
        "Take an item out of what {} {}",
        called(editor, key, recipe),
        side.verb()
    );
    change_recipe(editor, key, label, recipe, |recipe| {
        let stacks = side.of(recipe);
        if nth < stacks.len() {
            stacks.remove(nth);
        }
    })
}

pub fn remove_recipe(editor: &mut Editor, key: &str, recipe: usize) -> bool {
    let label = format!("Remove {}", called(editor, key, recipe));
    editor.change(ContentEdit::new(label), key, |book: &mut RecipeBookDef| {
        if recipe < book.recipes.len() {
            book.recipes.remove(recipe);
        }
        Ok(())
    })
}

/// Moves the recipe at `from` to stand at `to` in the book, which is the order a prompt lists it.
pub fn move_recipe(editor: &mut Editor, key: &str, from: usize, to: usize) -> bool {
    let label = format!("Move {}", called(editor, key, from));
    editor.change(ContentEdit::new(label), key, |book: &mut RecipeBookDef| {
        if from < book.recipes.len() && to < book.recipes.len() {
            let moved = book.recipes.remove(from);
            book.recipes.insert(to, moved);
        }
        Ok(())
    })
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

/// What the history calls the recipe at `recipe` of the book at `key`: by what it makes.
fn called(editor: &Editor, key: &str, recipe: usize) -> String {
    let made = editor
        .read::<RecipeBookDef>(key)
        .and_then(Result::ok)
        .and_then(|book| {
            let recipe = book.recipes.get(recipe)?;
            let names: Vec<String> = recipe
                .makes
                .iter()
                .map(|stack| shown_item(editor, stack.item).name)
                .collect();
            Some(names.join(" + "))
        });
    match made {
        Some(made) => format!("the {made} recipe"),
        None => "a recipe".to_owned(),
    }
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
            None => Err("the book has no such recipe".to_owned()),
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

/// What a field picking an item file is for.
#[derive(Clone, Copy)]
enum ItemField<'a> {
    /// The item at this path, to be changed for another.
    Held(&'a str),
    /// One more item of a list, asked for in words.
    Added,
    /// One more item of a list, asked for by a plus alone.
    AddedByPlus,
}

/// A field picking an item file, `wide` across: the file picked, when one just was.
fn pick_item(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    field: ItemField,
    pointer: &str,
    wide: f32,
) -> Option<String> {
    let mut actions = Vec::new();
    let mut picked = None;
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
                picked = match field {
                    ItemField::Held(held) => {
                        let mut item = held.to_owned();
                        picker::row(ui, &mut cx, ItemDef::TABLE, &mut item, pointer).then_some(item)
                    }
                    ItemField::Added => picker::row_added(ui, &mut cx, ItemDef::TABLE, pointer),
                    ItemField::AddedByPlus => {
                        picker::row_added_by_plus(ui, &mut cx, ItemDef::TABLE, pointer)
                    }
                };
            },
        );
    }
    for action in actions {
        row_form::act(editor, action, RecipeBookDef::TABLE, key);
    }
    picked.filter(|item| !item.is_empty())
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
    ui.horizontal_wrapped(|ui| {
        ui.label(theme::muted("new recipe makes"));
        if let Some(makes) = pick_item(editor, ui, key, ItemField::Added, "/recipes/new", 190.0) {
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

/// Where the columns of the list stand, the same for every row: the handle, what a recipe makes,
/// and what it is made from.
#[derive(Clone, Copy)]
struct Columns {
    handle: Rangef,
    makes: Rangef,
    from: Rangef,
}

impl Columns {
    fn across(ui: &Ui) -> Columns {
        let (left, right) = (ui.max_rect().left(), ui.max_rect().right());
        let makes = left + HANDLE;
        let from = makes + ((right - makes) * MAKES_SHARE).round();
        Columns {
            handle: Rangef::new(left, makes),
            makes: Rangef::new(makes, from - COLUMN_GAP),
            from: Rangef::new(from, right),
        }
    }

    fn whole(self) -> Rangef {
        Rangef::new(self.handle.min, self.from.max)
    }
}

/// A cell of the row standing at `top`, `across` its column and at least `high`: its content goes
/// left to right and on below where the column ends. How far down it reaches.
fn cell(
    ui: &mut Ui,
    name: &str,
    across: Rangef,
    (top, high): (f32, f32),
    content: impl FnOnce(&mut Ui),
) -> f32 {
    let room = Rect::from_x_y_ranges(across, top..=top + high);
    let mut cell = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(name)
            .max_rect(room)
            .layout(Layout::left_to_right(Align::Center).with_main_wrap(true)),
    );
    cell.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
    content(&mut cell);
    cell.min_rect().bottom().max(room.bottom())
}

/// Takes the room of a row of cells standing at `top` and reaching down to `bottom`.
fn row_of_cells(ui: &mut Ui, columns: Columns, top: f32, bottom: f32) -> Rect {
    let rect = Rect::from_x_y_ranges(columns.whole(), top..=bottom);
    ui.allocate_rect(rect, Sense::hover());
    rect
}

fn list(editor: &mut Editor, ui: &mut Ui, key: &str, rows: &[ShownRecipe]) {
    let columns = Columns::across(ui);
    let top = ui.cursor().top();
    let mut bottom = top + HEADING;
    for (title, across) in [("makes", columns.makes), ("from", columns.from)] {
        bottom = bottom.max(cell(ui, title, across, (top, HEADING), |ui| {
            ui.label(form::section_heading(title));
        }));
    }
    row_of_cells(ui, columns, top, bottom);
    let filter = editor.views.recipe_book.view(key).filter.clone();
    let chosen = selected(editor, key);
    let mut drawn = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        if !row.holds(&filter) {
            continue;
        }
        let on = chosen == Some(Picked::Recipe(index));
        let shown = ui
            .push_id(("recipe", index), |ui| {
                recipe_row(editor, ui, key, columns, index, row, &chosen)
            })
            .inner;
        drawn.push((index, shown));
        if on {
            ui.painter().rect_stroke(
                shown.expand(1.0),
                3.0,
                egui::Stroke::new(1.5, theme::ACCENT),
                egui::StrokeKind::Outside,
            );
        }
        form::rule(ui);
    }
    if let Some((from, to)) = controls::dropped_row(ui, list_id(key), &drawn)
        && move_recipe(editor, key, from, to)
    {
        select(editor, key, Some(Picked::Recipe(to)));
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

/// The list of the book at `key`, as its rows are carried through it.
fn list_id(key: &str) -> egui::Id {
    egui::Id::new(("recipes of", key))
}

fn recipe_row(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    columns: Columns,
    index: usize,
    row: &ShownRecipe,
    chosen: &Option<Picked>,
) -> Rect {
    let top = ui.cursor().top();
    let mut bottom = cell(ui, "handle", columns.handle, (top, ROW), |ui| {
        if controls::drag_handle(ui, list_id(key), index)
            .on_hover_text("Drag to move this recipe · click to select it")
            .clicked()
        {
            select(editor, key, Some(Picked::Recipe(index)));
        }
    });
    for (side, across) in [
        (RecipeSide::Makes, columns.makes),
        (RecipeSide::From, columns.from),
    ] {
        bottom = bottom.max(cell(ui, side.field(), across, (top, ROW), |ui| {
            let stacks = row.side(side);
            stack_chips(editor, ui, key, stacks, chosen);
            let pointer = format!("/recipes/{index}/{}/{}", side.field(), stacks.len());
            if let Some(item) = pick_item(editor, ui, key, ItemField::AddedByPlus, &pointer, PLUS) {
                add_stack(editor, key, index, side, &item);
            }
        }));
    }
    row_of_cells(ui, columns, top, bottom)
}

/// Each item of one side of a recipe as a chip saying its amount, which selects the item. A chip
/// stays with its picture where the row goes on below: it is placed as wide as it was last drawn.
fn stack_chips(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    stacks: &[(ShownItem, u32)],
    chosen: &Option<Picked>,
) {
    for (item, amount) in stacks {
        let label = format!("{} ×{amount}", item.name);
        let id = ui.id().with(("stack", &item.path));
        let wide = ui.data(|data| data.get_temp::<f32>(id)).unwrap_or_else(|| {
            let text = egui::WidgetText::from(label.as_str()).into_galley(
                ui,
                Some(egui::TextWrapMode::Extend),
                f32::INFINITY,
                egui::TextStyle::Body,
            );
            FACE + text.size().x + ROW
        });
        let placed = ui
            .allocate_ui_with_layout(
                Vec2::new(wide, ROW - 8.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    item_face(editor, ui, &item.path);
                    let picked = chosen == &Some(Picked::Item(item.path.clone()));
                    if chip::toggle(ui, &label, picked)
                        .on_hover_text(shown_path(&item.path))
                        .clicked()
                    {
                        select(editor, key, Some(Picked::Item(item.path.clone())));
                    }
                    ui.add_space(4.0);
                },
            )
            .response;
        ui.data_mut(|data| data.insert_temp(id, placed.rect.width()));
    }
}

/// The recipe being added, at the foot of the list until it has something to be made from.
fn draft_row(editor: &mut Editor, ui: &mut Ui, key: &str, len: usize) {
    let Some(makes) = editor.views.recipe_book.view(key).draft.clone() else {
        return;
    };
    let name = name_of(editor, &makes);
    let columns = Columns::across(ui);
    let top = ui.cursor().top();
    let made = cell(ui, "draft makes", columns.makes, (top, ROW), |ui| {
        item_face(editor, ui, &makes);
        ui.add(egui::Label::new(face::bold(format!("{name} ×1"))).truncate());
    });
    let from = cell(ui, "draft from", columns.from, (top, ROW), |ui| {
        let first = pick_item(
            editor,
            ui,
            key,
            ItemField::AddedByPlus,
            "/recipes/draft/from",
            PLUS,
        );
        if let Some(from) = first
            && add_recipe(editor, key, &makes, &from)
        {
            editor.views.recipe_book.view_mut(key).draft = None;
            select(editor, key, Some(Picked::Recipe(len)));
        }
        if controls::remove(ui, "Don't add this recipe").clicked() {
            editor.views.recipe_book.view_mut(key).draft = None;
        }
    });
    row_of_cells(ui, columns, top, made.max(from));
    form::hint(
        ui,
        &format!(
            "A recipe for {name} once it has something to be made from: add its first ingredient."
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
            "No recipes yet. Add what a new recipe makes above, or drag an item file in.",
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
            "Select a recipe by its handle to change it, or an item to follow it through the book.",
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
    sheet::section(ui, "recipe");
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
                let wide = (ui.available_width() - ITEM_TRAIL).max(100.0);
                let held = ItemField::Held(&item.path);
                if let Some(picked) = pick_item(editor, ui, key, held, &pointer, wide) {
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
    let wide = (ui.available_width() - ITEM_TRAIL).max(100.0);
    let pointer = format!("/recipes/{index}/{field}/{}", stacks.len());
    if let Some(item) = pick_item(editor, ui, key, ItemField::Added, &pointer, wide) {
        add_stack(editor, key, index, side, &item);
    }
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
    if ui.link(line).on_hover_text("Go to this recipe").clicked() {
        select(editor, key, Some(Picked::Recipe(index)));
    }
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
