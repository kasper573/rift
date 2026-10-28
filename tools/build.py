#!/usr/bin/env python3
"""Builds the models no pack ships, to the style sheet: three trees with rounded canopies of
lumps shaded smooth on gnarled trunks, the woodsman's cottage, and the small bone-camp things.

    build.py GAME

Writes each model under the game folder, textureless, coloured per vertex from the sheet's
palette and shaded flat, or smooth across the faces a part marks so.
"""

import os
import re
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import glb  # noqa: E402

FLOAT, UBYTE, USHORT, UINT = 5126, 5121, 5123, 5125
ARRAY_BUFFER, ELEMENT_ARRAY_BUFFER = 34962, 34963

BUDGET = {"bytes": 2_000_000, "triangles": 2000}

PHI = (1 + 5**0.5) / 2
ICOSAHEDRON = (
    np.array(
        [
            [-1, PHI, 0], [1, PHI, 0], [-1, -PHI, 0], [1, -PHI, 0],
            [0, -1, PHI], [0, 1, PHI], [0, -1, -PHI], [0, 1, -PHI],
            [PHI, 0, -1], [PHI, 0, 1], [-PHI, 0, -1], [-PHI, 0, 1],
        ],
        np.float64,
    )
    / (1 + PHI**2) ** 0.5,
    [
        [0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11],
        [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8],
        [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9],
        [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1],
    ],
)


