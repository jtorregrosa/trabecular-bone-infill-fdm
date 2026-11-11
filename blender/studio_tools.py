"""Tools for the demo-render studio (studio.blend).

This one file is used in three ways:

* From the command line, to render a model without opening Blender::

      blender -b blender/studio.blend -P blender/studio_tools.py -- model.3mf --shots hero,top --cut y

  Run it with ``-- --help`` for every option.

* Inside Blender. A copy is embedded in studio.blend as a text block; it adds the
  "Studio" tab to the sidebar of the 3D viewport (press N).

* As a module: build_studio.py imports the names and presets it shares with the scene.

Conventions of the scene, all of them set up by build_studio.py:

* The model hangs from the "Subject Pivot" empty, which scales it to a radius of
  SUBJECT_RADIUS and rests it on the floor at the origin. Lights, backdrop and
  cameras are tuned for that size, so models of any size render the same way and the
  mesh keeps its own coordinates (millimetres for a print).
* Angles place things around the subject: azimuth 0 looks at the front of the model
  (from -Y), positive azimuth moves to the right of it (towards +X), elevation rises
  from the floor.
"""

import math
import os
import re
import sys
import xml.etree.ElementTree as ET
import zipfile

import bpy
import numpy as np
from mathutils import Matrix, Vector

SUBJECT_COLLECTION = "Subject"
LIGHTS_COLLECTION = "Lights"
PIVOT = "Subject Pivot"
TURNTABLE = "Turntable"
FOCUS = "Focus"
SECTION_PLANE = "Section Plane"
BACKDROP = "Backdrop"
SHADOW_CATCHER = "Shadow Catcher"
BACKDROP_MATERIAL = "Backdrop"
SECTION_CUTTER = "Section Cutter"
SECTION_MODIFIER = "Section"
CUT_MATERIAL = "Cut Highlight"
LAYER_GROUP = "Studio Layer Lines"
DEFAULT_MATERIAL = "Bone PLA"

SUBJECT_RADIUS = 1.0

SHOTS = {
    "hero": "CAM_Hero",
    "front": "CAM_Front",
    "side": "CAM_Side",
    "top": "CAM_Top",
    "detail": "CAM_Detail",
    "section": "CAM_Section",
}

# Light powers are multiples of each light's own "base_power".
LOOKS = {
    "dark": {
        "backdrop": (0.010, 0.009, 0.008),
        "backdrop_roughness": 0.75,
        "backdrop_specular": 0.12,
        "world": (0.10, 0.11, 0.13),
        "world_strength": 0.15,
        "lights": {},
    },
    "light": {
        "backdrop": (0.60, 0.60, 0.60),
        "backdrop_roughness": 0.85,
        "backdrop_specular": 0.3,
        "world": (1.0, 1.0, 1.0),
        "world_strength": 0.35,
        "lights": {"Key": 0.8, "Rim": 0.4, "Kicker": 0.3, "Top": 0.8},
    },
    # Mid-tone paper sweeps, as in product photography: softer than black, and the
    # ivory of a print still stands out against them.
    "warm": {
        "backdrop": (0.30, 0.25, 0.20),
        "backdrop_roughness": 0.9,
        "backdrop_specular": 0.15,
        "world": (1.0, 0.92, 0.82),
        "world_strength": 0.3,
        "lights": {"Key": 0.85, "Rim": 0.6, "Kicker": 0.5, "Top": 0.8},
    },
    "slate": {
        "backdrop": (0.11, 0.14, 0.18),
        "backdrop_roughness": 0.9,
        "backdrop_specular": 0.15,
        "world": (0.80, 0.88, 1.0),
        "world_strength": 0.3,
        "lights": {"Rim": 0.8, "Kicker": 0.7},
    },
    "sage": {
        "backdrop": (0.17, 0.21, 0.16),
        "backdrop_roughness": 0.9,
        "backdrop_specular": 0.15,
        "world": (0.92, 1.0, 0.90),
        "world_strength": 0.3,
        "lights": {"Rim": 0.8, "Kicker": 0.7},
    },
}

MODEL_FORMATS = (".stl", ".3mf", ".obj", ".ply", ".glb", ".gltf")


def log(message):
    print(f"[studio] {message}", flush=True)


# ------------------------------------------------------------------ geometry helpers


def direction(azimuth, elevation):
    """Unit vector from the subject towards something placed at these angles (degrees)."""
    az, el = math.radians(azimuth), math.radians(elevation)
    return Vector((math.sin(az) * math.cos(el), -math.cos(az) * math.cos(el), math.sin(el)))


def aim_rotation(away):
    """Rotation of a camera or light that sits along `away` and looks back at its target."""
    if abs(away.z) > 0.9995:
        # Straight down (image up is +Y) or straight up: the general formula has no roll here.
        return Matrix.Identity(3) if away.z > 0 else Matrix.Rotation(math.pi, 3, "X")
    return away.to_track_quat("Z", "Y").to_matrix()


def _hull_points(points, count=256):
    """The points that reach furthest along `count` directions.

    A few hundred of them bound the shape from any side almost as well as all of it,
    which keeps the framing code independent of the size of the mesh.
    """
    i = np.arange(count, dtype=np.float32) + 0.5
    z = 1 - 2 * i / count
    r = np.sqrt(1 - z * z)
    phi = i * (math.pi * (3 - math.sqrt(5)))
    dirs = np.stack((r * np.cos(phi), r * np.sin(phi), z), axis=1)
    axes = np.array([(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)], dtype=np.float32)
    dirs = np.concatenate((axes, dirs))
    picks = set()
    for start in range(0, len(dirs), 16):
        picks.update(np.argmax(points @ dirs[start : start + 16].T, axis=0).tolist())
    return points[sorted(picks)]


