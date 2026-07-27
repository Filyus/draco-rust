import { formatFileSize } from './format.ts';
import { errorMessage, log } from './log.ts';
import { compressionStatFields, compressionStats, dracoOptions, element, encodingSpeed, exportFormat, useDraco } from './dom.ts';
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

  // Show/hide Draco options for glTF formats only
  if (format === 'gltf' || format === 'glb') {
    dracoOptions.style.display = 'flex';
  } else {
    dracoOptions.style.display = 'none';
  }
}

/** The export controls as the routes want them: plain values, read once. */
function exportSettings(): ExportSettings {
  return {
    format: exportFormat.value,
    includeNormals: element<HTMLInputElement>('include-normals').checked,
    includeUvs: element<HTMLInputElement>('include-uvs').checked,
    useDraco: useDraco.checked,
    encodingSpeed: Number(encodingSpeed.value),
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
      if (result.draco_stats) displayCompressionStats(result.draco_stats);
      else compressionStats.style.display = 'none';
      log(result.message || 'Export complete!', 'success');
    } else {
      log(`Export failed: ${result?.error || 'Unknown error'}`, 'error');
    }
  } catch (error) {
    log(`Export error: ${errorMessage(error)}`, 'error');
  }
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

/**
 * Show what the Draco pass actually wrote.
 *
 * Only the encoder knows which method and prediction scheme it chose, and the
 * binding reports neither, so those read as unknown rather than as a guess.
 */
export function displayCompressionStats(stats: DracoStats) {
  compressionStatFields.method.textContent = stats.method || '—';
  compressionStatFields.speed.textContent = `${stats.speed} (${stats.speed === 0 ? 'best compression' : stats.speed === 10 ? 'fastest' : 'balanced'})`;
  compressionStatFields.prediction.textContent = stats.prediction_scheme || '—';
  compressionStatFields.size.textContent = formatFileSize(stats.compressed_size);
  compressionStats.style.display = 'block';
  log(
    `Compression: ${stats.primitives ?? 0} primitives at speed ${stats.speed}, `
    + `${formatFileSize(stats.compressed_size)} of Draco payload`,
    'success',
  );
}
