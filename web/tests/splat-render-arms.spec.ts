/**
 * What a bit budget costs on screen.
 *
 * `splat_render_arms_probe` writes one PLY per encoding arm — the source, and
 * the same scene decoded back under each budget — and reports bytes. Bytes are
 * the cheap half of a lossy decision; this is the other half. Each arm is
 * loaded into the viewer, drawn from one camera that is stated rather than
 * derived, read back, and compared pixel for pixel against the `source` arm.
 *
 * The camera is spherical and fixed. Solving for an eye position would make
 * the numbers depend on a solver rather than on the encoder, and a camera that
 * drifts by a pixel between arms shows up as a difference the encoder did not
 * cause.
 *
 * ```text
 * DRACO_SPLAT_ARMS=../dev/splat-arms npm run -w web test:browser
 * ```
 *
 * Without `DRACO_SPLAT_ARMS` (or its default directory) this skips: the arms
 * are hundreds of megabytes and belong outside the repository.
 */
import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
import { existsSync, readdirSync } from 'node:fs';
import { writeFile } from 'node:fs/promises';
import { crc32, deflateSync } from 'node:zlib';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(here, '..', '..');

try {
  process.loadEnvFile(path.resolve(here, '..', '.env'));
} catch {
  // No web/.env: the default directory below decides whether this runs.
}

// The viewer's own module state, stashed on `window` so the wait below can be
// a plain synchronous expression. This exists only for this spec.
declare global {
  interface Window { __splatState: any }
}

const armsDir = process.env.DRACO_SPLAT_ARMS
  || path.resolve(repoRoot, 'dev', 'splat-arms');

/**
 * Where the camera stands, given a scene it has not seen.
 *
 * The direction is stated; the distance and the look-at point are measured
 * from the cloud, because a hand-written eye position frames one scene and
 * misses every other -- and a camera that ends up inside the cloud produces a
 * full white frame that every arm matches, which reads as a pass.
 *
 * The extent is a 2nd-to-98th percentile box rather than the bounding box: a
 * reconstruction keeps a handful of splats far outside the scene, and fitting
 * to those pushes the camera back until the subject is a few pixels wide.
 */
const CAMERA = {
  azimuth: Math.PI * 0.25,
  elevation: Math.PI * 0.2,
  fov: (44.2141 * Math.PI) / 180,
  /**
   * Box diagonals back from the centre. Under one deliberately: the diagonal
   * is longer than the scene is wide, and a frame that is mostly empty
   * background scores every arm generously for the pixels neither of them
   * drew on.
   */
  fill: 0.7,
};

/**
 * A camera to use instead of the solved one, as JSON.
 *
 * The frame above is chosen to work on a scene nobody has looked at, and a
 * percentile box has no idea which part of a reconstruction is the subject.
 * When a particular view is the point -- the one a question was asked about --
 * it is named here rather than by editing this file:
 *
 * ```text
 * DRACO_SPLAT_CAMERA='{"target":[-0.59,2.07,8.40],"distance":12.7,"azimuth":3.14}'
 * ```
 *
 * Every field is optional; what is left out keeps the value above, and leaving
 * out `target` or `distance` leaves the whole stand to be solved.
 */
const override: Partial<typeof CAMERA & Stand> = process.env.DRACO_SPLAT_CAMERA
  ? JSON.parse(process.env.DRACO_SPLAT_CAMERA)
  : {};
Object.assign(CAMERA, override);

const WIDTH = 900;
const HEIGHT = 600;

/** `source.ply` first: every other arm is measured against it. */
function armFiles(): string[] {
  const files = readdirSync(armsDir).filter((name) => name.endsWith('.ply'));
  return files.sort((a, b) => Number(b === 'source.ply') - Number(a === 'source.ply'));
}

/**
 * Where to write each arm's frame as a PNG, or nothing.
 *
 * The numbers below say how far an arm moved, not which way, and a camera that
 * frames the wrong part of the scene produces a perfectly consistent table of
 * meaningless differences. The frames are how that is checked by eye, once.
 */
const shotsDir = process.env.DRACO_SPLAT_SHOTS;

/** Where the camera stood, once solved: the same for every arm. */
interface Stand { target: number[]; distance: number }

