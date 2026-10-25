#!/usr/bin/env python3
"""Restyles pack models to the style sheet and writes them into the game folder.

    restyle.py KIT PACKS GAME [NAME...]

KIT is a kit file (see kit.json beside this script): where each model comes from and how it is
cut, scaled and recoloured. PACKS is the folder the packs are extracted in, GAME the game folder.
With NAMEs only the models whose output path contains one of them are built.

A restyled model is textureless, coloured per vertex from the palette of the sheet the kit names,
shaded flat or, when the kit marks it `smooth`, across its faces, cut to the nodes it keeps,
carries only the clips it maps, and is checked against the sheet's budget. Decimation is not implemented: a model over its triangle budget is reported
and the run fails.
"""

import fnmatch
import json
import os
import re
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import glb  # noqa: E402

FLOAT, UBYTE, USHORT, UINT = 5126, 5121, 5123, 5125
ARRAY_BUFFER, ELEMENT_ARRAY_BUFFER = 34962, 34963

BUDGET = {
    "prop": {"bytes": 2_000_000, "triangles": 2000},
    "character": {"bytes": 4_000_000, "triangles": 4000, "joints": 64},
}
CLIP_SECONDS = (0.1, 60.0)


def main():
    if len(sys.argv) < 4:
        sys.exit(__doc__)
    kit_path, packs, game = sys.argv[1:4]
    only = sys.argv[4:]
    kit = json.load(open(kit_path))
    palette = read_palette(os.path.join(game, kit["style"]))
    failures = []
    for out, entry in kit["models"].items():
        if only and not any(name in out for name in only):
            continue
        try:
            report = restyle(entry, kit["packs"], kit.get("materials", {}), packs, os.path.join(game, out), palette)
            print(f"{out}: {report}")
        except Budget as over:
            failures.append(f"{out}: {over}")
            print(f"{out}: OVER BUDGET: {over}")
    if failures:
        sys.exit("\n".join(["models over budget:"] + failures))


class Budget(Exception):
    pass


def read_palette(path):
    """Every `| name | #hex |` row of the sheet, as name -> linear rgb."""
    palette = {}
    for line in open(path):
        match = re.match(r"\|\s*([a-z][a-z0-9-]*)\s*\|\s*(#[0-9A-Fa-f]{6})\s*\|", line)
        if match:
            palette[match.group(1)] = glb.srgb_to_linear(glb.hex_color(match.group(2)))
    if not palette:
        sys.exit(f"{path} holds no palette rows")
    return palette


def restyle(entry, folders, folders_materials, packs, out, palette):
    source = Source(resolve(entry["from"], folders, packs), palette)
    keep = entry.get("keep")
    drop = entry.get("drop", [])
    colors = entry.get("colors", {})
    glow = set(entry.get("glow", []))
    kept = [
        index
        for index, node in enumerate(source.json["nodes"])
        if "mesh" in node
        and (keep is None or matches(node.get("name", ""), keep))
        and not matches(node.get("name", ""), drop)
    ]
    if not kept:
        raise ValueError(f"{entry['from']}: nothing kept")
    skinned = any("skin" in source.json["nodes"][index] for index in kept) and not entry.get("static", False)
    smooth = bool(entry.get("smooth", False))
    mesh = Mesh()
    materials = dict(folders_materials.get(entry["from"].split(":", 1)[0], {}))
    materials.update(colors)
    for index in kept:
        source.gather(index, mesh, materials, glow, skinned, smooth=smooth)
    for part in entry.get("parts", []):
        extra = Source(resolve(part["from"], folders, packs), palette)
        extra.skin = 0
        for index, node in enumerate(extra.json["nodes"]):
            if "mesh" in node and matches(node.get("name", ""), part["keep"]):
                extra.gather(index, mesh, materials, glow, skinned, joints_of=source, smooth=smooth)
        source.materials_met.update(extra.materials_met)
    mesh.weld()
    mesh.quantize_colors(palette, glow)
    budget = BUDGET["character" if skinned else "prop"]
    target = int(entry.get("triangles", budget["triangles"]))
    before = len(mesh.faces)
    if before > target:
        mesh.decimate(target)
    mesh.flatten(float(entry.get("smooth_angle", SMOOTH_ANGLE)))
    if skinned:
        document, writer = source.rig(mesh, entry)
    else:
        document, writer = source.prop(mesh, entry)
    size = writer.write(out, document)
    report = check(source, mesh, document, size, "character" if skinned else "prop")
    if before > target:
        report = f"decimated {before} to {len(mesh.indices) // 3} tris; " + report
    return report