def subject_collection():
    return bpy.data.collections[SUBJECT_COLLECTION]


def subject_meshes():
    return [o for o in subject_collection().all_objects if o.type == "MESH"]


def subject_points():
    """World-space points that bound the subject, as an (n, 3) array."""
    chunks = []
    for obj in subject_meshes():
        mesh = obj.data
        if not mesh.vertices:
            continue
        co = np.empty(len(mesh.vertices) * 3, dtype=np.float32)
        mesh.vertices.foreach_get("co", co)
        world = np.array(obj.matrix_world, dtype=np.float32)
        chunks.append(co.reshape(-1, 3) @ world[:3, :3].T + world[:3, 3])
    if not chunks:
        r = SUBJECT_RADIUS / math.sqrt(3)
        return np.array([(x, y, z) for x in (-r, r) for y in (-r, r) for z in (0, 2 * r)], dtype=np.float32)
    return _hull_points(np.concatenate(chunks))


def _rest_pose():
    """Puts the turntable at its first frame, where it is not rotated."""
    scene = bpy.context.scene
    if scene.frame_current != scene.frame_start:
        scene.frame_set(scene.frame_start)
    bpy.context.view_layer.update()


# ------------------------------------------------------------------ model import


def _local(tag):
    return tag.rsplit("}", 1)[-1]


def _attribute(element, name):
    return next((v for k, v in element.attrib.items() if _local(k) == name), None)


def _matrix_3mf(text):
    if not text:
        return Matrix.Identity(4)
    v = [float(x) for x in text.split()]
    return Matrix(
        (
            (v[0], v[3], v[6], v[9]),
            (v[1], v[4], v[7], v[10]),
            (v[2], v[5], v[8], v[11]),
            (0, 0, 0, 1),
        )
    )


_UNIT_MM = {"micron": 0.001, "millimeter": 1.0, "centimeter": 10.0, "inch": 25.4, "foot": 304.8, "meter": 1000.0}
_MESH_OPEN = re.compile(rb"<((?:\w+:)?)mesh\b[^>]*>")
_VERTEX = re.compile(rb'<(?:\w+:)?vertex\s+x="([^"]*)"\s+y="([^"]*)"\s+z="([^"]*)"')
_TRIANGLE = re.compile(rb'<(?:\w+:)?triangle\s+v1="(\d+)"\s+v2="(\d+)"\s+v3="(\d+)"')


def _parse_3mf_part(data):
    """Splits a .model part into its mesh blobs and the small XML document around them.

    The meshes are millions of elements, far too many for an XML tree; they are read
    with regular expressions instead, and only the structure goes through the parser.
    """
    blobs, skeleton, pos = [], [], 0
    for match in _MESH_OPEN.finditer(data):
        if match.start() < pos or match.group(0).endswith(b"/>"):
            continue
        closing = b"</" + match.group(1) + b"mesh>"
        end = data.find(closing, match.end())
        if end < 0:
            break
        skeleton += [data[pos : match.start()], b'<mesh blob="%d"/>' % len(blobs)]
        blobs.append(data[match.end() : end])
        pos = end + len(closing)
    skeleton.append(data[pos:])
    root = ET.fromstring(b"".join(skeleton))

    objects, build = {}, []
    for element in root.iter():
        tag = _local(element.tag)
        if tag == "object":
            for child in element:
                kind = _local(child.tag)
                if kind == "mesh" and child.get("blob") is not None:
                    objects[element.get("id")] = ("mesh", int(child.get("blob")))
                elif kind == "components":
                    objects[element.get("id")] = (
                        "components",
                        [
                            (_attribute(c, "path"), c.get("objectid"), _matrix_3mf(c.get("transform")))
                            for c in child
                            if _local(c.tag) == "component"
                        ],
                    )
        elif tag == "item":
            build.append((_attribute(element, "path"), element.get("objectid"), _matrix_3mf(element.get("transform"))))
    return {"unit": _UNIT_MM.get(root.get("unit", "millimeter"), 1.0), "objects": objects, "build": build, "blobs": blobs}


def _rows(blob, fast, tag, names):
    rows = fast.findall(blob)
    if len(rows) == len(re.findall(rb"<(?:\w+:)?" + tag + rb"\b", blob)):
        return rows
    # Attributes in another order or with other spacing: slower, but takes anything.
    rows = []
    for match in re.finditer(rb"<(?:\w+:)?" + tag + rb"\b([^>]*)>", blob):
        attributes = dict(re.findall(rb'([\w:]+)\s*=\s*"([^"]*)"', match.group(1)))
        rows.append([attributes[n] for n in names])
    return rows


def _mesh_from_arrays(name, vertices, triangles):
    mesh = bpy.data.meshes.new(name)
    mesh.vertices.add(len(vertices))
    mesh.vertices.foreach_set("co", vertices.ravel())
    mesh.loops.add(triangles.size)
    mesh.loops.foreach_set("vertex_index", triangles.ravel())
    mesh.polygons.add(len(triangles))
    mesh.polygons.foreach_set("loop_start", np.arange(0, triangles.size, 3, dtype=np.int32))
    mesh.update(calc_edges=True)
    mesh.validate()
    return mesh


