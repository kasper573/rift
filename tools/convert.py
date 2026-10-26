"""Turns a pack's FBX character into a glb `restyle.py` reads: its skin, its armature, and one
named clip per animation, each baked from the pose the pack's rig shows (constraints and IK
targets included) and starting at zero.

    blender -b --factory-startup -P convert.py -- KIT PACKS [NAME...]

KIT's `convert` section names each output under PACKS and how it is made: `skin`, the FBX holding
the mesh and armature; `textures`, a base colour image per material name (`*` for every
material), so restyle can sample the pack's colours; `clips`, each clip's name mapped to an FBX
whose own take is the clip, or to `[first, last]` frames of the skin's own take; and `fps`, the
frame rate the takes were made at.

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
    armature, meshes = imported(resolve(entry["skin"], folders, packs))
    for material_name, image in entry.get("textures", {}).items():
        for mesh in meshes:
            for material in mesh.data.materials:
                if material and material_name in ("*", material.name):
                    paint(material, resolve(image, folders, packs))
    own_take = armature.animation_data.action if armature.animation_data else None
    baked = []
    for name, source in entry["clips"].items():
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
