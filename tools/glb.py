"""Reading and writing glTF binaries and PNGs with nothing but the standard library and numpy."""

import base64
import json
import os
import struct
import zlib

import numpy as np

COMPONENT = {
    5120: np.int8,
    5121: np.uint8,
    5122: np.int16,
    5123: np.uint16,
    5125: np.uint32,
    5126: np.float32,
}
WIDTH = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4, "MAT4": 16}


class Gltf:
    """A glTF document with its binary buffers, read from a .glb or a .gltf beside its files."""

    def __init__(self, path):
        self.path = path
        data = open(path, "rb").read()
        if data[:4] == b"glTF":
            length = struct.unpack_from("<I", data, 12)[0]
            self.json = json.loads(data[20 : 20 + length])
            offset = 20 + length
            binary = b""
            if offset < len(data):
                chunk = struct.unpack_from("<I", data, offset)[0]
                binary = data[offset + 8 : offset + 8 + chunk]
            self.buffers = [binary]
        else:
            self.json = json.loads(data)
            self.buffers = [self._uri(buffer["uri"]) for buffer in self.json["buffers"]]

    def _uri(self, uri):
        if uri.startswith("data:"):
            return base64.b64decode(uri.split(",", 1)[1])
        return open(os.path.join(os.path.dirname(self.path), uri), "rb").read()

    def view(self, index):
        view = self.json["bufferViews"][index]
        start = view.get("byteOffset", 0)
        return self.buffers[view["buffer"]][start : start + view["byteLength"]], view.get("byteStride")

    def accessor(self, index):
        """The accessor's values as an array of shape (count, width), floats denormalized."""
        accessor = self.json["accessors"][index]
        dtype = COMPONENT[accessor["componentType"]]
        width = WIDTH[accessor["type"]]
        count = accessor["count"]
        if "bufferView" not in accessor:
            values = np.zeros((count, width), dtype)
        else:
            data, stride = self.view(accessor["bufferView"])
            size = np.dtype(dtype).itemsize * width
            offset = accessor.get("byteOffset", 0)
            if stride and stride != size:
                rows = np.frombuffer(data, np.uint8, count=stride * (count - 1) + size, offset=offset)
                rows = np.lib.stride_tricks.as_strided(rows, (count, size), (stride, 1))
                values = np.ascontiguousarray(rows).view(dtype).reshape(count, width)
            else:
                values = np.frombuffer(data, dtype, count=count * width, offset=offset).reshape(count, width)
        if "sparse" in accessor:
            raise ValueError(f"{self.path}: sparse accessors are not supported")
        if accessor.get("normalized"):
            return values.astype(np.float32) / np.iinfo(dtype).max
        return values

    def image(self, index):
        """An image as an (h, w, 4) uint8 array."""
        image = self.json["images"][index]
        if "bufferView" in image:
            data, _ = self.view(image["bufferView"])
        else:
            data = self._uri(image["uri"])
        return read_png_bytes(data)


class Writer:
    """Builds one glTF binary with one buffer."""

    def __init__(self):
        self.chunks = []
        self.length = 0
        self.views = []
        self.accessors = []

    def _view(self, data, target=None):
        data = bytes(data)
        self.chunks.append(data)
        view = {"buffer": 0, "byteOffset": self.length, "byteLength": len(data)}
        if target:
            view["target"] = target
        self.length += len(data)
        padding = (-len(data)) % 4
        if padding:
            self.chunks.append(b"\0" * padding)
            self.length += padding
        self.views.append(view)
        return len(self.views) - 1

    def accessor(self, values, component, kind, normalized=False, target=None, bounds=False):
        values = np.ascontiguousarray(values, dtype=COMPONENT[component])
        count = values.shape[0]
        accessor = {
            "bufferView": self._view(values.tobytes(), target),
            "componentType": component,
            "count": count,
            "type": kind,
        }
        if normalized:
            accessor["normalized"] = True
        if bounds:
            accessor["min"] = [float(v) for v in values.min(axis=0)]
            accessor["max"] = [float(v) for v in values.max(axis=0)]
        self.accessors.append(accessor)
        return len(self.accessors) - 1

    def write(self, path, document):
        document = dict(document)
        document["asset"] = {"version": "2.0", "generator": "rift restyle"}
        document["bufferViews"] = self.views
        document["accessors"] = self.accessors
        document["buffers"] = [{"byteLength": self.length}]
        text = json.dumps(document, separators=(",", ":")).encode()
        text += b" " * ((-len(text)) % 4)
        binary = b"".join(self.chunks)
        total = 12 + 8 + len(text) + 8 + len(binary)
        os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
        with open(path, "wb") as out:
            out.write(struct.pack("<4sII", b"glTF", 2, total))
            out.write(struct.pack("<I4s", len(text), b"JSON"))
            out.write(text)
            out.write(struct.pack("<I4s", len(binary), b"BIN\0"))
            out.write(binary)
        return total


def read_png(path):
    return read_png_bytes(open(path, "rb").read())


