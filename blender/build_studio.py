"""Builds studio.blend, the scene for rendering demo images of 3D models.

    blender -b --factory-startup -P blender/build_studio.py -- [--model FILE] [--out FILE.blend]

The scene is made from code so that it can be rebuilt and reviewed; studio_tools.py
explains its conventions. Without --model the stage holds a placeholder shape.
"""

import math
import os
import sys

import bmesh
import bpy
from mathutils import Vector

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import studio_tools as st  # noqa: E402

# Materials for the subject: base colour (linear), roughness and extra Principled inputs.
MATERIALS = {
    "Bone PLA": ((0.78, 0.72, 0.58), 0.42, {}),
    "Matte White": ((0.80, 0.80, 0.80), 0.60, {}),
    "Graphite PLA": ((0.030, 0.030, 0.035), 0.38, {}),
    "Terracotta": ((0.45, 0.16, 0.08), 0.70, {}),
    "Blue PETG": ((0.02, 0.12, 0.40), 0.22, {"Coat Weight": 0.3}),
    "Ceramic": ((0.85, 0.84, 0.80), 0.15, {"Coat Weight": 0.6}),
    "Silk Copper": ((0.85, 0.45, 0.30), 0.30, {"Metallic": 1.0}),
}
CUT_HIGHLIGHT = (0.48, 0.15, 0.07)

# name: (azimuth, elevation, distance, size, base power in W, colour)
LIGHTS = {
    "Key": (-40, 40, 5.0, 2.5, 330, (1.0, 0.96, 0.90)),
    "Fill": (60, 15, 6.0, 4.0, 80, (0.90, 0.95, 1.0)),
    "Rim": (155, 35, 5.0, 1.5, 550, (1.0, 1.0, 1.0)),
    "Kicker": (-150, 18, 5.0, 1.0, 220, (0.95, 0.97, 1.0)),
    "Top": (0, 85, 6.0, 5.0, 140, (1.0, 1.0, 1.0)),
}
LIGHT_TARGET = Vector((0, 0, 0.6))

# name: (azimuth, elevation, lens, fit lens, fill, orthographic)
CAMERAS = {
    "CAM_Hero": (35, 22, 85, 85, 0.78, False),
    "CAM_Front": (0, 8, 70, 70, 0.80, False),
    "CAM_Side": (90, 8, 70, 70, 0.80, False),
    "CAM_Top": (0, 90, 50, 50, 0.80, True),
    "CAM_Detail": (25, 30, 135, 50, 0.80, False),
    "CAM_Section": (0, 0, 70, 70, 0.85, False),
}


def collection(name, parent=None):
    coll = bpy.data.collections.new(name)
    (parent or bpy.context.scene.collection).children.link(coll)
    return coll


def link(obj, coll):
    coll.objects.link(obj)
    return obj


def empty(name, coll, display="PLAIN_AXES", size=0.3):
    obj = link(bpy.data.objects.new(name, None), coll)
    obj.empty_display_type = display
    obj.empty_display_size = size
    return obj


def clear_scene():
    for obj in list(bpy.data.objects):
        bpy.data.objects.remove(obj, do_unlink=True)
    for datablocks in (bpy.data.meshes, bpy.data.materials, bpy.data.lights, bpy.data.cameras, bpy.data.worlds):
        for block in list(datablocks):
            datablocks.remove(block)
    for coll in list(bpy.data.collections):
        bpy.data.collections.remove(coll)


# ------------------------------------------------------------------ node groups