def resolve(spec, folders, packs):
    alias, name = spec.split(":", 1)
    return os.path.join(packs, folders[alias], name)


def matches(name, patterns):
    return any(fnmatch.fnmatchcase(name, pattern) for pattern in patterns)


# Over this angle between two faces, in degrees, their shared edge is a corner that shades crisp;
# under it, the faces shade smooth across the edge.
SMOOTH_ANGLE = 48.0


def corner_normals(faces, face_normals, weighted, face_smooth, smooth_angle):
    """The normal at each corner of each face: the area-weighted mean of the faces meeting at
    that corner's vertex that turn no more than `smooth_angle` from the face, or of every smooth
    face there when both are marked smooth."""
    count = len(faces)
    corner_faces = np.repeat(np.arange(count), 3)
    corner_vertices = faces.reshape(-1)
    order = np.argsort(corner_vertices, kind="stable")
    sorted_vertices = corner_vertices[order]
    sorted_faces = corner_faces[order]
    starts = np.searchsorted(sorted_vertices, corner_vertices, "left")
    ends = np.searchsorted(sorted_vertices, corner_vertices, "right")
    counts = ends - starts
    pair_corner = np.repeat(np.arange(count * 3), counts)
    within = np.arange(counts.sum()) - np.repeat(np.cumsum(counts) - counts, counts)
    pair_other = sorted_faces[np.repeat(starts, counts) + within]
    pair_face = corner_faces[pair_corner]
    facing = (face_normals[pair_face] * face_normals[pair_other]).sum(axis=1)
    shares = (facing >= np.cos(np.radians(smooth_angle))) | (face_smooth[pair_face] & face_smooth[pair_other])
    summed = np.zeros((count * 3, 3))
    np.add.at(summed, pair_corner[shares], weighted[pair_other[shares]])
    return summed / np.maximum(np.linalg.norm(summed, axis=1, keepdims=True), 1e-12)


