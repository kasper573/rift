pub mod panel;

use bevy_app::App;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{
    Content, ContentFile, ContentId, ContentProblem, Loading, ModuleRule, ModuleSet, Nearby,
    RowCheck, SchemaMark, serial,
};
use crate::core::script::{Arrow, Compiler, EffectDef, EffectStep, ScriptFault, Value, ValueType};
use crate::systems::attention::AttentionId;
use crate::systems::item::{Inventory, ItemId, ItemStack};
use crate::systems::player::player_requests;
use crate::systems::prompt::{self, Asked, CLOSED, PromptKind, PromptStep};
use crate::systems::script::{
    self, Condition, EffectCall, EffectReach, EventKind, FlowKey, Owner, RiftEvent, RiftInstances,
    RiftScripting, RunnerKind, ScriptModule, ScriptedRow, effect as effects,
};
use crate::systems::shop::ShopDef;
use crate::systems::text::{self, LineText, Span};
use crate::systems::{GameSettings, dialogue};

pub type RecipeBookId = ContentId<RecipeBookDef>;

pub fn register(app: &mut App) {
    use bevy_replication::{AppMessageExt, AppReplicationExt};
    app.replicate_private::<CraftingView>()
        .add_client_message::<CraftingRequest>();
}

/// Recipes kept together, for scripts to open crafting prompts from.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct RecipeBookDef {
    pub mark: AttentionId,
    /// In the order a Recipes window lists them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recipes: Vec<Recipe>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<RecipeBookModule>,
}

#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    /// What one craft gives, all of it together.
    pub makes: Vec<ItemStack>,
    /// What one craft takes. Every one is spent.
    pub from: Vec<ItemStack>,
}

/// A recipe by the book that holds it and its place among the book's recipes.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecipeKey {
    pub book: RecipeBookId,
    pub index: usize,
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RecipeBookModule {
    Script(ScriptModule),
}

impl ContentFile for RecipeBookDef {
    const TABLE: &'static str = "recipe_book";

    fn fresh(near: &Nearby) -> Result<RecipeBookDef, String> {
        Ok(RecipeBookDef {
            mark: near.file()?,
            recipes: Vec::new(),
            modules: Vec::new(),
        })
    }

    fn check(&self, check: &mut RowCheck) {
        for (index, recipe) in self.recipes.iter().enumerate() {
            let at = format!("/recipes/{index}");
            check_stacks(
                check,
                format!("{at}/makes"),
                &recipe.makes,
                ContentProblem::new("nothing made", "no items made", "at least one item made"),
                "made item listed twice",
            );
            check_stacks(
                check,
                format!("{at}/from"),
                &recipe.from,
                ContentProblem::new(
                    "nothing to make it from",
                    "no ingredients",
                    "at least one ingredient",
                ),
                "ingredient listed twice",
            );
        }
    }
}

/// One side of a recipe: at least one stack, each of one item or more, and no item twice.
fn check_stacks(
    check: &mut RowCheck,
    at: String,
    stacks: &[ItemStack],
    none: ContentProblem,
    twice: &'static str,
) {
    if stacks.is_empty() {
        check.problem(at.clone(), none);
    }
    for (nth, stack) in stacks.iter().enumerate() {
        check.at_least(format!("{at}/{nth}/count"), stack.count, 1, "items");
        if stacks[..nth]
            .iter()
            .any(|earlier| earlier.item == stack.item)
        {
            check.problem(
                format!("{at}/{nth}"),
                ContentProblem::new(
                    twice,
                    format!("entry {}", nth + 1),
                    "each item once, with its whole amount",
                ),
            );
        }
    }
}

impl ModuleSet for RecipeBookDef {
    type Module = RecipeBookModule;

    fn modules(&self) -> &[RecipeBookModule] {
        &self.modules
    }

    fn modules_mut(&mut self) -> &mut Vec<RecipeBookModule> {
        &mut self.modules
    }

    fn rule(_: &RecipeBookModule) -> ModuleRule {
        ModuleRule::AnyNumber
    }
}

impl ScriptedRow for RecipeBookDef {
    const OWNER: RunnerKind = RunnerKind::RecipeBook;

    fn script(module: &RecipeBookModule) -> Option<&ScriptModule> {
        match module {
            RecipeBookModule::Script(script) => Some(script),
        }
    }
}

