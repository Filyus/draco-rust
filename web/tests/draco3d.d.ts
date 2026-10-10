/**
 * `draco3d` ships no types of its own. This covers only what the interop and
 * attribute-parity tests call against the official modules.
 */
declare module 'draco3d' {
  interface DecoderModule {
    Decoder: new () => any;
    DecoderBuffer: new () => any;
    Mesh: new () => any;
    destroy(instance: any): void;
  }

  function createDecoderModule(config: Record<string, unknown>): Promise<DecoderModule>;
  /** The encoder module; typed loosely, as only one test drives it. */
  function createEncoderModule(config: Record<string, unknown>): Promise<any>;

  const draco3d: {
    createDecoderModule: typeof createDecoderModule;
    createEncoderModule: typeof createEncoderModule;
  };
  export default draco3d;
}
