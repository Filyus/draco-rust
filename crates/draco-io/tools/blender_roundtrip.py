"""Blender helper script for round-trip testing.

Usage:
  # Export a test scene to the given output directory:
  blender --background --python tools/blender_roundtrip.py -- --export out_dir

  # Import a single file and print a JSON report to stdout:
  blender --background --python tools/blender_roundtrip.py -- --inspect path/to/file

The script creates two high-poly objects (subdivided plane and icosphere) and
exports OBJ, PLY, FBX and a GLB (with Draco compression when available).
"""

import sys
import os
import json
import argparse

try:
    import bpy
except Exception as e:
    print(json.dumps({"error": f"bpy import failed: {e}"}))
    sys.exit(1)


def enable_addons(mod_names):
    """Enable a list of addon module names (best-effort)."""
    for mod in mod_names:
        try:
            # Prefer addon_utils which handles enabling + reloading
            import addon_utils
            addon_utils.enable(mod)
        except Exception:
            try:
                bpy.ops.wm.addon_enable(module=mod)
            except Exception:
                # best-effort: ignore failures
                pass


def clear_scene():
    bpy.ops.object.select_all(action='SELECT')
    bpy.ops.object.delete(use_global=False)


def create_test_meshes():
    """Create a couple of meshes with many faces and varied topology (ngons, subsurf, primitives)."""
    clear_scene()

    # Subdivided plane (high poly quads)
    bpy.ops.mesh.primitive_plane_add(size=2.0, enter_editmode=False, location=(0, 0, 0))
    plane = bpy.context.active_object
    plane.name = "Plane_SUB"
    bpy.ops.object.modifier_add(type='SUBSURF')
    plane.modifiers['Subdivision'].levels = 6
    bpy.ops.object.modifier_apply(modifier='Subdivision')

    # Icosphere (triangulated topology)
    bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=4, radius=1.0, location=(3, 0, 0))
    ico = bpy.context.active_object
    ico.name = "Ico_Sphere"

    # Add a second copy for more meshes
    plane2 = plane.copy()
    plane2.data = plane.data.copy()
    plane2.location = (-3, 0, 0)
    plane2.name = "Plane_SUB_2"
    bpy.context.collection.objects.link(plane2)

    # Create an ngon by adding a circle and filling it (single face with many verts)
    bpy.ops.mesh.primitive_circle_add(vertices=12, radius=1.0, location=(6, 0, 0))
    ngon = bpy.context.active_object
    ngon.name = "Ngon_12"
    bpy.ops.object.mode_set(mode='EDIT')
    bpy.ops.mesh.fill()
    bpy.ops.object.mode_set(mode='OBJECT')

    # Add Suzanne (monkey) for complex, mixed topology
    bpy.ops.mesh.primitive_monkey_add(size=1.0, location=(9, 0, 0))
    monkey = bpy.context.active_object
    monkey.name = "Suzanne"

    # Add a torus
    bpy.ops.mesh.primitive_torus_add(location=(12, 0, 0))
    torus = bpy.context.active_object
    torus.name = "Torus"

    return [plane, ico, plane2, ngon, monkey, torus]


