use editor::app::Editor;
use editor::project::{ContentEdit, FileBytes, ProjectFiles};
use editor::rows;
use editor::schema::{self, SchemaKind};
use engine::core::audio::playback::SFX_CLIP;
use engine::core::babble::BABBLE_LETTERS;
use engine::core::content::{ContentFile, fields_file};
use engine::core::vfx::VfxDef;
use engine::field;
use engine::systems::npc::NpcDef;
use engine::systems::registry;
use serde_json::{Value, json};
use testkit::{Game, path, png};

use crate::support;

fn files(editor: &mut Editor) -> ProjectFiles {
    support::files(editor).clone()
}

/// The path of the model the orc wears.
fn orc_model(editor: &mut Editor) -> String {
    support::files(editor);
    let npc = editor
        .read::<NpcDef>(&path("npc", "orc"))
        .expect("a project")
        .expect("the orc reads");
    let reader = editor.reader().expect("a reader");
    reader.path_of(npc.model).expect("a model").to_owned()
}

fn camp() -> Game {
    support::game()
        .npc("orc", 10.0, json!([]))
        .prop(
            "plank",
            json!({ "type": "box", "width": 1.0, "depth": 1.0 }),
        )
        .asset("icons/cap.png", png(16, 16))
        .asset("art/badge.png", png(32, 32))
        .asset("art/badge_card.png", png(256, 180))
}

#[test]
fn an_edit_writing_a_field_its_type_does_not_have_is_refused_and_changes_nothing() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(camp(), &scratch);
    let before = files(&mut editor);
    let config = fields_file(&path("model", "orc"));
    let mut edit = ContentEdit::new("give the model a sheet");
    edit.update_json(&before, &config, |model| {
        model["sprites"] = json!("icons/cap.png");
        Some(())
    })
    .expect("the model is there");

    let refused = editor.try_apply(edit).expect_err("refused");
    assert!(refused.contains("unknown field"), "{refused}");
    assert_eq!(files(&mut editor).get(&config), before.get(&config));
}

#[test]
fn an_edit_writing_an_image_that_does_not_decode_is_refused() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(camp(), &scratch);
    let mut cut = png(16, 16);
    cut.truncate(cut.len() - 20);
    let mut edit = ContentEdit::new("cut the cap short");
    edit.write("icons/cap.png", FileBytes::from(cut));

    let refused = editor.try_apply(edit).expect_err("refused");
    assert!(refused.contains("unreadable image"), "{refused}");
}

#[test]
fn a_new_file_of_any_type_names_the_content_nearest_it_and_holds_as_it_is() {
    let scratch = tempfile::tempdir().expect("scratch");
    let orc = camp().folder();
    let game = orc
        .files
        .iter()
        .filter(|(file, _)| {
            **file == path("model", "orc")
                || file.starts_with(&format!("{}/", path("model_part", "orc")))
        })
        .fold(camp(), |game, (file, bytes)| {
            game.file(
                &format!("wilds/{}", file.replace("orc", "wolf")),
                bytes.to_vec(),
            )
        });
    let mut editor = support::open(game.babble("low"), &scratch);
    let tables: Vec<String> = editor
        .shapes
        .order
        .iter()
        .filter(|table| !registry().closed(table))
        .cloned()
        .collect();

    let mut taken_back = Vec::new();
    for table in &tables {
        let made = rows::new_file(&mut editor, "wilds", table)
            .unwrap_or_else(|refused| panic!("{table}: {refused}"));

        support::settled(&mut editor);
        let project = editor.project.as_mut().expect("a project");
        if let Some(refused) = project.take_refused() {
            taken_back.push(format!("{table}: {}", refused.refusal.sentence()));
            continue;
        }
        let errors = project.reader().check(std::slice::from_ref(&made)).errors;
        assert!(errors.is_empty(), "{table}: {errors:?}");
    }
    assert!(taken_back.is_empty(), "taken back: {taken_back:#?}");
    let made = editor
        .read::<NpcDef>("wilds/npc.npc.json")
        .expect("a project")
        .expect("a new npc");
    let reader = editor.reader().expect("a reader");
    assert_eq!(
        reader.path_of(made.model),
        Some(format!("wilds/{}", path("model", "wolf")).as_str())
    );
}

