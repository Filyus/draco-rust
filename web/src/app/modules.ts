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

// Load all WASM modules
export async function loadAllModules() {
    // Cache-bust to ensure fresh WASM/JS are loaded (helps avoid stale cached files during development)
    const CACHE_BUST = `?v=${Date.now()}`;
    // Resolve against the page, not against this module: the packages sit next
    // to index.html, while this code is served from a subdirectory.
    const pkg = (name: string) => new URL(`pkg/${name}.js${CACHE_BUST}`, document.baseURI).href;
    const moduleConfigs = [
        { key: 'obj', path: pkg('obj'), statusId: 'obj-status' },
        { key: 'ply', path: pkg('ply'), statusId: 'ply-status' },
        { key: 'gltf', path: pkg('gltf'), statusId: 'gltf-status' },
        { key: 'fbx', path: pkg('fbx'), statusId: 'fbx-status' },
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
        const module = await import(path);
        const wasmUrl = new URL(path.replace(/\.js(\?.*)?$/, '_bg.wasm$1'), window.location.href);
        await module.default(wasmUrl);
        
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
