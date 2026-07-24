"""Blender 5 headless validation for an exported FBX scene.

Usage:
  blender --background --python blender-fbx-smoke.py -- path/to/file.fbx
"""

import sys

import bpy


def main():
    try:
        path = sys.argv[sys.argv.index("--") + 1]
    except (ValueError, IndexError):
        raise SystemExit("expected an FBX path after --")

    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.wm.fbx_import(filepath=path)

    meshes = [obj for obj in bpy.context.scene.objects if obj.type == "MESH"]
    if not meshes:
        raise SystemExit("FBX import produced no mesh objects")
    uv_meshes = sum(bool(obj.data.uv_layers) for obj in meshes)
    material_meshes = sum(bool(obj.data.materials) for obj in meshes)

    armatures = [obj for obj in bpy.context.scene.objects if obj.type == "ARMATURE"]
    if armatures and not any(obj.animation_data and obj.animation_data.action for obj in armatures):
        raise SystemExit("FBX import produced an armature without an assigned action")
    if armatures and any(
        not any(mod.type == "ARMATURE" and mod.object in armatures for mod in obj.modifiers)
        for obj in meshes
    ):
        raise SystemExit("FBX mesh is not connected to the imported armature")

    shape_key_meshes = [obj for obj in meshes if obj.data.shape_keys]
    for obj in shape_key_meshes:
        if len(obj.data.shape_keys.key_blocks) < 2:
            raise SystemExit(f"{obj.name} has no imported blend shape")
        if obj.data.shape_keys.animation_data and not obj.data.shape_keys.animation_data.action:
            raise SystemExit(f"{obj.name} blend-shape action is missing")

    print(
        "Blender FBX smoke passed:",
        f"meshes={len(meshes)}",
        f"uv_meshes={uv_meshes}",
        f"material_meshes={material_meshes}",
        f"armatures={len(armatures)}",
        f"blend_shapes={len(shape_key_meshes)}",
        f"actions={len(bpy.data.actions)}",
    )


if __name__ == "__main__":
    main()