#[test]
fn a_vfx_layer_switched_to_any_of_its_kinds_holds_as_it_is() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(camp(), &scratch);
    let vfx = rows::new_file(&mut editor, "icons", VfxDef::TABLE).expect("a vfx");
    support::settled(&mut editor);
    let project = editor.project.as_ref().expect("a project");
    let root = editor.shapes.shape_of(VfxDef::TABLE);
    let layers = schema::properties(root)
        .into_iter()
        .find(|(name, _)| *name == &field!(VfxDef, layers)[1..])
        .map(|(_, layers)| layers)
        .expect("the layers");
    let SchemaKind::Array(layer) = schema::kind(root, layers) else {
        panic!("a list of layers");
    };
    let nearby = schema::Nearby::of(project, Some(&vfx));
    let kinds: Vec<(String, Value)> = schema::tagged_variants(root, layer)
        .iter()
        .map(|kind| {
            let template = schema::variant_template(root, kind, nearby);
            (kind.tag.clone(), template)
        })
        .collect();
    let files = project.files.clone();
    assert_eq!(kinds.len(), 2);

    for (kind, template) in kinds {
        let mut edit = ContentEdit::new(format!("Switch to a {kind} layer"));
        edit.update_json(&files, &vfx, |row| {
            *row.pointer_mut(&format!("{}/0", field!(VfxDef, layers)))? = template;
            Some(())
        });
        let switched = editor.try_apply(edit);
        assert!(switched.is_ok(), "{kind}: {switched:?}");
    }
}

#[test]
fn a_sound_and_a_voice_take_their_clips_from_files_under_the_names_their_types_give_them() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(support::game().sfx("hit").babble("low"), &scratch);
    let mut louder = testkit::wav();
    *louder.last_mut().expect("samples") ^= 0x10;
    let delivered = |name: &str| {
        let file = scratch.path().join(name);
        std::fs::write(&file, &louder).expect("a delivered file");
        file
    };
    let (hit, low) = (path("sfx", "hit"), path("babble_bank", "low"));

    rows::put_parts(&mut editor, &hit, SFX_CLIP, &[delivered("Coin Clink.WAV")]).expect("a clip");
    rows::put_parts(
        &mut editor,
        &low,
        BABBLE_LETTERS,
        &[delivered("A.wav"), delivered("b.wav")],
    )
    .expect("two letters");
    let refused = [
        rows::put_parts(&mut editor, &hit, SFX_CLIP, &[delivered("hit.ogg")]),
        rows::put_parts(&mut editor, &low, BABBLE_LETTERS, &[delivered("a b?.wav")]),
    ];

    let files = files(&mut editor);
    for part in [
        format!("{hit}/{}", SFX_CLIP.named("", "wav")),
        format!("{low}/{}", BABBLE_LETTERS.named("a", "wav")),
        format!("{low}/{}", BABBLE_LETTERS.named("b", "wav")),
    ] {
        assert_eq!(
            files.get(&part).map(|bytes| bytes.to_vec()),
            Some(louder.clone()),
            "{part}"
        );
    }
    assert_ne!(
        files
            .get(&format!("{low}/{}", BABBLE_LETTERS.named("c", "wav")))
            .map(|bytes| bytes.to_vec()),
        Some(louder.clone())
    );
    assert!(refused.iter().all(Result::is_err), "{refused:?}");
    let project = support::settled(&mut editor);
    let errors = project.reader().check(&[hit, low]).errors;
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn a_cue_placed_past_the_end_of_its_clip_is_refused() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(camp(), &scratch);
    let config = fields_file(&path("skeleton", testkit::BODY));
    let mut edit = ContentEdit::new("land the attack after the clip ends");
    edit.update_json(&files(&mut editor), &config, |skeleton| {
        skeleton["cues"]["attack"][0]["at"] = json!(testkit::CLIP.0 * 2.0);
        Some(())
    })
    .expect("the skeleton is there");

    let refused = editor.try_apply(edit).expect_err("refused");
    assert!(refused.contains("cue past the clip"), "{refused}");
}

