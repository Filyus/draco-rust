/** Minimal ASCII FBX geometry reader used when a binary parser is unavailable.
 *
 * It deliberately handles the common Autodesk 7.x mesh form: Vertices and
 * PolygonVertexIndex arrays. Scene transforms, materials and animation remain
 * the responsibility of the binary FBX path.
 */

function arrayBody(text, property) {
    const match = new RegExp(`${property}\\s*:\\s*\\*?\\d*\\s*\\{\\s*a\\s*:\\s*([\\s\\S]*?)\\}`, 'i').exec(text);
    return match?.[1] || null;
}

function numericArray(text, property) {
    const body = arrayBody(text, property);
    if (!body) return [];
    return (body.match(/[-+]?\d*\.?\d+(?:[eE][-+]?\d+)?/g) || []).map(Number);
}

export function isAsciiFbx(bytes) {
    return new TextDecoder('utf-8', { fatal: false }).decode(bytes.subarray(0, 32)).startsWith('; FBX');
}

export function parseAsciiFbx(bytes) {
    const text = new TextDecoder('utf-8', { fatal: false }).decode(bytes);
    const positions = numericArray(text, 'Vertices');
    const polygonIndices = numericArray(text, 'PolygonVertexIndex');
    if (positions.length < 9 || polygonIndices.length < 3) {
        return { success: false, meshes: [], error: 'ASCII FBX has no supported mesh geometry', warnings: [] };
    }

    const indices = [];
    let polygon = [];
    for (const encoded of polygonIndices) {
        const index = encoded < 0 ? -encoded - 1 : encoded;
        polygon.push(index);
        if (encoded < 0) {
            for (let i = 1; i + 1 < polygon.length; i += 1) indices.push(polygon[0], polygon[i], polygon[i + 1]);
            polygon = [];
        }
    }
    if (indices.length === 0) {
        return { success: false, meshes: [], error: 'ASCII FBX contains no triangle polygons', warnings: [] };
    }
    return {
        success: true,
        meshes: [{ name: 'ASCII FBX mesh', positions, indices }],
        warnings: ['ASCII FBX preview imports mesh geometry only; transforms, materials, skins, and animation require binary FBX.'],
        version: null,
        scene: null,
        materials: [],
        textures: [],
        animations: [],
    };
}
