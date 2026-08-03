import { formatFileSize } from './format.ts';
import { errorMessage, log } from './log.ts';
import { colorBits, dracoOptions, encodingSpeed, exportFormat, exportStatFields, exportStats, fbxOptions, genericBits, includeNormals, includeUvs, normalBits, positionBits, texcoordBits, useDraco, useFbxCompression, useFbxLegacy } from './dom.ts';
import { state } from './state.ts';
import { runExport } from './export-branches.ts';
import type { DracoStats, ExportOutcome, ExportResult, ExportSettings } from './export-branches.ts';
import { setWarningSource } from './warnings.ts';

/**
 * The export panel: reading its controls, driving one route, and reporting
 * what that route cost.
 *
 * Route selection and every writer call live in export-branches.ts, which has
 * no DOM dependency and can therefore be tested. What remains here is the
 * browser half — the controls, the console, the download — plus the single
 * place where an export's warnings reach the warnings card.
 */

// Update export options based on format
export function updateExportOptions() {
  const format = exportFormat.value;

  // The Draco controls belong to whatever the encoder will actually read: the
  // glTF pass, and the .drc container, which is the same encoder without a
  // document around it.
  if (format === 'gltf' || format === 'glb' || format === 'drc') {
    dracoOptions.style.display = 'flex';
  } else {
    dracoOptions.style.display = 'none';
  }
  fbxOptions.style.display = format === 'fbx' || format === 'fbx-legacy' ? 'block' : 'none';
}

/**
 * Put away what the last export reported.
 *
 * The panel describes a file that was written, not the one that is open, so it
 * has to go the moment a different file is loaded. Left up, it reads as a
 * property of the new file rather than as a report about the previous export.
 */
export function clearExportStats() {
  exportStats.style.display = 'none';
}

/** The export controls as the routes want them: plain values, read once. */
function exportSettings(): ExportSettings {
  return {
    format: exportFormat.value,
    includeNormals: includeNormals.checked,
    includeUvs: includeUvs.checked,
    useDraco: useDraco.checked,
    fbxCompression: useFbxCompression.checked,
    fbxLegacyCompatibility: useFbxLegacy.checked,
    encodingSpeed: Number(encodingSpeed.value),
    positionBits: Number(positionBits.value),
    normalBits: Number(normalBits.value),
    texcoordBits: Number(texcoordBits.value),
    colorBits: Number(colorBits.value),
    genericBits: Number(genericBits.value),
  };
}

// Export file
export async function exportFile() {
  if (!state.currentMeshData) {
    log('No mesh data to export', 'error');
    return;
  }

  const settings = exportSettings();
  log(`Exporting to ${settings.format.toUpperCase()}...`, 'info');

  try {
    const outcome = await runExport(settings);
    // One call site for every route. Routes that lose nothing return an empty
    // list, which still clears whatever the previous export reported.
    setWarningSource('export', outcome.warnings);
    for (const warning of outcome.warnings) log(warning, 'warning');
    logSceneDocumentCapabilities(outcome.capabilities);

    const result = outcome.result;
    if (result && result.success) {
      downloadResult(result, settings.format);
      displayExportStats(settings.format, result);
      log(result.message || 'Export complete!', 'success');
    } else {
      log(`Export failed: ${result?.error || 'Unknown error'}`, 'error');
    }
  } catch (error) {
    log(`Export error: ${errorMessage(error)}`, 'error');
  }
}

/** The byte size of exactly what the download button hands to the browser. */
function exportResultSize(result: ExportResult): number | undefined {
  if (result.binary_data) return new Uint8Array(result.binary_data).byteLength;
  if (result.json_data !== undefined) return new TextEncoder().encode(result.json_data).byteLength;
  if (result.data !== undefined) return new TextEncoder().encode(result.data).byteLength;
  return undefined;
}

export function logSceneDocumentCapabilities(capabilities: ExportOutcome['capabilities'] = {}) {
  const supported = Object.entries(capabilities || {})
    .filter(([, value]) => value === true)
    .map(([key]) => key);
  if (supported.length > 0) log(`SceneDocument capabilities: ${supported.join(', ')}`, 'info');
}

