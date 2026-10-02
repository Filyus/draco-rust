import { dracoSettings, element, useDraco, useDracoLabel } from './dom.ts';
import { errorMessage, log } from './log.ts';
import { modules, state } from './state.ts';

/**
 * Loading the per-format wasm-pack modules and reflecting their status.
 *
 * Each module is fetched on demand and its pill in the header follows the
 * outcome, so a format that failed to load is visible rather than merely
 * inert when a file of that type is dropped.
 */

/**
 * The query that keeps a module's glue and its wasm from the same build.
 *
 * A deployed page carries its build in `<meta name="build-id">`, stamped by
 * the Pages workflow beside an import map that gives every script of the page
 * the same query: the page, its scripts and these modules then come from one
 * build together -- all of them cached, or all of them fetched -- and a cached
 * page cannot pair with modules from the deploy after it. Served without the
 * stamp, as the dev server does, every load fetches them afresh.
 */
const CACHE_BUST = `?v=${
  document.querySelector<HTMLMetaElement>('meta[name="build-id"]')?.content || Date.now()
}`;

// Resolve against the page, not against this module: the packages sit next
// to index.html, while this code is served from a subdirectory.
const pkg = (file: string) => new URL(`pkg/${file}${CACHE_BUST}`, document.baseURI);

// Load all WASM modules
export async function loadAllModules() {
  const pkgModule = (name: string) => pkg(`${name}.js`).href;
  const moduleConfigs = [
    { key: 'obj', path: pkgModule('obj'), statusId: 'obj-status' },
    { key: 'ply', path: pkgModule('ply'), statusId: 'ply-status' },
    { key: 'stl', path: pkgModule('stl'), statusId: 'stl-status' },
    { key: 'drc', path: pkgModule('drc'), statusId: 'drc-status' },
    { key: 'gltf', path: pkgModule('gltf'), statusId: 'gltf-status' },
    { key: 'fbx', path: pkgModule('fbx'), statusId: 'fbx-status' },
  ];

  const loadPromises = moduleConfigs.map(config => loadModule(config));
  await Promise.allSettled(loadPromises);
}

// Load a single WASM module
export async function loadModule({ key, path, statusId }: { key: string; path: string; statusId: string }) {
  const statusEl = element(statusId);
  const indicator = statusEl.querySelector('.status-indicator')!;
  // ensure initial loading state
  if (indicator) {
    indicator.classList.remove('ready','error');
    indicator.classList.add('loading');
    const statusTextInit = indicator.querySelector('.status-text');
    if (statusTextInit) statusTextInit.textContent = 'Loading...';
    statusEl.removeAttribute('aria-label');
  }
  
  try {
    const module = await instantiate(key, path);
    modules[key].module = module;
    modules[key].loaded = true;
    if (key === 'gltf') {
      updateDracoEncoderAvailability();
    }
    
    // Update visual indicator (dot + aria label)
    const statusText = indicator.querySelector('.status-text');
    const statusDot = indicator.querySelector('.status-dot');
    if (statusText) statusText.textContent = 'Ready';
    indicator.classList.remove('loading','error');
    indicator.classList.add('ready');
    indicator.setAttribute('aria-label', 'Ready');
    if (statusDot) {
      statusDot.classList.remove('dot-loading','dot-error','dot-ready');
      // visual state is controlled by the parent .status-indicator class
    }
    
    const version = module.version ? module.version() : '?';
    log(`${key} v${version} loaded`, 'success');
  } catch (error) {
    const statusText = indicator.querySelector('.status-text');
    const statusDot = indicator.querySelector('.status-dot');
    if (statusText) statusText.textContent = 'Error';
    indicator.classList.remove('loading','ready');
    indicator.classList.add('error');
    indicator.setAttribute('aria-label', 'Error');
    if (statusDot) {
      statusDot.classList.remove('dot-loading','dot-ready','dot-error');
      // visual state is controlled by the parent .status-indicator class
    }
    log(`Failed to load ${key}: ${errorMessage(error)}`, 'error');
  }
}

/** Where a wasm32 memory stops growing: 65536 pages of 64 KiB. */
const WASM32_MEMORY_CAP = 2 ** 32;

/** How many fresh instances have been made, so each import is a new one. */
let instances = 0;

