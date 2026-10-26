use std::collections::BTreeMap;

use bevy::math::{Quat, Vec3};
use editor::app::Editor;
use editor::views::skeleton::{self, SkeletonPage, library, sets};
use engine::core::content::gltf::{Rest, TreeNode};
use engine::core::render::world::layers::Gait;
use engine::core::time::{Millis, Seconds};
use engine::core::units::Meters;
use engine::systems::actor::model::Skeletons;
use engine::systems::actor::skeleton::{
    CLIP_LIBRARIES, ClipName, ClipRef, Cue, GroupName, JointName, Layer, Leg, MoveName, SkeletonDef,
};
use engine::systems::actor::stance::StanceDef;
use engine::systems::actor::{Action, Pose};
use serde_json::json;
use testkit::{BODY, LIBRARY, path};
use ui::egui;

use crate::support::{self, Frames, click};

const SWORD: &str = "sword";

fn game() -> testkit::Game {
    let mut clips = testkit::rig_clips();
    clips.extend([("slash", Seconds(0.5)), ("sit", Seconds(0.8))]);
    support::game()
        .stance(SWORD, 1, "main")
        .skeleton_of(BODY, &clips, json!({}))
        .model("hero_body", 1.8, json!([]))
}

#[test]
fn each_edit_of_the_skeleton_view_lands_in_its_config() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(game(), &scratch);
    let key = path("skeleton", BODY);
    let sword = path("stance", SWORD);
    let slash = sets::SetRow::Move(MoveName::new("slash"));

    let slash_clip = ClipName::new("slash");
    assert!(skeleton::set_cues(
        &mut editor,
        &key,
        &slash_clip,
        vec![cue(300.0, false), cue(120.0, true)]
    ));
    assert_eq!(
        skeleton_of(&mut editor, &key).cues[&slash_clip],
        vec![cue(120.0, true), cue(300.0, false)],
        "cues are kept in order of time"
    );

    assert!(sets::set_cell(
        &mut editor,
        &key,
        Some(&sword),
        &slash,
        Some(whole("slash"))
    ));
    let row = skeleton_of(&mut editor, &key);
    let set = &row.stances[0];
    assert_eq!(stance_path(&editor, set.stance), sword);
    assert_eq!(set.set.moves[&MoveName::new("slash")], whole("slash"));
    assert!(
        !sets::set_cell(
            &mut editor,
            &key,
            None,
            &sets::SetRow::Action(Action::Attack),
            None
        ),
        "the base set keeps a clip for every engine action"
    );
    assert!(
        !sets::add_stance_column(&mut editor, &key, &sword),
        "one set per stance"
    );
    assert!(sets::set_cell(
        &mut editor,
        &key,
        Some(&sword),
        &slash,
        None
    ));
    assert!(
        skeleton_of(&mut editor, &key).stances[0]
            .set
            .moves
            .is_empty(),
        "a cell taken away falls back down the ladder"
    );

    assert!(skeleton::make_hand(&mut editor, &key, "body", "chest"));
    assert!(
        !skeleton::make_hand(&mut editor, &key, "body", "again"),
        "a joint is one hand"
    );
    let mut hands = skeleton_of(&mut editor, &key).hands;
    hands.swap(0, 2);
    assert!(skeleton::set_hands(&mut editor, &key, hands));
    assert_eq!(
        hand_joints(&skeleton_of(&mut editor, &key)),
        vec!["body", "hand.l", "hand.r"],
        "the first hand listed is the main hand"
    );

    assert!(skeleton::group_at(&mut editor, &key, "hand.r", "arms"));
    assert!(skeleton::group_at(&mut editor, &key, "hand.l", "arms"));
    assert_eq!(
        skeleton_of(&mut editor, &key).groups,
        BTreeMap::from([(
            GroupName::new("arms"),
            vec![JointName::new("hand.r"), JointName::new("hand.l")]
        )])
    );

    let feet = |joints: [&str; 2]| Layer::Feet {
        legs: vec![Leg {
            name: "leg".to_owned(),
            joints: joints.into_iter().map(JointName::new).collect(),
        }],
        gait: Gait::Lateral,
        stride: Meters(0.5),
        lift: Meters(0.1),
        during: vec![Action::Walk],
    };
    assert!(
        !skeleton::set_layers(&mut editor, &key, vec![feet(["hand.r", "body"])]),
        "a leg is a chain down the tree"
    );
    assert!(skeleton_of(&mut editor, &key).layers.is_empty());
    assert!(skeleton::set_layers(
        &mut editor,
        &key,
        vec![feet(["body", "hand.r"])]
    ));
    assert_eq!(
        skeleton_of(&mut editor, &key).layers,
        vec![feet(["body", "hand.r"])]
    );
}

