use std::collections::HashMap;
use std::sync::Arc;

use super::compile::{OwnerRow, Procedure, Scripts, compile_nodes, compile_row};
use super::vocabulary::Vocabulary;
use super::{Instances, Owner, RunnerKind, ScriptError, ScriptModule};
use crate::core::content::{
    Content, ContentId, ContentProblem, ContentRegistry, LoadStep, Loading, ModuleSet, fields_file,
};
use bevy_ecs::prelude::World;

/// A row whose files hold script modules among its own modules.
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
}

/// Where a script is written: the file and the place in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptSite {
    pub file: String,
    pub pointer: String,
}

/// How a feature compiles the scripts of the rows it owns as the game loads.
pub type Binder = fn(&mut Loading, &Content, &mut Bind);

/// The scripts compiled so far as the game loads, and what refused to compile.
#[derive(Default)]
pub struct Bind {
    all: HashMap<OwnerRow, Vec<Arc<Procedure>>>,
    errors: Vec<(ScriptSite, ScriptError)>,
}

impl Bind {
    /// Compiles the modules of `row`. Tells whether they all compiled: a row whose scripts
    /// don't has failed, so the rules about what its scripts do leave it to its own errors.
    pub fn bind(
        &mut self,
        content: &Content,
        row: OwnerRow,
        label: String,
        modules: Vec<(ScriptSite, &ScriptModule)>,
    ) -> bool {
        if modules.is_empty() {
            return true;
        }
        let scripts: Vec<&ScriptModule> = modules.iter().map(|(_, module)| *module).collect();
        match compile_row(content, row, &label, &scripts) {
            Ok(procedures) => {
                self.all.insert(row, procedures);
                true
            }
            Err(failed) => {
                self.errors.extend(
                    failed
                        .into_iter()
                        .map(|failed| (modules[failed.module].0.clone(), failed.error)),
                );
                false
            }
        }
    }

    /// Compiles `nodes` the engine wrote for `row` itself, which a file at `site` stands for in
    /// errors.
    pub fn bind_nodes(
        &mut self,
        content: &Content,
        row: OwnerRow,
        label: &str,
        nodes: &std::collections::BTreeMap<String, super::Node>,
        site: ScriptSite,
    ) {
        match compile_nodes(content, row, label, nodes) {
            Ok(procedure) => {
                self.all.insert(row, vec![procedure]);
            }
            Err(error) => self.errors.push((site, error)),
        }
    }

    /// The scripts compiled for `row` so far.
    pub fn of(&self, row: OwnerRow) -> &[Arc<Procedure>] {
        self.all.get(&row).map_or(&[][..], Vec::as_slice)
    }

    /// Binds the scripts of every row of `Row`.
    pub fn bind_table<Row: ScriptedRow>(&mut self, loading: &mut Loading, content: &Content) {
        for (id, def) in loading.rows::<Row>(content) {
            let key = id.path(content);
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
            if !self.bind(
                content,
                OwnerRow::of_row(id),
                owner_label(key, None),
                modules,
            ) {
                loading.fail(id);
            }
        }
    }
}

/// What a script's owner is called in errors: the file, and the object within it when there is one.
pub fn owner_label(path: &str, object: Option<&str>) -> String {
    match object {
        Some(object) => format!("{path} › {object}"),
        None => path.to_owned(),
    }
}

/// Where the scripts of `row` are written.
pub fn owner_site(content: &Content, row: OwnerRow) -> ScriptSite {
    (Vocabulary::get().spec(row.kind).site)(content, row)
}

/// What the file the scripts of `row` run in is called, where it has a name of its own.
pub fn owner_name(content: &Content, row: OwnerRow) -> Option<String> {
    (Vocabulary::get().spec(row.kind).name)(content, row)
}

pub(super) fn any_site(_: &Content, _: OwnerRow) -> ScriptSite {
    ScriptSite {
        file: fields_file(""),
        pointer: "/modules".to_owned(),
    }
}

/// Registers the compiling of every script as the game loads.
pub fn content(registry: &mut ContentRegistry) {
    registry.hook(LoadStep::Expand, build_all);
}

fn build_all(loading: &mut Loading) {
    let content = loading.content();
    let mut bind = Bind::default();
    for binder in Vocabulary::get().binders() {
        binder(loading, &content, &mut bind);
    }
    for (site, error) in bind.errors {
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
    loading.built(Scripts(Arc::new(bind.all)));
}

/// Starts the scripts of every owner that lives as long as a world of the game does.
pub fn spawn_world_owners(world: &mut World) {
    let scripts = world.resource::<Scripts>().clone();
    let vocabulary = Vocabulary::get();
    let mut instances = world.resource_mut::<Instances>();
    for (row, procedures) in scripts.owners() {
        if vocabulary.spec(row.kind).starts_with_world {
            instances.spawn_owner(Owner::Row(row.kind, row.row), procedures.clone());
        }
    }
}
