/**
 * What a bit budget costs on screen.
 *
 * `splat_render_arms_probe` writes one PLY per encoding arm — the source, and
 * the same scene decoded back under each budget — and reports bytes. Bytes are
 * the cheap half of a lossy decision; this is the other half. Each arm is
 * loaded into the viewer, drawn from a fixed set of cameras, read back, and
 * compared pixel for pixel against the `source` arm.
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
 * Where the cameras stand, given a scene none of them has seen.
 *
 * Four of them, a quarter turn apart, rather than one. A reconstruction has no
 * front, and every single view that could be written down here frames one
 * scene and misses the next — the first attempt put the camera inside the
 * cloud on two scenes out of three. Four views do not need to be right; they
 * need to not all be wrong at once, and an arm that survives all four has been
 * asked about the back of the scene as well as the front.
 *
 * The look-at point and the distance are solved from the cloud, per view, once
 * on the `source` arm, and every later arm stands in the same places. Solving
 * per arm would fold the encoder's own shift of the cloud into the camera, and
 * then each arm is compared against a slightly different view — which is how
 * four very different budgets once came out with the same score.
 */
const VIEWS = [0, 0.25, 0.5, 0.75].map((turn) => ({
  azimuth: Math.PI * 2 * turn,
  // Low, because these scenes are strips of ground: from above, most of the
  // frame is sky and most of the splats are seen edge-on.
  elevation: Math.PI * 0.08,
}));

const CAMERA = {
  fov: (44.2141 * Math.PI) / 180,
  /**
   * How much wider than the subject the frame is.
   *
   * Applied to the percentile box as each camera actually sees it, not to its
   * diagonal: these scenes are long strips, so a diagonal is mostly length,
   * and a distance scaled from it either buries the camera inside the cloud or
   * leaves the subject a few pixels wide in a mostly empty frame. Both
   * happened before this was worked out, and the first reads as a pass,
   * because a camera inside the cloud renders a flat wash every arm
   * reproduces.
   *
   * The box is 2nd-to-98th percentile rather than the full bounds: a
   * reconstruction keeps a handful of splats far outside the scene, and
   * fitting to those pushes every camera back until the subject is specks.
   */
  margin: 1.15,
};

/**
 * A single camera to use instead of the four, as JSON.
 *
 * A percentile box has no idea which part of a reconstruction is the subject.
 * When a particular view is the point — the one a question was asked about —
 * it is named here rather than by editing this file:
 *
 * ```text
 * DRACO_SPLAT_CAMERA='{"target":[-0.59,2.07,8.40],"distance":12.7,"azimuth":3.14}'
 * ```
 *
 * Any of `azimuth`, `elevation`, `target` and `distance` may be left out; what
 * is left out is solved. Naming any of them replaces all four views with one.
 */
const override: Record<string, any> = process.env.DRACO_SPLAT_CAMERA
  ? JSON.parse(process.env.DRACO_SPLAT_CAMERA)
  : {};
const views = Object.keys(override).length > 0
  ? [{ azimuth: override.azimuth ?? VIEWS[0].azimuth, elevation: override.elevation ?? VIEWS[0].elevation }]
  : VIEWS;

const WIDTH = 900;
const HEIGHT = 600;

/**
 * How bright a pixel must be for the scene to count as covering it.
 *
 * The metrics below are taken over these pixels alone. Over the whole frame,
 * an arm is rewarded for the background it did not draw on, so a view that
 * frames the scene badly scores every arm generously — and the score then
 * measures the framing rather than the encoder.
 */
const DRAWN_ON = 6;

/** `source.ply` first: every other arm is measured against it. */
function armFiles(): string[] {
  const files = readdirSync(armsDir).filter((name) => name.endsWith('.ply'));
  return files.sort((a, b) => Number(b === 'source.ply') - Number(a === 'source.ply'));
}

/**
 * Where to write the frames as PNGs, or nothing.
 *
 * The numbers say how far an arm moved, not which way, and a camera that
 * frames the wrong part of the scene produces a perfectly consistent table of
 * meaningless differences. The frames are how that is checked by eye.
 */
const shotsDir = process.env.DRACO_SPLAT_SHOTS;

/** Where one camera stood, once solved: the same for every arm. */
interface Stand { target: number[]; distance: number }

