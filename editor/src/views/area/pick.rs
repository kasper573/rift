use crate::tabs::file_title;
use engine::core::content::ContentFile;
use engine::core::math::Pos;
use engine::core::units::Meters;
use engine::systems::area::AreaModuleKind;
use engine::systems::area::blueprint::BlueprintDef;
use engine::systems::area::prop::PropDef;
use engine::systems::area::scene::{ModuleNode, SceneNode};
use engine::systems::npc::NpcDef;
use engine::systems::script::ScriptDef;

use super::{AreaFrame, modules, nodes, shapes};
use crate::app::{BottomTab, Editor, ModalDialog, PickRowDialog, PickRowThen};
use crate::schema;
use crate::views::sound;

/// The type of file a module of `kind` names, picked when one is added: whose npc, which blueprint.
pub fn named_table(kind: AreaModuleKind) -> Option<&'static str> {
    match kind {
        AreaModuleKind::Population | AreaModuleKind::Resident => Some(NpcDef::TABLE),
        AreaModuleKind::Blueprint => Some(BlueprintDef::TABLE),
        AreaModuleKind::Script => Some(ScriptDef::TABLE),
        AreaModuleKind::Prop | AreaModuleKind::Scatter => Some(PropDef::TABLE),
        AreaModuleKind::Object
        | AreaModuleKind::SafeZone
        | AreaModuleKind::Soundscape
        | AreaModuleKind::Light => None,
    }
}

/// Asks which file a new module of `kind` names, then adds it at `at`, or area-wide without one, to
/// `bundle`'s insertion folder.
pub fn pick_then_add(
    editor: &mut Editor,
    bundle: &str,
    kind: AreaModuleKind,
    at: Option<Pos<Meters>>,
) {
    let Some(table) = named_table(kind) else {
        return;
    };
    editor.dialogs.push(ModalDialog::PickRow(PickRowDialog {
        title: format!("Pick a {table} for the new {}", schema::title(kind.tag())),
        table: table.to_owned(),
        search: String::new(),
        picked: None,
        options: Vec::new(),
        then: PickRowThen::AddModule {
            area: bundle.to_owned(),
            kind,
            at,
        },
    }));
}

/// Adds a module of `kind` at `at`, or area-wide without one, to `area`, naming the file at
/// `picked`.
pub fn add_picked(
    editor: &mut Editor,
    area: &str,
    kind: AreaModuleKind,
    at: Option<Pos<Meters>>,
    picked: &str,
) {
    let Some(reader) = editor.reader() else {
        return;
    };
    let module = match modules::module_template(&reader, area, kind, at, Some(picked)) {
        Ok(module) => module,
        Err(why) => {
            editor.toast(why);
            return;
        }
    };
    let label = format!(
        "Add {} to {}",
        shapes::label(&module, reader.as_ref()),
        file_title(area)
    );
    nodes::add(
        editor,
        area,
        vec![SceneNode::Module(ModuleNode::new(module))],
        &label,
    );
}

pub fn listen(editor: &mut Editor, frame: &AreaFrame, tile: Pos<Meters>) {
    let Some(area) = frame.area_id() else {
        return;
    };
    sound::listen(editor, &frame.content, area, tile);
    editor.panels.bottom = BottomTab::Listen;
}