/// The orc's npc made to wear the elf's model, applied but not yet loaded.
fn orc_wearing_elf(editor: &mut Editor) {
    let before = editor.project.as_ref().expect("a project").files.clone();
    let mut edit = ContentEdit::new("dress the orc as an elf");
    edit.update_json(&before, &fields_file(&path("npc", "orc")), |npc| {
        npc["model"] = json!(path("model", "elf"));
        Some(())
    })
    .expect("the npc is there");
    editor.try_apply(edit).expect("applies");
}

#[test]
fn renaming_what_an_edit_still_loading_names_rewrites_that_edit_too() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(camp().model("elf", 1.5, json!([])), &scratch);
    orc_wearing_elf(&mut editor);

    let renamed = rows::rename(&mut editor, &path("model", "elf"), "fae").expect("renames");

    support::settled(&mut editor);
    assert_eq!(orc_model(&mut editor), renamed);
}

#[test]
fn removing_what_another_file_names_is_refused_even_while_that_file_is_still_loading() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(camp().model("elf", 1.5, json!([])), &scratch);
    let remove = |editor: &mut Editor, model: &str| {
        let mut edit = ContentEdit::new("remove a model");
        for file in editor
            .project
            .as_ref()
            .expect("a project")
            .files
            .files_of(model)
        {
            edit.remove(file);
        }
        editor.try_apply(edit).expect_err("refused")
    };

    let orc = path("model", "orc");
    let refused = remove(&mut editor, &orc);
    assert!(refused.contains("missing file"), "{refused}");
    assert!(files(&mut editor).contains(&fields_file(&orc)));

    orc_wearing_elf(&mut editor);
    let refused = remove(&mut editor, &path("model", "elf"));
    assert!(refused.contains("missing file"), "{refused}");
}

/// An area placing a yard blueprint, and a post blueprint the yard may place.
fn yard() -> Game {
    let post = json!([{
        "type": "object",
        "name": "post",
        "shape": { "type": "point", "x": 0.5, "y": 0.5 },
    }]);
    camp()
        .blueprint("post", (1, 1), json!({}), post)
        .blueprint("yard", (2, 2), json!({}), json!([]))
        .edit_scene("area", support::AREA, |scene| {
            scene.as_array_mut().expect("a scene").push(json!({
                "type": "blueprint",
                "label": "yard",
                "blueprint": path("blueprint", "yard"),
                "at": { "x": 3.0, "y": 3.0 },
            }));
        })
}

fn yard_scene() -> String {
    format!("{}/scene.json", path("blueprint", "yard"))
}

#[test]
fn an_edit_that_breaks_a_file_it_never_touched_is_taken_back_once_the_folder_loads() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(yard(), &scratch);
    let before = files(&mut editor);
    let post_at = |x: f32| {
        let post = json!([{
            "type": "blueprint",
            "label": "post",
            "blueprint": path("blueprint", "post"),
            "at": { "x": x, "y": x },
        }]);
        FileBytes::from(serde_json::to_vec_pretty(&post).expect("json"))
    };
    let mut edit = ContentEdit::new("place a post far out in the yard");
    edit.write(yard_scene(), post_at(6.0));

    editor.try_apply(edit).expect("the yard alone reads");
    let project = editor.project.as_mut().expect("a project");
    project.settle();

    let refused = project.take_refused().expect("the edit is taken back");
    assert_eq!(refused.label, "place a post far out in the yard");
    assert_eq!(
        refused.refusal.file,
        format!("{}/scene.json", support::area())
    );
    assert_eq!(project.files.get(&yard_scene()), before.get(&yard_scene()));
    assert!(project.history.steps().is_empty());
    assert!(project.loads());

    let near = post_at(1.0);
    let mut edit = ContentEdit::new("place a post in the yard");
    edit.write(yard_scene(), near.clone());
    editor.try_apply(edit).expect("applies");
    let project = editor.project.as_mut().expect("a project");
    project.settle();

    let refused = project.take_refused();
    assert!(
        refused.is_none(),
        "an edit keeping every file loading stays: {:?}",
        refused.map(|refused| refused.refusal.sentence())
    );
    assert_eq!(project.files.get(&yard_scene()), Some(&near));
    assert_eq!(project.history.steps().len(), 1);
}
