import { buildNormalizedWeightAttribute, buildSmoothNormalAttribute } from './geometry.js';
import { byteView } from './gl-utils.js';
import { uploadMorphTexture } from './morph-texture.js';

/** One primitive's VAO, buffers and attribute layout. */

export function uploadPrimitive(gl, primitive, locationMap) {
    const vao = gl.createVertexArray();
    gl.bindVertexArray(vao);

    const buffers = [];
    const positions = primitive.attributes.POSITION;
    if (!positions) throw new Error('primitive is missing POSITION attribute');

    function bindAccessor(attr, semantic, location, desiredComponents) {
        if (!attr || location < 0) return false;
        if (desiredComponents && attr.components !== desiredComponents) {
            gl.disableVertexAttribArray(location);
            return false;
        }
        const buf = gl.createBuffer();
        gl.bindBuffer(gl.ARRAY_BUFFER, buf);
        gl.bufferData(gl.ARRAY_BUFFER, byteView(attr.bytes), gl.STATIC_DRAW);
        buffers.push(buf);
        const normalized = attr.normalized
            || semantic.startsWith('COLOR_') || semantic.startsWith('WEIGHTS_');
        gl.enableVertexAttribArray(location);
        gl.vertexAttribPointer(location, attr.components, attr.componentType, normalized, 0, 0);
        return true;
    }

    function bindAttribute(semantic, location, desiredComponents) {
        return bindAccessor(primitive.attributes[semantic], semantic, location, desiredComponents);
    }

    const skinWeights = buildNormalizedWeightAttribute(primitive);
    const layout = {
        position: locationMap.position,
        normal: locationMap.normal,
        texCoord: locationMap.texCoord,
        texCoord1: locationMap.texCoord1,
        color: locationMap.color,
        joints: locationMap.joints,
        weights: locationMap.weights,
        smoothNormal: locationMap.smoothNormal,
    };

    const info = {
        vao,
        buffers,
        hasNormals: !!bindAttribute('NORMAL', layout.normal),
        hasSmoothNormals: !!bindAccessor(
            buildSmoothNormalAttribute(primitive),
            'SMOOTH_NORMAL',
            layout.smoothNormal,
            3,
        ),
        hasTexCoords0: !!bindAttribute('TEXCOORD_0', layout.texCoord),
        hasTexCoords1: !!bindAttribute('TEXCOORD_1', layout.texCoord1),
        hasColors: !!bindAttribute('COLOR_0', layout.color),
        hasJoints: !!bindAttribute('JOINTS_0', layout.joints),
        hasWeights: !!bindAccessor(skinWeights.attribute, 'WEIGHTS_0', layout.weights),
        driftedWeights: skinWeights.drifted,
        mode: primitive.mode,
        elementCount: 0,
        indexed: false,
    };

    bindAttribute('POSITION', layout.position);
    // Layers are indexed exactly like the mesh weights, so picking targets at
    // draw time is a plain lookup by target index.
    info.morph = uploadMorphTexture(gl, primitive, positions.count);
    info.morphTargetCount = info.morph ? info.morph.layerCount : 0;

    let indexBuffer = null;
    if (primitive.indices) {
        const idx = primitive.indices;
        const bytes = byteView(idx.bytes);
        indexBuffer = gl.createBuffer();
        gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, indexBuffer);
        gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, bytes, gl.STATIC_DRAW);
        info.indexed = true;
        info.elementCount = idx.count;
        info.indexType = idx.componentType;
        buffers.push(indexBuffer);
    } else {
        info.indexed = false;
        info.elementCount = positions.count;
    }

    gl.bindVertexArray(null);
    return info;
}