def import_3mf(path):
    """Reads the meshes of a 3MF file, with its components and build transforms."""
    name = os.path.splitext(os.path.basename(path))[0]
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        root_name = next((n for n in names if n.lower() == "3d/3dmodel.model"), None)
        if "_rels/.rels" in names:
            for rel in ET.fromstring(archive.read("_rels/.rels")):
                if rel.get("Type", "").endswith("/3dmodel") and rel.get("Target", "").lstrip("/") in names:
                    root_name = rel.get("Target").lstrip("/")
        if root_name is None:
            root_name = next((n for n in names if n.lower().endswith(".model")), None)
        if root_name is None:
            raise ValueError(f"{path}: no model part in the 3MF archive")

        parts = {}

        def part(part_name):
            key = part_name.lstrip("/")
            if key not in parts:
                parts[key] = _parse_3mf_part(archive.read(key))
            return parts[key]

        instances = []

        def walk(part_name, object_id, matrix, depth=0):
            kind, payload = part(part_name)["objects"].get(object_id, (None, None))
            if kind == "mesh":
                instances.append((part_name.lstrip("/"), payload, matrix))
            elif kind == "components" and depth < 16:
                for sub_part, sub_id, sub_matrix in payload:
                    walk(sub_part or part_name, sub_id, matrix @ sub_matrix, depth + 1)

        root = part(root_name)
        items = root["build"] or [(None, oid, Matrix.Identity(4)) for oid in root["objects"]]
        for item_part, object_id, matrix in items:
            walk(item_part or root_name, object_id, matrix)

    meshes, objects = {}, []
    for part_name, blob_index, matrix in instances:
        key = (part_name, blob_index)
        if key not in meshes:
            blob = parts[part_name]["blobs"][blob_index]
            vertices = np.array(_rows(blob, _VERTEX, b"vertex", (b"x", b"y", b"z")), dtype="S").astype(np.float32)
            triangles = np.array(_rows(blob, _TRIANGLE, b"triangle", (b"v1", b"v2", b"v3")), dtype="S").astype(np.int32)
            if vertices.size == 0 or triangles.size == 0:
                meshes[key] = None
                continue
            meshes[key] = _mesh_from_arrays(name, vertices.reshape(-1, 3), triangles.reshape(-1, 3))
        if meshes[key] is None:
            continue
        obj = bpy.data.objects.new(name, meshes[key])
        obj.matrix_world = Matrix.Scale(root["unit"], 4) @ matrix
        bpy.context.scene.collection.objects.link(obj)
        objects.append(obj)
    if not objects:
        raise ValueError(f"{path}: the 3MF file has no meshes")
    return objects


def import_model(path):
    """Imports a model file and returns its mesh objects. Z is up, as in a slicer."""
    path = os.path.abspath(path)
    extension = os.path.splitext(path)[1].lower()
    if not os.path.isfile(path):
        raise FileNotFoundError(path)
    if extension == ".3mf":
        return import_3mf(path)

    before = set(bpy.data.objects)
    if extension == ".stl":
        bpy.ops.wm.stl_import(filepath=path)
    elif extension == ".obj":
        bpy.ops.wm.obj_import(filepath=path, forward_axis="Y", up_axis="Z")
    elif extension == ".ply":
        bpy.ops.wm.ply_import(filepath=path)
    elif extension in (".glb", ".gltf"):
        bpy.ops.import_scene.gltf(filepath=path)
    else:
        raise ValueError(f"{path}: unsupported format, use one of {', '.join(MODEL_FORMATS)}")

    bpy.context.view_layer.update()
    created = [o for o in bpy.data.objects if o not in before]
    meshes = [o for o in created if o.type == "MESH"]
    worlds = {o: o.matrix_world.copy() for o in meshes}
    for obj in meshes:
        obj.parent = None
        obj.matrix_world = worlds[obj]
    for obj in created:
        if obj.type != "MESH":
            bpy.data.objects.remove(obj, do_unlink=True)
    if not meshes:
        raise ValueError(f"{path}: no meshes found")
    return meshes


# ------------------------------------------------------------------ subject


def set_material(name):
    """Gives every mesh of the subject the material `name`."""
    material = bpy.data.materials.get(name)
    if material is None:
        known = ", ".join(sorted(m.name for m in bpy.data.materials if m.get("studio_material")))
        raise ValueError(f"no material named {name!r}; the studio has: {known}")
    for obj in subject_meshes():
        obj.data.materials.clear()
        obj.data.materials.append(material)
    bpy.context.scene["studio_material"] = name


def _shade(obj, flat=False, angle=35.0):
    mesh = obj.data
    mesh.polygons.foreach_set("use_smooth", np.full(len(mesh.polygons), not flat, dtype=bool))
    if not flat and hasattr(mesh, "set_sharp_from_angle"):
        mesh.set_sharp_from_angle(angle=math.radians(angle))
    mesh.update()


def fit_subject(yaw=0.0):
    """Scales the subject to SUBJECT_RADIUS and rests it on the floor at the origin.

    `yaw` turns it around the vertical axis, in degrees, to choose the side that
    faces the cameras.
    """
    _rest_pose()
    pivot = bpy.data.objects[PIVOT]
    pivot.location = (0, 0, 0)
    pivot.rotation_euler = (0, 0, math.radians(yaw))
    pivot.scale = (1, 1, 1)
    bpy.context.view_layer.update()

    points = subject_points()
    low, high = points.min(axis=0), points.max(axis=0)
    centre = (low + high) / 2
    radius = float(np.linalg.norm(points - centre, axis=1).max()) or 1.0
    scale = SUBJECT_RADIUS / radius
    pivot.scale = (scale, scale, scale)
    pivot.location = (-scale * centre[0], -scale * centre[1], -scale * low[2])
    bpy.context.view_layer.update()