async function shoot(page: Page, file: string, stand: Stand | null) {
  await page.locator('#file-input').setInputFiles(file);
  // The splat path is not the mesh path, so waiting on "Preview ready" would
  // pass before there is anything to draw. The cloud itself is the signal.
  //
  // Polled through the stashed module rather than an `import()` inside the
  // predicate: an async predicate hands `waitForFunction` a promise, which is
  // truthy before it settles, so the wait would return at once and every arm
  // would be shot off whatever was on screen.
  await page.waitForFunction(
    () => Boolean(window.__splatState?.viewer?._splats),
    null,
    { timeout: 300_000, polling: 250 },
  );

  const shot = await page.evaluate(({ camera, width, height, stand }) => {
    const viewer = window.__splatState.viewer;

    // Solved once, on the first arm, and handed back so every later arm stands
    // in the same place. Re-solving per arm would fold the encoder's own shift
    // of the cloud into the camera, and then each arm would be compared
    // against a slightly different view -- which is how four very different
    // budgets came out with the same score.
    const place = stand ?? (() => {
      const positions = viewer._splats.cloud.positions;
      const count = viewer._splats.cloud.count;
      // Sampled rather than exhaustive: the percentile of twenty thousand
      // splats and of seven hundred thousand agree to far less than this
      // camera can show.
      const step = Math.max(1, Math.floor(count / 20000));
      const centre: number[] = [];
      const span: number[] = [];
      for (let axis = 0; axis < 3; axis += 1) {
        const values: number[] = [];
        for (let splat = 0; splat < count; splat += step) values.push(positions[splat * 3 + axis]);
        values.sort((a, b) => a - b);
        const low = values[Math.floor(values.length * 0.02)];
        const high = values[Math.floor(values.length * 0.98)];
        centre.push((low + high) / 2);
        span.push(high - low);
      }
      const diagonal = Math.hypot(span[0], span[1], span[2]);
      return {
        target: centre,
        distance: (camera.fill * diagonal) / (2 * Math.tan(camera.fov / 2)),
      };
    })();

    viewer.canvas.width = width;
    viewer.canvas.height = height;
    viewer.showGrid = false;
    viewer.backdropLevel = 0;
    viewer.autoRotate = false;
    viewer.camera.target.set(place.target);
    viewer.camera.distance = place.distance;
    viewer.camera.azimuth = camera.azimuth;
    viewer.camera.elevation = camera.elevation;
    viewer.camera.fov = camera.fov;
    viewer._render();
    // Read back in the same task as the draw. The drawing buffer is not
    // preserved, so anything that reaches it later -- `toDataURL` from a
    // second call, for one -- sees a cleared canvas and reports a blank frame.
    const rgba = new Uint8Array(width * height * 4);
    viewer.gl.readPixels(0, 0, width, height, viewer.gl.RGBA, viewer.gl.UNSIGNED_BYTE, rgba);
    return { pixels: Array.from(rgba), place };
  }, { camera: CAMERA, width: WIDTH, height: HEIGHT, stand });

  const pixels = Uint8Array.from(shot.pixels);
  if (shotsDir) {
    await writeFile(
      path.join(shotsDir, `${path.basename(file, '.ply')}.png`),
      encodePng(pixels, WIDTH, HEIGHT),
    );
  }

  await page.locator('#clear-file').click();
  await expect(page.locator('#drop-zone')).toBeVisible();
  return { pixels, place: shot.place as Stand };
}

/**
 * The frame as a PNG.
 *
 * Written here rather than taken from the canvas, because the canvas has
 * nothing left to give by the time a second call reaches it, and because a
 * picture that is encoded from anything other than the measured pixels can
 * disagree with the table it is meant to explain.
 *
 * GL reads bottom-up; PNG stores top-down, so the rows go out in reverse.
 */
