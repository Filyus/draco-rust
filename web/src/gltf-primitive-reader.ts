import type { GltfAsset, PackedGeometry } from './wasm-modules.ts';

/**
 * How many primitives one `readPrimitives` call takes. The document is
 * validated once a call, so a batch saves that many validations, and a batch
 * is held in WASM memory until its primitives are read, so it stays small.
 */
const PRIMITIVE_BATCH = 32;

/**
 * Stands in for `asset.readPrimitive` when the primitives are read in an
 * order known up front: each `read` returns the same geometry, and the reader
 * fetches the next batch with one `readPrimitives` call.
 *
 * The caller frees what `read` returns, as with `readPrimitive`, and calls
 * `dispose` when done, which frees whatever was fetched and not read. A
 * request off the order given is read on its own with `readPrimitive`.
 */
export class PrimitiveReader {
  private readonly asset: GltfAsset;
  private readonly order: ReadonlyArray<readonly [number, number]>;
  private readonly batch: number;
  private pending: PackedGeometry[] = [];
  private consumed = 0;

  constructor(
    asset: GltfAsset,
    order: ReadonlyArray<readonly [number, number]>,
    batch = PRIMITIVE_BATCH,
  ) {
    this.asset = asset;
    this.order = order;
    this.batch = Math.max(1, batch);
  }

  read(mesh: number, primitive: number): PackedGeometry {
    const expected = this.order[this.consumed];
    if (!expected || expected[0] !== mesh || expected[1] !== primitive) {
      return this.asset.readPrimitive(mesh, primitive);
    }
    if (this.pending.length === 0) {
      const slice = this.order.slice(this.consumed, this.consumed + this.batch);
      const pairs = new Uint32Array(slice.length * 2);
      slice.forEach(([meshIndex, primitiveIndex], index) => {
        pairs[index * 2] = meshIndex;
        pairs[index * 2 + 1] = primitiveIndex;
      });
      this.pending = this.asset.readPrimitives(pairs);
    }
    this.consumed += 1;
    return this.pending.shift()!;
  }

  dispose(): void {
    for (const packed of this.pending) packed.free();
    this.pending = [];
  }
}

/** Every primitive of every mesh, in document order. */
export function documentPrimitiveOrder(
  primitiveCounts: ReadonlyArray<number>,
): Array<[number, number]> {
  const order: Array<[number, number]> = [];
  primitiveCounts.forEach((count, mesh) => {
    for (let primitive = 0; primitive < count; primitive += 1) order.push([mesh, primitive]);
  });
  return order;
}