def set_subject(objects, material=None, yaw=0.0, flat=False):
    """Replaces the subject with `objects`, then fits it, shades it and frames it."""
    _rest_pose()
    pivot = bpy.data.objects[PIVOT]
    pivot.location = (0, 0, 0)
    pivot.rotation_euler = (0, 0, 0)
    pivot.scale = (1, 1, 1)
    bpy.context.view_layer.update()

    collection = subject_collection()
    keep = set(objects)
    for obj in list(collection.all_objects):
        if obj not in keep:
            data = obj.data
            bpy.data.objects.remove(obj, do_unlink=True)
            if isinstance(data, bpy.types.Mesh) and data.users == 0:
                bpy.data.meshes.remove(data)

    worlds = {obj: obj.matrix_world.copy() for obj in objects}
    for obj in objects:
        for other in list(obj.users_collection):
            other.objects.unlink(obj)
        collection.objects.link(obj)
        obj.parent = pivot
        obj.matrix_parent_inverse = Matrix.Identity(4)
        obj.matrix_world = worlds[obj]
        _shade(obj, flat=flat)

    set_material(material or bpy.context.scene.get("studio_material", DEFAULT_MATERIAL))
    fit_subject(yaw)
    if section_enabled():
        set_section(bpy.context.scene.get("studio_section", "off"))
    if bpy.context.scene.get("studio_layers"):
        set_layer_lines(bpy.context.scene["studio_layers"])
    fit_cameras()


def load_model(path, material=None, yaw=0.0, flat=False):
    """Imports a model file and makes it the subject."""
    objects = import_model(path)
    set_subject(objects, material=material, yaw=yaw, flat=flat)
    bpy.context.scene["studio_subject"] = os.path.splitext(os.path.basename(path))[0]
    triangles = sum(len(o.data.polygons) for o in objects)
    log(f"loaded {os.path.basename(path)}: {len(objects)} object(s), {triangles:,} triangles")
    return objects


# ------------------------------------------------------------------ cameras


def studio_cameras():
    return [bpy.data.objects[name] for name in SHOTS.values() if name in bpy.data.objects]


def fit_cameras(turntable=False):
    """Moves every camera so that the subject fills its frame.

    Each camera keeps the direction given by its "azimuth" and "elevation" custom
    properties and fills the "fill" share of the frame. With `turntable`, the subject
    must fit at every angle of its turn.
    """
    scene = bpy.context.scene
    _rest_pose()
    points = subject_points()
    low, high = points.min(axis=0), points.max(axis=0)
    centre = Vector(((low + high) / 2).tolist())
    if turntable:
        turns = []
        for step in range(36):
            c, s = math.cos(step * math.tau / 36), math.sin(step * math.tau / 36)
            turns.append(points @ np.array(((c, s, 0), (-s, c, 0), (0, 0, 1)), dtype=np.float32))
        points = np.concatenate(turns)
        centre.x = centre.y = 0.0

    render = scene.render
    aspect = (render.resolution_x * render.pixel_aspect_x) / (render.resolution_y * render.pixel_aspect_y)
    relative = points - np.array(centre, dtype=np.float32)
    for camera in studio_cameras():
        away = direction(camera.get("azimuth", 0.0), camera.get("elevation", 0.0))
        rotation = aim_rotation(away)
        # Camera axes: x right, y up, z towards the camera.
        x, y, z = (relative @ np.array(rotation, dtype=np.float32)).T
        fill = camera.get("fill", 0.8)
        data = camera.data
        if data.type == "ORTHO":
            width, height = 2 * np.abs(x).max(), 2 * np.abs(y).max()
            data.ortho_scale = float(max(width, height * aspect) if aspect >= 1 else max(width / aspect, height)) / fill
            distance = float(z.max()) + 8.0
        else:
            # "fit_lens" frames the subject; a longer real lens then crops into it.
            half = data.sensor_width / (2 * camera.get("fit_lens", data.lens))
            tan_x, tan_y = (half, half / aspect) if aspect >= 1 else (half * aspect, half)
            distance = float((z + np.maximum(np.abs(x) / (tan_x * fill), np.abs(y) / (tan_y * fill))).max())
        camera.location = centre + away * distance
        camera.rotation_euler = rotation.to_euler()

    bpy.context.view_layer.update()
    _focus(centre)


def _focus(centre):
    """Puts the Focus empty on the surface that the detail camera looks at."""
    focus = bpy.data.objects.get(FOCUS)
    camera = bpy.data.objects.get(SHOTS["detail"])
    if focus is None or camera is None:
        return
    focus.location = centre
    subject = set(subject_meshes())
    depsgraph = bpy.context.evaluated_depsgraph_get()
    origin = camera.matrix_world.translation.copy()
    forward = -(camera.matrix_world.to_3x3() @ Vector((0, 0, 1))).normalized()
    hit, location, _normal, _index, obj, _matrix = bpy.context.scene.ray_cast(depsgraph, origin, forward)
    if hit and obj.original in subject:
        focus.location = location


# ------------------------------------------------------------------ look, section, layers


