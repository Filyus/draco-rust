import { createEnvironmentIbl } from '../environment-ibl.ts';
import { MATERIAL_EXTENSION_UNIFORMS } from '../material-extensions.ts';
import { linkProgram } from './gl-utils.ts';
import {
  BACKGROUND_FRAG_SRC,
  BACKGROUND_VERT_SRC,
  LINE_FRAG_SRC,
  LINE_VERT_SRC,
  TEXTURE_SLOTS,
  TEXTURE_SLOT_SAMPLERS,
  VERT_SRC,
  buildSurfaceFragmentSource,
} from './shaders.ts';
import type { TextureSlotName } from './shaders.ts';

/**
 * Shader programs and the uniform/attribute locations that address them.
 *
 * Attribute locations are fixed in the shader sources rather than queried, so
 * every primitive can be uploaded against one layout regardless of which
 * program is current.
 */

/** The uniform names every surface program declares, whatever its slots. */
const SURFACE_UNIFORMS = [
  'uProjection', 'uView', 'uModel', 'uNormalMatrix',
  'uUseSkin', 'uJointCount', 'uUseSmoothNormals',
  'uMorphDeltas', 'uMorphCount', 'uMorphStride', 'uMorphWidth',
  'uHasNormals', 'uHasVertexColors', 'uUnlit', 'uBaseColorOnly',
  'uBaseColorFactor', 'uMetallic', 'uRoughness', 'uEmissiveFactor',
  'uNormalScale', 'uOcclusionStrength',
  'uClearcoatNormalScale',
  'uIrradianceMap', 'uPrefilteredMap', 'uBrdfLut', 'uEnvironmentMaxLod',
  'uCameraPos',
] as const;

/** Array uniforms are addressed by their first element. */
const SURFACE_ARRAY_UNIFORMS = ['uJointMatrix', 'uMorphWeights', 'uMorphLayers', 'uTexCoordSlot', 'uTexMatrix'] as const;

export type SurfaceUniforms = Record<string, WebGLUniformLocation | null>;

export interface SurfaceProgram {
  program: WebGLProgram;
  uniforms: SurfaceUniforms;
  /** The slots this program declares, in the order it indexes them. */
  slots: readonly TextureSlotName[];
}

/**
 * One surface program per set of texture slots, built on demand.
 *
 * A material that binds a base colour map and nothing else gets a program with
 * one sampler; one that also carries clearcoat gets its own. That is what
 * keeps the sampler count a property of a material rather than of the whole
 * supported feature set — the alternative is one program declaring every slot,
 * which stops fitting in the sixteen units WebGL2 guarantees.
 *
 * Programs are cached by the slot set and live for the viewer's lifetime:
 * scenes reuse them, and a real asset settles on a handful.
 */
export interface SurfaceProgramCache {
  get(slots: readonly TextureSlotName[]): SurfaceProgram;
  /** How many distinct programs have been linked; read by tests. */
  readonly size: number;
  dispose(): void;
}

export function createSurfaceProgramCache(gl: WebGL2RenderingContext): SurfaceProgramCache {
  const cache = new Map<number, SurfaceProgram>();
  const key = (slots: readonly TextureSlotName[]) => slots
    .reduce((bits, name) => bits | (1 << TEXTURE_SLOTS.indexOf(name)), 0);

  return {
    get(slots) {
      const bits = key(slots);
      let entry = cache.get(bits);
      if (entry) return entry;
      const program = linkProgram(gl, VERT_SRC, buildSurfaceFragmentSource(slots));
      const uniforms: SurfaceUniforms = {};
      for (const name of SURFACE_UNIFORMS) uniforms[name] = gl.getUniformLocation(program, name);
      // The layered extension factors are named by the table rather than here,
      // so declaring one there is enough for the renderer to reach it.
      for (const { uniform } of MATERIAL_EXTENSION_UNIFORMS) {
        uniforms[uniform] = gl.getUniformLocation(program, uniform);
      }
      for (const name of SURFACE_ARRAY_UNIFORMS) uniforms[name] = gl.getUniformLocation(program, `${name}[0]`);
      for (const slot of slots) {
        const sampler = TEXTURE_SLOT_SAMPLERS[slot];
        uniforms[sampler] = gl.getUniformLocation(program, sampler);
      }
      entry = { program, uniforms, slots: [...slots] };
      cache.set(bits, entry);
      return entry;
    },
    get size() {
      return cache.size;
    },
    dispose() {
      for (const { program } of cache.values()) gl.deleteProgram(program);
      cache.clear();
    },
  };
}

export function buildViewerPrograms(
  gl: WebGL2RenderingContext,
  onLog: (message: string, level: string) => void,
) {
  const lineProgram = linkProgram(gl, LINE_VERT_SRC, LINE_FRAG_SRC);
  const backgroundProgram = linkProgram(gl, BACKGROUND_VERT_SRC, BACKGROUND_FRAG_SRC);

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
    surfacePrograms: createSurfaceProgramCache(gl),
    lineProgram,
    backgroundProgram,
    locations,
    lineUniforms,
    backgroundUniforms,
    backgroundVao,
    environmentIbl,
  };
}
