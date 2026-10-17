pub mod condition;
pub mod effect;
pub mod event;
pub mod query;

pub use effect::RiftEffect;
pub use event::{EventKind, RiftEvent, Who};
pub use query::RiftQuery;

use std::collections::HashMap;
use std::sync::Arc;

use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{EnumString, IntoStaticStr, VariantArray};

use crate::core::audio::playback::SfxId;
use crate::core::content::{Content, ContentProblem, ContentRow, Loading, ModuleSet, row_file};
use crate::core::math::Pos;
use crate::core::script::{
    self, BoxEnding, DialogueView, Expr, Instances, LineDef, OwnerKindDef, ScriptError, Scripts,
    TextDef, Vocabulary, compile::compile_row,
};
use crate::core::tiling::Tiles;
use crate::systems::actor::model::Face;
use crate::systems::area::blueprint::BlueprintDef;
use crate::systems::area::{self, AreaDef, AreaId, AreaObject};
use crate::systems::class::{ClassDef, ClassId};
use crate::systems::dialogue::ChoiceReveal;
use crate::systems::item::{ItemDef, ItemId, ItemKind};
use crate::systems::memory::{MemoryDef, MemoryId};
use crate::systems::notification::{NotificationDef, NotificationId};
use crate::systems::npc::{Npc, NpcDef, NpcId};
use crate::systems::quest::{Parties, QuestDef, QuestId};
use crate::systems::shop::{ShopDef, ShopId};
use crate::systems::skill::{SkillDef, SkillId, SkillKind};
use crate::systems::stat::StatKind;
use crate::systems::status::{StatusEffectDef, StatusEffectId};
use crate::systems::text::{self, Span};
use crate::systems::{WorldArea, dialogue};

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
pub enum OwnerKind {
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

impl OwnerKindDef for OwnerKind {
    const ANY: OwnerKind = OwnerKind::Any;

    fn keeps_state(self) -> bool {
        !matches!(
            self,
            OwnerKind::Item
                | OwnerKind::Quest
                | OwnerKind::Shop
                | OwnerKind::Skill
                | OwnerKind::StatusEffect
        )
    }

    fn name(self) -> &'static str {
        self.into()
    }
}