def subdivide(vertices, faces):
    """Each face split into four at its edges' midpoints, pushed out onto the unit sphere."""
    vertices = [np.array(v, np.float64) for v in vertices]
    midpoints = {}
    new_faces = []

    def midpoint(a, b):
        key = (min(a, b), max(a, b))
        if key not in midpoints:
            point = (vertices[a] + vertices[b]) / 2
            vertices.append(point / np.linalg.norm(point))
            midpoints[key] = len(vertices) - 1
        return midpoints[key]

    for a, b, c in faces:
        ab, bc, ca = midpoint(a, b), midpoint(b, c), midpoint(c, a)
        new_faces += [[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]
    return np.array(vertices), new_faces


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    game = sys.argv[1]
    palette = read_palette(os.path.join(game, "assets/STYLE.md"))
    for name, build in MODELS.items():
        model = Model(palette)
        build(model, np.random.default_rng(sum(map(ord, name))))
        path = os.path.join(game, f"world/props/{name}.prop/model.glb")
        print(f"world/props/{name}.prop/model.glb: {model.write(path)}")


class Model:
    def __init__(self, palette):
        self.palette = palette
        self.mesh = Mesh()

    def triangles(self, corners, swatch, glow=False, smooth=False):
        """Faces as an (n, 3, 3) array of corners, counter-clockwise seen from outside, in one
        swatch, one per face, or one per corner."""
        corners = np.asarray(corners, np.float64).reshape(-1, 3)
        count = len(corners)
        if isinstance(swatch, str):
            colors = np.tile(self.palette[swatch], (count, 1))
        elif len(swatch) == count:
            colors = np.array([self.palette[name] for name in swatch])
        else:
            colors = np.repeat(np.array([self.palette[name] for name in swatch]), 3, axis=0)
        self.mesh.add(
            colors=colors,
            positions=corners,
            alphas=np.full(count, 0.0 if glow else 1.0),
            smooth=smooth,
        )

    def blob(self, center, radius, swatch, squash=0.75, rng=None, smooth=False, rounds=0, shade=None):
        """A flattened sphere of `rounds` subdivisions of an icosahedron, its vertices nudged so no
        two read alike. With `shade`, a (low, high, swatches) triple, each corner takes the swatch
        its height between low and high falls in, from the first at the bottom up, so the colours
        run into one another over a smooth blob and step from face to face over a flat one."""
        vertices, faces = ICOSAHEDRON
        for _ in range(rounds):
            vertices, faces = subdivide(vertices, faces)
        points = vertices * np.array([1.0, squash, 1.0]) * radius
        if rng is not None:
            points = points * rng.uniform(0.8, 1.2, (len(points), 1))
        points = points + center
        corners = points[np.array(faces)]
        if shade is None:
            self.triangles(corners, swatch, smooth=smooth)
            return
        low, high, swatches = shade
        heights = corners[:, :, 1] if smooth else np.repeat(corners[:, :, 1].mean(axis=1, keepdims=True), 3, axis=1)
        band = np.clip(((heights - low) / max(high - low, 1e-6) * len(swatches)).astype(int), 0, len(swatches) - 1)
        self.triangles(corners, [swatches[b] for b in band.reshape(-1)], smooth=smooth)

    def tube(self, path, radii, swatch, sides=6, cap=True, smooth=False):
        """A tapered, bending tube through `path` points with a radius at each, polygonal, or
        shaded round when `smooth`."""
        rings = []
        for index, (point, radius) in enumerate(zip(path, radii)):
            axis = path[min(index + 1, len(path) - 1)] - path[max(index - 1, 0)]
            axis = axis / np.linalg.norm(axis)
            side = np.cross(axis, [0.0, 0.0, 1.0] if abs(axis[2]) < 0.9 else [1.0, 0.0, 0.0])
            side /= np.linalg.norm(side)
            up = np.cross(side, axis)
            angles = np.arange(sides) * 2 * np.pi / sides
            rings.append(point + radius * (np.outer(np.cos(angles), side) + np.outer(np.sin(angles), up)))
        faces = []
        for low, high in zip(rings, rings[1:]):
            for i in range(sides):
                j = (i + 1) % sides
                faces.append([low[i], low[j], high[j]])
                faces.append([low[i], high[j], high[i]])
        if cap:
            top = rings[-1]
            for i in range(1, sides - 1):
                faces.append([top[0], top[i], top[i + 1]])
        self.triangles(faces, swatch, smooth=smooth)

    def cone(self, base, height, radius, swatch, sides=8, lift=0.0):
        """A cone whose base is lifted by `lift` of its radius toward the middle, so a pine's tiers
        hang like boughs."""
        angles = np.arange(sides) * 2 * np.pi / sides
        rim = base + np.stack([radius * np.cos(angles), np.full(sides, lift), radius * np.sin(angles)], axis=1)
        apex = base + [0.0, height, 0.0]
        center = base + [0.0, 0.0, 0.0]
        faces = []
        for i in range(sides):
            j = (i + 1) % sides
            faces.append([rim[i], apex, rim[j]])
            faces.append([rim[j], center, rim[i]])
        self.triangles(faces, swatch)

    def write(self, path):
        mesh = self.mesh
        mesh.weld()
        mesh.flatten()
        low, high = mesh.bounds()
        shift = np.eye(4)
        shift[:3, 3] = [-(low[0] + high[0]) / 2, -low[1] - 0.02, -(low[2] + high[2]) / 2]
        mesh.transform(shift)
        writer = glb.Writer()
        document = {
            "scene": 0,
            "scenes": [{"nodes": [0]}],
            "nodes": [{"name": "model", "mesh": 0}],
            "meshes": [{"name": "model", "primitives": [mesh.primitive(writer)]}],
            "materials": [MATERIAL],
        }
        size = writer.write(path, document)
        if mesh.triangles() > BUDGET["triangles"] or size > BUDGET["bytes"]:
            sys.exit(f"{path}: {mesh.triangles()} tris, {size} bytes: over the prop budget")
        return f"{mesh.triangles()} tris, {size} bytes, size {np.round(high - low, 2).tolist()} m"


def trunk(model, rng, height, radius, bend=0.35, roots=4):
    steps = 4
    path = [np.array([0.0, -0.1, 0.0])]
    for step in range(1, steps + 1):
        lean = rng.normal(0, bend, 2) * (step / steps)
        path.append(np.array([lean[0], height * step / steps, lean[1]]))
    path = np.array(path)
    radii = [radius * 1.25] + [radius * (1.1 - 0.7 * step / steps) for step in range(1, steps + 1)]
    model.tube(path, radii, "bark", sides=7, smooth=True)
    for root in range(roots):
        angle = root * 2 * np.pi / roots + rng.uniform(-0.3, 0.3)
        direction = np.array([np.cos(angle), 0.0, np.sin(angle)])
        foot = direction * radius * 2.2 + [0.0, -0.1, 0.0]
        knee = direction * radius * 1.1 + [0.0, radius * 0.6, 0.0]
        model.tube(np.array([foot, knee, [0.0, radius * 1.4, 0.0]]), [radius * 0.35, radius * 0.45, radius * 0.6], "bark-dark", sides=5, cap=False, smooth=True)
    return path[-1]


def canopy(model, rng, center, radius, lumps, squash=0.8):
    """A rounded cloud of overlapping lumps shaded smooth, lit yellow-green over its top, a body of
    leaf green, and a dark green only at the very bottom, where the light does not reach anyway."""
    low = center[1] - radius * squash
    high = center[1] + radius * squash * 0.9
    for _ in range(lumps):
        offset = rng.uniform(-1.0, 1.0, 3) * [radius * 0.55, radius * 0.35, radius * 0.55]
        size = radius * rng.uniform(0.38, 0.58)
        model.blob(center + offset, size, None, squash, rng, smooth=True, rounds=1, shade=(low, high, ["leaf-dark", "leaf", "leaf", "leaf-light", "leaf-light"]))


def oak(model, rng, height=5.5, spread=2.6, lumps=15):
    """An oak: a bent trunk with roots under a wide rounded canopy."""
    trunk_height = height - spread * 0.8
    trunk(model, rng, trunk_height, 0.32)
    canopy(model, rng, np.array([0.0, trunk_height + spread * 0.55, 0.0]), spread, lumps)


def tall_oak(model, rng):
    oak(model, rng, height=7.0, spread=2.8, lumps=17)


def pine(model, rng, height=7.0):
    """A pine: a straight trunk under tiers of rounded lumps narrowing to the top."""
    trunk(model, rng, height * 0.3, 0.26, bend=0.1, roots=3)
    tiers = 4
    for tier in range(tiers):
        share = tier / (tiers - 1)
        center = np.array([0.0, height * (0.32 + 0.6 * share), 0.0])
        radius = 2.0 * (1.0 - 0.6 * share)
        canopy(model, rng, center, radius, 5, squash=0.55)


def bone(model, rng, at=np.zeros(3), yaw=0.0, pitch=0.0, length=0.45, swatch="bone"):
    rotation = np.array([[np.cos(yaw), 0.0, np.sin(yaw)], [0.0, 1.0, 0.0], [-np.sin(yaw), 0.0, np.cos(yaw)]]) @ np.array(
        [[np.cos(pitch), -np.sin(pitch), 0.0], [np.sin(pitch), np.cos(pitch), 0.0], [0.0, 0.0, 1.0]]
    )
    place = lambda p: (np.asarray(p) @ rotation.T) + at
    half = length / 2
    shaft = np.array([place([-half, 0.0, 0.0]), place([half, 0.0, 0.0])])
    model.tube(shaft, [0.045, 0.045], swatch, sides=6, cap=False)
    for end in (-1, 1):
        for side in (-1, 1):
            model.blob(place([end * half, 0.0, side * 0.05]), 0.075, swatch, squash=0.9)


def bone_pile(model, rng):
    for index in range(7):
        angle = index * 2 * np.pi / 7
        at = np.array([np.cos(angle), 0.0, np.sin(angle)]) * rng.uniform(0.15, 0.5) + [0.0, 0.05 + 0.04 * (index % 3), 0.0]
        bone(model, rng, at, yaw=rng.uniform(0, np.pi), pitch=rng.uniform(-0.25, 0.25), length=rng.uniform(0.35, 0.55), swatch="bone" if index % 3 else "bone-dark")
    model.blob(np.array([0.0, 0.16, 0.0]), 0.2, "bone", squash=0.85, rng=rng)
    model.blob(np.array([0.05, 0.1, 0.14]), 0.08, "eyes", squash=0.6)
    model.blob(np.array([-0.08, 0.1, 0.14]), 0.08, "eyes", squash=0.6)


def box(model, low, high, swatch, glow=False):
    """An axis-aligned block from corner `low` to corner `high`, faces wound outward."""
    (x0, y0, z0), (x1, y1, z1) = low, high
    c = lambda x, y, z: np.array([x, y, z], np.float64)
    faces = [
        [c(x0, y0, z1), c(x1, y0, z1), c(x1, y1, z1)], [c(x0, y0, z1), c(x1, y1, z1), c(x0, y1, z1)],
        [c(x1, y0, z0), c(x0, y0, z0), c(x0, y1, z0)], [c(x1, y0, z0), c(x0, y1, z0), c(x1, y1, z0)],
        [c(x1, y0, z1), c(x1, y0, z0), c(x1, y1, z0)], [c(x1, y0, z1), c(x1, y1, z0), c(x1, y1, z1)],
        [c(x0, y0, z0), c(x0, y0, z1), c(x0, y1, z1)], [c(x0, y0, z0), c(x0, y1, z1), c(x0, y1, z0)],
        [c(x0, y1, z1), c(x1, y1, z1), c(x1, y1, z0)], [c(x0, y1, z1), c(x1, y1, z0), c(x0, y1, z0)],
        [c(x0, y0, z0), c(x1, y0, z0), c(x1, y0, z1)], [c(x0, y0, z0), c(x1, y0, z1), c(x0, y0, z1)],
    ]
    model.triangles(faces, swatch, glow)


def cottage(model, rng):
    """A log cottage, 6 by 4 m, its door on the +z side: plank walls under a dark gable roof with
    an overhang, a stone chimney, a door and two lit windows."""
    half_x, half_z, eaves, ridge = 3.0, 2.0, 2.6, 4.3
    box(model, (-half_x, 0.0, -half_z), (half_x, eaves, half_z), "planks")
    for sign in (-1, 1):
        gable = [np.array([sign * half_x, eaves, -half_z]), np.array([sign * half_x, eaves, half_z]), np.array([sign * half_x, ridge, 0.0])]
        if sign < 0:
            gable = [gable[1], gable[0], gable[2]]
        model.triangles([gable], "planks")
    over, drop = 0.35, 0.15
    for sign in (-1, 1):
        low_a = np.array([-half_x - over, eaves - drop, sign * (half_z + over)])
        low_b = np.array([half_x + over, eaves - drop, sign * (half_z + over)])
        top_a = np.array([-half_x - over, ridge + 0.1, 0.0])
        top_b = np.array([half_x + over, ridge + 0.1, 0.0])
        quad = [low_a, low_b, top_b, top_a] if sign > 0 else [low_b, low_a, top_a, top_b]
        model.triangles([[quad[0], quad[1], quad[2]], [quad[0], quad[2], quad[3]]], "wood-dark")
        under = [quad[2], quad[1], quad[0]], [quad[3], quad[2], quad[0]]
        model.triangles(list(under), "bark-dark")
    box(model, (1.6, eaves - 0.2, -0.9), (2.3, ridge + 0.5, -0.2), "stone-dark")
    box(model, (1.5, ridge + 0.5, -1.0), (2.4, ridge + 0.7, -0.1), "stone")
    box(model, (-0.6, 0.0, half_z - 0.02), (0.6, 1.9, half_z + 0.06), "wood-dark")
    box(model, (-0.05, 0.9, half_z + 0.06), (0.05, 1.0, half_z + 0.12), "gold")
    for x in (-2.0, 2.0):
        box(model, (x - 0.45, 1.1, half_z - 0.02), (x + 0.45, 1.9, half_z + 0.05), "bark-dark")
        box(model, (x - 0.35, 1.2, half_z + 0.05), (x + 0.35, 1.8, half_z + 0.08), "glow-fire", glow=True)
    box(model, (-half_x - 0.04, 1.1, -0.5), (-half_x + 0.02, 1.9, 0.5), "bark-dark")
    box(model, (-half_x - 0.07, 1.2, -0.4), (-half_x - 0.04, 1.8, 0.4), "glow-fire", glow=True)
    for y in np.arange(0.35, eaves, 0.45):
        for sign in (-1, 1):
            box(model, (-half_x - 0.03, y, sign * half_z - 0.02 * sign), (half_x + 0.03, y + 0.05, sign * half_z + 0.02 * sign), "wood-dark")


def bat_wing(model, rng):
    root = np.zeros(3)
    tips = [np.array([0.5, 0.15, 0.0]), np.array([0.42, -0.12, 0.0]), np.array([0.22, -0.26, 0.0])]
    membrane = [[root, tips[0], tips[1]], [root, tips[1], tips[2]]]
    model.triangles(membrane, "cloth-black")
    model.triangles([[a, c, b] for a, b, c in membrane], "cloth-black")
    for tip in tips:
        model.tube(np.array([root, tip]), [0.03, 0.015], "hair", sides=4)


def tusk(model, rng):
    path = np.array([[0.0, 0.0, 0.0], [0.12, 0.05, 0.0], [0.22, 0.14, 0.0], [0.28, 0.26, 0.0]])
    model.tube(path, [0.06, 0.05, 0.035, 0.012], "bone", sides=6)


MODELS = {
    "oak": oak,
    "oak-tall": tall_oak,
    "pine": pine,
    "cottage": cottage,
    "bone": lambda model, rng: bone(model, rng),
    "bone-pile": bone_pile,
    "bat-wing": bat_wing,
    "tusk": tusk,
}



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


# Over this angle between two faces, in degrees, their shared edge is a corner that shades crisp;
# under it, the faces shade smooth across the edge.
SMOOTH_ANGLE = 48.0

MATERIAL = {
    "name": "style",
    "doubleSided": True,
    "pbrMetallicRoughness": {"baseColorFactor": [1, 1, 1, 1], "metallicFactor": 0, "roughnessFactor": 1},
}


class Mesh:
    """Triangles gathered from every part of a model. Faces share their corners by position, and
    each face carries one colour: a palette-coloured surface has no colour between its faces. A
    face shaded `smooth` shares its corners' normals with the smooth faces around it and may colour
    each corner on its own, so a colour runs across a rounded surface; any other has a normal and a
    colour of its own."""

    def __init__(self):
        self.gathered = []

    def add(self, positions, colors, alphas, smooth=False):
        self.gathered.append((positions, colors, alphas, np.full(len(positions) // 3, smooth)))

    def weld(self):
        """Corners at one position are one vertex, so faces know their neighbours whatever their
        colour; a face's colour is the one most of its corners carry."""
        positions = np.concatenate([g[0] for g in self.gathered]).astype(np.float64)
        colors = np.concatenate([g[1] for g in self.gathered]).astype(np.float32)
        alphas = np.concatenate([g[2] for g in self.gathered]).astype(np.float32)
        self.face_smooth = np.concatenate([g[3] for g in self.gathered])
        _, first, inverse = np.unique(np.round(positions, 5), axis=0, return_index=True, return_inverse=True)
        self.faces = inverse.reshape(-1, 3)
        self.positions = positions[first]
        corners = colors.reshape(-1, 3, 3)
        second_agrees = np.all(np.isclose(corners[:, 1], corners[:, 2]), axis=1)
        self.face_colors = np.where(second_agrees[:, None], corners[:, 1], corners[:, 0])
        self.corner_colors = corners
        self.face_alphas = alphas.reshape(-1, 3)[:, 0]
        faces = self.faces
        distinct = (faces[:, 0] != faces[:, 1]) & (faces[:, 1] != faces[:, 2]) & (faces[:, 0] != faces[:, 2])
        spans = self.positions[faces]
        area = np.linalg.norm(np.cross(spans[:, 1] - spans[:, 0], spans[:, 2] - spans[:, 0]), axis=1)
        kept = distinct & (area > 1e-12)
        self.faces = faces[kept]
        self.face_colors = self.face_colors[kept]
        self.corner_colors = self.corner_colors[kept]
        self.face_alphas = self.face_alphas[kept]
        self.face_smooth = self.face_smooth[kept]

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
        rows = np.concatenate(
            [np.round(positions, 5), np.round(normals, 4), np.round(colors, 4), np.round(alphas[:, None], 3)],
            axis=1,
        )
        _, first, inverse = np.unique(rows, axis=0, return_index=True, return_inverse=True)
        self.indices = inverse.reshape(-1).astype(np.uint32)
        self.positions = positions[first]
        self.normals = normals[first]
        self.colors = colors[first]
        self.alphas = alphas[first]

    def triangles(self):
        return len(self.indices) // 3

    def transform(self, matrix):
        self.positions = (self.positions @ matrix[:3, :3].T + matrix[:3, 3]).astype(np.float32)
        normals = self.normals @ np.linalg.inv(matrix[:3, :3])
        self.normals = (normals / np.linalg.norm(normals, axis=1, keepdims=True)).astype(np.float32)

    def bounds(self):
        return self.positions.min(axis=0), self.positions.max(axis=0)

    def primitive(self, writer):
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
        if len(self.positions) < 65536:
            indices = writer.accessor(self.indices.astype(np.uint16), USHORT, "SCALAR", target=ELEMENT_ARRAY_BUFFER)
        else:
            indices = writer.accessor(self.indices, UINT, "SCALAR", target=ELEMENT_ARRAY_BUFFER)
        return {"attributes": attributes, "indices": indices, "material": 0, "mode": 4}


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

if __name__ == "__main__":
    main()