function encodePng(pixels: Uint8Array, width: number, height: number): Buffer {
  const raw = Buffer.alloc(height * (width * 4 + 1));
  for (let row = 0; row < height; row += 1) {
    const source = (height - 1 - row) * width * 4;
    raw[row * (width * 4 + 1)] = 0; // filter: none
    Buffer.from(pixels.buffer, source, width * 4)
      .copy(raw, row * (width * 4 + 1) + 1);
  }
  const chunk = (type: string, body: Buffer) => {
    const out = Buffer.alloc(body.length + 12);
    out.writeUInt32BE(body.length, 0);
    out.write(type, 4, 'ascii');
    body.copy(out, 8);
    out.writeUInt32BE(crc32(out.subarray(4, 8 + body.length)) >>> 0, 8 + body.length);
    return out;
  };
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header[8] = 8;  // bits a channel
  header[9] = 6;  // truecolour with alpha
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', header),
    chunk('IDAT', deflateSync(raw)),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

/** Standard deviation of the frame's luminance. */
function spread(pixels: Uint8Array): number {
  let sum = 0;
  let squares = 0;
  const count = pixels.length / 4;
  for (let pixel = 0; pixel < count; pixel += 1) {
    const luminance = 0.2126 * pixels[pixel * 4]
      + 0.7152 * pixels[pixel * 4 + 1] + 0.0722 * pixels[pixel * 4 + 2];
    sum += luminance;
    squares += luminance * luminance;
  }
  const mean = sum / count;
  return Math.sqrt(Math.max(0, squares / count - mean * mean));
}

/** Difference against the reference, on the three colour channels only. */
function compare(reference: Uint8Array, arm: Uint8Array) {
  let squared = 0;
  let worst = 0;
  let moved = 0;
  const pixels = reference.length / 4;
  for (let pixel = 0; pixel < pixels; pixel += 1) {
    let changed = false;
    for (let channel = 0; channel < 3; channel += 1) {
      const delta = arm[pixel * 4 + channel] - reference[pixel * 4 + channel];
      squared += delta * delta;
      if (Math.abs(delta) > worst) worst = Math.abs(delta);
      if (delta !== 0) changed = true;
    }
    if (changed) moved += 1;
  }
  const rmse = Math.sqrt(squared / (pixels * 3));
  // A run with no difference at all has infinite PSNR rather than a division
  // by zero, and saying so is more useful than capping it at some large number.
  const psnr = rmse === 0 ? Infinity : 20 * Math.log10(255 / rmse);
  return { rmse, psnr, worst, movedPercent: (moved / pixels) * 100 };
}

test('encoding arms render within the difference their budget buys', async ({ page }) => {
  test.skip(!existsSync(armsDir), `no arms in ${armsDir}: run splat_render_arms_probe first`);
  const files = armFiles();
  test.skip(files[0] !== 'source.ply', `${armsDir} has no source.ply to measure against`);

  test.setTimeout(30 * 60_000);
  await page.setViewportSize({ width: WIDTH + 400, height: HEIGHT + 200 });
  await page.goto('/index.html');
  await expect(page.locator('#console')).toContainText('Ready to convert 3D files!');
  await page.evaluate(async () => {
    const { state } = await import('/app/state.js' as string);
    window.__splatState = state;
  });

  let reference: Uint8Array | null = null;
  let stand: Stand | null = override.target && override.distance !== undefined
    ? { target: override.target, distance: override.distance }
    : null;
  const rows: string[] = [];
  for (const name of files) {
    const shot = await shoot(page, path.join(armsDir, name), stand);
    const pixels = shot.pixels;
    if (!reference) {
      reference = pixels;
      stand ??= shot.place;
      // The reference has to contain a scene, and "not black" does not say so:
      // a camera standing inside the cloud renders a flat white frame that
      // every arm reproduces exactly, and the table reads as a pass. What a
      // drawn scene has and a flat frame does not is variation.
      expect(spread(pixels), 'the source arm drew a flat frame').toBeGreaterThan(8);
      continue;
    }
    const { rmse, psnr, worst, movedPercent } = compare(reference, pixels);
    rows.push(
      `${path.basename(name, '.ply').padEnd(14)}`
      + `${psnr.toFixed(2).padStart(9)}`
      + `${rmse.toFixed(3).padStart(9)}`
      + `${String(worst).padStart(7)}`
      + `${movedPercent.toFixed(1).padStart(9)}%`,
    );
  }

  console.log(`\n${'arm'.padEnd(14)}${'PSNR'.padStart(9)}${'RMSE'.padStart(9)}${'worst'.padStart(7)}${'moved'.padStart(10)}`);
  for (const row of rows) console.log(row);
  expect(rows.length).toBeGreaterThan(0);
});