def export_scene(out_dir):
    os.makedirs(out_dir, exist_ok=True)

    meshes = create_test_meshes()

    # Ensure exporters are enabled (OBJ, PLY, FBX, glTF) via helper
    enable_addons(['io_scene_obj', 'io_mesh_ply', 'io_scene_fbx', 'io_scene_gltf2'])

    # Export OBJ
    obj_path = os.path.join(out_dir, 'scene.obj')
    def write_obj_fallback(path, meshes):
        # Simple OBJ exporter: writes all meshes into a single OBJ file.
        with open(path, 'w', encoding='utf-8') as f:
            v_offset = 1
            for m in meshes:
                f.write(f"o {m.name}\n")
                for v in m.data.vertices:
                    co = v.co
                    f.write(f"v {co.x} {co.y} {co.z}\n")
                for p in m.data.polygons:
                    # OBJ indices are 1-based and vertices in polygons are in order
                    idxs = [str(i + v_offset) for i in p.vertices]
                    f.write(f"f {' '.join(idxs)}\n")
                v_offset += len(m.data.vertices)

    try:
        bpy.ops.export_scene.obj(filepath=obj_path, use_selection=False, axis_forward='-Z', axis_up='Y')
    except Exception as e:
        # Fallback: write a basic OBJ file directly
        try:
            meshes = [o for o in bpy.context.scene.objects if o.type == 'MESH']
            write_obj_fallback(obj_path, meshes)
        except Exception as e2:
            print(json.dumps({'error': f'OBJ export failed: {e}; fallback failed: {e2}'}))
            return

    # Export PLY (requires io_mesh_ply addon usually available)
    ply_path = os.path.join(out_dir, 'scene.ply')
    def write_ply_fallback(path, meshes):
        # Simple ASCII PLY exporter for all meshes concatenated
        verts = []
        faces = []
        for m in meshes:
            for v in m.data.vertices:
                verts.append((v.co.x, v.co.y, v.co.z))
            for p in m.data.polygons:
                faces.append(tuple(p.vertices))
        with open(path, 'w', encoding='utf-8') as f:
            f.write('ply\nformat ascii 1.0\n')
            f.write(f'element vertex {len(verts)}\n')
            f.write('property float x\nproperty float y\nproperty float z\n')
            f.write(f'element face {len(faces)}\n')
            f.write('property list uchar int vertex_indices\nend_header\n')
            for v in verts:
                f.write(f"{v[0]} {v[1]} {v[2]}\n")
            for p in faces:
                f.write(str(len(p)) + ' ' + ' '.join(map(str, p)) + '\n')

    try:
        bpy.ops.export_mesh.ply(filepath=ply_path, use_selection=False)
    except Exception as e:
        try:
            meshes = [o for o in bpy.context.scene.objects if o.type == 'MESH']
            write_ply_fallback(ply_path, meshes)
        except Exception as e2:
            print(json.dumps({'error': f'PLY export failed: {e}; fallback failed: {e2}'}))
            return

    # Export FBX
    fbx_path = os.path.join(out_dir, 'scene.fbx')
    try:
        bpy.ops.export_scene.fbx(filepath=fbx_path, axis_forward='-Z', axis_up='Y')
    except Exception as e:
        print(json.dumps({'error': f'FBX export failed: {e}'}))
        return

    # Export glTF/GLB with Draco compression when available. Use safe
    # quantization parameters (0..30) to avoid invalid encoder state.
    glb_path = os.path.join(out_dir, 'scene.glb')
    draco_used = False
    if hasattr(bpy.ops.export_scene, 'gltf'):
        try:
            bpy.ops.export_scene.gltf(
                filepath=glb_path,
                export_format='GLB',
                export_draco_mesh_compression_enable=True,
                export_draco_position_quantization=14,
                export_draco_normal_quantization=8,
                export_draco_texcoord_quantization=12,
                export_draco_color_quantization=8,
                export_draco_generic_quantization=8,
            )
            draco_used = True
        except Exception:
            try:
                bpy.ops.export_scene.gltf(filepath=glb_path, export_format='GLB')
                draco_used = False
            except Exception as e:
                print(json.dumps({'error': f'glTF export failed: {e}'}))
                return
    else:
        draco_used = False

    # Collect mesh metadata for validation
    meshes_info = []
    has_ngon = False
    for m in meshes:
        max_face_verts = max((len(p.vertices) for p in m.data.polygons), default=0)
        is_ngon = max_face_verts > 4
        has_ngon = has_ngon or is_ngon
        meshes_info.append({
            'name': m.name,
            'faces': len(m.data.polygons),
            'verts': len(m.data.vertices),
            'max_face_verts': max_face_verts,
            'has_ngon': is_ngon,
        })

    report = {
        'paths': {
            'obj': obj_path,
            'ply': ply_path,
            'fbx': fbx_path,
            'glb': glb_path,
        },
        'draco_used': draco_used,
        'meshes': meshes_info,
        'has_ngon': has_ngon,
    }

    # Also write a deterministic report file so external callers can read it
    report_path = os.path.join(out_dir, 'blender_report.json')
    try:
        with open(report_path, 'w', encoding='utf-8') as rf:
            json.dump(report, rf)
    except Exception:
        pass

    print(json.dumps(report))


def inspect_file(path):
    """Import the given file into Blender and report counts."""
    clear_scene()
    ext = os.path.splitext(path)[1].lower()
    try:
        if ext == '.obj':
            bpy.ops.import_scene.obj(filepath=path)
        elif ext == '.ply':
            bpy.ops.import_mesh.ply(filepath=path)
        elif ext == '.fbx':
            bpy.ops.import_scene.fbx(filepath=path)
        elif ext in ['.glb', '.gltf']:
            bpy.ops.import_scene.gltf(filepath=path)
        else:
            print(json.dumps({'error': f'Unsupported extension: {ext}'}))
            return
    except Exception as e:
        print(json.dumps({'error': f'import failed: {e}'}))
        return

    meshes = [o for o in bpy.context.scene.objects if o.type == 'MESH']
    total_faces = sum(len(m.data.polygons) for m in meshes)
    total_verts = sum(len(m.data.vertices) for m in meshes)
    meshes_info = []
    has_ngon = False
    for m in meshes:
        max_face_verts = max((len(p.vertices) for p in m.data.polygons), default=0)
        is_ngon = max_face_verts > 4
        has_ngon = has_ngon or is_ngon
        meshes_info.append({ 'name': m.name, 'faces': len(m.data.polygons), 'verts': len(m.data.vertices), 'max_face_verts': max_face_verts, 'has_ngon': is_ngon })
    report = {
        'file': path,
        'mesh_count': len(meshes),
        'total_faces': total_faces,
        'total_vertices': total_verts,
        'meshes': meshes_info,
        'has_ngon': has_ngon,
    }
    # Write a deterministic report file next to the inspected file for callers
    try:
        report_path = os.path.splitext(path)[0] + '.report.json'
        with open(report_path, 'w', encoding='utf-8') as rf:
            json.dump(report, rf)
    except Exception:
        pass

    print(json.dumps(report))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--export', help='Export a test scene to this directory')
    parser.add_argument('--inspect', help='Import a file and print JSON report')
    args, unknown = parser.parse_known_args()

    # Prefer environment variables set by the caller if present (helps with
    # argument forwarding quirks in some Blender invocations).
    env_export = os.environ.get('BLENDER_EXPORT_DIR')
    env_inspect = os.environ.get('BLENDER_INSPECT_FILE')

    if args.export or env_export:
        export_target = args.export if args.export else env_export
        export_scene(export_target)
    elif args.inspect or env_inspect:
        inspect_target = args.inspect if args.inspect else env_inspect
        inspect_file(inspect_target)
    else:
        print(json.dumps({'error': 'no action specified'}))

if __name__ == '__main__':
    main()