def layer_group():
    group = bpy.data.node_groups.new(st.LAYER_GROUP, "ShaderNodeTree")
    group.interface.new_socket("Normal", in_out="OUTPUT", socket_type="NodeSocketVector")
    nodes, links = group.nodes, group.links
    outputs = nodes.new("NodeGroupOutput")
    outputs.location = (600, 0)

    coords = nodes.new("ShaderNodeTexCoord")
    coords.location = (-800, 0)
    separate = nodes.new("ShaderNodeSeparateXYZ")
    separate.location = (-600, 0)
    links.new(coords.outputs["Object"], separate.inputs[0])
    height = nodes.new("ShaderNodeValue")
    height.name = height.label = "Layer Height"
    height.outputs[0].default_value = 0.2
    height.location = (-600, -200)
    divide = nodes.new("ShaderNodeMath")
    divide.operation = "DIVIDE"
    divide.location = (-400, 0)
    links.new(separate.outputs["Z"], divide.inputs[0])
    links.new(height.outputs[0], divide.inputs[1])
    turns = nodes.new("ShaderNodeMath")
    turns.operation = "MULTIPLY"
    turns.inputs[1].default_value = math.tau
    turns.location = (-200, 0)
    links.new(divide.outputs[0], turns.inputs[0])
    wave = nodes.new("ShaderNodeMath")
    wave.operation = "SINE"
    wave.location = (0, 0)
    links.new(turns.outputs[0], wave.inputs[0])

    strength = nodes.new("ShaderNodeValue")
    strength.name = strength.label = "Strength"
    strength.outputs[0].default_value = 0.0
    strength.location = (0, -200)
    bump = nodes.new("ShaderNodeBump")
    bump.name = "Bump"
    bump.inputs["Distance"].default_value = 0.0005
    bump.location = (300, 0)
    links.new(strength.outputs[0], bump.inputs["Strength"])
    links.new(wave.outputs[0], bump.inputs["Height"])
    links.new(bump.outputs[0], outputs.inputs[0])
    return group


# ------------------------------------------------------------------ materials


def subject_material(name, base, roughness, extra, layers):
    material = bpy.data.materials.new(name)
    material["studio_material"] = True
    # Kept in the file even while no model uses it.
    material.use_fake_user = True
    material.use_nodes = True
    material.diffuse_color = (*base, 1)
    nodes, links = material.node_tree.nodes, material.node_tree.links
    nodes.clear()
    shader = nodes.new("ShaderNodeBsdfPrincipled")
    shader.location = (-300, 0)
    shader.inputs["Base Color"].default_value = (*base, 1)
    shader.inputs["Roughness"].default_value = roughness
    for key, value in extra.items():
        shader.inputs[key].default_value = value
    relief = nodes.new("ShaderNodeGroup")
    relief.node_tree = layers
    relief.label = "Layer Lines"
    relief.location = (-550, -300)
    links.new(relief.outputs[0], shader.inputs["Normal"])
    output = nodes.new("ShaderNodeOutputMaterial")
    output.location = (50, 0)
    links.new(shader.outputs[0], output.inputs["Surface"])
    return material


def cutter_box(name, size=4.0):
    """The box that the section removes: the +Z side of the Section Plane, beyond any subject."""
    mesh = bmesh.new()
    bmesh.ops.create_cube(mesh, size=1.0)
    bmesh.ops.scale(mesh, vec=(2 * size, 2 * size, size), verts=mesh.verts)
    bmesh.ops.translate(mesh, vec=(0, 0, size / 2), verts=mesh.verts)
    data = bpy.data.meshes.new(name)
    mesh.to_mesh(data)
    mesh.free()
    return bpy.data.objects.new(name, data)


def plain_material(name, colour, roughness):
    material = bpy.data.materials.new(name)
    material.use_nodes = True
    material.diffuse_color = (*colour, 1)
    shader = material.node_tree.nodes["Principled BSDF"]
    shader.inputs["Base Color"].default_value = (*colour, 1)
    shader.inputs["Roughness"].default_value = roughness
    return material


# ------------------------------------------------------------------ stage