def set_look(name):
    """Switches backdrop, world and light balance to one of LOOKS."""
    look = LOOKS[name]
    scene = bpy.context.scene
    backdrop = bpy.data.materials[BACKDROP_MATERIAL].node_tree.nodes["Principled BSDF"]
    backdrop.inputs["Base Color"].default_value = (*look["backdrop"], 1)
    backdrop.inputs["Roughness"].default_value = look["backdrop_roughness"]
    backdrop.inputs["Specular IOR Level"].default_value = look["backdrop_specular"]
    bpy.data.materials[BACKDROP_MATERIAL].diffuse_color = (*look["backdrop"], 1)
    background = scene.world.node_tree.nodes["Background"]
    background.inputs["Color"].default_value = (*look["world"], 1)
    background.inputs["Strength"].default_value = look["world_strength"]
    for light in bpy.data.collections[LIGHTS_COLLECTION].objects:
        if light.type == "LIGHT":
            light.data.energy = light.get("base_power", light.data.energy) * look["lights"].get(light.name, 1.0)
    scene["studio_look"] = name


def section_enabled():
    return bpy.context.scene.get("studio_section", "off") != "off"


def _section_modifiers(enabled, highlight):
    """Adds the Boolean that cuts each subject mesh, or removes it."""
    cutter = bpy.data.objects[SECTION_CUTTER]
    cutter.data.materials.clear()
    if highlight:
        # The cut faces take the cutter's material ("Transfer" below).
        cutter.data.materials.append(bpy.data.materials[CUT_MATERIAL])
    for obj in subject_meshes():
        modifier = obj.modifiers.get(SECTION_MODIFIER)
        if not enabled:
            if modifier is not None:
                obj.modifiers.remove(modifier)
            continue
        if modifier is None:
            modifier = obj.modifiers.new(SECTION_MODIFIER, "BOOLEAN")
        modifier.operation = "DIFFERENCE"
        modifier.operand_type = "OBJECT"
        modifier.object = cutter
        modifier.solver = "EXACT"
        modifier.material_mode = "TRANSFER"
        modifier.use_self = False
        modifier.use_hole_tolerant = False


def set_section(spec="off", highlight=None):
    """Cuts the subject open with a plane, or closes it again with "off".

    `spec` is an axis of the model with an optional side and position: "y", "+x:0.4",
    "-z:0.7". The position is the share of the model's extent along the axis where the
    plane sits (0.5, the middle, by default). The side is the half that is removed;
    without it, the half facing the hero camera goes.

    The cut is real geometry, made by a Boolean modifier on each mesh of the subject
    with the "Section Cutter" box, which covers the +Z side of the "Section Plane" empty.
    The cut faces have the subject's own material and are lit like the rest of it;
    with `highlight` they get the "Cut Highlight" colour instead (None keeps the
    current choice). The plane can be moved by hand, and it turns with the turntable.
    """
    scene = bpy.context.scene
    if highlight is None:
        highlight = bool(scene.get("studio_cut_highlight", False))
    scene["studio_cut_highlight"] = bool(highlight)
    section_camera = bpy.data.objects.get(SHOTS["section"])
    front = bpy.data.objects.get(SHOTS["front"])
    hero = bpy.data.objects.get(SHOTS["hero"])
    spec = (spec or "off").strip().lower()
    if spec in ("off", "none", ""):
        _section_modifiers(False, highlight)
        scene["studio_section"] = "off"
        if section_camera and front:
            section_camera["azimuth"], section_camera["elevation"] = front["azimuth"], front["elevation"]
        return

    match = re.fullmatch(r"([+-]?)([xyz])(?::([0-9]*\.?[0-9]+))?", spec)
    if not match:
        raise ValueError(f"bad section {spec!r}: use an axis like y, +x:0.4 or -z:0.7")
    sign, axis, offset = match.group(1), "xyz".index(match.group(2)), float(match.group(3) or 0.5)

    _rest_pose()
    pivot = bpy.data.objects[PIVOT]
    to_model = np.array(pivot.matrix_world.inverted(), dtype=np.float32)
    points = subject_points() @ to_model[:3, :3].T + to_model[:3, 3]
    low, high = points.min(axis=0), points.max(axis=0)
    position = (low + high) / 2
    # Slightly off the exact share: generated meshes often have vertices on round
    # planes, and a cut through them is more fragile.
    position[axis] = low[axis] + (offset + 1.3e-4) * (high[axis] - low[axis])

    unit = Vector((0, 0, 0))
    unit[axis] = 1.0
    normal = (pivot.matrix_world.to_3x3() @ unit).normalized()
    if sign == "-" or (not sign and hero and normal.dot(direction(hero["azimuth"], hero["elevation"])) < 0):
        normal.negate()

    plane = bpy.data.objects[SECTION_PLANE]
    plane.matrix_world = Matrix.Translation(pivot.matrix_world @ Vector(position.tolist())) @ aim_rotation(normal).to_4x4()
    _section_modifiers(True, highlight)
    scene["studio_section"] = spec

    if section_camera:
        if normal.z < -0.5 and hero:
            # The cut faces the floor: there is nowhere to look at it head-on.
            section_camera["azimuth"], section_camera["elevation"] = hero["azimuth"], hero["elevation"]
        else:
            section_camera["azimuth"] = math.degrees(math.atan2(normal.x, -normal.y))
            section_camera["elevation"] = math.degrees(math.asin(max(-1.0, min(1.0, normal.z))))


def set_layer_lines(height=None):
    """Shows FDM layer lines of `height` model units (millimetres) on the subject, or hides them.

    The relief is drawn in the model's own coordinates, so it follows the print
    direction (the model's Z) and keeps its size whatever the scale of the stage.
    """
    nodes = bpy.data.node_groups[LAYER_GROUP].nodes
    nodes["Strength"].outputs[0].default_value = 1.0 if height else 0.0
    if height:
        nodes["Layer Height"].outputs[0].default_value = height
        scale = bpy.data.objects[PIVOT].matrix_world.to_scale().x
        nodes["Bump"].inputs["Distance"].default_value = 0.12 * height * scale
    bpy.context.scene["studio_layers"] = float(height or 0.0)