#[test]
fn a_dropped_library_adds_only_the_clips_a_set_may_name_and_a_new_rig_must_fit_every_wearer() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(game(), &scratch);
    let key = path("skeleton", BODY);
    support::settled(&mut editor);

    let dropped = testkit::library_glb(
        &testkit::rig(),
        &[
            ("thrust", Seconds(0.6)),
            ("idle", Seconds(0.9)),
            ("T-Pose", Seconds(0.5)),
        ],
    );
    let mut draft =
        library::draft(&editor, &key, dropped, "spear.glb".to_owned(), "Spear").expect("a draft");
    assert_eq!(
        draft
            .clips
            .iter()
            .map(|clip| (clip.name.as_str(), clip.kind.clone()))
            .collect::<Vec<_>>(),
        vec![
            ("thrust", library::ClipKind::New),
            ("idle", library::ClipKind::Present(LIBRARY.to_owned())),
            (
                "T-Pose",
                library::ClipKind::Utility("a pose to rig against".to_owned())
            ),
        ]
    );
    assert_eq!(draft.name, "spear");
    draft.taken.insert("idle".to_owned());
    assert!(draft.refusals().is_empty(), "{:?}", draft.refusals());
    assert!(library::apply(&mut editor, &key, &draft));
    support::settled(&mut editor);
    let content = support::content(&mut editor).clone();
    let id = content.by_path::<SkeletonDef>(&key).expect("the skeleton");
    let built = Skeletons::loaded(&content, id).expect("the skeleton loads");
    let library_of = |clip: &str| {
        let found = built.clips[&ClipName::new(clip)];
        built.def.libraries[found.library].key.clone()
    };
    assert_eq!(library_of("thrust"), "spear");
    assert_eq!(
        library_of("idle"),
        "spear",
        "the library took the clip over"
    );
    assert_eq!(library_of("walk"), LIBRARY);
    assert!(!built.clips.contains_key(&ClipName::new("T-Pose")));
    assert!(
        support::files(&mut editor)
            .contains(&format!("{key}/{}", CLIP_LIBRARIES.named("spear", "glb")))
    );

    let mut handless = testkit::rig();
    handless.nodes.retain(|node| node.name != "hand.l");
    let misfit = library::draft(
        &editor,
        &key,
        testkit::library_glb(&handless, &[("jab", Seconds(0.4))]),
        "jab.glb".to_owned(),
        "jab",
    )
    .expect("a draft");
    assert!(
        misfit.refusals().join("\n").contains("no joint hand.l"),
        "{:?}",
        misfit.refusals()
    );
    assert!(!library::apply(&mut editor, &key, &misfit));

    let mut tailed = testkit::rig();
    tailed.nodes.push(TreeNode {
        name: "tail".to_owned(),
        parent: Some(1),
        rest: Rest {
            translation: Vec3::new(0.0, -0.5, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        },
        mesh: false,
    });
    let mut rig = library::draft(
        &editor,
        &key,
        testkit::rig_glb(&tailed),
        "tailed.glb".to_owned(),
        "tailed",
    )
    .expect("a draft");
    rig.as_rig = true;
    assert!(
        rig.refusals()
            .iter()
            .any(|refusal| refusal.contains("no joint tail")),
        "every model wearing the skeleton is refitted: {:?}",
        rig.refusals()
    );
}

