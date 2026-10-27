"""Turns a pack's FBX character into a glb `restyle.py` reads: its skin, its armature, and one
named clip per animation, each baked from the pose the pack's rig shows (constraints and IK
targets included) and starting at zero.

    blender -b --factory-startup -P convert.py -- KIT PACKS [NAME...]

KIT's `convert` section names each output under PACKS and how it is made: `skin`, the FBX holding
the mesh and armature; `textures`, a base colour image per material name (`*` for every
material), so restyle can sample the pack's colours; `clips`, each clip's name mapped to an FBX
whose own take is the clip, or to `[first, last]` frames of the skin's own take; and `fps`, the
frame rate the takes were made at.

A `skin` may also be a list of FBXs whose armatures join into one, each adding the bones the
ones before it lack; `parents` then hangs a bone under another and `drop` removes bones.

Clips made on another rig are baked on with `retarget`, each bone of the skin mapped to the bone
of the clips' rig it follows. A mapped bone turns in the world as its source turns from its rest,
so both rigs must rest in the same pose, the source's rest carried wherever its root bone stands
in each frame (a pack whose rig rests lying down and is stood up by its root); an unmapped bone
keeps its place under its parent; the topmost mapped bone also travels as its source does, scaled
by how high each rests. A retargeted clip may name a list of FBXs, played one after another as one
clip.

`rebind` names the FBXs of a modular pack whose parts all wear the skin's armature: each is
written alone as a glb in a folder named after the output, bound to that armature in metres under
a `model` root, so its joints stand at the paths and rests of a rig restyle cuts from the output.
A part's mesh is taken where its own armature shows it at rest, or where it poses it when only the
pose agrees with the skin's armature (packs whose files carry a stray bind pose); a part matching
neither is refused, as is one weighted to a bone the armature lacks.

A model of rigid parts and no rig (a turret) is given as `parts` instead: the glTF, and `bones`,
each named, under its `parent`, carrying the mesh node `part` whole and pivoting where that node
stands, or standing `at` the front of its parent's part. Its `keys` are its clips: per clip a
`length` in seconds and, per bone, `[time, [x, y, z] degrees turned, [x, y, z] metres moved]`.
"""

import json
import os
import sys

import bpy


def main():
    argv = sys.argv[sys.argv.index("--") + 1 :]
    kit_path, packs = argv[:2]
    only = argv[2:]
    kit = json.load(open(kit_path))
    for out, entry in kit["convert"].items():
        if only and not any(name in out for name in only):
            continue
        convert(entry, kit["packs"], packs, os.path.join(packs, out))


def resolve(spec, folders, packs):
    alias, name = spec.split(":", 1)
    return os.path.join(packs, folders[alias], name)


def convert(entry, folders, packs, out):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.render.fps = int(entry.get("fps", 30))
    if "parts" in entry:
        armature, meshes, baked = rigged(entry, resolve(entry["parts"], folders, packs), scene.render.fps)
        export(armature, meshes, baked, out)
        return
    armature, meshes = joined([resolve(skin, folders, packs) for skin in listed(entry["skin"])])
    reshape(armature, entry.get("parents", {}), entry.get("drop", []))
    if "rebind" in entry:
        upright(armature)
    for material_name, image in entry.get("textures", {}).items():
        for mesh in meshes:
            for material in mesh.data.materials:
                if material and material_name in ("*", material.name):
                    paint(material, resolve(image, folders, packs))
    own_take = armature.animation_data.action if armature.animation_data else None
    baked = []
    for name, source in entry["clips"].items():
        if "retarget" in entry:
            sources = [resolve(spec, folders, packs) for spec in listed(source)]
            baked.append(retarget(armature, entry["retarget"], sources, name, scene.render.fps))
            continue
        if isinstance(source, list):
            offset = own_take.frame_range[0]
            action, first, last = own_take, source[0] + offset, source[1] + offset
        else:
            other, other_meshes = imported(resolve(source, folders, packs))
            action = other.animation_data.action
            first, last = action.frame_range
            remove([other, *other_meshes])
        baked.append(bake(armature, action, int(first), int(last), name))
    for bone in armature.pose.bones:
        for constraint in list(bone.constraints):
            bone.constraints.remove(constraint)
    kept = [armature, *meshes]
    placed = {ob: ob.matrix_world.copy() for ob in kept}
    remove([ob for ob in bpy.data.objects if ob not in kept])
    for ob in kept:
        ob.matrix_world = placed[ob]
    for action in list(bpy.data.actions):
        if action not in baked:
            bpy.data.actions.remove(action)
    export(armature, meshes, baked, out)
    if "rebind" in entry:
        remove(meshes)
        rebind(armature, resolve(entry["rebind"], folders, packs), os.path.splitext(out)[0])