impl Recipe {
    /// How many times over `offer` is this recipe: every ingredient in the same whole multiple
    /// of its amount, and nothing besides.
    pub fn times(&self, offer: &[ItemStack]) -> Option<u32> {
        let offered = |item: ItemId| {
            offer
                .iter()
                .filter(|stack| stack.item == item)
                .fold(0u32, |total, stack| total.saturating_add(stack.count))
        };
        let wanted = |item: ItemId| self.from.iter().any(|ingredient| ingredient.item == item);
        if offer.iter().any(|stack| !wanted(stack.item)) {
            return None;
        }
        let first = self.from.first()?;
        let times = offered(first.item).checked_div(first.count)?;
        let exact = self.from.iter().all(|ingredient| {
            ingredient.count.checked_mul(times) == Some(offered(ingredient.item))
        });
        (times > 0 && exact).then_some(times)
    }

    /// What `times` crafts give.
    pub fn products(&self, times: u32) -> Vec<ItemStack> {
        scaled(&self.makes, times)
    }

    /// What `times` crafts take.
    pub fn ingredients(&self, times: u32) -> Vec<ItemStack> {
        scaled(&self.from, times)
    }

    pub fn makes_item(&self, item: ItemId) -> bool {
        self.makes.iter().any(|stack| stack.item == item)
    }
}

fn scaled(stacks: &[ItemStack], times: u32) -> Vec<ItemStack> {
    stacks
        .iter()
        .map(|stack| ItemStack::new(stack.item, stack.count.saturating_mul(times)))
        .collect()
}

/// Where a crafting prompt reads the recipes it makes.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CraftSource {
    /// Every recipe of `book`.
    Book { book: RecipeBookId },
    /// Only the recipes of `book` that make one of `items`.
    Subset {
        book: RecipeBookId,
        items: Vec<ItemId>,
    },
}

impl CraftSource {
    pub fn book(&self) -> RecipeBookId {
        match self {
            CraftSource::Book { book } | CraftSource::Subset { book, .. } => *book,
        }
    }

    fn takes(&self, recipe: &Recipe) -> bool {
        match self {
            CraftSource::Book { .. } => true,
            CraftSource::Subset { items, .. } => items.iter().any(|item| recipe.makes_item(*item)),
        }
    }
}

/// A recipe a crafting prompt makes, and where its book holds it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Craftable<'a> {
    pub key: RecipeKey,
    pub recipe: &'a Recipe,
}

/// The recipes a prompt opened from `from` makes: each source's, in the order the sources and
/// their books hold them, a recipe two sources share once.
pub fn recipes<'a>(content: &'a Content, from: &[CraftSource]) -> Vec<Craftable<'a>> {
    let mut found: Vec<Craftable> = Vec::new();
    for source in from {
        let book = source.book();
        for (index, recipe) in book.get(content).recipes.iter().enumerate() {
            let key = RecipeKey { book, index };
            if source.takes(recipe) && found.iter().all(|craftable| craftable.key != key) {
                found.push(Craftable { key, recipe });
            }
        }
    }
    found
}

/// The recipes `keys` name, in their order.
pub fn craftables<'a>(content: &'a Content, keys: &[RecipeKey]) -> Vec<Craftable<'a>> {
    keys.iter()
        .filter_map(|&key| {
            let recipe = key.book.get(content).recipes.get(key.index)?;
            Some(Craftable { key, recipe })
        })
        .collect()
}

/// The recipes among `recipes` that `offer` is a whole number of times, with that number.
pub fn matches<'a>(recipes: &[Craftable<'a>], offer: &[ItemStack]) -> Vec<(Craftable<'a>, u32)> {
    recipes
        .iter()
        .filter_map(|craftable| Some((*craftable, craftable.recipe.times(offer)?)))
        .collect()
}

/// Opens a crafting prompt until it closes: an item prompt whose slots are matched against the
/// recipes of `from`, crafting what they make as often as its player likes.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("crafting"))]
pub struct OpenCrafting {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Condition>,
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text: Vec<Span>,
    pub from: Vec<CraftSource>,
    /// Whether it has a Recipes window. True unless said.
    #[schemars(extend("default" = true))]
    #[serde(default = "serial::yes", skip_serializing_if = "serial::is_true")]
    pub show_recipes: bool,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub closed: Arrow,
}