// Download the export result
export function downloadResult(result: ExportResult, format: string) {
  let blob;
  const extension = format === 'fbx-legacy' ? 'fbx' : format;
  let filename = `export.${extension}`;

  if (result.binary_data) {
    blob = new Blob([new Uint8Array(result.binary_data)], { type: 'application/octet-stream' });
  } else if (result.json_data) {
    blob = new Blob([result.json_data], { type: 'application/json' });
  } else if (result.data) {
    blob = new Blob([result.data], { type: 'text/plain' });
  } else {
    log('No data to download', 'error');
    return;
  }

  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}

/** Show the complete result of an export, with Draco details when available. */
export function displayExportStats(format: string, result: ExportResult) {
  const outputSize = exportResultSize(result);
  const stats = result.draco_stats;
  const fbxStats = result.fbx_stats;
  exportStatFields.format.textContent = format === 'fbx-legacy'
    ? 'FBX (legacy)'
    : format.toUpperCase();
  exportStatFields.fileSize.textContent = outputSize === undefined ? '—' : formatFileSize(outputSize);
  exportStatFields.compression.textContent = stats
    ? `Draco (${stats.primitives} ${stats.primitives === 1 ? 'primitive' : 'primitives'})`
    : fbxStats?.requested
      ? fbxStats.compressed_arrays > 0 ? 'FBX (zlib arrays)' : 'FBX (zlib requested; no arrays compressed)'
      : 'None';

  if (stats) {
    const dracoSize = stats.compressed_size;
    exportStatFields.method.textContent = displayMethodName(stats.method);
    exportStatFields.speed.textContent = `${stats.speed} (${stats.speed === 0 ? 'best compression' : stats.speed === 10 ? 'fastest' : 'balanced'})`;
    renderPredictionSchemes(stats.prediction_scheme);
    exportStatFields.dracoSize.textContent = formatFileSize(dracoSize);
    exportStatFields.share.textContent = outputSize && outputSize > 0
      ? `${(dracoSize / outputSize * 100).toFixed(1)}%`
      : '—';
    exportStatFields.dracoDetails.style.display = 'block';
  } else {
    exportStatFields.dracoDetails.style.display = 'none';
  }
  if (fbxStats?.requested) {
    const rawBytes = fbxStats.compressed_raw_bytes;
    const storedBytes = fbxStats.compressed_stored_bytes;
    exportStatFields.fbxMethod.textContent = 'zlib';
    exportStatFields.fbxArrays.textContent = String(fbxStats.compressed_arrays);
    exportStatFields.fbxStored.textContent = formatFileSize(storedBytes);
    exportStatFields.fbxRaw.textContent = formatFileSize(rawBytes);
    exportStatFields.fbxSavings.textContent = rawBytes > 0
      ? `${((1 - storedBytes / rawBytes) * 100).toFixed(1)}%`
      : '—';
    exportStatFields.fbxDetails.style.display = 'block';
  } else {
    exportStatFields.fbxDetails.style.display = 'none';
  }
  exportStats.style.display = 'block';
}

function displayMethodName(method?: string) {
  if (!method) return '—';
  return method.charAt(0).toUpperCase() + method.slice(1);
}

/** Render a compact attribute/predictor table and keep full choices in tooltips. */
function renderPredictionSchemes(value?: string) {
  const container = exportStatFields.prediction;
  const body = container.querySelector('tbody');
  if (!body) return;
  body.replaceChildren();
  if (!value) {
    const row = document.createElement('tr');
    const cell = document.createElement('td');
    cell.colSpan = 3;
    cell.textContent = '—';
    row.append(cell);
    body.append(row);
    return;
  }

  for (const entry of value.split('; ')) {
    const match = entry.match(/^(.+): (.+) \((.+)\)$/);
    if (!match) {
      const row = document.createElement('tr');
      const cell = document.createElement('td');
      cell.colSpan = 3;
      cell.textContent = entry;
      row.append(cell);
      body.append(row);
      continue;
    }
    const row = document.createElement('tr');
    const attribute = document.createElement('th');
    attribute.scope = 'row';
    attribute.textContent = match[1];
    const predictor = document.createElement('td');
    predictor.textContent = match[2];
    const transform = document.createElement('td');
    transform.textContent = match[3];
    row.append(attribute, predictor, transform);
    body.append(row);
  }
}