def export(armature, meshes, baked, out):
    armature.animation_data.action = None
    for action in baked:
        track = armature.animation_data.nla_tracks.new()
        track.name = action.name
        track.strips.new(action.name, int(action.frame_range[0]), action)
        track.mute = True
    os.makedirs(os.path.dirname(out), exist_ok=True)
    bpy.ops.object.select_all(action="DESELECT")
    for ob in [armature, *meshes]:
        ob.select_set(True)
    bpy.ops.export_scene.gltf(
        filepath=out,
        export_format="GLB",
        use_selection=True,
        export_animations=True,
        export_animation_mode="NLA_TRACKS",
        export_anim_slide_to_zero=True,
        export_force_sampling=True,
        export_def_bones=False,
        export_morph=False,
        export_apply=False,
        export_yup=True,
    )
    print(f"{out}: {len(baked)} clips, {sum(len(m.data.polygons) for m in meshes)} faces, {len(armature.data.bones)} bones")


def rigged(entry, path, fps):
    """The parts joined into one mesh skinned rigidly to bones placed at the parts' pivots, and
    the clips keyed on those bones."""
    from mathutils import Euler, Vector

    bpy.ops.import_scene.gltf(filepath=path)
    parts = {ob.name: ob for ob in bpy.data.objects if ob.type == "MESH"}
    armature_data = bpy.data.armatures.new("rig")
    armature = bpy.data.objects.new("rig", armature_data)
    bpy.context.scene.collection.objects.link(armature)
    bpy.context.view_layer.objects.active = armature
    bpy.ops.object.mode_set(mode="EDIT")
    for spec in entry["bones"]:
        if "part" in spec:
            head = parts[spec["part"]].matrix_world.translation.copy()
        else:
            parent_part = parts[next(b["part"] for b in entry["bones"] if b["name"] == spec["parent"])]
            corners = [parent_part.matrix_world @ Vector(corner) for corner in parent_part.bound_box]
            center = sum(corners, Vector()) / len(corners)
            head = Vector((center.x, min(corner.y for corner in corners), center.z))
        bone = armature_data.edit_bones.new(spec["name"])
        bone.head = head
        bone.tail = head + Vector((0, 0, 0.1))
        if "parent" in spec:
            bone.parent = armature_data.edit_bones[spec["parent"]]
    bpy.ops.object.mode_set(mode="OBJECT")
    meshes = []
    for spec in entry["bones"]:
        if "part" not in spec:
            continue
        part = parts[spec["part"]]
        placed = part.matrix_world.copy()
        part.parent = None
        part.matrix_world = placed
        bpy.context.view_layer.objects.active = part
        bpy.ops.object.select_all(action="DESELECT")
        part.select_set(True)
        bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
        group = part.vertex_groups.new(name=spec["name"])
        group.add(list(range(len(part.data.vertices))), 1.0, "REPLACE")
        meshes.append(part)
    bpy.ops.object.select_all(action="DESELECT")
    for part in meshes:
        part.select_set(True)
    bpy.context.view_layer.objects.active = meshes[0]
    bpy.ops.object.join()
    body = meshes[0]
    body.name = "body"
    body.parent = armature
    modifier = body.modifiers.new("rig", "ARMATURE")
    modifier.object = armature
    remove([ob for ob in bpy.data.objects if ob not in (armature, body)])
    armature.animation_data_create()
    baked = []
    for name, clip in entry["keys"].items():
        action = bpy.data.actions.new(name)
        action.use_fake_user = True
        armature.animation_data.action = action
        for spec in entry["bones"]:
            pose = armature.pose.bones[spec["name"]]
            pose.rotation_mode = "XYZ"
            for time, turn, *move in clip.get(spec["name"], [[0, [0, 0, 0]], [clip["length"], [0, 0, 0]]]):
                frame = 1 + time * fps
                pose.rotation_euler = Euler([v * 3.141592653589793 / 180 for v in turn], "XYZ")
                pose.location = Vector(move[0] if move else (0, 0, 0))
                pose.keyframe_insert("rotation_euler", frame=frame)
                pose.keyframe_insert("location", frame=frame)
        baked.append(action)
    armature.animation_data.action = None
    return armature, [body], baked


