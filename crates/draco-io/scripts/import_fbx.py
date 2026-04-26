#!/usr/bin/env python3
"""Import an FBX file with bpy and print basic info.
Usage: python import_fbx.py <file.fbx>
"""
import sys

path = sys.argv[1]

try:
    import bpy
except Exception as e:
    print("bpy not available:", e)
    sys.exit(2)

print(f"Importing {path}")
# Clear existing data
bpy.ops.wm.read_factory_settings(use_empty=True)
try:
    bpy.ops.import_scene.fbx(filepath=path)
except Exception as e:
    print("Import failed:", e)
    sys.exit(3)

meshes = [o for o in bpy.data.objects if o.type == 'MESH']
print(f"Meshes: {len(meshes)}")
if meshes:
    m = meshes[0]
    verts = len(m.data.vertices)
    faces = len(m.data.polygons)
    print(f"First mesh: name={m.name}, verts={verts}, faces={faces}")

sys.exit(0)