def cyclorama(name, floor=12.0, bend=6.0, height=30.0, segments=128):
    """A round, seamless studio sweep: a floor that curves up into a wall all around.

    Cameras sit inside it, so it fills the background from every side and from above.
    """
    profile = [(r, 0.0) for r in (0.0, 1.5, 3.0, 4.5, 6.0, 7.5, 9.0, 10.5)]
    profile += [(floor + bend * math.sin(a), bend - bend * math.cos(a)) for a in (i * math.pi / 32 for i in range(17))]
    profile += [(floor + bend, z) for z in (bend + 4, bend + 10, height)]

    mesh = bmesh.new()
    centre = mesh.verts.new((0, 0, 0))
    rings = []
    for r, z in profile[1:]:
        rings.append([mesh.verts.new((r * math.cos(t), r * math.sin(t), z)) for t in (i * math.tau / segments for i in range(segments))])
    for i in range(segments):
        j = (i + 1) % segments
        mesh.faces.new((centre, rings[0][i], rings[0][j]))
        for inner, outer in zip(rings, rings[1:]):
            mesh.faces.new((inner[i], outer[i], outer[j], inner[j]))
    data = bpy.data.meshes.new(name)
    mesh.to_mesh(data)
    mesh.free()
    for polygon in data.polygons:
        polygon.use_smooth = True
    return bpy.data.objects.new(name, data)


def disc(name, radius=12.0, segments=96):
    mesh = bmesh.new()
    bmesh.ops.create_circle(mesh, cap_ends=True, radius=radius, segments=segments)
    data = bpy.data.meshes.new(name)
    mesh.to_mesh(data)
    mesh.free()
    return bpy.data.objects.new(name, data)


def placeholder(coll):
    """A smooth monkey head, so that the scene shows something before a model is loaded."""
    bpy.ops.mesh.primitive_monkey_add(size=2)
    obj = bpy.context.active_object
    obj.name = "Placeholder"
    obj.rotation_euler = (0, 0, 0)
    modifier = obj.modifiers.new("Subdivision", "SUBSURF")
    modifier.levels = modifier.render_levels = 2
    bpy.ops.object.modifier_apply(modifier=modifier.name)
    for other in list(obj.users_collection):
        other.objects.unlink(obj)
    return obj


def build(model=None):
    clear_scene()
    scene = bpy.context.scene
    scene.name = "Studio"

    stage = collection("Stage")
    rig = collection("Rig")
    lights = collection(st.LIGHTS_COLLECTION)
    cameras = collection("Cameras")
    subject = collection(st.SUBJECT_COLLECTION)

    # Rig: Turntable > Subject Pivot > model. The pivot fits the model to the stage;
    # the turntable turns it for videos.
    turntable = empty(st.TURNTABLE, rig, "CIRCLE", 1.3)
    pivot = empty(st.PIVOT, rig, "PLAIN_AXES", 0.4)
    pivot.parent = turntable
    focus = empty(st.FOCUS, rig, "SPHERE", 0.05)
    # The section plane turns with the model, so a cut model can also turn.
    plane = empty(st.SECTION_PLANE, rig, "SINGLE_ARROW", 0.8)
    plane.parent = turntable
    plane.location = (0, 0, 0.6)
    plane.rotation_euler = (math.pi / 2, 0, 0)
    cutter = link(cutter_box(st.SECTION_CUTTER), rig)
    cutter.parent = plane
    cutter.display_type = "WIRE"
    cutter.hide_render = True
    cutter.hide_select = True
    cutter.visible_camera = cutter.visible_shadow = False
    cutter.hide_set(True)  # hidden from view, still used by the Boolean

    layers = layer_group()
    for name, (base, roughness, extra) in MATERIALS.items():
        subject_material(name, base, roughness, extra, layers)
    highlight = plain_material(st.CUT_MATERIAL, CUT_HIGHLIGHT, 0.7)
    highlight.node_tree.nodes["Principled BSDF"].inputs["Specular IOR Level"].default_value = 0.15
    highlight.use_fake_user = True

    backdrop = link(cyclorama(st.BACKDROP), stage)
    backdrop.data.materials.append(plain_material(st.BACKDROP_MATERIAL, (0.012, 0.011, 0.010), 0.55))
    backdrop.hide_select = True
    catcher = link(disc(st.SHADOW_CATCHER), stage)
    catcher.is_shadow_catcher = True
    catcher.hide_render = True
    catcher.hide_viewport = True
    catcher.hide_select = True

    for name, (azimuth, elevation, distance, size, power, colour) in LIGHTS.items():
        data = bpy.data.lights.new(name, "AREA")
        data.shape = "DISK"
        data.size = size
        data.energy = power
        data.color = colour
        light = link(bpy.data.objects.new(name, data), lights)
        light["base_power"] = float(power)
        away = st.direction(azimuth, elevation)
        light.location = LIGHT_TARGET + away * distance
        light.rotation_euler = st.aim_rotation(away).to_euler()
        light.visible_camera = False

    for name, (azimuth, elevation, lens, fit_lens, fill, ortho) in CAMERAS.items():
        data = bpy.data.cameras.new(name)
        data.type = "ORTHO" if ortho else "PERSP"
        data.lens = lens
        data.sensor_width = 36
        data.clip_start = 0.05
        data.clip_end = 200
        camera = link(bpy.data.objects.new(name, data), cameras)
        camera["azimuth"] = float(azimuth)
        camera["elevation"] = float(elevation)
        camera["fit_lens"] = float(fit_lens)
        camera["fill"] = float(fill)
    detail = bpy.data.objects["CAM_Detail"].data
    detail.dof.use_dof = True
    detail.dof.focus_object = focus
    detail.dof.aperture_fstop = 4.0
    scene.camera = bpy.data.objects["CAM_Hero"]

    world = bpy.data.worlds.new("Studio")
    world.use_nodes = True
    scene.world = world

    configure_render(scene)

    objects = st.import_model(model) if model else [placeholder(subject)]
    st.set_subject(objects, material=st.DEFAULT_MATERIAL)
    scene["studio_subject"] = os.path.splitext(os.path.basename(model))[0] if model else "placeholder"
    st.set_look("slate")
    st.set_section("off")
    st.set_layer_lines(None)
    st.set_transparent(False)
    st.fit_cameras()

    text = bpy.data.texts.load(os.path.join(HERE, "studio_tools.py"), internal=True)
    text.name = "studio_tools.py"
    text.use_module = True
    st.register()
    viewport()