def listed(spec):
    return [spec] if isinstance(spec, str) else spec


def joined(paths):
    """The skins of every file on the first one's armature, grown by the bones the others add."""
    armature, meshes = imported(paths[0])
    for path in paths[1:]:
        other, other_meshes = imported(path)
        known = {bone.name for bone in armature.data.bones}
        bpy.ops.object.select_all(action="DESELECT")
        bpy.context.view_layer.objects.active = other
        bpy.ops.object.mode_set(mode="EDIT")
        for bone in [bone for bone in other.data.edit_bones if bone.name in known]:
            other.data.edit_bones.remove(bone)
        bpy.ops.object.mode_set(mode="OBJECT")
        bpy.ops.object.select_all(action="DESELECT")
        other.select_set(True)
        armature.select_set(True)
        bpy.context.view_layer.objects.active = armature
        bpy.ops.object.join()
        for mesh in other_meshes:
            placed = mesh.matrix_world.copy()
            mesh.parent = armature
            mesh.matrix_world = placed
            for modifier in mesh.modifiers:
                if modifier.type == "ARMATURE":
                    modifier.object = armature
        meshes += other_meshes
    return armature, meshes


def reshape(armature, parents, drop):
    if not parents and not drop:
        return
    bpy.ops.object.select_all(action="DESELECT")
    bpy.context.view_layer.objects.active = armature
    bpy.ops.object.mode_set(mode="EDIT")
    bones = armature.data.edit_bones
    for child, parent in parents.items():
        bones[child].use_connect = False
        bones[child].parent = bones[parent]
    for name in drop:
        bones.remove(bones[name])
    bpy.ops.object.mode_set(mode="OBJECT")


def upright(armature):
    """The armature's turn and scale applied to its bones, so it stands unturned in metres."""
    bpy.ops.object.select_all(action="DESELECT")
    armature.select_set(True)
    bpy.context.view_layer.objects.active = armature
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)