pub type ScriptDef = script::ScriptDef<RiftScripting>;
pub type ScriptId = script::ScriptId<RiftScripting>;
pub type InlineScript = script::InlineScript<RiftScripting>;
pub type ScriptModule = script::ScriptModule<RiftScripting>;
pub type Condition = Expr<RiftScripting>;
pub type VarValue = script::VarValue<RiftScripting>;
pub type Variable = script::Variable<RiftScripting>;
pub type Node = script::Node<RiftScripting>;
pub type DialogueNode = script::DialogueNode<RiftScripting>;
pub type Choice = script::Choice<RiftScripting>;
pub type Procedure = script::Procedure<RiftScripting>;
pub type MayDo = script::MayDo<RiftScripting>;
pub type Fill<T> = script::Fill<T>;
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
    pub at: Pos<Tiles>,
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
    pub face: Option<Face>,
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
        face: Option<Face>,
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
    type OwnerKind = OwnerKind;
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
            "area" => RowRef::Area(content.by_key::<AreaDef>(key)?),
            "item" => RowRef::Item(content.by_key::<ItemDef>(key)?),
            "npc" => RowRef::Npc(content.by_key::<NpcDef>(key)?),
            "quest" => RowRef::Quest(content.by_key::<QuestDef>(key)?),
            "shop" => RowRef::Shop(content.by_key::<ShopDef>(key)?),
            "memory" => RowRef::Memory(content.by_key::<MemoryDef>(key)?),
            "notification" => RowRef::Notification(content.by_key::<NotificationDef>(key)?),
            "class" => RowRef::Class(content.by_key::<ClassDef>(key)?),
            "skill" => RowRef::Skill(content.by_key::<SkillDef>(key)?),
            "status_effect" => RowRef::StatusEffect(content.by_key::<StatusEffectDef>(key)?),
            _ => return None,
        })
    }

    fn audio_value(content: &Content, key: &str) -> Option<SfxId> {
        content.by_key(key)
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

pub fn owner_label(kind: OwnerKind, key: &str, object: Option<&str>) -> String {
    match object {
        Some(object) => format!("{}/{key} › {object}", kind.name()),
        None => format!("{}/{key}", kind.name()),
    }
}

pub fn row_key(content: &Content, row: RowRef) -> String {
    match row {
        RowRef::Area(id) => id.key(content).to_owned(),
        RowRef::Item(id) => id.key(content).to_owned(),
        RowRef::Npc(id) => id.key(content).to_owned(),
        RowRef::Quest(id) => id.key(content).to_owned(),
        RowRef::Shop(id) => id.key(content).to_owned(),
        RowRef::Memory(id) => id.key(content).to_owned(),
        RowRef::Notification(id) => id.key(content).to_owned(),
        RowRef::Class(id) => id.key(content).to_owned(),
        RowRef::Skill(id) => id.key(content).to_owned(),
        RowRef::StatusEffect(id) => id.key(content).to_owned(),
    }
}

pub fn owner_of(world: &World, entity: Entity) -> Option<Owner> {
    if world.get::<Npc>(entity).is_some() {
        return Some(Owner::Entity(OwnerKind::Npc, entity));
    }
    if world.get::<AreaObject>(entity).is_some() {
        return Some(Owner::Entity(OwnerKind::Object, entity));
    }
    None
}

pub trait ScriptedRow: ModuleSet {
    const OWNER: OwnerKind;

    fn script(module: &Self::Module) -> Option<&ScriptModule>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptSite {
    pub file: String,
    pub pointer: String,
}

pub fn build_all(loading: &mut Loading) {
    let content = loading.content();
    let mut all = HashMap::new();
    let mut errors: Vec<(ScriptSite, ScriptError)> = Vec::new();
    let mut bind = |row: OwnerRow, label: String, modules: Vec<(ScriptSite, &ScriptModule)>| {
        if modules.is_empty() {
            return;
        }
        let scripts: Vec<&ScriptModule> = modules.iter().map(|(_, module)| *module).collect();
        match compile_row(&content, row, &label, &scripts) {
            Ok(procedures) => {
                all.insert(row, procedures);
            }
            Err(failed) => errors.extend(
                failed
                    .into_iter()
                    .map(|(index, error)| (modules[index].0.clone(), error)),
            ),
        }
    };
    bind_table::<AreaDef>(loading, &mut bind);
    bind_table::<NpcDef>(loading, &mut bind);
    bind_table::<ItemDef>(loading, &mut bind);
    bind_table::<QuestDef>(loading, &mut bind);
    bind_table::<ShopDef>(loading, &mut bind);
    bind_table::<SkillDef>(loading, &mut bind);
    bind_table::<StatusEffectDef>(loading, &mut bind);
    for (id, _) in loading.rows::<AreaDef>(&content) {
        let key = id.key(&content);
        let placed = area::placed(&content, id);
        for (index, object) in area::objects(placed) {
            let name = object
                .name
                .as_deref()
                .or(object.label.as_deref())
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
                OwnerRow {
                    kind: OwnerKind::Object,
                    row: id.raw(),
                    object: Some(index),
                },
                owner_label(OwnerKind::Object, key, Some(name)),
                modules,
            );
        }
        for (ordinal, placement) in area::placements(&content, id).iter().enumerate() {
            let blueprint = placement.blueprint.key(&content);
            let modules = placement
                .scripts
                .iter()
                .enumerate()
                .map(|(at, module)| {
                    (
                        ScriptSite {
                            file: row_file(BlueprintDef::TABLE, blueprint),
                            pointer: format!("/modules/{at}"),
                        },
                        module,
                    )
                })
                .collect();
            bind(
                OwnerRow {
                    kind: OwnerKind::Blueprint,
                    row: id.raw(),
                    object: Some(ordinal as u32),
                },
                owner_label(
                    OwnerKind::Blueprint,
                    blueprint,
                    Some(&format!("{key} {ordinal}")),
                ),
                modules,
            );
        }
    }
    for (site, error) in errors {
        let (file, pointer) = match (&error.script, &error.node) {
            (Some(script), Some(node)) => {
                (row_file(ScriptDef::TABLE, script), format!("/nodes/{node}"))
            }
            (Some(script), None) => (row_file(ScriptDef::TABLE, script), String::new()),
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

fn bind_table<Row: ScriptedRow>(
    loading: &mut Loading,
    bind: &mut impl FnMut(OwnerRow, String, Vec<(ScriptSite, &ScriptModule)>),
) {
    let content = loading.content();
    for (id, def) in loading.rows::<Row>(&content) {
        let key = id.key(&content);
        let modules = def
            .modules()
            .iter()
            .enumerate()
            .filter_map(|(index, module)| {
                let site = ScriptSite {
                    file: row_file(Row::TABLE, key),
                    pointer: format!("/modules/{index}"),
                };
                Some((site, Row::script(module)?))
            })
            .collect();
        let row = OwnerRow {
            kind: Row::OWNER,
            row: id.raw(),
            object: None,
        };
        bind(row, owner_label(Row::OWNER, key, None), modules);
    }
}

fn owner_site(content: &Content, row: OwnerRow) -> ScriptSite {
    let (table, key) = match row.kind {
        OwnerKind::Area | OwnerKind::Object => {
            let area: AreaId = row.id();
            if let Some(index) = row.object {
                let (file, pointer) =
                    area::placed(content, area)[index as usize].site(content, area);
                return ScriptSite { file, pointer };
            }
            (AreaDef::TABLE, area.key(content))
        }
        OwnerKind::Npc => (NpcDef::TABLE, row.id::<NpcDef>().key(content)),
        OwnerKind::Item => (ItemDef::TABLE, row.id::<ItemDef>().key(content)),
        OwnerKind::Quest => (QuestDef::TABLE, row.id::<QuestDef>().key(content)),
        OwnerKind::Shop => (ShopDef::TABLE, row.id::<ShopDef>().key(content)),
        OwnerKind::Skill => (SkillDef::TABLE, row.id::<SkillDef>().key(content)),
        OwnerKind::StatusEffect => (
            StatusEffectDef::TABLE,
            row.id::<StatusEffectDef>().key(content),
        ),
        OwnerKind::Blueprint => {
            let area: AreaId = row.id();
            let placement = row
                .object
                .and_then(|ordinal| area::placements(content, area).get(ordinal as usize));
            (
                BlueprintDef::TABLE,
                placement.map_or("", |placement| placement.blueprint.key(content)),
            )
        }
        OwnerKind::Any => (ScriptDef::TABLE, ""),
    };
    ScriptSite {
        file: row_file(table, key),
        pointer: "/modules".to_owned(),
    }
}

/// The owner of the scripts of the area this world serves.
pub fn area_owner(world: &World) -> Owner {
    Owner::Row(OwnerKind::Area, world.resource::<WorldArea>().0.raw())
}

pub fn run_tick(world: &mut World) {
    script::run::tick::<RiftScripting>(world);
}

pub fn spawn_stateless(world: &mut World) {
    let scripts = world.resource::<RiftScripts>().clone();
    let area = world.resource::<WorldArea>().0;
    let mut instances = world.resource_mut::<RiftInstances>();
    for (row, procedures) in scripts.owners() {
        let owner = match row.kind {
            OwnerKind::Area if row.row == area.raw() => Owner::Row(OwnerKind::Area, row.row),
            OwnerKind::Blueprint if row.row == area.raw() => {
                Owner::Row(OwnerKind::Blueprint, row.object.unwrap_or_default())
            }
            OwnerKind::Item
            | OwnerKind::Quest
            | OwnerKind::Shop
            | OwnerKind::Skill
            | OwnerKind::StatusEffect => Owner::Row(row.kind, row.row),
            _ => continue,
        };
        instances.spawn_owner(owner, procedures.clone());
        if matches!(owner, Owner::Row(OwnerKind::Area | OwnerKind::Blueprint, _)) {
            instances.raise(owner, EventKind::Started, None, Vec::new(), |_| true);
        }
    }
}

pub fn check_all(loading: &mut Loading) {
    let content = loading.content();
    let scripts = content.built::<RiftScripts>();
    for (row, procedures) in scripts.owners() {
        let label = procedures
            .first()
            .map(|procedure| procedure.label.clone())
            .unwrap_or_default();
        let site = owner_site(&content, *row);
        let may_do = scripts.may_do(*row);
        for reach in &may_do.reaches {
            match reach {
                Reach::Warp(place) => {
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
                                format!("{} speaks in {label}", npc.key(&content)),
                                "an npc with a voice module",
                            ),
                        );
                    }
                    let placed = match row.kind {
                        OwnerKind::Npc => row.row == npc.raw(),
                        OwnerKind::Object | OwnerKind::Area | OwnerKind::Blueprint => {
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
                                format!("{} in {label}", npc.key(&content)),
                                "a line by the owner itself or a resident of its area",
                            ),
                        );
                    }
                }
                _ => {}
            }
        }
        if row.kind == OwnerKind::Npc {
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
        if row.kind == OwnerKind::Object && may_do.answers(EventKind::Interacted) {
            let area: AreaId = row.id();
            let labelled = area::objects(area::placed(&content, area))
                .any(|(index, object)| Some(index) == row.object && object.label.is_some());
            if !labelled {
                loading.json_error(
                    &site.file,
                    &site.pointer,
                    ContentProblem::new(
                        "interactive without label",
                        label.clone(),
                        "a label on every object a script of which answers interacted",
                    ),
                );
            }
        }
    }
    for (id, def) in loading.rows::<SkillDef>(&content) {
        let may_do = scripts.may_do(OwnerRow {
            kind: OwnerKind::Skill,
            row: id.raw(),
            object: None,
        });
        let (rule, kind, expected) = match def.kind {
            SkillKind::Passive if may_do.answers(EventKind::Used) => (
                "passive answers used",
                EventKind::Used,
                "activated, deactivated or interval: a passive skill is never used",
            ),
            SkillKind::Active
                if [
                    EventKind::Activated,
                    EventKind::Deactivated,
                    EventKind::Interval,
                ]
                .into_iter()
                .any(|kind| may_do.answers(kind)) =>
            {
                (
                    "active answers an aspect event",
                    EventKind::Used,
                    "used: an active skill's script runs when it is used",
                )
            }
            _ => continue,
        };
        let _ = kind;
        loading.row_error(
            id,
            "/modules",
            ContentProblem::new(rule, format!("skill/{}", id.key(&content)), expected),
        );
    }
    for (id, def) in loading.rows::<ItemDef>(&content) {
        let usable = matches!(def.kind, ItemKind::Usable);
        let answers = scripts
            .may_do(OwnerRow {
                kind: OwnerKind::Item,
                row: id.raw(),
                object: None,
            })
            .answers(EventKind::Used);
        if usable && !answers {
            loading.row_error(
                id,
                "/modules",
                ContentProblem::new(
                    "usable without used",
                    format!("item/{}", id.key(&content)),
                    "a script answering used on every usable item",
                ),
            );
        }
    }
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
                    format!("shop/{}", id.key(&content)),
                    "a script that may reach open_shop for it",
                ),
            );
        }
    }
    let parties = Parties::of(&content, scripts);
    crate::systems::quest::check_parties(loading, scripts, &parties);
    loading.built(parties);
}