def read_png_bytes(data):
    """Decodes an 8-bit, non-interlaced PNG to an (h, w, 4) uint8 array."""
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("not a PNG")
    position = 8
    idat = []
    palette = None
    transparency = None
    width = height = depth = kind = interlace = None
    while position < len(data):
        length, name = struct.unpack_from(">I4s", data, position)
        body = data[position + 8 : position + 8 + length]
        position += 12 + length
        if name == b"IHDR":
            width, height, depth, kind, _, _, interlace = struct.unpack(">IIBBBBB", body)
        elif name == b"PLTE":
            palette = np.frombuffer(body, np.uint8).reshape(-1, 3)
        elif name == b"tRNS":
            transparency = np.frombuffer(body, np.uint8)
        elif name == b"IDAT":
            idat.append(body)
    if depth != 8 or interlace:
        raise ValueError(f"only 8-bit non-interlaced PNGs are read, not depth {depth} interlace {interlace}")
    channels = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}[kind]
    stride = width * channels
    raw = np.frombuffer(zlib.decompress(b"".join(idat)), np.uint8).reshape(height, stride + 1)
    rows = np.zeros((height, stride), np.int32)
    previous = np.zeros(stride, np.int32)
    for y in range(height):
        kind_of_filter = raw[y, 0]
        line = raw[y, 1:].astype(np.int32)
        current = _unfilter(kind_of_filter, line, previous, channels)
        rows[y] = current
        previous = current
    pixels = rows.astype(np.uint8).reshape(height, width, channels)
    if kind == 3:
        rgb = palette[pixels[:, :, 0]]
        alpha = np.full((height, width, 1), 255, np.uint8)
        if transparency is not None:
            padded = np.concatenate([transparency, np.full(256 - len(transparency), 255, np.uint8)])
            alpha = padded[pixels[:, :, 0]][:, :, None]
        return np.concatenate([rgb, alpha], axis=2)
    if channels == 1:
        pixels = np.repeat(pixels, 3, axis=2)
        channels = 3
    if channels == 2:
        pixels = np.concatenate([np.repeat(pixels[:, :, :1], 3, axis=2), pixels[:, :, 1:]], axis=2)
        channels = 4
    if channels == 3:
        pixels = np.concatenate([pixels, np.full((height, width, 1), 255, np.uint8)], axis=2)
    return pixels


def _unfilter(kind, line, previous, channels):
    if kind == 0:
        return line
    if kind == 2:
        return (line + previous) & 255
    if kind == 1:
        out = line.copy()
        for channel in range(channels):
            out[channel::channels] = np.cumsum(line[channel::channels]) & 255
        return out
    out = line.copy()
    for i in range(len(line)):
        left = out[i - channels] if i >= channels else 0
        up = previous[i]
        if kind == 3:
            out[i] = (line[i] + (left + up) // 2) & 255
        else:
            corner = previous[i - channels] if i >= channels else 0
            estimate = left + up - corner
            pa, pb, pc = abs(estimate - left), abs(estimate - up), abs(estimate - corner)
            nearest = left if pa <= pb and pa <= pc else (up if pb <= pc else corner)
            out[i] = (line[i] + nearest) & 255
    return out


def write_png(path, pixels):
    """Writes an (h, w, c) uint8 array (c in 1, 3, 4) or an (h, w) uint16 array as a PNG."""
    pixels = np.asarray(pixels)
    if pixels.dtype == np.uint16:
        height, width = pixels.shape
        kind, depth = 0, 16
        rows = pixels.astype(">u2").tobytes()
        stride = width * 2
    else:
        pixels = pixels.astype(np.uint8)
        if pixels.ndim == 2:
            pixels = pixels[:, :, None]
        height, width, channels = pixels.shape
        kind, depth = {1: 0, 3: 2, 4: 6}[channels], 8
        rows = pixels.tobytes()
        stride = width * channels
    body = bytearray()
    for y in range(height):
        body.append(0)
        body += rows[y * stride : (y + 1) * stride]

    def chunk(name, data):
        return struct.pack(">I", len(data)) + name + data + struct.pack(">I", zlib.crc32(name + data) & 0xFFFFFFFF)

    header = struct.pack(">IIBBBBB", width, height, depth, kind, 0, 0, 0)
    out = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(bytes(body), 9)) + chunk(b"IEND", b"")
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    open(path, "wb").write(out)


def srgb_to_linear(c):
    c = np.asarray(c, np.float64)
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def linear_to_srgb(c):
    c = np.clip(np.asarray(c, np.float64), 0, 1)
    return np.where(c <= 0.0031308, c * 12.92, 1.055 * c ** (1 / 2.4) - 0.055)


def linear_to_oklab(rgb):
    rgb = np.asarray(rgb, np.float64)
    m1 = np.array([[0.4122214708, 0.5363325363, 0.0514459929], [0.2119034982, 0.6806995451, 0.1073969566], [0.0883024619, 0.2817188376, 0.6299787005]])
    m2 = np.array([[0.2104542553, 0.7936177850, -0.0040720468], [1.9779984951, -2.4285922050, 0.4505937099], [0.0259040371, 0.7827717662, -0.8086757660]])
    lms = np.cbrt(rgb @ m1.T)
    return lms @ m2.T


def hex_color(text):
    """'#RRGGBB' as sRGB floats 0..1."""
    text = text.strip().lstrip("#")
    return np.array([int(text[i : i + 2], 16) / 255 for i in (0, 2, 4)])