def rebind(armature, pattern, out):
    import glob

    from mathutils import Matrix

    root = bpy.data.objects.new("model", None)
    bpy.context.scene.collection.objects.link(root)
    armature.parent = root
    rest = {bone.name: armature.matrix_world @ bone.matrix_local for bone in armature.data.bones}
    os.makedirs(out, exist_ok=True)
    paths = sorted(glob.glob(pattern))
    if not paths:
        raise ValueError(f"no file matches {pattern}")
    for path in paths:
        source, meshes = imported(path)
        if source.animation_data and source.animation_data.action:
            bpy.context.scene.frame_set(int(source.animation_data.action.frame_range[0]))
        weighted = set()
        for mesh in meshes:
            groups = {group.index: group.name for group in mesh.vertex_groups}
            weighted |= {groups[g.group] for vertex in mesh.data.vertices for g in vertex.groups if g.weight > 0}
        stray = weighted - set(rest)
        if stray:
            raise ValueError(f"{path}: weighted to {sorted(stray)}, which the armature lacks")

        def agrees(names, placed, turns):
            for name in names:
                here, there = placed(name), rest[name]
                if (here.to_translation() - there.to_translation()).length > 1e-3:
                    return False
                if turns and here.to_quaternion().rotation_difference(there.to_quaternion()).angle > 0.02:
                    return False
            return True

        at_rest = agrees(weighted, lambda name: source.matrix_world @ source.data.bones[name].matrix_local, True)
        # A stray bind pose also turns the bones' own axes, so a pose is judged by where it stands
        # every joint the armature knows.
        posed = agrees(
            [bone.name for bone in source.pose.bones if bone.name in rest],
            lambda name: source.matrix_world @ source.pose.bones[name].matrix,
            False,
        )
        if not at_rest and not posed:
            raise ValueError(f"{path}: neither rests nor poses as the armature")
        for mesh in meshes:
            bpy.ops.object.select_all(action="DESELECT")
            bpy.context.view_layer.objects.active = mesh
            for modifier in list(mesh.modifiers):
                if modifier.type == "ARMATURE" and not at_rest:
                    bpy.ops.object.modifier_apply(modifier=modifier.name)
                else:
                    mesh.modifiers.remove(modifier)
            placed = mesh.matrix_world.copy()
            mesh.parent = None
            mesh.matrix_world = placed
            mesh.select_set(True)
            bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
            mesh.parent = armature
            mesh.matrix_parent_inverse = Matrix.Identity(4)
            mesh.modifiers.new("rig", "ARMATURE").object = armature
        remove([source])
        bpy.ops.object.select_all(action="DESELECT")
        for ob in [root, armature, *meshes]:
            ob.select_set(True)
        bpy.ops.export_scene.gltf(
            filepath=os.path.join(out, os.path.splitext(os.path.basename(path))[0] + ".glb"),
            export_format="GLB",
            use_selection=True,
            export_animations=False,
            export_def_bones=False,
            export_morph=False,
            export_apply=False,
            export_yup=True,
        )
        remove(meshes)
    print(f"{out}: {len(paths)} parts")


def retarget(armature, mapping, paths, name, fps):
    """The clips of `paths`, one after another, baked onto the armature by `mapping`, keyed on
    every bone `fps` times a second whatever rate each source was made at."""
    from mathutils import Matrix

    scene = bpy.context.scene
    order = []
    pending = [bone for bone in armature.data.bones if bone.parent is None]
    while pending:
        bone = pending.pop(0)
        order.append(bone)
        pending += list(bone.children)
    rest_world = {bone.name: (armature.matrix_world @ bone.matrix_local).decompose() for bone in order}
    under = {bone.name: bone.parent.matrix_local.inverted() @ bone.matrix_local if bone.parent else bone.matrix_local for bone in order}
    travel = next(bone.name for bone in order if bone.name in mapping and not any(above.name in mapping for above in bone.parent_recursive))
    to_armature = armature.matrix_world.inverted()
    turn_to_armature = armature.matrix_world.decompose()[1].inverted()
    keys = {bone.name: [] for bone in order}
    moves = []
    for path in paths:
        source, source_meshes = imported(path)
        remove(source_meshes)
        rate = scene.render.fps / scene.render.fps_base
        scene.render.fps, scene.render.fps_base = fps, 1.0
        action = source.animation_data.action
        first, last = action.frame_range
        steps = round((last - first) / rate * fps)
        source_rest = {bone.name: source.matrix_world @ bone.matrix_local for bone in source.data.bones}
        root = source.data.bones[mapping[travel]].parent_recursive
        root = root[-1].name if root else None
        for step in range(steps + 1):
            frame = first + step / fps * rate
            scene.frame_set(int(frame), subframe=frame - int(frame))
            carry = Matrix.Identity(4)
            if root:
                carry = source.matrix_world @ source.pose.bones[root].matrix @ source_rest[root].inverted()
            carried = {origin: (carry @ source_rest[origin]).decompose() for origin in mapping.values()}
            height = rest_world[travel][0].z / carried[mapping[travel]][0].z
            posed = {}
            for bone in order:
                attached = posed[bone.parent.name] @ under[bone.name] if bone.parent else under[bone.name]
                origin = mapping.get(bone.name)
                if origin is None:
                    posed[bone.name] = attached
                    keys[bone.name].append(Matrix.Identity(4))
                    continue
                place, turn, _ = (source.matrix_world @ source.pose.bones[origin].matrix).decompose()
                rest_place, rest_turn, _ = carried[origin]
                world_turn = turn @ rest_turn.inverted() @ rest_world[bone.name][1]
                at = attached.to_translation()
                if bone.name == travel:
                    at = at + to_armature.to_3x3() @ ((place - rest_place) * height)
                posed[bone.name] = Matrix.LocRotScale(at, turn_to_armature @ world_turn, attached.to_scale())
                parent_posed = posed[bone.parent.name] if bone.parent else Matrix.Identity(4)
                keys[bone.name].append(under[bone.name].inverted() @ parent_posed.inverted() @ posed[bone.name])
            moves.append(keys[travel][-1].to_translation())
        remove([source])
    action = bpy.data.actions.new(name)
    action.use_fake_user = True
    for bone in order:
        path = f'pose.bones["{bone.name}"]'
        turns = []
        for basis in keys[bone.name]:
            turn = basis.to_quaternion()
            if turns and turn.dot(turns[-1]) < 0:
                turn.negate()
            turns.append(turn)
        channels = [(f"{path}.rotation_quaternion", index, [turn[index] for turn in turns]) for index in range(4)]
        if bone.name == travel:
            channels += [(f"{path}.location", index, [move[index] for move in moves]) for index in range(3)]
        for data_path, index, values in channels:
            curve = action.fcurves.new(data_path, index=index, action_group=bone.name)
            curve.keyframe_points.add(len(values))
            curve.keyframe_points.foreach_set("co", [v for frame, value in enumerate(values) for v in (frame, value)])
            for point in curve.keyframe_points:
                point.interpolation = "LINEAR"
    for bone in armature.pose.bones:
        bone.rotation_mode = "QUATERNION"
    if armature.animation_data is None:
        armature.animation_data_create()
    return action


