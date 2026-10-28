use std::collections::BTreeMap;

use editor::app::Editor;
use editor::views::skeleton::{self, SkeletonPage, rigs, sets};
use engine::core::assets::ModelRef;
use engine::core::render::world::layers::Gait;
use engine::core::time::{Millis, Seconds};
use engine::core::units::{Meters, MetersPerSec};
use engine::systems::actor::skeleton::{
    BuildName, Clip, ClipCell, ClipOrder, ClipRef, Cue, Hand, JointName, Layer, Leg, MoveName,
    SkeletonDef,
};
use engine::systems::actor::stance::StanceDef;
use engine::systems::actor::{Action, Pose};
use serde_json::json;
use strum::VariantArray;
use testkit::{BODY, clip, path};
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

    let slash_clip = file_clip(&clip("slash"));
    assert!(skeleton::set_cues(
        &mut editor,
        &key,
        &slash_clip,
        vec![cue(300.0, false), cue(120.0, true)]
    ));
    assert_eq!(
        skeleton_of(&mut editor, &key).cues_of(&slash_clip),
        [cue(120.0, true), cue(300.0, false)],
        "cues are kept in order of time"
    );

    let walk_clip = file_clip(&clip("walk"));
    assert!(skeleton::set_pace(
        &mut editor,
        &key,
        &walk_clip,
        MetersPerSec(Meters(1.4))
    ));
    assert_eq!(
        skeleton_of(&mut editor, &key).pace(&walk_clip),
        Some(MetersPerSec(Meters(1.4))),
        "a clip's pace lands in the skeleton's paces"
    );

    assert!(sets::set_cell(
        &mut editor,
        &key,
        None,
        Some(&sword),
        &slash,
        Some(whole("slash"))
    ));
    let row = skeleton_of(&mut editor, &key);
    let set = &row.stances[0];
    assert_eq!(stance_path(&editor, set.stance), sword);
    assert_eq!(set.set.moves[&MoveName::new("slash")], whole("slash"));
    let combo = ClipCell::Several {
        clips: ["slash", "attack"]
            .map(|name| ClipRef::Whole(file_clip(&clip(name))))
            .to_vec(),
        order: ClipOrder::InTurn,
    };
    assert!(sets::set_cell(
        &mut editor,
        &key,
        None,
        Some(&sword),
        &slash,
        Some(combo.clone())
    ));
    assert_eq!(
        skeleton_of(&mut editor, &key).stances[0].set.moves[&MoveName::new("slash")],
        combo,
        "a cell's clips land in order, with the order they take turns in"
    );
    assert!(
        !sets::set_cell(
            &mut editor,
            &key,
            None,
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
        None,
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

    let mut hands = skeleton_of(&mut editor, &key).hands;
    hands.push(Hand {
        name: "chest".to_owned(),
        joint: JointName::new("body"),
        offset: Default::default(),
        turn: Default::default(),
        clasp: None,
    });
    hands.swap(0, 2);
    assert!(skeleton::set_hands(&mut editor, &key, hands));
    assert_eq!(
        hand_joints(&skeleton_of(&mut editor, &key)),
        vec!["body", "hand.l", "hand.r"],
        "the first hand listed is the main hand"
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
fn a_source_rigs_clips_play_on_ours_once_its_joints_are_paired_with_ours() {
    let mut his = testkit::rig();
    for node in &mut his.nodes {
        node.name = format!("his_{}", node.name);
    }
    let jab = "seller/jab.glb";
    let game = game().asset(jab, testkit::library_glb(&his, &[("jab", Seconds(0.4))]));
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(game, &scratch);
    let key = path("skeleton", BODY);
    let jab = file_clip(jab);
    let rigging = |editor: &mut Editor| {
        let def = skeleton_of(editor, &key);
        skeleton::rigging(editor, &key, &def).expect("the skeleton's rigging")
    };
    assert!(rigging(&mut editor).playable(&jab).is_none());

    let joints: Vec<JointName> = his
        .nodes
        .iter()
        .map(|node| JointName::new(&node.name))
        .collect();
    assert!(rigs::add_source(
        &mut editor,
        &key,
        "seller".to_owned(),
        joints
    ));
    let pairs: BTreeMap<JointName, JointName> = his
        .nodes
        .iter()
        .map(|node| {
            let ours = node.name.trim_start_matches("his_");
            (JointName::new(&node.name), JointName::new(ours))
        })
        .collect();
    assert!(rigs::set_pairs(&mut editor, &key, 0, pairs));
    let paired = rigging(&mut editor);
    assert_eq!(
        paired.playable(&jab).map(|found| found.source),
        Some(Some(0)),
        "the seller's clip plays on our rig through its pairs"
    );
    assert!(
        sets::set_cell(
            &mut editor,
            &key,
            None,
            None,
            &sets::SetRow::Action(Action::Attack),
            Some(ClipCell::One(ClipRef::Whole(jab.clone())))
        ),
        "a set names it like any clip of ours"
    );
}

#[test]
fn a_build_fills_its_empty_cells_from_the_first_builds_by_rewriting_their_file_names() {
    let female = |action: &str| format!("clips/female/{action}.glb");
    let mut game = game()
        .edit("skeleton", BODY, |body| {
            body["builds"] = json!(["male", "female"])
        })
        .edit("model", "hero_body", |hero| hero["build"] = json!("male"))
        .edit("model", "hero", |hero| hero["build"] = json!("male"));
    let reshaped: Vec<&str> = Action::VARIANTS
        .iter()
        .map(|action| action.as_ref())
        .filter(|action| *action != "idle")
        .collect();
    for action in &reshaped {
        game = game.asset(
            &female(action),
            testkit::library_glb(&testkit::rig(), &[(action, Seconds(0.3))]),
        );
    }
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(game, &scratch);
    let key = path("skeleton", BODY);
    let build = BuildName::new("female");
    let filled = |editor: &mut Editor| {
        let def = skeleton_of(editor, &key);
        let rigging = skeleton::rigging(editor, &key, &def).expect("the skeleton's rigging");
        sets::fill(
            editor,
            &key,
            &rigging,
            &build,
            &[("clips/body/".to_owned(), "clips/female/".to_owned())],
        )
    };
    assert_eq!(
        filled(&mut editor),
        reshaped.len(),
        "a cell whose rewritten file holds no clip stays empty"
    );
    let row = skeleton_of(&mut editor, &key);
    let base = &row.build_sets[&build].base;
    assert_eq!(
        base.clips[&Action::Attack],
        ClipCell::One(ClipRef::Whole(file_clip(&female("attack"))))
    );
    assert!(!base.clips.contains_key(&Action::Idle));
    assert_eq!(
        row.pace(&file_clip(&female("walk"))),
        Some(testkit::WALK_PACE),
        "a rewritten clip covers the ground its first build's does until told otherwise"
    );
    assert_eq!(
        filled(&mut editor),
        0,
        "a cell filled once is the build's own"
    );
}

#[test]
fn the_skeleton_view_draws_every_page_and_sets_a_cell_from_the_pointer() {
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

    draw(&mut editor, Vec::new());
    editor.views.skeleton.view_mut(&key).page = SkeletonPage::Sets;
    draw(&mut editor, Vec::new());
    let sets_page = draw(&mut editor, Vec::new());
    let sit = egui::pos2(sets_page.at("sword").x, sets_page.at("sit · pose").y);
    for events in click(sit) {
        draw(&mut editor, events);
    }
    let picker = draw(&mut editor, Vec::new());
    for events in click(picker.at(&clip("sit"))) {
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
        SkeletonPage::Rigs,
        SkeletonPage::Sets,
        SkeletonPage::Hands,
        SkeletonPage::Body,
        SkeletonPage::Layers,
    ] {
        editor.views.skeleton.view_mut(&key).page = page;
        let drawn = draw(&mut editor, Vec::new());
        assert!(
            !drawn.any(|text| text.contains("did not load") || text.contains("no rig stands")),
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
    let view = editor.views.skeleton.view_mut(&key);
    view.page = SkeletonPage::Sets;
    view.sets.open = Some((sets::SetRow::Action(Action::Walk), None));
    view.clip = Some(file_clip(&clip("walk")));
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

fn file_clip(file: &str) -> Clip {
    Clip::File(ModelRef::new(file))
}

fn whole(name: &str) -> ClipCell {
    ClipCell::One(ClipRef::Whole(file_clip(&clip(name))))
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