async function shoot(page: Page, file: string, stands: Stand[] | null) {
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

  const shot = await page.evaluate(({ camera, views, width, height, stands }) => {
    const viewer = window.__splatState.viewer;

    // The cloud's shape, once: the same box serves every view.
    const positions = viewer._splats.cloud.positions;
    const count = viewer._splats.cloud.count;
    // Sampled rather than exhaustive: the percentile of twenty thousand splats
    // and of seven hundred thousand agree to far less than a camera can show.
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

    const solve = (view: { azimuth: number; elevation: number }): Stand => {
      // Where the eye is, relative to what it looks at: the viewer's own
      // spherical terms, with azimuth 0 putting it on +z.
      const cosElevation = Math.cos(view.elevation);
      const dir = [
        Math.sin(view.azimuth) * cosElevation,
        Math.sin(view.elevation),
        Math.cos(view.azimuth) * cosElevation,
      ];
      const right = [Math.cos(view.azimuth), 0, -Math.sin(view.azimuth)];
      const up = [
        dir[1] * right[2] - dir[2] * right[1],
        dir[2] * right[0] - dir[0] * right[2],
        dir[0] * right[1] - dir[1] * right[0],
      ];

      // The box as this camera sees it: how far its corners reach across the
      // frame, up it, and towards the eye.
      let across = 0;
      let tall = 0;
      let deep = 0;
      for (let corner = 0; corner < 8; corner += 1) {
        const offset = [
          (corner & 1 ? 0.5 : -0.5) * span[0],
          (corner & 2 ? 0.5 : -0.5) * span[1],
          (corner & 4 ? 0.5 : -0.5) * span[2],
        ];
        const along = (axis: number[]) => Math.abs(
          offset[0] * axis[0] + offset[1] * axis[1] + offset[2] * axis[2],
        );
        across = Math.max(across, along(right));
        tall = Math.max(tall, along(up));
        deep = Math.max(deep, along(dir));
      }
      const halfVertical = Math.tan(camera.fov / 2);
      // The frame is wider than it is tall, so the horizontal half-angle is
      // the vertical one scaled by the aspect.
      const halfHorizontal = (halfVertical * width) / height;
      // `deep` first, so the eye is outside the box whatever the fit says.
      return {
        target: centre,
        distance: deep + camera.margin * Math.max(across / halfHorizontal, tall / halfVertical),
      };
    };

    viewer.canvas.width = width;
    viewer.canvas.height = height;
    viewer.showGrid = false;
    viewer.backdropLevel = 0;
    viewer.autoRotate = false;
    viewer.camera.fov = camera.fov;

    const places = stands ?? views.map(solve);
    const frames: number[][] = [];
    for (let view = 0; view < views.length; view += 1) {
      viewer.camera.target.set(places[view].target);
      viewer.camera.distance = places[view].distance;
      viewer.camera.azimuth = views[view].azimuth;
      viewer.camera.elevation = views[view].elevation;
      viewer._render();
      // Read back in the same task as the draw. The drawing buffer is not
      // preserved, so anything that reaches it later -- `toDataURL` from a
      // second call, for one -- sees a cleared canvas and reports a blank
      // frame, which is how four solid white PNGs once accompanied a table of
      // perfectly plausible numbers.
      const rgba = new Uint8Array(width * height * 4);
      viewer.gl.readPixels(0, 0, width, height, viewer.gl.RGBA, viewer.gl.UNSIGNED_BYTE, rgba);
      frames.push(Array.from(rgba));
    }
    return { frames, places };
  }, { camera: CAMERA, views, width: WIDTH, height: HEIGHT, stands });

  const frames = shot.frames.map((frame) => Uint8Array.from(frame));
  if (shotsDir) {
    const arm = path.basename(file, '.ply');
    for (const [view, pixels] of frames.entries()) {
      await writeFile(
        path.join(shotsDir, `${arm}-view${view}.png`),
        encodePng(pixels, WIDTH, HEIGHT),
      );
    }
  }

  await page.locator('#clear-file').click();
  await expect(page.locator('#drop-zone')).toBeVisible();
  return { frames, places: shot.places as Stand[] };
}

/**
 * The frame as a PNG.
 *
 * Written here rather than taken from the canvas, because the canvas has
 * nothing left to give by the time a second call reaches it, and because a
 * picture encoded from anything other than the measured pixels can disagree
 * with the table it is meant to explain.
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

function luminance(pixels: Uint8Array, pixel: number): number {
  return 0.2126 * pixels[pixel * 4]
    + 0.7152 * pixels[pixel * 4 + 1] + 0.0722 * pixels[pixel * 4 + 2];
}

/** The share of a frame the scene drew on at all. */
function coverage(pixels: Uint8Array): number {
  const count = pixels.length / 4;
  let drawn = 0;
  for (let pixel = 0; pixel < count; pixel += 1) {
    if (luminance(pixels, pixel) > DRAWN_ON) drawn += 1;
  }
  return (drawn / count) * 100;
}