#[test]
fn the_skeleton_view_marks_its_rig_and_edits_its_sets_from_the_pointer_and_keys() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(game(), &scratch);
    let key = path("skeleton", BODY);
    let sword = path("stance", SWORD);
    assert!(sets::add_stance_column(&mut editor, &key, &sword));
    support::settled(&mut editor);
    let mut frames = Frames::new(egui::vec2(1600.0, 1000.0));
    let mut draw = |editor: &mut Editor, events: Vec<egui::Event>| {
        frames.run(events, |ui| skeleton::show(editor, ui, &key))
    };

    let joints = draw(&mut editor, Vec::new());
    assert!(joints.any(|text| text == "hand right · main"), "{joints:?}");
    let body = joints.at("body");
    for events in click(body) {
        draw(&mut editor, events);
    }
    draw(
        &mut editor,
        support::key(egui::Key::G, egui::Modifiers::NONE),
    );
    support::settled(&mut editor);
    assert_eq!(
        skeleton_of(&mut editor, &key).groups,
        BTreeMap::from([(GroupName::new("body"), vec![JointName::new("body")])]),
        "G starts a group at the chosen joint"
    );

    editor.views.skeleton.view_mut(&key).page = SkeletonPage::Sets;
    draw(&mut editor, Vec::new());
    let sets_page = draw(&mut editor, Vec::new());
    let sit = egui::pos2(sets_page.at("sword").x, sets_page.at("sit · pose").y);
    for events in click(sit) {
        draw(&mut editor, events);
    }
    let picker = draw(&mut editor, Vec::new());
    for events in click(picker.at("sit")) {
        draw(&mut editor, events);
    }
    draw(&mut editor, Vec::new());
    let row = skeleton_of(&mut editor, &key);
    assert_eq!(
        row.stances[0].set.poses.get(&Pose::Sit),
        Some(&whole("sit")),
        "the clip picked for the sword column's sit lands in its set: {:?}",
        row.stances
    );

    for page in [
        SkeletonPage::Clips,
        SkeletonPage::Hands,
        SkeletonPage::Layers,
    ] {
        editor.views.skeleton.view_mut(&key).page = page;
        let drawn = draw(&mut editor, Vec::new());
        assert!(
            !drawn.any(|text| text.contains("did not load")),
            "{page:?}: {drawn:?}"
        );
    }
}

#[test]
fn a_clip_on_the_skeleton_view_plays_without_being_asked() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(game(), &scratch);
    let key = path("skeleton", BODY);
    let mut frames = Frames::new(egui::vec2(1600.0, 1000.0));
    let mut draw =
        |editor: &mut Editor| frames.run(Vec::new(), |ui| skeleton::show(editor, ui, &key));
    draw(&mut editor);
    editor.views.skeleton.view_mut(&key).page = SkeletonPage::Clips;
    let shown: Vec<String> = (0..4)
        .map(|_| {
            let drawn = draw(&mut editor);
            drawn
                .0
                .iter()
                .find_map(|(text, _)| text.split_once(" ms · ").map(|(ms, _)| ms.to_owned()))
                .unwrap_or_else(|| panic!("the timeline tells its playhead: {drawn:?}"))
        })
        .collect();
    assert!(
        shown.windows(2).all(|pair| pair[0] != pair[1]),
        "the playhead moves on frame after frame: {shown:?}"
    );
}

fn whole(clip: &str) -> ClipRef {
    ClipRef::Whole(ClipName::new(clip))
}

fn cue(at: f32, apex: bool) -> Cue {
    Cue {
        at: Millis(at),
        step: !apex,
        apex,
        sound: None,
    }
}

fn hand_joints(row: &SkeletonDef) -> Vec<&str> {
    row.hands.iter().map(|hand| hand.joint.as_str()).collect()
}

fn stance_path(editor: &Editor, stance: engine::systems::actor::stance::StanceId) -> String {
    editor
        .reader()
        .and_then(|reader| reader.path_of::<StanceDef>(stance).map(str::to_owned))
        .expect("the stance has a file")
}

fn skeleton_of(editor: &mut Editor, key: &str) -> SkeletonDef {
    support::settled(editor);
    editor
        .read::<SkeletonDef>(key)
        .expect("the skeleton is in the project")
        .expect("the skeleton reads")
        .as_ref()
        .clone()
}