/**
 * A fresh instance of a module, with every plain function it exports guarded.
 *
 * The glue keeps its instance in module scope and will not initialise twice,
 * so a fresh one needs the glue imported again under a URL it has not been
 * imported under. The wasm is the same file and comes from the cache.
 */
async function instantiate(key: string, path: string) {
  const fresh = instances++ === 0 ? path : `${path}${path.includes('?') ? '&' : '?'}instance=${instances}`;
  const module = await import(fresh);
  const wasmUrl = new URL(path.replace(/\.js(\?.*)?$/, '_bg.wasm$1'), window.location.href);
  // wasm-bindgen deprecated the positional form and warns about it on
  // every load; the object is what current glue expects.
  const exports = await module.default({ module_or_path: wasmUrl });
  return guard(key, path, module, exports.memory as WebAssembly.Memory);
}

/**
 * The module with its calls watched for a trap.
 *
 * A release build aborts on a panic, and running out of memory is one, so
 * either arrives as `RuntimeError: unreachable` -- and leaves the instance
 * holding everything it had allocated, because no destructor ran. A 4 GiB
 * memory that a large file filled stays full, and every file after it fails
 * the same way however small. So a trapped module is dropped and made again
 * with an empty memory, and the error says what happened in words: when the
 * memory stood at the cap, it ran out of it.
 *
 * Classes pass through unwrapped -- `new` on a wrapper is not `new` on the
 * class -- and so do their methods; a trap there is still reported, only not
 * recovered from.
 */
function guard(key: string, path: string, module: any, memory: WebAssembly.Memory): any {
  const wrapped: Record<string, unknown> = {};
  for (const [name, value] of Object.entries(module)) {
    const isClass = typeof value === 'function' && /^class\b/.test(Function.prototype.toString.call(value));
    wrapped[name] = typeof value !== 'function' || isClass || name === 'default' || name === 'initSync'
      ? value
      : (...args: unknown[]) => {
        try {
          return (value as (...args: unknown[]) => unknown)(...args);
        } catch (error) {
          if (!(error instanceof WebAssembly.RuntimeError)) throw error;
          throw restart(key, path, memory, error);
        }
      };
  }
  return wrapped;
}

/** Drop a trapped module, start making a fresh one, and say what happened. */
function restart(key: string, path: string, memory: WebAssembly.Memory, error: Error) {
  const full = memory.buffer.byteLength > WASM32_MEMORY_CAP - 64 * 2 ** 20;
  modules[key].loaded = false;
  modules[key].module = null;
  instantiate(key, path).then((module) => {
    modules[key].module = module;
    modules[key].loaded = true;
    log(`${key} module restarted with empty memory`, 'info');
  }, (reload) => log(`Failed to restart ${key}: ${errorMessage(reload)}`, 'error'));
  return new Error(full
    ? `the ${key} module ran out of memory: a WebAssembly module holds at most 4 GiB, and this file needs more`
    : `the ${key} module stopped (${error.message}) and is being restarted`);
}

/**
 * Load the KTX2 transcoder, the first time a file needs one.
 *
 * Not part of `loadAllModules`: it carries the baked block-format tables and
 * is several times the size of the others, and most files never meet a KTX2
 * texture. A failure here is reported by the caller as a texture that could
 * not be decoded, which is what it means to the user.
 */
export async function loadKtx2Module() {
  if (modules.ktx2.loaded) return modules.ktx2.module;
  try {
    const module = await import(pkg('ktx2.js').href);
    await module.default({ module_or_path: pkg('ktx2_bg.wasm') });
    modules.ktx2.module = module;
    modules.ktx2.loaded = true;
    log(`ktx2 v${module.version ? module.version() : '?'} loaded`, 'success');
    return module;
  } catch (error) {
    log(`Failed to load ktx2: ${errorMessage(error)}`, 'error');
    return null;
  }
}

export function updateDracoEncoderAvailability() {
  const prototype = modules.gltf.module?.GltfAsset?.prototype;
  const available = typeof prototype?.compressPrimitive === 'function';
  useDraco.disabled = !available;
  useDraco.checked = available;
  useDracoLabel.textContent = available
    ? 'Enable Draco Compression'
    : 'Draco Compression (not included in this build)';
  dracoSettings.style.display = available && useDraco.checked ? 'grid' : 'none';
}
