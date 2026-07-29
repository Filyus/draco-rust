// Checks that each optional glTF wasm API is exported exactly when its cargo
// feature is on, so a profile cannot quietly ship a method it excluded or drop
// one it promised.
//
// The profile under test is named by flags, or taken from the build stamp when
// none are given:
//
//   npm run test:gltf-features                 # whatever ./build.ps1 built
//   npm run test:gltf-features -- --accessors  # after building that profile
//
// Naming a profile the package was not built with is refused rather than
// reported as a broken gate: that mistake is a stale www/pkg, not a defect.

import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { embeddedTriangle } from './smoke-fixtures.ts';

const here = dirname(fileURLToPath(import.meta.url));
const pkg = resolve(here, '..', 'www', 'pkg');

const PROFILE_FLAGS = ['--write', '--draco-encode', '--accessors', '--strict-validation', '--raw-resources'];

/**
 * The feature profile the package in `www/pkg` was actually built with.
 *
 * The build tool records the feature list it passed to cargo in a stamp beside
 * the wasm. It never passes `--no-default-features`, so `read` and
 * `draco-decode` are always on; the only implication that matters here is that
 * `draco-encode` pulls in `write`.
 */
async function builtProfile() {
  let stamp;
  try {
    stamp = JSON.parse(await readFile(resolve(pkg, 'gltf.build-stamp.json'), 'utf8'));
  } catch {
    return null;
  }
  const features = new Set((/features=([^;]*)/.exec(stamp.config_key)?.[1] ?? '').split(',').filter(Boolean));
  return {
    features: [...features].sort(),
    flags: new Set([
      ...(features.has('write') || features.has('draco-encode') ? ['--write'] : []),
      ...(features.has('draco-encode') ? ['--draco-encode'] : []),
      ...(features.has('accessors') ? ['--accessors'] : []),
      ...(features.has('strict-validation') ? ['--strict-validation'] : []),
      ...(features.has('raw-resources') ? ['--raw-resources'] : []),
    ]),
  };
}

const built = await builtProfile();
const requested = PROFILE_FLAGS.filter((flag) => process.argv.includes(flag));

// The gates below assert a method is present exactly when its feature is on,
// so the expected profile has to be the one the package was built with. CI
// rebuilds for each profile and names it on the command line; a developer
// running this bare gets whatever the last build produced, and asking them to
// remember its feature list is how this ends up asserting a profile that
// nothing builds.
let profile;
if (requested.length > 0) {
  profile = new Set(requested);
  const disagrees = built
    && (profile.size !== built.flags.size || [...profile].some((flag) => !built.flags.has(flag)));
  if (disagrees) {
    throw new Error(
      `asked for the ${[...profile].join(' ')} profile but www/pkg was built with `
      + `features=${built.features.join(',')}, which is the ${[...built.flags].join(' ') || 'read only'} profile.\n`
      + 'Rebuild for the profile you want to test:\n'
      + '  cargo run --manifest-path build-tool/Cargo.toml -- --module gltf-wasm --features <list> --force\n'
      + 'or run with no flags to test whatever is built.',
    );
  }
} else if (built) {
  profile = built.flags;
} else {
  throw new Error('no gltf.build-stamp.json in www/pkg: build the package first, or name the profile with --write/--accessors/...');
}

const wantsWrite = profile.has('--write');
const wantsEncoder = profile.has('--draco-encode');
const wantsAccessors = profile.has('--accessors');
const wantsStrictValidation = profile.has('--strict-validation');
const wantsRawResources = profile.has('--raw-resources');
const api = await import(pathToFileURL(resolve(pkg, 'gltf.js')));
const wasm = await readFile(resolve(pkg, 'gltf_bg.wasm'));
await api.default({ module_or_path: wasm });

const asset = new api.GltfAsset(new TextEncoder().encode(embeddedTriangle()), '2.0');

/** A method must be exported exactly when the feature gating it is on. */
function gate(method, expected, feature) {
  const present = typeof asset[method] === 'function';
  if (present === expected) return;
  throw new Error(
    `${method} is ${present ? 'exported' : 'missing'} but the ${feature} feature is ${expected ? 'on' : 'off'}`
    + ` in the profile under test (${[...profile].join(' ') || 'read only'})`
    + (built ? `, built from features=${built.features.join(',')}` : ''),
  );
}

gate('decompress', wantsWrite, 'write');
gate('compressPrimitive', wantsEncoder, 'draco-encode');
gate('bufferBytes', wantsRawResources, 'raw-resources');
gate('bufferViewBytes', wantsRawResources, 'raw-resources');
gate('readAccessor', wantsAccessors, 'accessors');
gate('validate', wantsStrictValidation, 'strict-validation');
if (wantsAccessors) {
  const accessor = asset.readAccessor(0);
  if (accessor.count() !== 3 || accessor.components() !== 3 || accessor.bytes().length !== 36) {
    throw new Error('glTF accessor API returned invalid data');
  }
}
if (wantsRawResources) {
  if (asset.bufferCount() !== 1 || asset.bufferBytes(0).length !== 36) {
    throw new Error('glTF raw resource API returned an invalid buffer');
  }
}
if (wantsEncoder) {
  const bytes = asset.compressPrimitive(0, 0, 5, 5);
  if (bytes === 0) {
    throw new Error('glTF Draco encoder wrote no payload');
  }
  const reloaded = new api.GltfAsset(asset.glb(2), '2.0').summary();
  if (!reloaded.success || !reloaded.usesDraco) {
    throw new Error(`glTF Draco output failed to reload: ${JSON.stringify(reloaded)}`);
  }
}
console.log(
  `glTF features smoke passed (${[
    wantsWrite && 'write',
    wantsEncoder && 'encode',
    wantsAccessors && 'accessors',
    wantsStrictValidation && 'strict-validation',
    wantsRawResources && 'raw-resources',
  ].filter(Boolean).join(',') || 'read'})`,
);
