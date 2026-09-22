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
 * Four azimuths a quarter turn apart, each shot twice: once from the middle of
 * the cloud looking out, and once from outside it looking in. A reconstruction
 * has no front, and no single rule frames both kinds of scene in this corpus.
 * A room's splats face inwards, so a camera outside the cloud is behind the
 * walls and sees their backs; a forward-facing capture of a street has no
 * middle to stand in, and from inside it the camera is under the foliage. Each
 * framing works on one and fails on the other, and which scene is which is not
 * something a percentile box can be asked.
 *
 * So both are shot. The metrics pool over the pixels the source drew on, so
 * the views that actually show the scene are the ones that carry the numbers,
 * and the coverage line says how much each one showed.
 *
 * Every stand is solved from the cloud once, on the `source` arm, and every
 * later arm stands in the same places. Solving per arm would fold the
 * encoder's own shift of the cloud into the camera, and then each arm is
 * compared against a slightly different view -- which is how four very
 * different budgets once came out with the same score.
 */
const VIEWS = [0, 0.25, 0.5, 0.75].flatMap((turn) => [
  // Level from the middle: a room or a street is around the camera, not below.
  { azimuth: Math.PI * 2 * turn, elevation: 0, from: 'inside' as const },
  // And low from outside, because from above most of the frame is sky.
  { azimuth: Math.PI * 2 * turn, elevation: Math.PI * 0.08, from: 'outside' as const },
]);

const CAMERA = {
  fov: (44.2141 * Math.PI) / 180,
  /**
   * For a view from inside: how far into the scene it looks, as a share of how
   * far the cloud reaches that way.
   *
   * The eye sits at the middle of the box; this only sets where it focuses,
   * which is what the viewer's orbit distance means. Under one, so the look-at
   * point is inside the cloud rather than out past its far wall.
   */
  reach: 0.45,
  /**
   * For a view from outside: how much wider than the subject the frame is.
   *
   * Applied to the box as that camera sees it, not to its diagonal. These
   * scenes are long strips, so a diagonal is mostly length, and a distance
   * scaled from it buries the eye inside the cloud -- which renders a flat
   * wash that every arm reproduces exactly, and reads as a pass.
   */
  margin: 1.15,
};

/**
 * The box both framings are solved from: 2nd to 98th percentile, not the full
 * bounds. A reconstruction keeps a handful of splats far outside the scene;
 * fitting to those pushes every outside camera back until the subject is
 * specks, and the middle of the full bounds is not the middle of anything.
 */
const PERCENTILE = 0.02;

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
  ? [{
    azimuth: override.azimuth ?? VIEWS[0].azimuth,
    elevation: override.elevation ?? VIEWS[0].elevation,
    from: (override.from ?? 'outside') as 'inside' | 'outside',
  }]
  : VIEWS;

const WIDTH = 900;
const HEIGHT = 600;

/**
 * How bright a pixel must be, in linear radiance, for the scene to count as
 * covering it. Roughly what 6 of 255 was on a gamma-encoded frame.
 *
 * The metrics below are taken over these pixels alone. Over the whole frame,
 * an arm is rewarded for the background it did not draw on, so a view that
 * frames the scene badly scores every arm generously — and the score then
 * measures the framing rather than the encoder.
 */
const DRAWN_ON = 0.0003;

/**
 * How far a channel must move to count as having moved.
 *
 * On an 8-bit frame the test was `!= 0`, which said something: a channel had
 * crossed a quantization step. In linear radiance nothing is ever exactly
 * equal, so that test reported 99% for every arm alike and measured nothing.
 * This is roughly a quarter of a display step at mid-grey -- under what a
 * screen can show, and far above float noise.
 */
const MOVED_BY = 0.001;

/**
 * The file to shoot for each arm, `source.ply` first.
 *
 * The `.drc` when the probe wrote one, because that is the product: a PLY
 * rewritten from the decoded cloud is a faithful proxy but still a proxy, and
 * the point of the payload carrying its own attribute names is that a consumer
 * can read it directly. An arm with no `.drc` is one whose stream a reader
 * cannot be given -- the probe says which and why -- and its PLY stands in.
 */
