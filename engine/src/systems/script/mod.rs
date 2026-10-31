pub mod condition;
pub mod effect;
pub mod event;
pub mod query;

pub use crate::core::script::Fill;
pub use effect::{RiftEffect, RiftEffectKind};
pub use event::{EventKind, RiftEvent, Who};
pub use query::{RiftQuery, RiftQueryKind};

use std::collections::HashMap;
use std::sync::Arc;

use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{EnumString, IntoStaticStr, VariantArray};

use crate::core::audio::playback::SfxId;
use crate::core::content::{
    Content, ContentFile, ContentId, ContentProblem, Loading, ModuleSet, fields_file,
};
use crate::core::math::Pos;
use crate::core::script::{
    self, BoxEnding, DialogueView, Expr, Instances, LineDef, RunnerKindDef, ScriptError, Scripts,
    TextDef, Vocabulary, compile::compile_row,
};
use crate::core::units::Meters;
use crate::systems::area::scene::scene_file;
use crate::systems::area::{self, AreaDef, AreaId, AreaModule, AreaObject, Cell};
use crate::systems::class::{ClassDef, ClassId};
use crate::systems::dialogue::bust::Expression;
use crate::systems::dialogue::{self, ChoiceReveal};
use crate::systems::item::{ItemDef, ItemId, ItemKind};
use crate::systems::memory::{MemoryDef, MemoryId};
use crate::systems::notification::{NotificationDef, NotificationId};
use crate::systems::npc::{Npc, NpcDef, NpcId};
use crate::systems::quest::{Parties, QuestDef, QuestId};
use crate::systems::shop::{ShopDef, ShopId};
use crate::systems::skill::{SkillDef, SkillId};
use crate::systems::stat::StatKind;
use crate::systems::status::{StatusEffectDef, StatusEffectId};
use crate::systems::text::{self, Span};
use crate::systems::{WorldArea, WorldCell};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RiftScripting;

#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    EnumString,
    IntoStaticStr,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum RunnerKind {
    Object,
    Npc,
    Item,
    Area,
    Quest,
    Shop,
    Skill,
    StatusEffect,
    Blueprint,
    Any,
}

impl RunnerKindDef for RunnerKind {
    const ANY: RunnerKind = RunnerKind::Any;

    fn keeps_state(self) -> bool {
        !matches!(
            self,
            RunnerKind::Item
                | RunnerKind::Quest
                | RunnerKind::Shop
                | RunnerKind::Skill
                | RunnerKind::StatusEffect
        )
    }

    fn name(self) -> &'static str {
        self.into()
    }
}

pub type ScriptDef = script::ScriptDef<RiftScripting>;
pub type ScriptId = script::ScriptId<RiftScripting>;
pub type ScriptModule = script::ScriptModule<RiftScripting>;
pub type ListedScriptModule = script::ListedScriptModule<RiftScripting>;
pub type Condition = Expr<RiftScripting>;
pub type VarValue = script::VarValue<RiftScripting>;
pub type Variable = script::Variable<RiftScripting>;
pub type Node = script::Node<RiftScripting>;
pub type DialogueNode = script::DialogueNode<RiftScripting>;
pub type Choice = script::Choice<RiftScripting>;
pub type Procedure = script::Procedure<RiftScripting>;
pub type MayDo = script::MayDo<RiftScripting>;
pub type Owner = script::Owner<RiftScripting>;
pub type OwnerRow = script::OwnerRow<RiftScripting>;
pub type FlowKey = script::FlowKey<RiftScripting>;
pub type FlowTrace = script::FlowTrace<RiftScripting>;
pub type TraceStep = script::TraceStep<RiftScripting>;
pub type EffectCall = script::EffectCall<RiftScripting>;

#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(from = "PlaceRow", into = "PlaceRow")]
#[schemars(inline)]
pub struct Place {
    pub area: AreaId,
    pub at: Pos<Meters>,
}