# ------------------------------------------------------------------ rendering


def enable_gpu():
    """Turns on the fastest GPU backend for Cycles in this session; returns its name."""
    scene = bpy.context.scene
    preferences = bpy.context.preferences.addons["cycles"].preferences
    for backend in ("OPTIX", "CUDA", "HIP", "METAL", "ONEAPI"):
        try:
            preferences.compute_device_type = backend
        except TypeError:
            continue
        preferences.get_devices()
        if any(d.type == backend for d in preferences.devices):
            for device in preferences.devices:
                device.use = device.type == backend
            scene.cycles.device = "GPU"
            return backend
    scene.cycles.device = "CPU"
    return None


def set_transparent(transparent):
    """Renders over a transparent background, keeping only the shadow on the floor."""
    bpy.context.scene.render.film_transparent = transparent
    bpy.data.objects[BACKDROP].hide_render = transparent
    bpy.data.objects[SHADOW_CATCHER].hide_render = not transparent


def output_directory():
    if bpy.data.filepath:
        return os.path.normpath(os.path.join(os.path.dirname(bpy.data.filepath), "..", "out", "renders"))
    return os.path.abspath("renders")


def render_shot(shot, filepath):
    """Renders one still from the camera of `shot` into `filepath` (a PNG)."""
    scene = bpy.context.scene
    scene.camera = bpy.data.objects[SHOTS[shot]]
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGBA" if scene.render.film_transparent else "RGB"
    scene.render.filepath = filepath
    os.makedirs(os.path.dirname(filepath), exist_ok=True)
    bpy.ops.render.render(write_still=True)
    log(f"wrote {filepath}")


def set_turntable(frames):
    """Makes the Turntable empty do one full turn in `frames` frames, looping cleanly."""
    scene = bpy.context.scene
    turntable = bpy.data.objects[TURNTABLE]
    turntable.animation_data_clear()
    scene.frame_start, scene.frame_end = 1, frames
    edit = bpy.context.preferences.edit
    previous = edit.keyframe_new_interpolation_type
    edit.keyframe_new_interpolation_type = "LINEAR"
    try:
        for frame, angle in ((1, 0.0), (frames + 1, math.tau)):
            turntable.rotation_euler = (0, 0, angle)
            turntable.keyframe_insert("rotation_euler", index=2, frame=frame)
    finally:
        edit.keyframe_new_interpolation_type = previous
    scene.frame_set(1)


def render_turntable(filepath, frames=120, shot="hero"):
    """Renders a full turn of the subject as an H.264 video into `filepath` (an .mp4)."""
    scene = bpy.context.scene
    set_turntable(frames)
    fit_cameras(turntable=True)
    scene.camera = bpy.data.objects[SHOTS[shot]]
    render = scene.render
    render.image_settings.file_format = "FFMPEG"
    render.ffmpeg.format = "MPEG4"
    render.ffmpeg.codec = "H264"
    render.ffmpeg.constant_rate_factor = "HIGH"
    render.ffmpeg.audio_codec = "NONE"
    stem = os.path.splitext(filepath)[0]
    render.filepath = stem + "_"
    os.makedirs(os.path.dirname(filepath), exist_ok=True)
    bpy.ops.render.render(animation=True)
    written = f"{stem}_{scene.frame_start:04d}-{scene.frame_end:04d}.mp4"
    if os.path.isfile(written):
        os.replace(written, filepath)
    render.image_settings.file_format = "PNG"
    scene.frame_set(1)
    fit_cameras()
    log(f"wrote {filepath}")


# ------------------------------------------------------------------ command line


def _parse_shots(text):
    shots = list(SHOTS) if text == "all" else [s.strip() for s in text.split(",") if s.strip()]
    unknown = [s for s in shots if s not in SHOTS]
    if unknown:
        raise SystemExit(f"unknown shot(s) {', '.join(unknown)}; choose from {', '.join(SHOTS)} or all")
    return shots


