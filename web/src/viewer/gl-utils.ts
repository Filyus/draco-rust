/** Thin WebGL helpers shared by every pass. */

// Read off `globalThis` rather than named directly: this module is reachable
// from code that only wants `byteView`, and a bare reference throws a
// `ReferenceError` at import time wherever the class does not exist -- which
// is every Node test that never touches a GL context.
export const GL = (globalThis as { WebGL2RenderingContext?: typeof WebGL2RenderingContext })
  .WebGL2RenderingContext as typeof WebGL2RenderingContext;

// createShader and createProgram return null only on a lost context, where
// the original code already failed at its next call.
export function compileShader(gl: WebGL2RenderingContext, type: number, source: string): WebGLShader {
  const shader = gl.createShader(type)!;
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    const log = gl.getShaderInfoLog(shader);
    gl.deleteShader(shader);
    throw new Error(`Shader compile error: ${log}`);
  }
  return shader;
}

export function linkProgram(gl: WebGL2RenderingContext, vert: string, frag: string): WebGLProgram {
  const program = gl.createProgram()!;
  gl.attachShader(program, compileShader(gl, gl.VERTEX_SHADER, vert));
  gl.attachShader(program, compileShader(gl, gl.FRAGMENT_SHADER, frag));
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    const log = gl.getProgramInfoLog(program);
    gl.deleteProgram(program);
    throw new Error(`Program link error: ${log}`);
  }
  return program;
}

/** Return the original byte layout of an ArrayBuffer or typed-array view. */
export function byteView(data: unknown): Uint8Array {
  if (data instanceof Uint8Array) return data;
  if (ArrayBuffer.isView(data)) {
    return new Uint8Array(data.buffer as ArrayBuffer, data.byteOffset, data.byteLength);
  }
  if (data instanceof ArrayBuffer) return new Uint8Array(data);
  throw new Error('attribute payload is not binary data');
}
