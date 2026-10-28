use bevy_math::{Quat, Vec3};
use engine::core::content::gltf::{ClipKeys, Keys, NodeKeys, NodeTree, Rest, TreeNode};
use engine::core::content::retarget;
use engine::core::time::Seconds;

/// His rig, at `scale` times our size: a root, hips above it and an arm off the hips.
fn his(scale: f32) -> NodeTree {
    tree(&[
        ("his.root", None, [0.0, 0.0, 0.0], scale),
        ("his.hips", Some(0), [0.0, 1.0, 0.0], 1.0),
        ("his.arm", Some(1), [-0.3, 0.2, 0.0], 1.0),
    ])
}

/// Our rig: the test rig's joints, the hand's finger beneath it.
fn ours() -> NodeTree {
    testkit::rig()
}

/// Our body and right hand copy his hips and arm.
fn pairs() -> Vec<(usize, usize)> {
    vec![(1, 1), (2, 2)]
}

#[test]
fn a_paired_joint_turns_as_his_does_and_the_rest_keep_their_place_under_their_parents() {
    let raised = Quat::from_rotation_z(90f32.to_radians());
    let keys = clip(vec![turned(2, raised)]);
    let pose = at_end(&ours(), &his(1.0), &keys);
    let worlds = worlds(&ours(), &pose);
    let turn = |joint: usize| worlds[joint].to_scale_rotation_translation().1;
    let at = |joint: usize| worlds[joint].w_axis.truncate();

    assert!(
        turn(2).angle_between(raised) < 1.0e-3,
        "the hand turns as his arm"
    );
    assert!(
        turn(4).angle_between(raised) < 1.0e-3,
        "the finger, paired to nothing, turns with its hand"
    );
    assert!(
        at(4).abs_diff_eq(at(2) + raised * Vec3::new(-0.05, 0.0, 0.0), 1.0e-4),
        "the finger keeps its place under the hand"
    );
    assert!(
        turn(3).angle_between(Quat::IDENTITY) < 1.0e-3,
        "the left hand rests"
    );
}

#[test]
fn the_body_travels_as_his_hips_do_but_not_as_his_whole_rig_does() {
    for scale in [1.0, 100.0] {
        let keys = clip(vec![
            moved(0, Vec3::new(5.0, 0.0, 0.0)),
            moved(1, Vec3::new(1.0, 0.5, 0.0)),
        ]);
        let pose = at_end(&ours(), &his(scale), &keys);
        let body = worlds(&ours(), &pose)[1].w_axis.truncate();
        assert!(
            body.abs_diff_eq(Vec3::new(1.0, 0.5, 0.0), 1.0e-3),
            "at {scale}× his size: the hips step 1 m aside and crouch to half height, the rig \
             under them moving 5 m along: {body}"
        );
    }
}

fn at_end(ours: &NodeTree, his: &NodeTree, keys: &ClipKeys) -> Vec<Rest> {
    let played = retarget::retarget(ours, his, &pairs(), keys);
    played.pose(ours, keys.length.0)
}

fn worlds(tree: &NodeTree, pose: &[Rest]) -> Vec<bevy_math::Mat4> {
    let posed = NodeTree {
        nodes: tree
            .nodes
            .iter()
            .zip(pose)
            .map(|(node, rest)| TreeNode {
                rest: *rest,
                ..node.clone()
            })
            .collect(),
    };
    posed.worlds()
}

fn clip(tracks: Vec<NodeKeys>) -> ClipKeys {
    ClipKeys {
        length: Seconds(1.0),
        tracks,
    }
}

fn turned(node: usize, to: Quat) -> NodeKeys {
    NodeKeys {
        node,
        rotation: Some(keys(Quat::IDENTITY, to)),
        ..NodeKeys::default()
    }
}

fn moved(node: usize, to: Vec3) -> NodeKeys {
    NodeKeys {
        node,
        translation: Some(keys(Vec3::ZERO, to)),
        ..NodeKeys::default()
    }
}

fn keys<T>(from: T, to: T) -> Keys<T> {
    Keys {
        times: vec![0.0, 1.0],
        values: vec![from, to],
        stepped: false,
    }
}

fn tree(joints: &[(&str, Option<usize>, [f32; 3], f32)]) -> NodeTree {
    NodeTree {
        nodes: joints
            .iter()
            .map(|&(name, parent, at, scale)| TreeNode {
                name: name.to_owned(),
                parent,
                rest: Rest {
                    translation: Vec3::from(at),
                    rotation: Quat::IDENTITY,
                    scale: Vec3::splat(scale),
                },
                mesh: false,
            })
            .collect(),
    }
}