def imported(path):
    before = set(bpy.data.objects)
    bpy.ops.import_scene.fbx(filepath=path)
    new = set(bpy.data.objects) - before
    armatures = [ob for ob in new if ob.type == "ARMATURE"]
    if len(armatures) != 1:
        raise ValueError(f"{path}: {len(armatures)} armatures, one expected")
    return armatures[0], [ob for ob in new if ob.type == "MESH"]


def paint(material, image_path):
    """The material's base colour becomes the image, so the exported glb carries the pack's
    colours wherever the FBX left the texture unlinked; written as PNG, the one image format
    restyle reads."""
    material.use_nodes = True
    nodes = material.node_tree.nodes
    shader = next((node for node in nodes if node.type == "BSDF_PRINCIPLED"), None) or nodes.new("ShaderNodeBsdfPrincipled")
    output = next((node for node in nodes if node.type == "OUTPUT_MATERIAL"), None) or nodes.new("ShaderNodeOutputMaterial")
    texture = nodes.new("ShaderNodeTexImage")
    image = bpy.data.images.load(image_path, check_existing=True)
    if image.file_format != "PNG":
        image.filepath_raw = os.path.join(bpy.app.tempdir, os.path.splitext(os.path.basename(image_path))[0] + ".png")
        image.file_format = "PNG"
        image.save()
    texture.image = image
    links = material.node_tree.links
    links.new(texture.outputs["Color"], shader.inputs["Base Color"])
    links.new(shader.outputs["BSDF"], output.inputs["Surface"])


def bake(armature, action, first, last, name):
    """The pose the armature shows playing `action` from `first` to `last`, keyed every frame."""
    if armature.animation_data is None:
        armature.animation_data_create()
    armature.animation_data.action = action
    bpy.context.view_layer.objects.active = armature
    bpy.ops.object.select_all(action="DESELECT")
    armature.select_set(True)
    bpy.ops.object.mode_set(mode="POSE")
    bpy.ops.pose.select_all(action="SELECT")
    bpy.ops.nla.bake(
        frame_start=first,
        frame_end=last,
        only_selected=False,
        visual_keying=True,
        clear_constraints=False,
        use_current_action=False,
        bake_types={"POSE"},
    )
    bpy.ops.object.mode_set(mode="OBJECT")
    result = armature.animation_data.action
    result.name = name
    result.use_fake_user = True
    armature.animation_data.action = None
    return result


def remove(objects):
    for ob in objects:
        bpy.data.objects.remove(ob, do_unlink=True)


main()