def cli(argv):
    import argparse

    parser = argparse.ArgumentParser(
        prog="blender -b blender/studio.blend -P blender/studio_tools.py --",
        description="Render demo images of 3D models in the studio scene.",
    )
    parser.add_argument("models", nargs="+", help="model files: " + ", ".join(MODEL_FORMATS))
    parser.add_argument("--shots", default="hero", help=f"comma-separated shots ({', '.join(SHOTS)}) or all [hero]")
    parser.add_argument("--look", choices=sorted(LOOKS), help="backdrop colour and light balance [slate]")
    parser.add_argument("--material", help=f"material of the model [{DEFAULT_MATERIAL}]")
    parser.add_argument("--cut", metavar="AXIS[:POS]", help="open the model with a plane: y, +x:0.4, --cut=-z:0.7")
    parser.add_argument("--cut-color", action="store_true", help="paint the cut faces in a highlight colour")
    parser.add_argument("--yaw", type=float, default=0.0, help="turn the model around Z, in degrees")
    parser.add_argument("--flat", action="store_true", help="flat shading instead of smooth")
    parser.add_argument("--layers", type=float, metavar="MM", help="show FDM layer lines of this height")
    parser.add_argument("--turntable", type=int, nargs="?", const=120, metavar="FRAMES", help="also render a turning video [120 frames]")
    parser.add_argument("--res", metavar="WxH", help="resolution in pixels [2000x1500]")
    parser.add_argument("--samples", type=int, help="render samples [256]")
    parser.add_argument("--transparent", action="store_true", help="transparent background with the floor shadow")
    parser.add_argument("--cpu", action="store_true", help="render on the CPU")
    parser.add_argument("--out", help="output directory [out/renders]")
    parser.add_argument("--name", help="output file prefix [the model's file name]")
    parser.add_argument("--save", metavar="FILE.blend", help="save the scene with the model loaded, to adjust it by hand")
    args = parser.parse_args(argv)

    scene = bpy.context.scene
    shots = _parse_shots(args.shots)
    if args.res:
        match = re.fullmatch(r"(\d+)[xX](\d+)", args.res)
        if not match:
            raise SystemExit(f"bad --res {args.res!r}: use WIDTHxHEIGHT, for example 1920x1080")
        scene.render.resolution_x, scene.render.resolution_y = int(match.group(1)), int(match.group(2))
        scene.render.resolution_percentage = 100
    if args.samples:
        scene.cycles.samples = args.samples
    if args.look:
        set_look(args.look)
    set_transparent(args.transparent)
    set_layer_lines(args.layers)
    backend = None if args.cpu else enable_gpu()
    if backend is None:
        scene.cycles.device = "CPU"
    log(f"rendering with Cycles on {backend or 'CPU'}, {scene.cycles.samples} samples, "
        f"{scene.render.resolution_x}x{scene.render.resolution_y}")

    out = os.path.abspath(args.out) if args.out else output_directory()
    for index, model in enumerate(args.models):
        load_model(model, material=args.material, yaw=args.yaw, flat=args.flat)
        set_section(args.cut or "off", highlight=args.cut_color)
        fit_cameras()
        name = args.name or os.path.splitext(os.path.basename(model))[0] + ("_cut" if args.cut else "")
        if args.name and len(args.models) > 1:
            name = f"{args.name}_{index + 1}"
        for shot in shots:
            render_shot(shot, os.path.join(out, f"{name}_{shot}.png"))
        if args.turntable:
            render_turntable(os.path.join(out, f"{name}_turntable.mp4"), frames=args.turntable)
        if args.save:
            target = os.path.abspath(args.save)
            if len(args.models) > 1:
                target = f"{os.path.splitext(target)[0]}_{index + 1}.blend"
            scene.camera = bpy.data.objects[SHOTS[shots[0]]]
            bpy.ops.wm.save_as_mainfile(filepath=target, copy=True)
            log(f"saved {target}")


# ------------------------------------------------------------------ sidebar panel


class STUDIO_OT_import_model(bpy.types.Operator):
    bl_idname = "studio.import_model"
    bl_label = "Import Model"
    bl_description = "Replace the subject with a model file, fitted to the stage"
    bl_options = {"REGISTER", "UNDO"}

    filepath: bpy.props.StringProperty(subtype="FILE_PATH")
    filter_glob: bpy.props.StringProperty(default=";".join("*" + e for e in MODEL_FORMATS), options={"HIDDEN"})

    def invoke(self, context, event):
        context.window_manager.fileselect_add(self)
        return {"RUNNING_MODAL"}

    def execute(self, context):
        try:
            load_model(self.filepath)
        except (OSError, ValueError) as error:
            self.report({"ERROR"}, str(error))
            return {"CANCELLED"}
        return {"FINISHED"}


class STUDIO_OT_use_selected(bpy.types.Operator):
    bl_idname = "studio.use_selected"
    bl_label = "Use Selected as Subject"
    bl_description = "Replace the subject with the selected meshes, fitted to the stage"
    bl_options = {"REGISTER", "UNDO"}

    def execute(self, context):
        meshes = [o for o in context.selected_objects if o.type == "MESH" and o.name not in (BACKDROP, SHADOW_CATCHER)]
        if not meshes:
            self.report({"ERROR"}, "Select at least one mesh")
            return {"CANCELLED"}
        set_subject(meshes)
        context.scene["studio_subject"] = meshes[0].name
        return {"FINISHED"}


class STUDIO_OT_refit(bpy.types.Operator):
    bl_idname = "studio.refit"
    bl_label = "Refit"
    bl_description = "Fit the subject to the stage and the cameras to the subject again"
    bl_options = {"REGISTER", "UNDO"}

    yaw: bpy.props.FloatProperty(name="Yaw", description="Turn the subject around Z", subtype="ANGLE", default=0.0)

    def execute(self, context):
        fit_subject(math.degrees(self.yaw))
        fit_cameras()
        return {"FINISHED"}


class STUDIO_OT_look(bpy.types.Operator):
    bl_idname = "studio.look"
    bl_label = "Look"
    bl_description = "Switch the backdrop and the light balance"
    bl_options = {"REGISTER", "UNDO"}

    look: bpy.props.StringProperty()

    def execute(self, context):
        set_look(self.look)
        return {"FINISHED"}


class STUDIO_OT_material(bpy.types.Operator):
    bl_idname = "studio.material"
    bl_label = "Material"
    bl_description = "Give the subject this material"
    bl_options = {"REGISTER", "UNDO"}

    material: bpy.props.StringProperty()

    def execute(self, context):
        set_material(self.material)
        return {"FINISHED"}