class Mesh:
    """Triangles gathered from every kept primitive, in the model's space. Faces share their
    corners by position and skinning, and each face carries one colour: a palette-coloured
    surface has no colour between its faces. A face shaded `smooth` shares its corners' normals
    with the smooth faces around it and may colour each corner on its own, so a colour runs
    across a rounded surface; any other has a normal and a colour of its own."""

    def __init__(self):
        self.gathered = []

    def add(self, positions, colors, alphas, joints, weights, smooth=False):
        self.gathered.append((positions, colors, alphas, joints, weights, np.full(len(positions) // 3, smooth)))

    def weld(self):
        """Corners that agree in position and skinning are one vertex, so faces know their
        neighbours whatever their colour; a face's colour is the one most of its corners carry."""
        positions = np.concatenate([g[0] for g in self.gathered]).astype(np.float64)
        colors = np.concatenate([g[1] for g in self.gathered]).astype(np.float32)
        alphas = np.concatenate([g[2] for g in self.gathered]).astype(np.float32)
        joints = np.concatenate([g[3] for g in self.gathered]).astype(np.uint8)
        weights = np.concatenate([g[4] for g in self.gathered]).astype(np.float32)
        self.face_smooth = np.concatenate([g[5] for g in self.gathered])
        rows = np.concatenate([np.round(positions, 5), joints.astype(np.float32), np.round(weights, 4)], axis=1)
        _, first, inverse = np.unique(rows, axis=0, return_index=True, return_inverse=True)
        self.faces = inverse.reshape(-1, 3)
        self.positions = positions[first]
        self.joints = joints[first]
        self.weights = weights[first]
        corners = colors.reshape(-1, 3, 3)
        second_agrees = np.all(np.isclose(corners[:, 1], corners[:, 2]), axis=1)
        self.face_colors = np.where(second_agrees[:, None], corners[:, 1], corners[:, 0])
        self.corner_colors = corners
        self.face_alphas = alphas.reshape(-1, 3)[:, 0]
        self.drop_degenerate()

    def drop_degenerate(self):
        faces = self.faces
        distinct = (faces[:, 0] != faces[:, 1]) & (faces[:, 1] != faces[:, 2]) & (faces[:, 0] != faces[:, 2])
        corners = self.positions[faces]
        area = np.linalg.norm(np.cross(corners[:, 1] - corners[:, 0], corners[:, 2] - corners[:, 0]), axis=1)
        kept = distinct & (area > 1e-12)
        self.faces = faces[kept]
        self.face_colors = self.face_colors[kept]
        self.corner_colors = self.corner_colors[kept]
        self.face_alphas = self.face_alphas[kept]
        self.face_smooth = self.face_smooth[kept]

    def decimate(self, target):
        """Collapses the cheapest edges, by quadric error, onto one of their ends until no more
        than `target` faces remain. A vertex on a colour border or an open edge only slides along
        that border, so the lines the artist drew stay where they are, and no collapse turns a
        face over."""
        import heapq

        positions = self.positions
        faces = [list(face) for face in self.faces]
        shades = np.unique(np.concatenate([self.face_colors, self.face_alphas[:, None]], axis=1), axis=0, return_inverse=True)[1].reshape(-1)
        alive_face = [True] * len(faces)
        count = len(faces)
        faces_of = [set() for _ in positions]
        for index, face in enumerate(faces):
            for vertex in face:
                faces_of[vertex].add(index)
        quadrics = np.zeros((len(positions), 4, 4))
        for face in faces:
            a, b, c = positions[face]
            normal = np.cross(b - a, c - a)
            normal /= np.linalg.norm(normal)
            plane = np.append(normal, -normal @ a)
            quadric = np.outer(plane, plane)
            for vertex in face:
                quadrics[vertex] += quadric
        alive = [True] * len(positions)

        def shades_of(vertex):
            return {shades[index] for index in faces_of[vertex]}

        def on_border(vertex):
            if len(shades_of(vertex)) > 1:
                return True
            edges = {}
            for index in faces_of[vertex]:
                for other in faces[index]:
                    if other != vertex:
                        edges[other] = edges.get(other, 0) + 1
            return any(count == 1 for count in edges.values())

        def along_border(gone, kept, shared):
            return len(shared) == 1 or len({shades[index] for index in shared}) > 1

        def cost(a, b):
            quadric = quadrics[a] + quadrics[b]
            pa = np.append(positions[a], 1.0)
            pb = np.append(positions[b], 1.0)
            onto_a = pa @ quadric @ pa
            onto_b = pb @ quadric @ pb
            return (onto_b, a, b) if onto_b < onto_a else (onto_a, b, a)

        heap = []
        pushed = set()

        def push(a, b):
            key = (min(a, b), max(a, b))
            if key in pushed:
                return
            pushed.add(key)
            heapq.heappush(heap, (cost(a, b)[0], key))

        for face in faces:
            for i in range(3):
                push(face[i], face[(i + 1) % 3])

        def flips(gone, kept):
            for index in faces_of[gone]:
                face = faces[index]
                if kept in face:
                    continue
                before = faces_normal(positions[face])
                moved = [kept if v == gone else v for v in face]
                after = faces_normal(positions[moved])
                if before @ after <= 0.0:
                    return True
            return False

        while count > target and heap:
            _, (a, b) = heapq.heappop(heap)
            pushed.discard((a, b))
            if not alive[a] or not alive[b]:
                continue
            shared = faces_of[a] & faces_of[b]
            if not shared:
                continue
            _, gone, kept = cost(a, b)
            if on_border(gone) and not (along_border(gone, kept, shared) and shades_of(gone) <= shades_of(kept)):
                if on_border(kept) and not along_border(kept, gone, shared):
                    continue
                gone, kept = kept, gone
                if on_border(gone) and not (along_border(gone, kept, shared) and shades_of(gone) <= shades_of(kept)):
                    continue
            if flips(gone, kept):
                continue
            for index in shared:
                alive_face[index] = False
                count -= 1
                for vertex in faces[index]:
                    faces_of[vertex].discard(index)
            for index in list(faces_of[gone]):
                faces[index] = [kept if v == gone else v for v in faces[index]]
                faces_of[kept].add(index)
            faces_of[gone] = set()
            alive[gone] = False
            quadrics[kept] += quadrics[gone]
            for index in faces_of[kept]:
                for vertex in faces[index]:
                    if vertex != kept:
                        push(kept, vertex)
        kept_faces = [index for index in range(len(faces)) if alive_face[index]]
        self.faces = np.array([faces[index] for index in kept_faces], dtype=np.int64).reshape(-1, 3)
        self.face_colors = self.face_colors[kept_faces]
        self.corner_colors = self.corner_colors[kept_faces]
        self.face_alphas = self.face_alphas[kept_faces]
        self.face_smooth = self.face_smooth[kept_faces]
        self.drop_degenerate()

    def flatten(self, smooth_angle=SMOOTH_ANGLE):
        """Every face gets its own three vertices, its normal and its colour; vertices that then
        agree in everything are shared again. A corner's normal is the area-weighted mean of the
        faces meeting there that turn no more than `smooth_angle` degrees from its own, so a
        rounded surface shades smooth across its facets while a real corner stays crisp; a face
        marked smooth shares with every smooth face at the corner whatever the angle. A flat face
        carries its colour on all its corners; a smooth face carries each corner's own, so a colour
        may run across a rounded surface."""
        faces = self.faces
        corners = self.positions[faces]
        weighted = np.cross(corners[:, 1] - corners[:, 0], corners[:, 2] - corners[:, 0])
        face_normals = weighted / np.linalg.norm(weighted, axis=1, keepdims=True)
        normals = corner_normals(faces, face_normals, weighted, self.face_smooth, smooth_angle)
        smooth_corners = np.repeat(self.face_smooth, 3)
        flat = faces.reshape(-1)
        positions = self.positions[flat].astype(np.float32)
        normals = normals.astype(np.float32)
        colors = np.repeat(self.face_colors, 3, axis=0)
        colors[smooth_corners] = self.corner_colors.reshape(-1, 3)[smooth_corners]
        alphas = np.repeat(self.face_alphas, 3)
        joints = self.joints[flat]
        weights = self.weights[flat]
        rows = np.concatenate(
            [np.round(positions, 5), np.round(normals, 4), np.round(colors, 4), np.round(alphas[:, None], 3), joints.astype(np.float32), np.round(weights, 4)],
            axis=1,
        )
        _, first, inverse = np.unique(rows, axis=0, return_index=True, return_inverse=True)
        self.indices = inverse.reshape(-1).astype(np.uint32)
        self.positions = positions[first]
        self.normals = normals[first]
        self.colors = colors[first]
        self.alphas = alphas[first]
        self.joints = joints[first]
        self.weights = weights[first]

    def quantize_colors(self, palette, glow):
        """Every face's colour becomes the nearest swatch, in Oklab; a glow swatch is reached only
        when the kit lists it for the model, so nothing lights up by accident, and what lands on
        one glows."""
        names = [name for name in palette if not name.startswith("glow-") or name in glow]
        swatches = glb.linear_to_oklab(np.array([palette[name] for name in names]))
        lab = glb.linear_to_oklab(self.face_colors)
        nearest = np.argmin(((lab[:, None, :] - swatches[None, :, :]) ** 2).sum(axis=2), axis=1)
        exact = {tuple(np.round(color, 5)): name for name, color in palette.items()}
        chosen = [exact.get(tuple(np.round(color, 5)), names[i]) for color, i in zip(self.face_colors, nearest)]
        self.face_colors = np.array([palette[name] for name in chosen], np.float32).reshape(-1, 3)
        self.corner_colors = np.repeat(self.face_colors[:, None, :], 3, axis=1)
        self.face_alphas = np.where([name in glow for name in chosen], 0.0, self.face_alphas).astype(np.float32)
        self.swatches = sorted(set(chosen))

    def triangles(self):
        return len(self.indices) // 3

    def transform(self, matrix):
        self.positions = (self.positions @ matrix[:3, :3].T + matrix[:3, 3]).astype(np.float32)
        normals = self.normals @ np.linalg.inv(matrix[:3, :3])
        self.normals = (normals / np.linalg.norm(normals, axis=1, keepdims=True)).astype(np.float32)

    def bounds(self):
        return self.positions.min(axis=0), self.positions.max(axis=0)

    def primitive(self, writer, skinned):
        attributes = {
            "POSITION": writer.accessor(self.positions, FLOAT, "VEC3", target=ARRAY_BUFFER, bounds=True),
            "NORMAL": writer.accessor(self.normals, FLOAT, "VEC3", target=ARRAY_BUFFER),
            "COLOR_0": writer.accessor(
                np.round(np.concatenate([glb.linear_to_srgb(self.colors), self.alphas[:, None]], axis=1) * 255),
                UBYTE,
                "VEC4",
                normalized=True,
                target=ARRAY_BUFFER,
            ),
        }
        if skinned:
            attributes["JOINTS_0"] = writer.accessor(self.joints, UBYTE, "VEC4", target=ARRAY_BUFFER)
            attributes["WEIGHTS_0"] = writer.accessor(self.weights, FLOAT, "VEC4", target=ARRAY_BUFFER)
        if len(self.positions) < 65536:
            indices = writer.accessor(self.indices.astype(np.uint16), USHORT, "SCALAR", target=ELEMENT_ARRAY_BUFFER)
        else:
            indices = writer.accessor(self.indices, UINT, "SCALAR", target=ELEMENT_ARRAY_BUFFER)
        return {"attributes": attributes, "indices": indices, "material": 0, "mode": 4}


def faces_normal(corners):
    normal = np.cross(corners[1] - corners[0], corners[2] - corners[0])
    length = np.linalg.norm(normal)
    return normal / length if length > 0 else normal


MATERIAL = {
    "name": "style",
    "doubleSided": True,
    "pbrMetallicRoughness": {"baseColorFactor": [1, 1, 1, 1], "metallicFactor": 0, "roughnessFactor": 1},
}


class Source:
    def __init__(self, path, palette):
        self.gltf = glb.Gltf(path)
        self.palette = palette
        self.json = self.gltf.json
        self.parent = {}
        for index, node in enumerate(self.json["nodes"]):
            for child in node.get("children", []):
                self.parent[child] = index
        self.images = {}
        self.skin = None
        self.materials_met = {}

    def local(self, index):
        node = self.json["nodes"][index]
        if "matrix" in node:
            return np.array(node["matrix"], np.float64).reshape(4, 4).T
        matrix = np.eye(4)
        x, y, z, w = node.get("rotation", [0, 0, 0, 1])
        rotation = np.array(
            [
                [1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
                [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
                [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)],
            ]
        )
        matrix[:3, :3] = rotation * np.array(node.get("scale", [1, 1, 1]))
        matrix[:3, 3] = node.get("translation", [0, 0, 0])
        return matrix

    def world(self, index):
        matrix = self.local(index)
        while index in self.parent:
            index = self.parent[index]
            matrix = self.local(index) @ matrix
        return matrix

    def ancestors(self, index):
        while index in self.parent:
            index = self.parent[index]
            yield index

    def color_of(self, primitive, positions_count, uvs, vertex_colors, colors, glow):
        """Linear rgb and lit alpha per vertex: the material's factor, its texture at the
        vertex's uv, and the vertex's own colour, multiplied; or the swatch the kit names."""
        material_index = primitive.get("material")
        material = self.json["materials"][material_index] if material_index is not None else {}
        name = material.get("name", "")
        alpha = 0.0 if name in glow else 1.0
        self.materials_met[name] = colors.get(name, "nearest")
        if name in colors:
            return np.tile(self.hex_or_swatch(colors[name]), (positions_count, 1)), np.full(positions_count, alpha)
        pbr = material.get("pbrMetallicRoughness", {})
        factor = np.array(pbr.get("baseColorFactor", [1, 1, 1, 1])[:3], np.float64)
        rgb = np.tile(factor, (positions_count, 1))
        texture = pbr.get("baseColorTexture")
        if texture is not None and uvs is not None:
            image_index = self.json["textures"][texture["index"]]["source"]
            if image_index not in self.images:
                self.images[image_index] = self.gltf.image(image_index)
            image = self.images[image_index]
            height, width = image.shape[:2]
            u = np.mod(uvs[:, 0], 1.0)
            v = np.mod(uvs[:, 1], 1.0)
            x = np.clip((u * width).astype(int), 0, width - 1)
            y = np.clip((v * height).astype(int), 0, height - 1)
            rgb = rgb * glb.srgb_to_linear(image[y, x, :3] / 255)
        if vertex_colors is not None:
            rgb = rgb * vertex_colors[:, :3]
        for key, swatch in colors.items():
            if key.startswith("#"):
                target = glb.srgb_to_linear(glb.hex_color(key))
                close = np.abs(rgb - target).max(axis=1) < 0.02
                rgb[close] = self.hex_or_swatch(swatch)
        return rgb, np.full(positions_count, alpha)

    def hex_or_swatch(self, value):
        if value.startswith("#"):
            return glb.srgb_to_linear(glb.hex_color(value))
        return self.palette[value]

    def gather(self, index, mesh, colors, glow, skinned, joints_of=None, smooth=False):
        node = self.json["nodes"][index]
        posed = None
        if skinned:
            if "skin" in node:
                if self.skin is None:
                    self.skin = node["skin"]
                elif self.skin != node["skin"]:
                    raise ValueError("kept meshes ride on different skins")
                bake = None
            else:
                bake = self.rigid_binding(index, joints_of)
        elif "skin" in node:
            self.skin = node["skin"]
            bake = None
            posed = self.rest_matrices()
        elif self.json.get("skins") and any(ancestor in self.json["skins"][0]["joints"] for ancestor in self.ancestors(index)):
            self.skin = 0
            bake = (self.rest_matrices()[self.rigid_binding(index)[1]] @ self.rigid_binding(index)[0], None)
        else:
            bake = (self.world(index), None)
        for primitive in self.json["meshes"][node["mesh"]]["primitives"]:
            if primitive.get("mode", 4) != 4:
                continue
            attributes = primitive["attributes"]
            positions = self.gltf.accessor(attributes["POSITION"]).astype(np.float64)
            uvs = self.gltf.accessor(attributes["TEXCOORD_0"]) if "TEXCOORD_0" in attributes else None
            vertex_colors = self.gltf.accessor(attributes["COLOR_0"]) if "COLOR_0" in attributes else None
            if vertex_colors is not None and vertex_colors.dtype != np.float32:
                vertex_colors = vertex_colors.astype(np.float32) / np.iinfo(vertex_colors.dtype).max
            rgb, alpha = self.color_of(primitive, len(positions), uvs, vertex_colors, colors, glow)
            if bake is not None:
                matrix, joint = bake
                positions = positions @ matrix[:3, :3].T + matrix[:3, 3]
                joints = np.zeros((len(positions), 4), np.uint8)
                weights = np.zeros((len(positions), 4), np.float32)
                if joint is not None:
                    joints[:, 0] = joint
                    weights[:, 0] = 1.0
            else:
                joints = self.gltf.accessor(attributes["JOINTS_0"]).astype(np.uint8)
                weights = self.gltf.accessor(attributes["WEIGHTS_0"]).astype(np.float32)
                if posed is not None:
                    positions = pose(positions, joints, weights, posed)
                    joints = np.zeros_like(joints)
                    weights = np.zeros_like(weights)
            if "indices" in primitive:
                order = self.gltf.accessor(primitive["indices"]).reshape(-1)
            else:
                order = np.arange(len(positions))
            if bake is not None and np.linalg.det(bake[0][:3, :3]) < 0:
                order = order.reshape(-1, 3)[:, ::-1].reshape(-1)
            mesh.add(positions[order], rgb[order], alpha[order], joints[order], weights[order], smooth)

    def rigid_binding(self, index, joints_of=None):
        """A mesh that rides on a joint unskinned is bound to that joint: its vertices moved into
        the skin's bind space so the joint carries them as a skinned vertex with one weight. A
        part taken from another rig binds to the joint of the same name in `joints_of`."""
        joints = self.json["skins"][self.skin if self.skin is not None else 0]["joints"]
        matrix = self.local(index)
        for ancestor in self.ancestors(index):
            if ancestor in joints:
                joint = joints.index(ancestor)
                bind = np.linalg.inv(self.inverse_bind()[joint])
                if joints_of is not None and joints_of is not self:
                    name = self.json["nodes"][ancestor].get("name")
                    names = [joints_of.json["nodes"][j].get("name") for j in joints_of.json["skins"][joints_of.skin]["joints"]]
                    if name not in names:
                        raise ValueError(f"the rig has no joint named {name!r} to carry {self.json['nodes'][index].get('name')}")
                    joint = names.index(name)
                    bind = np.linalg.inv(joints_of.inverse_bind()[joint])
                return bind @ matrix, joint
            matrix = self.local(ancestor) @ matrix
        raise ValueError(f"mesh node {self.json['nodes'][index].get('name')} rides on no joint")

    def rest_matrices(self):
        """Each joint's matrix in the rig's rest pose, taking a bind-space vertex to the model."""
        skin = self.json["skins"][self.skin]
        inverse_bind = self.inverse_bind()
        return np.array([self.world(joint) @ inverse_bind[i] for i, joint in enumerate(skin["joints"])])

    def inverse_bind(self):
        skin = self.json["skins"][self.skin if self.skin is not None else 0]
        if "inverseBindMatrices" not in skin:
            return np.tile(np.eye(4), (len(skin["joints"]), 1, 1))
        return self.gltf.accessor(skin["inverseBindMatrices"]).reshape(-1, 4, 4).transpose(0, 2, 1).astype(np.float64)

    def prop(self, mesh, entry):
        """One node holding the whole mesh, turned, scaled, centred on its base and sunk."""
        scale = float(entry.get("scale", 1.0))
        yaw = np.radians(float(entry.get("yaw", 0.0)))
        matrix = np.eye(4)
        matrix[:3, :3] = np.array([[np.cos(yaw), 0, np.sin(yaw)], [0, 1, 0], [-np.sin(yaw), 0, np.cos(yaw)]]) * scale
        mesh.transform(matrix)
        low, high = mesh.bounds()
        offset = np.array(entry.get("offset", [0, 0, 0]), np.float64)
        if entry.get("center", True):
            offset = offset - [(low[0] + high[0]) / 2, low[1] + entry.get("sink", 0.02), (low[2] + high[2]) / 2]
        shift = np.eye(4)
        shift[:3, 3] = offset
        mesh.transform(shift)
        writer = glb.Writer()
        document = {
            "scene": 0,
            "scenes": [{"nodes": [0]}],
            "nodes": [{"name": "model", "mesh": 0}],
            "meshes": [{"name": "model", "primitives": [mesh.primitive(writer, skinned=False)]}],
            "materials": [MATERIAL],
        }
        return document, writer

    def rig(self, mesh, entry):
        """The skin's joints and their ancestors, the one skinned mesh, and the mapped clips,
        under a root that scales and turns the whole rig. A scale on a node above the joints (an
        armature exported in centimetres) is baked away: the nodes under it move that much
        further apart, the vertices grow by it and the bind matrices follow, so every rig's
        joints stand in the model's metres and skin alike."""
        skin = self.json["skins"][self.skin]
        kept_nodes = set(skin["joints"])
        for joint in skin["joints"]:
            kept_nodes.update(self.ancestors(joint))
        order = [index for index in range(len(self.json["nodes"])) if index in kept_nodes]
        renumber = {old: new + 1 for new, old in enumerate(order)}
        nodes = [{"name": "model"}]
        for old in order:
            source = self.json["nodes"][old]
            node = {key: source[key] for key in ("name", "translation", "rotation", "scale", "matrix") if key in source}
            children = [renumber[child] for child in source.get("children", []) if child in renumber]
            if children:
                node["children"] = children
            nodes.append(node)
        grown = {old: 1.0 for old in order}
        for old in order:
            node = nodes[renumber[old]]
            factor = node.get("scale")
            if old in skin["joints"] or factor is None or abs(factor[0] - 1.0) < 1e-6:
                continue
            if max(abs(factor[0] - factor[1]), abs(factor[0] - factor[2])) > 1e-6:
                raise ValueError(f"{node.get('name')}: a rig's armature may only be scaled alike on every axis")
            node["scale"] = [1.0, 1.0, 1.0]
            for below in order:
                if old in self.ancestors(below):
                    grown[below] *= factor[0]
                    if "translation" in nodes[renumber[below]]:
                        nodes[renumber[below]]["translation"] = [float(v * factor[0]) for v in nodes[renumber[below]]["translation"]]
        growth = {grown[joint] for joint in skin["joints"]}
        if len(growth) != 1:
            raise ValueError("a rig's joints must all stand under the same scale")
        growth = growth.pop()
        roots = [renumber[old] for old in order if old not in self.parent or self.parent[old] not in renumber]
        mesh_node = len(nodes)
        nodes.append({"name": "skin", "mesh": 0, "skin": 0})
        nodes[0]["children"] = roots + [mesh_node]
        scale = float(entry.get("scale", 1.0))
        yaw = float(entry.get("yaw", 0.0))
        nodes[0]["scale"] = [scale, scale, scale]
        nodes[0]["rotation"] = [0, float(np.sin(np.radians(yaw) / 2)), 0, float(np.cos(np.radians(yaw) / 2))]
        nodes[0]["translation"] = [float(v) for v in entry.get("offset", [0, 0, 0])]
        writer = glb.Writer()
        inverse_bind = self.inverse_bind()
        grown_bind = inverse_bind.copy()
        grown_bind[:, :3, 3] *= growth
        joints = [renumber[joint] for joint in skin["joints"]]
        skins = [{"name": "skin", "joints": joints, "inverseBindMatrices": writer.accessor(grown_bind.transpose(0, 2, 1).reshape(-1, 16), FLOAT, "MAT4")}]
        animations = []
        self.clip_lengths = {}
        for action, clip_name in entry.get("clips", {}).items():
            source = next((a for a in self.json.get("animations", []) if a.get("name") == clip_name), None)
            if source is None:
                raise ValueError(f"no clip named {clip_name!r} for {action}; the model has {[a.get('name') for a in self.json.get('animations', [])]}")
            samplers = []
            channels = []
            length = 0.0
            for channel in source["channels"]:
                target = channel["target"]
                if target.get("node") not in renumber:
                    continue
                sampler = source["samplers"][channel["sampler"]]
                times = self.gltf.accessor(sampler["input"]).astype(np.float32)
                values = self.gltf.accessor(sampler["output"]).astype(np.float32)
                if target["path"] == "translation":
                    values = values * np.float32(grown[target["node"]])
                elif target["path"] == "scale" and nodes[renumber[target["node"]]].get("scale") == [1.0, 1.0, 1.0] and target["node"] not in skin["joints"]:
                    values = values / np.float32(self.json["nodes"][target["node"]].get("scale", [1.0])[0])
                length = max(length, float(times.max()))
                samplers.append(
                    {
                        "input": writer.accessor(times, FLOAT, "SCALAR", bounds=True),
                        "output": writer.accessor(values, FLOAT, {1: "SCALAR", 3: "VEC3", 4: "VEC4"}[values.shape[1]]),
                        "interpolation": sampler.get("interpolation", "LINEAR"),
                    }
                )
                channels.append({"sampler": len(samplers) - 1, "target": {"node": renumber[target["node"]], "path": target["path"]}})
            if not channels:
                raise ValueError(f"clip {clip_name!r} animates nothing kept")
            self.clip_lengths[action] = length
            animations.append({"name": action, "samplers": samplers, "channels": channels})
        self.bind_bounds = self.rest_bounds(mesh, inverse_bind, scale)
        mesh.transform(np.diag([growth, growth, growth, 1.0]))
        document = {
            "scene": 0,
            "scenes": [{"nodes": [0]}],
            "nodes": nodes,
            "meshes": [{"name": "model", "primitives": [mesh.primitive(writer, skinned=True)]}],
            "skins": skins,
            "materials": [MATERIAL],
        }
        if animations:
            document["animations"] = animations
        return document, writer

    def rest_bounds(self, mesh, inverse_bind, scale):
        """Where the skinned vertices stand in the rig's rest pose, scaled as the root scales them."""
        placed = pose(mesh.positions, mesh.joints, mesh.weights, self.rest_matrices())
        return placed.min(axis=0) * scale, placed.max(axis=0) * scale


def pose(positions, joints, weights, matrices):
    homogeneous = np.concatenate([positions, np.ones((len(positions), 1))], axis=1).astype(np.float64)
    placed = np.zeros((len(positions), 3))
    for slot in range(4):
        placed += weights[:, slot, None] * np.einsum("nij,nj->ni", matrices[joints[:, slot]], homogeneous)[:, :3]
    return placed


def check(source, mesh, document, size, kind):
    budget = BUDGET[kind]
    problems = []
    if mesh.triangles() > budget["triangles"]:
        problems.append(f"{mesh.triangles()} triangles, at most {budget['triangles']} (decimation is not implemented)")
    if size > budget["bytes"]:
        problems.append(f"{size} bytes, at most {budget['bytes']}")
    if kind == "character":
        joints = len(document["skins"][0]["joints"])
        if joints > budget["joints"]:
            problems.append(f"{joints} joints, at most {budget['joints']}")
        for action, length in source.clip_lengths.items():
            if not CLIP_SECONDS[0] <= length <= CLIP_SECONDS[1]:
                problems.append(f"clip {action} lasts {length:.2f} s, not {CLIP_SECONDS[0]} to {CLIP_SECONDS[1]}")
    if problems:
        raise Budget("; ".join(problems))
    materials = " ".join(f"{name or '(unnamed)'}→{swatch}" for name, swatch in sorted(source.materials_met.items()))
    report = f"{mesh.triangles()} tris, {len(mesh.positions)} verts, {size} bytes, materials {materials}, swatches {' '.join(mesh.swatches)}"
    if kind == "prop":
        low, high = mesh.bounds()
        report += f", size {np.round(high - low, 2).tolist()} m"
    else:
        low, high = source.bind_bounds
        clips = " ".join(f"{action} {length:.2f}s" for action, length in source.clip_lengths.items())
        report += f", {len(document['skins'][0]['joints'])} joints, rest pose {np.round(low, 2).tolist()} to {np.round(high, 2).tolist()} m, clips {clips}"
    return report


if __name__ == "__main__":
    main()