/** Standard deviation of a frame's luminance. */
function spread(pixels: Uint8Array): number {
  let sum = 0;
  let squares = 0;
  const count = pixels.length / 4;
  for (let pixel = 0; pixel < count; pixel += 1) {
    const value = luminance(pixels, pixel);
    sum += value;
    squares += value * value;
  }
  const mean = sum / count;
  return Math.sqrt(Math.max(0, squares / count - mean * mean));
}

/** Running difference against the reference, over the pixels it drew on. */
class Difference {
  private squared = 0;
  private counted = 0;
  private moved = 0;
  worst = 0;

  add(reference: Uint8Array, arm: Uint8Array) {
    const pixels = reference.length / 4;
    for (let pixel = 0; pixel < pixels; pixel += 1) {
      if (luminance(reference, pixel) <= DRAWN_ON) continue;
      this.counted += 1;
      let changed = false;
      for (let channel = 0; channel < 3; channel += 1) {
        const delta = arm[pixel * 4 + channel] - reference[pixel * 4 + channel];
        this.squared += delta * delta;
        if (Math.abs(delta) > this.worst) this.worst = Math.abs(delta);
        if (delta !== 0) changed = true;
      }
      if (changed) this.moved += 1;
    }
  }

  get rmse() { return Math.sqrt(this.squared / (this.counted * 3)); }

  get psnr() {
    // No difference at all is infinite PSNR rather than a division by zero,
    // and saying so is more useful than capping it at some large number.
    return this.rmse === 0 ? Infinity : 20 * Math.log10(255 / this.rmse);
  }

  get movedPercent() { return (this.moved / this.counted) * 100; }
}

test('encoding arms render within the difference their budget buys', async ({ page }) => {
  test.skip(!existsSync(armsDir), `no arms in ${armsDir}: run splat_render_arms_probe first`);
  const files = armFiles();
  test.skip(files[0] !== 'source.ply', `${armsDir} has no source.ply to measure against`);

  test.setTimeout(60 * 60_000);
  await page.setViewportSize({ width: WIDTH + 400, height: HEIGHT + 200 });
  await page.goto('/index.html');
  await expect(page.locator('#console')).toContainText('Ready to convert 3D files!');
  await page.evaluate(async () => {
    const { state } = await import('/app/state.js' as string);
    window.__splatState = state;
  });

  let reference: Uint8Array[] | null = null;
  let stands: Stand[] | null = override.target && override.distance !== undefined
    ? views.map(() => ({ target: override.target, distance: override.distance }))
    : null;
  const rows: string[] = [];
  for (const name of files) {
    const shot = await shoot(page, path.join(armsDir, name), stands);
    if (!reference) {
      reference = shot.frames;
      stands ??= shot.places;
      for (const [view, pixels] of shot.frames.entries()) {
        // The reference has to contain a scene, and "not black" does not say
        // so: a camera standing inside the cloud renders a flat wash that
        // every arm reproduces exactly, and the table reads as a pass. What a
        // drawn scene has and a flat frame does not is variation.
        expect(spread(pixels), `view ${view} of the source arm is a flat frame`).toBeGreaterThan(8);
        // And a frame the scene barely reaches makes every later number a
        // measurement of a few hundred pixels.
        expect(coverage(pixels), `view ${view} barely shows the scene`).toBeGreaterThan(5);
      }
      console.log(`\ncoverage: ${shot.frames.map((f) => `${coverage(f).toFixed(0)}%`).join('  ')}`);
      continue;
    }
    const difference = new Difference();
    for (const [view, pixels] of shot.frames.entries()) difference.add(reference[view], pixels);
    rows.push(
      `${path.basename(name, '.ply').padEnd(14)}`
      + `${difference.psnr.toFixed(2).padStart(9)}`
      + `${difference.rmse.toFixed(3).padStart(9)}`
      + `${String(difference.worst).padStart(7)}`
      + `${difference.movedPercent.toFixed(1).padStart(9)}%`,
    );
  }

  console.log(`\n${'arm'.padEnd(14)}${'PSNR'.padStart(9)}${'RMSE'.padStart(9)}${'worst'.padStart(7)}${'moved'.padStart(10)}`);
  for (const row of rows) console.log(row);
  console.log(`  over ${views.length} view(s), on the pixels the source drew on\n`);
  expect(rows.length).toBeGreaterThan(0);
});