impl EffectDef<RiftScripting> for OpenCrafting {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(CLOSED => closed);
    fn pauses(&self) -> bool {
        true
    }
    fn keeps_box(&self) -> bool {
        true
    }
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        for source in &self.from {
            let CraftSource::Subset { book, items } = source else {
                continue;
            };
            let made = &book.get(cx.content).recipes;
            if let Some(item) = items
                .iter()
                .find(|item| !made.iter().any(|recipe| recipe.makes_item(**item)))
            {
                return Err(ScriptFault::new(
                    "not in the book",
                    format!(
                        "an item {} has a recipe for, got {}",
                        book.path(cx.content),
                        item.path(cx.content)
                    ),
                ));
            }
        }
        let known = recipes(cx.content, &self.from);
        let slots = known
            .iter()
            .map(|craftable| craftable.recipe.from.len() as u32)
            .max()
            .ok_or_else(|| ScriptFault::new("no recipes", "a book with a recipe to make"))?;
        let made: Vec<RecipeKey> = known.iter().map(|craftable| craftable.key).collect();
        let who = effects::aim(cx, &self.who)?;
        let values = text::values(&self.text)
            .into_iter()
            .map(|value| cx.expr(value, ValueType::Text))
            .collect::<Result<Vec<_>, _>>()?;
        let (title, spans) = (self.title.clone(), self.text.clone());
        let show_recipes = self.show_recipes;
        Ok(Box::new(move |world, flow, cx| {
            let Some(player) = effects::aimed_character(world, cx, &who) else {
                return EffectStep::Done;
            };
            let spoken: Vec<_> = values.iter().map(|value| value(world, cx)).collect();
            let asked = Asked {
                title: title.clone(),
                text: (!spans.is_empty()).then(|| LineText::spoken(&spans, &spoken)),
                slots,
            };
            if !prompt::open(world, player, flow, asked, PromptKind::Hosted) {
                return EffectStep::Done;
            }
            world.entity_mut(player).insert((
                CraftingSession { flow },
                CraftingView {
                    recipes: made.clone(),
                    show_recipes,
                    crafted: 0,
                },
            ));
            EffectStep::Pause
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<EffectReach> {
        self.from
            .iter()
            .map(|source| EffectReach::Crafts(source.book()))
            .collect()
    }
}

/// What a crafting prompt adds to the prompt its player sees: the recipes it makes, whether it
/// lists them, and how many crafts it has made.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CraftingView {
    pub recipes: Vec<RecipeKey>,
    pub show_recipes: bool,
    pub crafted: u32,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum CraftingRequest {
    /// Crafts `recipe` from `offer`: the one meant, where what is placed is several recipes.
    Craft {
        step: PromptStep,
        offer: Vec<ItemStack>,
        recipe: RecipeKey,
    },
}

#[derive(Component)]
pub struct CraftingSession {
    flow: FlowKey,
}

pub fn requests(world: &mut World) {
    player_requests::<CraftingRequest>(world, false, |world, player, request| {
        let CraftingRequest::Craft {
            step,
            offer,
            recipe,
        } = request;
        let Some(session) = world.get::<CraftingSession>(player) else {
            return Ok(());
        };
        let flow = session.flow;
        match prompt::submitted(world, player, step, &offer) {
            Ok(Some(asked)) if asked == flow => {}
            Ok(_) => return Ok(()),
            Err(reason) => {
                prompt::ask_again(world, player);
                return Err(reason);
            }
        }
        let crafted = craft(world, player, &offer, recipe);
        prompt::ask_again(world, player);
        let made = crafted?;
        if let Some(mut view) = world.get_mut::<CraftingView>(player) {
            view.crafted = view.crafted.wrapping_add(1);
        }
        for made in made {
            world.resource_mut::<RiftInstances>().raise(
                Owner::Row(RunnerKind::RecipeBook, recipe.book.raw()),
                EventKind::Crafted,
                Some(player),
                vec![
                    Value::Int(i64::from(made.item.raw())),
                    Value::Int(i64::from(made.count)),
                ],
                move |event| match event {
                    RiftEvent::Crafted { item } => item.is_none_or(|item| item == made.item),
                    _ => false,
                },
            );
        }
        Ok(())
    });
}

/// Ends the crafting of whoever no longer has the prompt it was opened in.
pub fn hold(world: &mut World, sessions: &mut QueryState<(Entity, &'static CraftingSession)>) {
    let over: Vec<Entity> = sessions
        .iter(world)
        .filter(|(player, session)| {
            dialogue::flow_of(world, *player) != Some(session.flow)
                || !prompt::hosted(world, *player)
        })
        .map(|(player, _)| player)
        .collect();
    for player in over {
        world
            .entity_mut(player)
            .remove::<(CraftingSession, CraftingView)>();
    }
}

pub(crate) fn marks(
    world: &World,
    target: Entity,
    viewers: &[Entity],
    out: &mut dyn FnMut(Entity, AttentionId),
) {
    let Some(owner) = script::owner_of(world, target) else {
        return;
    };
    let Some(may_do) = world.resource::<RiftInstances>().may_do(owner) else {
        return;
    };
    let content = world.resource::<Content>();
    for reach in &may_do.reaches {
        if let EffectReach::Crafts(book) = reach {
            let mark = book.get(content).mark;
            for &player in viewers {
                out(player, mark);
            }
        }
    }
}

/// What a book's recipes must hold between them, and what none may do at the shops: lead back to
/// what it makes, make the same things twice from the same ingredients, or turn what the shops
/// sell into what they pay more for.
pub fn check_all(loading: &mut Loading) {
    let content = loading.content();
    let shops: Vec<&ShopDef> = content.table::<ShopDef>().rows().collect();
    for (id, book) in loading.rows::<RecipeBookDef>(&content) {
        for (index, recipe) in book.recipes.iter().enumerate() {
            let at = format!("/recipes/{index}");
            let twin = book.recipes[..index]
                .iter()
                .any(|earlier| same_recipe(earlier, recipe));
            if twin {
                loading.row_error(
                    id,
                    &at,
                    ContentProblem::new(
                        "made twice",
                        named(&content, &recipe.makes),
                        "one recipe making the same items from the same ingredients",
                    ),
                );
            }
            if let Some(profit) = profit(&content, &shops, recipe) {
                loading.row_error(
                    id,
                    &at,
                    ContentProblem::new(
                        "crafting mints money",
                        profit,
                        "ingredients that cost at the shops at least what the shops pay for \
                         what they make",
                    ),
                );
            }
        }
        let mut spelled: Vec<Vec<ItemId>> = Vec::new();
        for (index, recipe) in book.recipes.iter().enumerate().rev() {
            let Some(chain) = loop_from(book, recipe) else {
                continue;
            };
            let mut items = chain.clone();
            items.sort_unstable();
            items.dedup();
            if spelled.contains(&items) {
                continue;
            }
            spelled.push(items);
            let named: Vec<&str> = chain.iter().map(|item| item.path(&content)).collect();
            loading.row_error(
                id,
                &format!("/recipes/{index}"),
                ContentProblem::new(
                    "recipe loop",
                    named.join(" → "),
                    "recipes that never lead back to what they make",
                ),
            );
        }
    }
}

fn same_recipe(one: &Recipe, other: &Recipe) -> bool {
    same_items(&one.makes, &other.makes) && same_items(&one.from, &other.from)
}

fn same_items(one: &[ItemStack], other: &[ItemStack]) -> bool {
    one.len() == other.len()
        && one
            .iter()
            .all(|stack| other.iter().any(|twin| twin.item == stack.item))
}

fn named(content: &Content, stacks: &[ItemStack]) -> String {
    let paths: Vec<&str> = stacks
        .iter()
        .map(|stack| stack.item.path(content))
        .collect();
    paths.join(" + ")
}

/// The items leading from something `recipe` makes back to it through the recipes of `book`,
/// when its ingredients do.
fn loop_from(book: &RecipeBookDef, recipe: &Recipe) -> Option<Vec<ItemId>> {
    let mut trails: Vec<Vec<ItemId>> = recipe.from.iter().map(|stack| vec![stack.item]).collect();
    let mut seen: Vec<ItemId> = Vec::new();
    while let Some(trail) = trails.pop() {
        let at = *trail.last()?;
        if recipe.makes_item(at) {
            return Some(std::iter::once(at).chain(trail).collect());
        }
        if seen.contains(&at) {
            continue;
        }
        seen.push(at);
        for made in book.recipes.iter().filter(|made| made.makes_item(at)) {
            for stack in &made.from {
                let mut longer = trail.clone();
                longer.push(stack.item);
                trails.push(longer);
            }
        }
    }
    None
}

/// How buying `recipe`'s ingredients at the shops and selling what they make pays, when it does.
fn profit(content: &Content, shops: &[&ShopDef], recipe: &Recipe) -> Option<String> {
    let mut spent: Vec<(ItemId, f64)> = Vec::new();
    for ingredient in &recipe.from {
        let price = shops
            .iter()
            .flat_map(|shop| shop.sells())
            .filter(|offer| offer.item == ingredient.item && offer.count > 0)
            .map(|offer| {
                let share = f64::from(ingredient.count) / f64::from(offer.count);
                offer
                    .price
                    .iter()
                    .map(|stack| (stack.item, f64::from(stack.count) * share))
                    .collect::<Vec<_>>()
            })
            .min_by(|one, other| total(one).total_cmp(&total(other)))?;
        for (currency, amount) in price {
            add(&mut spent, currency, amount);
        }
    }
    shops.iter().find_map(|shop| {
        let mut earned: Vec<(ItemId, f64)> = Vec::new();
        for made in &recipe.makes {
            let Ok(pays) = shop.pays_for(content, made.item, &[]) else {
                continue;
            };
            for stack in pays {
                let amount = f64::from(stack.count) * f64::from(made.count);
                add(&mut earned, stack.item, amount);
            }
        }
        if earned.is_empty() {
            return None;
        }
        let of = |amounts: &[(ItemId, f64)], item: ItemId| {
            amounts
                .iter()
                .find(|(currency, _)| *currency == item)
                .map_or(0.0, |(_, amount)| *amount)
        };
        let currencies = spent.iter().chain(&earned).map(|(item, _)| *item);
        let never_short = currencies
            .clone()
            .all(|item| of(&earned, item) >= of(&spent, item));
        let ahead = currencies
            .clone()
            .any(|item| of(&earned, item) > of(&spent, item));
        (never_short && ahead).then(|| {
            format!(
                "{} sold for {} from ingredients that cost {}",
                named(content, &recipe.makes),
                amounts_text(content, &earned),
                amounts_text(content, &spent)
            )
        })
    })
}

fn total(amounts: &[(ItemId, f64)]) -> f64 {
    amounts.iter().map(|(_, amount)| amount).sum()
}

fn add(amounts: &mut Vec<(ItemId, f64)>, item: ItemId, amount: f64) {
    match amounts.iter_mut().find(|(held, _)| *held == item) {
        Some((_, held)) => *held += amount,
        None => amounts.push((item, amount)),
    }
}

fn amounts_text(content: &Content, amounts: &[(ItemId, f64)]) -> String {
    let parts: Vec<String> = amounts
        .iter()
        .map(|(item, amount)| format!("{amount} {}", item.path(content)))
        .collect();
    if parts.is_empty() {
        "nothing".to_owned()
    } else {
        parts.join(" + ")
    }
}

/// Crafts `recipe` from `offer` for `player` in one exchange, when their prompt makes it and the
/// offer is a whole number of times it: everything that was made.
fn craft(
    world: &mut World,
    player: Entity,
    offer: &[ItemStack],
    recipe: RecipeKey,
) -> Result<Vec<ItemStack>, String> {
    let content = world.resource::<Content>().clone();
    let no_recipe = || content.built::<GameSettings>().words.no_recipe.clone();
    let view = world.get::<CraftingView>(player).ok_or_else(no_recipe)?;
    let known = craftables(&content, &view.recipes);
    let (craftable, times) = matches(&known, offer)
        .into_iter()
        .find(|(craftable, _)| craftable.key == recipe)
        .ok_or_else(no_recipe)?;
    let made = craftable.recipe.products(times);
    world
        .get_mut::<Inventory>(player)
        .ok_or_else(no_recipe)?
        .exchange(&content, offer, &made)
        .map_err(|refusal| refusal.describe(&content))?;
    Ok(made)
}
