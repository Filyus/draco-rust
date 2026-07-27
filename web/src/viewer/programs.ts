import { createEnvironmentIbl } from '../environment-ibl.ts';
import { linkProgram } from './gl-utils.ts';
import {
  BACKGROUND_FRAG_SRC,
  BACKGROUND_VERT_SRC,
  FRAG_SRC,
  LINE_FRAG_SRC,
  LINE_VERT_SRC,
  VERT_SRC,
} from './shaders.ts';

/**
 * Shader programs and the uniform/attribute locations that address them.
 *
 * Attribute locations are fixed in the shader sources rather than queried, so
 * every primitive can be uploaded against one layout regardless of which
 * program is current.
 */

export function buildViewerPrograms(
  gl: WebGL2RenderingContext,
  onLog: (message: string, level: string) => void,
) {
  const program = linkProgram(gl, VERT_SRC, FRAG_SRC);
  const lineProgram = linkProgram(gl, LINE_VERT_SRC, LINE_FRAG_SRC);
  const backgroundProgram = linkProgram(gl, BACKGROUND_VERT_SRC, BACKGROUND_FRAG_SRC);

  const p = program;
  const uniforms = {
    uProjection: gl.getUniformLocation(p, 'uProjection'),
    uView: gl.getUniformLocation(p, 'uView'),
    uModel: gl.getUniformLocation(p, 'uModel'),
    uNormalMatrix: gl.getUniformLocation(p, 'uNormalMatrix'),
    uUseSkin: gl.getUniformLocation(p, 'uUseSkin'),
    uJointCount: gl.getUniformLocation(p, 'uJointCount'),
    uJointMatrix: gl.getUniformLocation(p, `uJointMatrix[0]`),
    uMorphDeltas: gl.getUniformLocation(p, 'uMorphDeltas'),
    uMorphCount: gl.getUniformLocation(p, 'uMorphCount'),
    uMorphStride: gl.getUniformLocation(p, 'uMorphStride'),
    uMorphWidth: gl.getUniformLocation(p, 'uMorphWidth'),
    uMorphWeights: gl.getUniformLocation(p, 'uMorphWeights[0]'),
    uMorphLayers: gl.getUniformLocation(p, 'uMorphLayers[0]'),
    uUseSmoothNormals: gl.getUniformLocation(p, 'uUseSmoothNormals'),
    uHasTexture: gl.getUniformLocation(p, 'uHasTexture'),
    uHasNormals: gl.getUniformLocation(p, 'uHasNormals'),
    uHasVertexColors: gl.getUniformLocation(p, 'uHasVertexColors'),
    uUnlit: gl.getUniformLocation(p, 'uUnlit'),
    uBaseColorOnly: gl.getUniformLocation(p, 'uBaseColorOnly'),
    uBaseColor: gl.getUniformLocation(p, 'uBaseColor'),
    uBaseColorFactor: gl.getUniformLocation(p, 'uBaseColorFactor'),
    uTexCoordSlot: gl.getUniformLocation(p, 'uTexCoordSlot[0]'),
    uTexMatrix: gl.getUniformLocation(p, 'uTexMatrix[0]'),
    uHasMetallicRoughnessTexture: gl.getUniformLocation(p, 'uHasMetallicRoughnessTexture'),
    uMetallicRoughness: gl.getUniformLocation(p, 'uMetallicRoughness'),
    uMetallic: gl.getUniformLocation(p, 'uMetallic'),
    uRoughness: gl.getUniformLocation(p, 'uRoughness'),
    uHasEmissiveTexture: gl.getUniformLocation(p, 'uHasEmissiveTexture'),
    uEmissive: gl.getUniformLocation(p, 'uEmissive'),
    uEmissiveFactor: gl.getUniformLocation(p, 'uEmissiveFactor'),
    uHasNormalTexture: gl.getUniformLocation(p, 'uHasNormalTexture'),
    uNormalTexture: gl.getUniformLocation(p, 'uNormalTexture'),
    uNormalScale: gl.getUniformLocation(p, 'uNormalScale'),
    uHasOcclusionTexture: gl.getUniformLocation(p, 'uHasOcclusionTexture'),
    uOcclusionTexture: gl.getUniformLocation(p, 'uOcclusionTexture'),
    uOcclusionStrength: gl.getUniformLocation(p, 'uOcclusionStrength'),
    uIor: gl.getUniformLocation(p, 'uIor'),
    uSpecularFactor: gl.getUniformLocation(p, 'uSpecularFactor'),
    uSpecularColorFactor: gl.getUniformLocation(p, 'uSpecularColorFactor'),
    uHasSpecularTexture: gl.getUniformLocation(p, 'uHasSpecularTexture'),
    uSpecularTexture: gl.getUniformLocation(p, 'uSpecularTexture'),
    uHasSpecularColorTexture: gl.getUniformLocation(p, 'uHasSpecularColorTexture'),
    uSpecularColorTexture: gl.getUniformLocation(p, 'uSpecularColorTexture'),
    uClearcoatFactor: gl.getUniformLocation(p, 'uClearcoatFactor'),
    uClearcoatRoughnessFactor: gl.getUniformLocation(p, 'uClearcoatRoughnessFactor'),
    uHasClearcoatTexture: gl.getUniformLocation(p, 'uHasClearcoatTexture'),
    uClearcoatTexture: gl.getUniformLocation(p, 'uClearcoatTexture'),
    uHasClearcoatRoughnessTexture: gl.getUniformLocation(p, 'uHasClearcoatRoughnessTexture'),
    uClearcoatRoughnessTexture: gl.getUniformLocation(p, 'uClearcoatRoughnessTexture'),
    uHasClearcoatNormalTexture: gl.getUniformLocation(p, 'uHasClearcoatNormalTexture'),
    uClearcoatNormalTexture: gl.getUniformLocation(p, 'uClearcoatNormalTexture'),
    uClearcoatNormalScale: gl.getUniformLocation(p, 'uClearcoatNormalScale'),
    uIrradianceMap: gl.getUniformLocation(p, 'uIrradianceMap'),
    uPrefilteredMap: gl.getUniformLocation(p, 'uPrefilteredMap'),
    uBrdfLut: gl.getUniformLocation(p, 'uBrdfLut'),
    uEnvironmentMaxLod: gl.getUniformLocation(p, 'uEnvironmentMaxLod'),
    uCameraPos: gl.getUniformLocation(p, 'uCameraPos'),
  };
  const locations = {
    position: 0,
    normal: 1,
    texCoord: 2,
    texCoord1: 6,
    color: 3,
    joints: 4,
    weights: 5,
    smoothNormal: 15,
  };
  const lineUniforms = {
    uProjectionView: gl.getUniformLocation(lineProgram, 'uProjectionView'),
    uColor: gl.getUniformLocation(lineProgram, 'uColor'),
  };
  const backgroundUniforms = {
    uInverseProjection: gl.getUniformLocation(backgroundProgram, 'uInverseProjection'),
    uInverseView: gl.getUniformLocation(backgroundProgram, 'uInverseView'),
    uEnvironment: gl.getUniformLocation(backgroundProgram, 'uEnvironment'),
  };
  // WebGL2 requires a VAO even for a shader driven solely by gl_VertexID.
  const backgroundVao = gl.createVertexArray();
  const environmentIbl = createEnvironmentIbl(gl, onLog);

  return {
    program,
    lineProgram,
    backgroundProgram,
    uniforms,
    locations,
    lineUniforms,
    backgroundUniforms,
    backgroundVao,
    environmentIbl,
  };
}
