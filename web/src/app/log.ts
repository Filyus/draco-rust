import { consoleEl } from './dom.ts';

/**
 * The shell's two diagnostic channels.
 *
 * `log` is the visible console panel and carries anything a user should see.
 * `debugLog` stays off unless ?debug is in the query string: it exists so
 * importer and exporter work can dump payloads without burying that panel.
 */

const debugLogging = new URLSearchParams(globalThis.location?.search || '').has('debug');

export function debugLog(...values: unknown[]) {
    if (debugLogging) console.debug('[Draco debug]', ...values);
}

export function log(message: string, type = 'info') {
    const timestamp = new Date().toLocaleTimeString();
    const line = document.createElement('div');
    line.className = `console-line ${type}`;
    const timestampEl = document.createElement('span');
    timestampEl.className = 'timestamp';
    timestampEl.textContent = `[${timestamp}]`;
    line.append(timestampEl, document.createTextNode(` ${String(message)}`));
    consoleEl.appendChild(line);
    consoleEl.scrollTop = consoleEl.scrollHeight;
}

export function errorMessage(error: unknown) {
    if (error && typeof (error as Error).message === 'string') {
        return (error as Error).message;
    }
    return String(error);
}