def configure_render(scene):
    scene.render.engine = "CYCLES"
    cycles = scene.cycles
    cycles.samples = 256
    cycles.preview_samples = 32
    cycles.use_adaptive_sampling = True
    cycles.adaptive_threshold = 0.01
    cycles.use_denoising = True
    cycles.use_preview_denoising = True
    cycles.max_bounces = 12
    cycles.diffuse_bounces = 4
    cycles.glossy_bounces = 4
    cycles.transmission_bounces = 8
    cycles.transparent_max_bounces = 16
    cycles.caustics_reflective = False
    cycles.caustics_refractive = False
    cycles.blur_glossy = 1.0
    cycles.device = "GPU"

    render = scene.render
    render.resolution_x, render.resolution_y = 2000, 1500
    render.resolution_percentage = 100
    render.fps = 30
    render.film_transparent = False
    render.image_settings.file_format = "PNG"
    render.image_settings.color_depth = "8"

    view = scene.view_settings
    view.view_transform = "AgX"
    try:
        view.look = "AgX - Medium High Contrast"
    except TypeError:
        pass
    view.exposure = 0.0
    scene.display_settings.display_device = "sRGB"
    scene.frame_start, scene.frame_end = 1, 120


def viewport():
    """Opens the file looking through the hero camera, in material preview."""
    for screen in bpy.data.screens:
        for area in screen.areas:
            if area.type != "VIEW_3D":
                continue
            space = area.spaces.active
            space.shading.type = "MATERIAL"
            space.shading.use_scene_lights = True
            space.shading.use_scene_world = True
            space.region_3d.view_perspective = "CAMERA"
            space.show_region_ui = True
            space.overlay.show_relationship_lines = False


def main():
    import argparse

    parser = argparse.ArgumentParser(prog="blender -b --factory-startup -P blender/build_studio.py --")
    parser.add_argument("--model", help="model to put on the stage instead of the placeholder")
    parser.add_argument("--out", default=os.path.join(HERE, "studio.blend"), help="where to save the scene")
    args = parser.parse_args(sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else [])

    build(args.model)
    bpy.ops.wm.save_as_mainfile(filepath=os.path.abspath(args.out), compress=True)
    st.log(f"saved {os.path.abspath(args.out)}")


if __name__ == "__main__":
    main()
