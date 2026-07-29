/**
 * `draco3d` ships no types of its own. This covers only what the interop test
 * actually calls against the official decoder module.
 */
declare module 'draco3d' {
  interface DecoderModule {
    Decoder: new () => any;
    DecoderBuffer: new () => any;
    Mesh: new () => any;
    destroy(instance: any): void;
  }

  function createDecoderModule(config: Record<string, unknown>): Promise<DecoderModule>;

  const draco3d: { createDecoderModule: typeof createDecoderModule };
  export default draco3d;
}
