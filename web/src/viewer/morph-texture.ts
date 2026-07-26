import { byteView } from './gl-utils.ts';
import type { RuntimeAccessor, ViewerPrimitive } from '../viewer-scene.ts';

/** Morph target deltas, packed into an array texture for the shader loop. */

function fillMorphLayer(
    layer: Float32Array,
    attr: RuntimeAccessor | null,
    vertexCount: number,
    stride: number,
    slot: number,
): boolean {
    if (!attr || attr.componentType !== 5126 || attr.components !== 3) return false;
    const bytes = byteView(attr.bytes);
    const count = Math.min(vertexCount, attr.count);
    if (bytes.byteLength < count * 12) return false;
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    for (let i = 0; i < count; i++) {
        const texel = (i * stride + slot) * 4;
        layer[texel] = view.getFloat32(i * 12, true);
        layer[texel + 1] = view.getFloat32(i * 12 + 4, true);
        layer[texel + 2] = view.getFloat32(i * 12 + 8, true);
    }
    return true;
}

/**
 * Pack every morph target of a primitive into one RGBA32F array texture: a
 * layer per target, `stride` texels per vertex, addressed by gl_VertexID.
 *
 * Vertex attributes are capped at 16 by WebGL2 and the preview already spends
 * them all, so attribute-fed deltas could never exceed four targets. A texture
 * has no such budget, which is what lets a mesh declare any number of targets.
 */
export function uploadMorphTexture(
    gl: WebGL2RenderingContext,
    primitive: ViewerPrimitive,
    vertexCount: number,
) {
    const positions = primitive.morphPositions || [];
    const normals = primitive.morphNormals || [];
    const targetCount = Math.max(positions.length, normals.length);
    if (targetCount === 0 || vertexCount === 0) return null;

    const layerCount = Math.min(targetCount, gl.getParameter(gl.MAX_ARRAY_TEXTURE_LAYERS));
    const stride = normals.some(Boolean) ? 2 : 1;
    const texels = vertexCount * stride;
    const maxSize = gl.getParameter(gl.MAX_TEXTURE_SIZE);
    const width = Math.max(1, Math.min(maxSize, Math.ceil(Math.sqrt(texels))));
    const height = Math.ceil(texels / width);
    if (height > maxSize) return null;

    const texture = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D_ARRAY, texture);
    gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.texStorage3D(gl.TEXTURE_2D_ARRAY, 1, gl.RGBA32F, width, height, layerCount);

    const layer = new Float32Array(width * height * 4);
    // A target whose accessor was rejected upstream stays an all-zero layer, so
    // its weight simply contributes nothing.
    const filled = new Array(layerCount).fill(false);
    for (let target = 0; target < layerCount; target++) {
        layer.fill(0);
        filled[target] = fillMorphLayer(layer, positions[target], vertexCount, stride, 0);
        if (stride > 1) {
            filled[target] = fillMorphLayer(layer, normals[target], vertexCount, stride, 1)
                || filled[target];
        }
        gl.texSubImage3D(
            gl.TEXTURE_2D_ARRAY, 0, 0, 0, target, width, height, 1, gl.RGBA, gl.FLOAT, layer,
        );
    }
    gl.bindTexture(gl.TEXTURE_2D_ARRAY, null);
    return { texture, width, stride, layerCount, filled, dropped: targetCount - layerCount };
}

/**
 * Read `WEIGHTS_0` as unit-range scalars whatever storage the source chose.
 *
 * Returns null for a component type the glTF skin contract does not allow, so
 * the caller can fall back to uploading the attribute untouched.
 */