function armFiles(): string[] {
  const present = new Set(readdirSync(armsDir));
  const arms = [...present]
    .filter((name) => name.endsWith('.ply'))
    .map((name) => path.basename(name, '.ply'))
    .filter((arm) => arm !== 'source')
    .sort();
  return [
    'source.ply',
    ...arms.map((arm) => (present.has(`${arm}.drc`) ? `${arm}.drc` : `${arm}.ply`)),
  ];
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

  const shot = await page.evaluate(({ camera, views, width, height, stands, percentile }) => {
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
      const low = values[Math.floor(values.length * percentile)];
      const high = values[Math.floor(values.length * (1 - percentile))];
      centre.push((low + high) / 2);
      span.push(high - low);
    }

    const solve = (view: { azimuth: number; elevation: number; from: string }): Stand => {
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
      // frame, up it, and back along the view.
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

      if (view.from === 'inside') {
        // The eye is `distance` along `dir` from the target, so putting the
        // target that far the other side of the middle stands the eye exactly
        // at the middle, looking out.
        const distance = camera.reach * deep;
        return {
          target: [
            centre[0] - dir[0] * distance,
            centre[1] - dir[1] * distance,
            centre[2] - dir[2] * distance,
          ],
          distance,
        };
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
    const frames: string[] = [];
    let size: number[] = [width, height];
    for (let view = 0; view < views.length; view += 1) {
      viewer.camera.target.set(places[view].target);
      viewer.camera.distance = places[view].distance;
      viewer.camera.azimuth = views[view].azimuth;
      viewer.camera.elevation = views[view].elevation;
      viewer._render();

      // Read the scene's own float target, not the canvas.
      //
      // The canvas holds the frame after the output pass, which is a tone
      // curve and a gamma. The curve compresses the bright end, and these
      // scenes are mostly bright end, so a difference measured there is
      // measured through the thing most able to hide it -- and the bias is not
      // neutral: what the curve flattens reads as "no difference", which is
      // the answer that says a saving was free. The resolve target is linear
      // radiance, before any of that.
      //
      // Read back in the same task as the draw either way. Nothing here
      // preserves a buffer for a later call, which is how four solid white
      // PNGs once accompanied a table of perfectly plausible numbers.
      const gl = viewer.gl;
      const target = viewer._sceneTarget;
      if (!target || !target.hdr) throw new Error('no float scene target to read');
      // The frame is drawn wider than it is shown -- the output pass crops by
      // the guard -- so the visible rectangle is the middle of it.
      const cropWidth = Math.round(target.renderWidth / target.guard);
      const cropHeight = Math.round(target.renderHeight / target.guard);
      const left = Math.round((target.renderWidth - cropWidth) / 2);
      const bottom = Math.round((target.renderHeight - cropHeight) / 2);
      gl.bindFramebuffer(gl.READ_FRAMEBUFFER, target.resolveFramebuffer);
      const radiance = new Float32Array(cropWidth * cropHeight * 4);
      gl.readPixels(left, bottom, cropWidth, cropHeight, gl.RGBA, gl.FLOAT, radiance);
      const error = gl.getError();
      gl.bindFramebuffer(gl.READ_FRAMEBUFFER, null);
      if (error !== gl.NO_ERROR) throw new Error(`float readback failed: ${error}`);
      size = [cropWidth, cropHeight];

      // Handed back as base64 of the raw bytes rather than as an array of
      // numbers. Everything returned from the page is serialized as JSON, and
      // two million floats spelled out in decimal is tens of megabytes a view
      // -- slow enough that a run stopped looking like it was working at all.
      let binary = '';
      const bytes = new Uint8Array(radiance.buffer);
      const CHUNK = 0x8000;
      for (let at = 0; at < bytes.length; at += CHUNK) {
        binary += String.fromCharCode(...bytes.subarray(at, at + CHUNK));
      }
      frames.push(btoa(binary));
    }
    return { frames, places, size };
  }, { camera: CAMERA, views, width: WIDTH, height: HEIGHT, stands, percentile: PERCENTILE });

  const frames = shot.frames.map((frame) => {
    const bytes = Buffer.from(frame, 'base64');
    return new Float32Array(bytes.buffer, bytes.byteOffset, bytes.length / 4);
  });
  const [width, height] = shot.size;
  if (shotsDir) {
    const arm = path.basename(file).replace(/\.(ply|drc)$/, '');
    for (const [view, pixels] of frames.entries()) {
      await writeFile(
        path.join(shotsDir, `${arm}-${views[view].from}${view}.png`),
        encodePng(pixels, width, height),
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
 * The frames are linear radiance, so the picture is gamma-encoded and clipped
 * here. That makes it a rough likeness rather than what the viewer shows --
 * there is no tone curve on it -- which is the right trade for something whose
 * only job is to say whether the camera found the scene.
 *
 * GL reads bottom-up; PNG stores top-down, so the rows go out in reverse.
 */
function encodePng(pixels: Float32Array, width: number, height: number): Buffer {
  const raw = Buffer.alloc(height * (width * 4 + 1));
  for (let row = 0; row < height; row += 1) {
    const source = (height - 1 - row) * width * 4;
    const out = row * (width * 4 + 1);
    raw[out] = 0; // filter: none
    for (let i = 0; i < width * 4; i += 1) {
      const value = pixels[source + i];
      // Alpha is already 0..1 and linear in the sense that matters.
      raw[out + 1 + i] = Math.round(
        255 * (i % 4 === 3 ? Math.min(1, Math.max(0, value))
          : Math.min(1, Math.max(0, value)) ** (1 / 2.2)),
      );
    }
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

/**
 * What counts as full brightness, for the decibels.
 *
 * The frames are linear radiance now, not a 0-255 display signal, so the peak
 * a PSNR is taken against has to be named rather than inherited from the
 * encoding. One is the radiance of a white diffuse surface under the viewer's
 * own lighting -- the brightest thing a splat scene is normally made of --
 * and values above it exist and are kept rather than clipped.
 *
 * These decibels are therefore not comparable with any taken off an 8-bit
 * frame: same formula, different reference, and no tone curve in the way.
 */
const PEAK = 1;

function luminance(pixels: Float32Array, pixel: number): number {
  return 0.2126 * pixels[pixel * 4]
    + 0.7152 * pixels[pixel * 4 + 1] + 0.0722 * pixels[pixel * 4 + 2];
}

/** The share of a frame the scene drew on at all. */
function coverage(pixels: Float32Array): number {
  const count = pixels.length / 4;
  let drawn = 0;
  for (let pixel = 0; pixel < count; pixel += 1) {
    if (luminance(pixels, pixel) > DRAWN_ON) drawn += 1;
  }
  return (drawn / count) * 100;
}

/** Standard deviation of a frame's luminance. */
function spread(pixels: Float32Array): number {
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

  add(reference: Float32Array, arm: Float32Array) {
    const pixels = reference.length / 4;
    for (let pixel = 0; pixel < pixels; pixel += 1) {
      if (luminance(reference, pixel) <= DRAWN_ON) continue;
      this.counted += 1;
      let changed = false;
      for (let channel = 0; channel < 3; channel += 1) {
        const delta = arm[pixel * 4 + channel] - reference[pixel * 4 + channel];
        this.squared += delta * delta;
        if (Math.abs(delta) > this.worst) this.worst = Math.abs(delta);
        if (Math.abs(delta) > MOVED_BY) changed = true;
      }
      if (changed) this.moved += 1;
    }
  }

  get rmse() { return Math.sqrt(this.squared / (this.counted * 3)); }

  get psnr() {
    // No difference at all is infinite PSNR rather than a division by zero,
    // and saying so is more useful than capping it at some large number.
    return this.rmse === 0 ? Infinity : 20 * Math.log10(PEAK / this.rmse);
  }

  get movedPercent() { return (this.moved / this.counted) * 100; }
}

test('encoding arms render within the difference their budget buys', async ({ page }) => {
  test.skip(!existsSync(armsDir), `no arms in ${armsDir}: run splat_render_arms_probe first`);
  test.skip(
    !existsSync(path.join(armsDir, 'source.ply')),
    `${armsDir} has no source.ply to measure against`,
  );
  const files = armFiles();

  // Generous, and it has been hit: eleven arms of a three-million-splat scene
  // over eight views is an hour and a half of software rasterizing, and an
  // hour was not enough. A run that dies here loses everything it drew, so the
  // limit is set for the largest scene rather than for a typical one.
  test.setTimeout(4 * 60 * 60_000);
  await page.setViewportSize({ width: WIDTH + 400, height: HEIGHT + 200 });
  await page.goto('/index.html');
  await expect(page.locator('#console')).toContainText('Ready to convert 3D files!');
  await page.evaluate(async () => {
    const { state } = await import('/app/state.js' as string);
    window.__splatState = state;
  });

  let reference: Float32Array[] | null = null;
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
        expect(spread(pixels), `${views[view].from} view ${view} of the source arm is a flat frame`).toBeGreaterThan(0.01);
      }
      // One framing or the other is expected to do badly on any given scene --
      // that is why both are shot -- so the guard is on the set rather than on
      // each view: enough of them have to show the scene for the pooled
      // numbers to be about the scene.
      const shown = shot.frames.map(coverage);
      expect(
        shown.filter((share) => share > 15).length,
        'too few views show the scene',
      ).toBeGreaterThan(2);
      console.log(`\ncoverage: ${shot.frames.map((f, view) => `${views[view].from.slice(0, 3)} ${coverage(f).toFixed(0)}%`).join('  ')}`);
      continue;
    }
    const difference = new Difference();
    for (const [view, pixels] of shot.frames.entries()) difference.add(reference[view], pixels);
    rows.push(
      `${path.basename(name).replace(/\.(ply|drc)$/, '').padEnd(14)}`
      + `${difference.psnr.toFixed(2).padStart(9)}`
      + `${difference.rmse.toFixed(5).padStart(10)}`
      + `${difference.worst.toFixed(3).padStart(9)}`
      + `${difference.movedPercent.toFixed(1).padStart(9)}%`,
    );
  }

  console.log(`\n${'arm'.padEnd(14)}${'PSNR'.padStart(9)}${'RMSE'.padStart(10)}${'worst'.padStart(9)}${'moved'.padStart(10)}`);
  for (const row of rows) console.log(row);
  console.log(`  over ${views.length} view(s), on the pixels the source drew on,`);
  console.log('  in linear radiance against a peak of 1 -- not comparable with');
  console.log('  decibels taken off a tone-mapped 8-bit frame\n');
  expect(rows.length).toBeGreaterThan(0);
});