class STUDIO_OT_section(bpy.types.Operator):
    bl_idname = "studio.section"
    bl_label = "Section"
    bl_description = "Cut the subject open with a plane to show its inside"
    bl_options = {"REGISTER", "UNDO"}

    axis: bpy.props.EnumProperty(
        name="Axis",
        items=[("off", "Off", "No cut"), ("x", "X", ""), ("y", "Y", ""), ("z", "Z", "")],
        default="y",
    )
    position: bpy.props.FloatProperty(name="Position", min=0.0, max=1.0, default=0.5, subtype="FACTOR")
    flip: bpy.props.BoolProperty(name="Flip", description="Remove the other half")

    def execute(self, context):
        if self.axis == "off":
            set_section("off")
        else:
            hero = bpy.data.objects[SHOTS["hero"]]
            unit = Vector((0, 0, 0))
            unit["xyz".index(self.axis)] = 1.0
            normal = bpy.data.objects[PIVOT].matrix_world.to_3x3() @ unit
            towards_hero = normal.dot(direction(hero["azimuth"], hero["elevation"])) >= 0
            sign = "+" if towards_hero != self.flip else "-"
            set_section(f"{sign}{self.axis}:{self.position:.4f}")
        fit_cameras()
        return {"FINISHED"}


class STUDIO_OT_cut_highlight(bpy.types.Operator):
    bl_idname = "studio.cut_highlight"
    bl_label = "Highlight Cut"
    bl_description = "Paint the cut faces in a highlight colour instead of the subject's material"
    bl_options = {"REGISTER", "UNDO"}

    def execute(self, context):
        scene = context.scene
        set_section(scene.get("studio_section", "off"), highlight=not scene.get("studio_cut_highlight", False))
        return {"FINISHED"}


class STUDIO_OT_shot(bpy.types.Operator):
    bl_idname = "studio.shot"
    bl_label = "Shot"
    bl_description = "Look through the camera of this shot"

    shot: bpy.props.StringProperty()

    def execute(self, context):
        context.scene.camera = bpy.data.objects[SHOTS[self.shot]]
        for area in context.screen.areas:
            if area.type == "VIEW_3D":
                area.spaces.active.region_3d.view_perspective = "CAMERA"
        return {"FINISHED"}


class STUDIO_OT_render_shots(bpy.types.Operator):
    bl_idname = "studio.render_shots"
    bl_label = "Render All Shots"
    bl_description = "Render every shot into out/renders next to the project (Blender waits until it is done)"

    def execute(self, context):
        scene = context.scene
        camera, name = scene.camera, scene.get("studio_subject", "subject")
        for shot in SHOTS:
            if shot == "section" and not section_enabled():
                continue
            render_shot(shot, os.path.join(output_directory(), f"{name}_{shot}.png"))
        scene.camera = camera
        self.report({"INFO"}, f"Renders written to {output_directory()}")
        return {"FINISHED"}


class STUDIO_PT_panel(bpy.types.Panel):
    bl_label = "Studio"
    bl_space_type = "VIEW_3D"
    bl_region_type = "UI"
    bl_category = "Studio"

    def draw(self, context):
        layout = self.layout
        scene = context.scene

        column = layout.column(align=True)
        column.operator("studio.import_model", icon="IMPORT")
        column.operator("studio.use_selected", icon="RESTRICT_SELECT_OFF")
        column.operator("studio.refit", icon="FULLSCREEN_ENTER")

        layout.label(text="Look")
        row = layout.row(align=True)
        for name in LOOKS:
            row.operator("studio.look", text=name.title(), depress=scene.get("studio_look") == name).look = name

        layout.label(text="Material")
        grid = layout.grid_flow(columns=2, align=True)
        for material in bpy.data.materials:
            if material.get("studio_material"):
                active = scene.get("studio_material") == material.name
                grid.operator("studio.material", text=material.name, depress=active).material = material.name

        layout.label(text="Section")
        row = layout.row(align=True)
        current = str(scene.get("studio_section", "off")).lstrip("+-")[:1]
        for axis in ("off", "x", "y", "z"):
            row.operator("studio.section", text=axis.upper() if axis != "off" else "Off", depress=current == axis[:1]).axis = axis
        layout.operator("studio.cut_highlight", icon="BRUSH_DATA", depress=bool(scene.get("studio_cut_highlight", False)))

        layout.label(text="Shots")
        grid = layout.grid_flow(columns=3, align=True)
        for shot, camera in SHOTS.items():
            active = scene.camera is not None and scene.camera.name == camera
            grid.operator("studio.shot", text=shot.title(), depress=active).shot = shot
        layout.operator("studio.render_shots", icon="RENDER_STILL")


CLASSES = (
    STUDIO_OT_import_model,
    STUDIO_OT_use_selected,
    STUDIO_OT_refit,
    STUDIO_OT_look,
    STUDIO_OT_material,
    STUDIO_OT_section,
    STUDIO_OT_cut_highlight,
    STUDIO_OT_shot,
    STUDIO_OT_render_shots,
    STUDIO_PT_panel,
)


def register():
    for cls in CLASSES:
        registered = getattr(bpy.types, cls.__name__, None)
        if registered is not None:
            bpy.utils.unregister_class(registered)
        bpy.utils.register_class(cls)


def unregister():
    for cls in reversed(CLASSES):
        registered = getattr(bpy.types, cls.__name__, None)
        if registered is not None:
            bpy.utils.unregister_class(registered)


if __name__ == "__main__":
    if bpy.app.background:
        cli(sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else ["--help"])
    else:
        register()
elif __name__ == "studio_tools.py":
    # The copy embedded in studio.blend, run when the file opens with scripts allowed.
    register()
