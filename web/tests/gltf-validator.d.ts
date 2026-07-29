/**
 * `gltf-validator` ships no types of its own. This covers only what the
 * suite's `validateBytes` calls actually use.
 */
declare module 'gltf-validator' {
  interface ValidationMessage {
    severity: number;
    [key: string]: unknown;
  }

  interface ValidationResult {
    issues: {
      numErrors: number;
      messages: ValidationMessage[];
      [key: string]: unknown;
    };
    [key: string]: unknown;
  }

  interface ValidateOptions {
    uri?: string;
    externalResourceFunction?: (uri: string) => Promise<Uint8Array>;
  }

  function validateBytes(bytes: Uint8Array, options?: ValidateOptions): Promise<ValidationResult>;
}