#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(inline, extend("x-place" = true))]
struct PlaceRow {
    area: AreaId,
    x: f32,
    y: f32,
}

impl From<PlaceRow> for Place {
    fn from(row: PlaceRow) -> Place {
        Place {
            area: row.area,
            at: Pos::new(row.x, row.y),
        }
    }
}

impl From<Place> for PlaceRow {
    fn from(place: Place) -> PlaceRow {
        PlaceRow {
            area: place.area,
            x: place.at.x,
            y: place.at.y,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowRef {
    Area(AreaId),
    Item(ItemId),
    Npc(NpcId),
    Quest(QuestId),
    Shop(ShopId),
    Memory(MemoryId),
    Notification(NotificationId),
    Class(ClassId),
    Skill(SkillId),
    StatusEffect(StatusEffectId),
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(from = "LineRow", into = "LineRow")]
pub struct Line {
    pub by: Speaker,
    pub face: Option<Expression>,
    pub text: Vec<Span>,
}

#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LineRow {
    by: SpeakerRow,
    text: Vec<Span>,
}

#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SpeakerRow {
    Npc {
        npc: NpcId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        face: Option<Expression>,
    },
    Object,
    Narrator,
    Player,
}

impl From<LineRow> for Line {
    fn from(row: LineRow) -> Line {
        let (by, face) = match row.by {
            SpeakerRow::Npc { npc, face } => (Speaker::Npc(npc), face),
            SpeakerRow::Object => (Speaker::Object, None),
            SpeakerRow::Narrator => (Speaker::Narrator, None),
            SpeakerRow::Player => (Speaker::Player, None),
        };
        Line {
            by,
            face,
            text: row.text,
        }
    }
}

impl From<Line> for LineRow {
    fn from(line: Line) -> LineRow {
        LineRow {
            by: match line.by {
                Speaker::Npc(npc) => SpeakerRow::Npc {
                    npc,
                    face: line.face,
                },
                Speaker::Object => SpeakerRow::Object,
                Speaker::Narrator => SpeakerRow::Narrator,
                Speaker::Player => SpeakerRow::Player,
            },
            text: line.text,
        }
    }
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Speaker {
    Npc(NpcId),
    Object,
    Narrator,
    Player,
}

impl LineDef<RiftScripting> for Line {
    fn values(&self) -> Vec<&Expr<RiftScripting>> {
        self.text.values()
    }
}

impl TextDef<RiftScripting> for Vec<Span> {
    fn values(&self) -> Vec<&Expr<RiftScripting>> {
        text::values(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reach {
    Warp(Place),
    Takes(ItemId),
    Gives(ItemId),
    Drops(ItemId),
    Offers(QuestId),
    Accepts(QuestId),
    TurnsIn(QuestId),
    Fails(QuestId),
    Completes(QuestId),
    Decides(QuestId),
    Remembers(MemoryId),
    OpensShop(ShopId),
    Spawns(NpcId),
    Hostile,
    Friendly,
    Despawns,
    Targets,
    Walks,
    Damages,
    Heals,
    Applies(StatusEffectId),
    Cleanses,
    Boosts(StatKind, Option<f32>),
    GrantsXp,
    Casts(SkillId),
    ResetsSkills,
    Animates,
    ShowsVfx,
    Notifies(NotificationId),
    Says,
    Plays(SfxId),
    Speaks(NpcId),
}

impl Vocabulary for RiftScripting {
    type RunnerKind = RunnerKind;
    type Event = RiftEvent;
    type Query = RiftQuery;
    type Reveal = ChoiceReveal;
    type Effect = RiftEffect;
    type Line = Line;
    type Text = Vec<Span>;
    type Place = Place;
    type Row = RowRef;
    type Audio = SfxId;
    type Reach = Reach;

    fn row_value(content: &Content, table: &str, key: &str) -> Option<RowRef> {
        Some(match table {
            AreaDef::TABLE => RowRef::Area(content.by_path(key)?),
            ItemDef::TABLE => RowRef::Item(content.by_path(key)?),
            NpcDef::TABLE => RowRef::Npc(content.by_path(key)?),
            QuestDef::TABLE => RowRef::Quest(content.by_path(key)?),
            ShopDef::TABLE => RowRef::Shop(content.by_path(key)?),
            MemoryDef::TABLE => RowRef::Memory(content.by_path(key)?),
            NotificationDef::TABLE => RowRef::Notification(content.by_path(key)?),
            ClassDef::TABLE => RowRef::Class(content.by_path(key)?),
            SkillDef::TABLE => RowRef::Skill(content.by_path(key)?),
            StatusEffectDef::TABLE => RowRef::StatusEffect(content.by_path(key)?),
            _ => return None,
        })
    }

    fn audio_value(content: &Content, key: &str) -> Option<SfxId> {
        content.by_path(key)
    }

    fn show_dialogue(world: &mut World, flow: FlowKey, subject: Entity, view: DialogueView<Self>) {
        dialogue::show(world, flow, subject, view);
    }

    fn close_dialogue(world: &mut World, flow: FlowKey, subject: Entity, ending: BoxEnding) {
        dialogue::close(world, flow, subject, ending);
    }

    fn log_dropped_flow(_world: &World, owner: &Owner, why: &str) {
        bevy::log::warn!("{owner:?}: flow dropped: {why}");
    }
}

pub type RiftInstances = Instances<RiftScripting>;
pub type RiftScripts = Scripts<RiftScripting>;

pub fn owner_label(path: &str, object: Option<&str>) -> String {
    match object {
        Some(object) => format!("{path} › {object}"),
        None => path.to_owned(),
    }
}

pub fn owner_of(world: &World, entity: Entity) -> Option<Owner> {
    if world.get::<Npc>(entity).is_some() {
        return Some(Owner::Entity(RunnerKind::Npc, entity));
    }
    if world.get::<AreaObject>(entity).is_some() {
        return Some(Owner::Entity(RunnerKind::Object, entity));
    }
    None
}

pub trait ScriptedRow: ModuleSet {
    const OWNER: RunnerKind;

    fn script(module: &Self::Module) -> Option<&ScriptModule>;
}

impl OwnerRow {
    pub fn of_row<Row: ScriptedRow>(id: ContentId<Row>) -> OwnerRow {
        OwnerRow {
            kind: Row::OWNER,
            row: id.raw(),
            object: None,
        }
    }

    pub fn of_area(area: AreaId) -> OwnerRow {
        OwnerRow {
            kind: RunnerKind::Area,
            row: area.raw(),
            object: None,
        }
    }

    pub fn of_object(area: AreaId, index: u32) -> OwnerRow {
        OwnerRow {
            kind: RunnerKind::Object,
            row: area.raw(),
            object: Some(index),
        }
    }

    pub fn of_placement(area: AreaId, ordinal: u32) -> OwnerRow {
        OwnerRow {
            kind: RunnerKind::Blueprint,
            row: area.raw(),
            object: Some(ordinal),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptSite {
    pub file: String,
    pub pointer: String,
}

pub async fn build_all(loading: &mut Loading) {
    let content = loading.content();
    let mut all = HashMap::new();
    let mut errors: Vec<(ScriptSite, ScriptError)> = Vec::new();
    let mut bind = |row: OwnerRow, label: String, modules: Vec<(ScriptSite, &ScriptModule)>| {
        if modules.is_empty() {
            return true;
        }
        let scripts: Vec<&ScriptModule> = modules.iter().map(|(_, module)| *module).collect();
        match compile_row(&content, row, &label, &scripts) {
            Ok(procedures) => {
                all.insert(row, procedures);
                true
            }
            Err(failed) => {
                errors.extend(
                    failed
                        .into_iter()
                        .map(|failed| (modules[failed.module].0.clone(), failed.error)),
                );
                false
            }
        }
    };
    bind_table::<NpcDef>(loading, &mut bind);
    loading.breathe().await;
    bind_table::<ItemDef>(loading, &mut bind);
    loading.breathe().await;
    bind_table::<QuestDef>(loading, &mut bind);
    loading.breathe().await;
    bind_table::<ShopDef>(loading, &mut bind);
    loading.breathe().await;
    bind_table::<SkillDef>(loading, &mut bind);
    loading.breathe().await;
    bind_table::<StatusEffectDef>(loading, &mut bind);
    for (id, _) in loading.rows::<AreaDef>(&content) {
        loading.breathe().await;
        let key = id.path(&content);
        let placed = area::placed(&content, id);
        let scene = scene_file(key);
        let own = placed
            .iter()
            .filter_map(|placed| match &placed.module {
                AreaModule::Script(script) => Some((
                    ScriptSite {
                        file: scene.clone(),
                        pointer: placed.origin.clone(),
                    },
                    script,
                )),
                _ => None,
            })
            .collect();
        bind(OwnerRow::of_area(id), owner_label(key, None), own);
        for (index, object) in area::objects(placed) {
            let name = object
                .name
                .as_deref()
                .or(object.title.as_deref())
                .unwrap_or("object");
            let (file, pointer) = placed[index as usize].site(&content, id);
            let modules = object
                .scripts
                .iter()
                .enumerate()
                .map(|(at, module)| {
                    (
                        ScriptSite {
                            file: file.clone(),
                            pointer: format!("{pointer}/scripts/{at}"),
                        },
                        module,
                    )
                })
                .collect();
            bind(
                OwnerRow::of_object(id, index),
                owner_label(key, Some(name)),
                modules,
            );
        }
        for (ordinal, placement) in area::placements(&content, id).iter().enumerate() {
            let blueprint = placement.blueprint.path(&content);
            let modules = placement
                .scripts
                .iter()
                .map(|(pointer, module)| {
                    (
                        ScriptSite {
                            file: scene_file(blueprint),
                            pointer: pointer.clone(),
                        },
                        module,
                    )
                })
                .collect();
            bind(
                OwnerRow::of_placement(id, ordinal as u32),
                owner_label(blueprint, Some(&format!("{key} {ordinal}"))),
                modules,
            );
        }
    }
    for (site, error) in errors {
        let (file, pointer) = match (&error.script, &error.node) {
            (Some(script), Some(node)) => (fields_file(script), format!("/nodes/{node}")),
            (Some(script), None) => (fields_file(script), String::new()),
            (None, Some(node)) => (site.file.clone(), format!("{}/nodes/{node}", site.pointer)),
            (None, None) => (site.file.clone(), site.pointer.clone()),
        };
        let found = match &error.node {
            Some(node) => format!("{node} in {}", error.owner),
            None => error.owner.clone(),
        };
        loading.json_error(
            file,
            &pointer,
            ContentProblem::new(error.rule, found, error.expected),
        );
    }
    loading.built(Scripts(Arc::new(all)));
}

/// Binds each row's scripts. A row whose scripts don't compile has failed, so the rules about what
/// its scripts do leave it to its own errors.
fn bind_table<Row: ScriptedRow>(
    loading: &mut Loading,
    bind: &mut impl FnMut(OwnerRow, String, Vec<(ScriptSite, &ScriptModule)>) -> bool,
) {
    let content = loading.content();
    for (id, def) in loading.rows::<Row>(&content) {
        let key = id.path(&content);
        let modules = def
            .modules()
            .iter()
            .enumerate()
            .filter_map(|(index, module)| {
                let site = ScriptSite {
                    file: fields_file(key),
                    pointer: format!("/modules/{index}"),
                };
                Some((site, Row::script(module)?))
            })
            .collect();
        if !bind(OwnerRow::of_row(id), owner_label(key, None), modules) {
            loading.fail(id);
        }
    }
}

pub fn owner_site(content: &Content, row: OwnerRow) -> ScriptSite {
    let path = match row.kind {
        RunnerKind::Area | RunnerKind::Object => {
            let area: AreaId = row.id();
            if let Some(index) = row.object {
                let (file, pointer) =
                    area::placed(content, area)[index as usize].site(content, area);
                return ScriptSite { file, pointer };
            }
            return ScriptSite {
                file: scene_file(area.path(content)),
                pointer: String::new(),
            };
        }
        RunnerKind::Blueprint => {
            let area: AreaId = row.id();
            let placement = row
                .object
                .and_then(|ordinal| area::placements(content, area).get(ordinal as usize));
            return ScriptSite {
                file: placement.map_or_else(String::new, |placement| {
                    scene_file(placement.blueprint.path(content))
                }),
                pointer: String::new(),
            };
        }
        RunnerKind::Npc => row.id::<NpcDef>().path(content),
        RunnerKind::Item => row.id::<ItemDef>().path(content),
        RunnerKind::Quest => row.id::<QuestDef>().path(content),
        RunnerKind::Shop => row.id::<ShopDef>().path(content),
        RunnerKind::Skill => row.id::<SkillDef>().path(content),
        RunnerKind::StatusEffect => row.id::<StatusEffectDef>().path(content),
        RunnerKind::Any => "",
    };
    ScriptSite {
        file: fields_file(path),
        pointer: "/modules".to_owned(),
    }
}

/// The owner of the scripts of the area this world serves.
pub fn area_owner(world: &World) -> Owner {
    Owner::Row(RunnerKind::Area, world.resource::<WorldArea>().0.raw())
}

pub fn run_tick(world: &mut World) {
    script::run::tick::<RiftScripting>(world);
}

pub fn spawn_stateless(world: &mut World) {
    let scripts = world.resource::<RiftScripts>().clone();
    let content = world.resource::<Content>().clone();
    let area = world.resource::<WorldArea>().0;
    let mine = world.resource::<WorldCell>().0;
    let cells = area::cells(&content, area);
    let mut instances = world.resource_mut::<RiftInstances>();
    for (row, procedures) in scripts.owners() {
        let owner = match row.kind {
            // The area's own scripts run in its first cell.
            RunnerKind::Area if row.row == area.raw() && mine == Cell(0) => {
                Owner::Row(RunnerKind::Area, row.row)
            }
            RunnerKind::Blueprint if row.row == area.raw() => {
                let ordinal = row
                    .object
                    .expect("a placement's owner row names its ordinal");
                let placed = cells.at(area::placements(&content, area)[ordinal as usize].at);
                if placed != mine {
                    continue;
                }
                Owner::Row(RunnerKind::Blueprint, ordinal)
            }
            RunnerKind::Item
            | RunnerKind::Quest
            | RunnerKind::Shop
            | RunnerKind::Skill
            | RunnerKind::StatusEffect => Owner::Row(row.kind, row.row),
            _ => continue,
        };
        instances.spawn_owner(owner, procedures.clone());
        if matches!(
            owner,
            Owner::Row(RunnerKind::Area | RunnerKind::Blueprint, _)
        ) {
            instances.raise(owner, EventKind::Started, None, Vec::new(), |_| true);
        }
    }
}

pub async fn check_all(loading: &mut Loading) {
    let content = loading.content();
    let scripts = content.built::<RiftScripts>();
    let thorough = loading.thorough();
    for (row, procedures) in scripts.owners() {
        let label = procedures
            .first()
            .map(|procedure| procedure.label.clone())
            .unwrap_or_default();
        let site = owner_site(&content, *row);
        let may_do = scripts.may_do(*row);
        for reach in &may_do.reaches {
            match reach {
                Reach::Warp(place) if thorough => {
                    if let Err(problem) = area::check_place(&content, *place) {
                        loading.json_error(
                            &site.file,
                            &site.pointer,
                            ContentProblem::new(
                                problem.rule,
                                format!("{label} warps to {}", problem.found),
                                problem.expected,
                            )
                            .near(problem.nearest),
                        );
                    }
                }
                Reach::Speaks(npc) => {
                    if content.table::<NpcDef>().get(*npc).is_none() {
                        continue;
                    }
                    let def = npc.get(&content);
                    if def.voice().is_none() {
                        loading.json_error(
                            &site.file,
                            &site.pointer,
                            ContentProblem::new(
                                "needs a voice",
                                format!("{} speaks in {label}", npc.path(&content)),
                                "an npc with a voice module",
                            ),
                        );
                    }
                    let placed = match row.kind {
                        RunnerKind::Npc => row.row == npc.raw(),
                        RunnerKind::Object | RunnerKind::Area | RunnerKind::Blueprint => {
                            let area: AreaId = row.id();
                            area::residents(area::placed(&content, area))
                                .any(|resident| resident == *npc)
                        }
                        _ => false,
                    };
                    if !placed {
                        loading.json_error(
                            &site.file,
                            &site.pointer,
                            ContentProblem::new(
                                "speaker not here",
                                format!("{} in {label}", npc.path(&content)),
                                "a line by the owner itself or a resident of its area",
                            ),
                        );
                    }
                }
                _ => {}
            }
        }
        if row.kind == RunnerKind::Npc {
            let npc: NpcId = row.id();
            let watched = procedures
                .iter()
                .flat_map(|procedure| procedure.entry(EventKind::StoodOn));
            for (_, event) in watched {
                let RiftEvent::StoodOn { object, .. } = event else {
                    continue;
                };
                let placed = content.ids::<AreaDef>().any(|area| {
                    let modules = area::placed(&content, area);
                    area::npcs(modules).any(|placed| placed == npc)
                        && area::objects(modules)
                            .any(|(_, placed)| placed.name.as_deref() == Some(object))
                });
                if !placed {
                    loading.json_error(
                        &site.file,
                        &site.pointer,
                        ContentProblem::new(
                            "watched object not here",
                            format!("{label} watches {object}"),
                            "an object of that name in an area the npc is placed in",
                        ),
                    );
                }
            }
        }
        if row.kind == RunnerKind::Object && may_do.answers(EventKind::Interacted) {
            let area: AreaId = row.id();
            let titled = area::objects(area::placed(&content, area))
                .any(|(index, object)| Some(index) == row.object && object.title.is_some());
            if !titled {
                loading.json_error(
                    &site.file,
                    &site.pointer,
                    ContentProblem::new(
                        "interactive without title",
                        label.clone(),
                        "a title on every object a script of which answers interacted",
                    ),
                );
            }
        }
    }
    for (id, def) in loading.rows::<ItemDef>(&content) {
        let usable = matches!(def.kind, ItemKind::Usable);
        let answers = scripts
            .may_do(OwnerRow::of_row(id))
            .answers(EventKind::Used);
        if usable && !answers {
            loading.row_error(
                id,
                "/modules",
                ContentProblem::new(
                    "usable without used",
                    format!("item/{}", id.path(&content)),
                    "a script answering used on every usable item",
                ),
            );
        }
    }
    loading.built(Parties::of(&content, scripts));
}

/// What a whole game must do with its quests and shops: offer and take back every quest, from an
/// owner placed somewhere, and open every shop.
pub async fn check_reached(loading: &mut Loading) {
    let content = loading.content();
    let scripts = content.built::<RiftScripts>();
    for (id, _) in loading.rows::<ShopDef>(&content) {
        let opened = scripts.owners().any(|(row, _)| {
            scripts
                .may_do(*row)
                .reaches
                .iter()
                .any(|reach| matches!(reach, Reach::OpensShop(shop) if *shop == id))
        });
        if !opened {
            loading.row_error(
                id,
                "",
                ContentProblem::new(
                    "never opened",
                    format!("shop/{}", id.path(&content)),
                    "a script that may reach open_shop for it",
                ),
            );
        }
    }
    crate::systems::quest::check_parties(loading, scripts, content.built::<Parties>());
}
