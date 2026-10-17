/**
 * M0 spike 0.E step 5: does each webview draw sRGB and Display P3 frames with
 * the right colors? Run at the end of the autobench.
 *
 * Two paths can go wrong, in different ways:
 * - Raw RGBA8 tiles upload as typed arrays, which skip `unpackColorSpace`, so
 *   they are right as long as the webview honors `drawingBufferColorSpace`.
 *   If it ignores it, P3 values show as sRGB (too saturated). Reading the
 *   buffer back cannot see that; `support.drawingBufferColorSpace` can.
 * - JPEG drag previews are decoded to bitmaps with `colorSpaceConversion:
 *   'none'` and uploaded as images. If the webview converts them at upload,
 *   a P3 preview comes out desaturated. It did on WebView2 while
 *   `unpackColorSpace` followed the frame's space; it now stays sRGB (see
 *   `WebGLViewport.setColorSpace`). Reading the buffer back does see this.
 *
 * So for each space, the synthetic scene is rendered as raw tiles and as a
 * JPEG preview, drawn through the app's own upload path into a detached
 * canvas, and read back at the centers of its 12 flat patches. The engine's
 * raw bytes at the same points are the truth. Read-back happens before the
 * compositor, so it needs no wide-gamut monitor; what a person sees on one is
 * a separate, visual check.
 */
import type { Event, PixelFormat } from '$contract/genzo';
import type { Tile } from '../engine/tiles';
import { decodeFrame } from '../engine/tiles';
import type { Engine } from '../engine/types';
import { putFrame } from '../viewport/frame';
import { WebGLViewport } from '../viewport/webgl';

/** Scene size: the proxy is half of it, so both fill the probe canvas exactly. */
const SCENE: [number, number] = [1200, 800];
const CANVAS: [number, number] = [600, 400];
/** Largest per-channel error (8-bit codes) a path may show and still pass. */
export const TOLERANCE = { raw: 2, jpeg: 6 };

/** Centers of the scene's 6 × 2 grid of flat patches (pipeline/src/gpu/synthetic.rs). */
export const PATCHES: [number, number][] = [0.5, 0.7].flatMap((v) =>
  [0, 1, 2, 3, 4, 5].map((k): [number, number] => [(k + 0.5) / 6, v]),
);

export interface ColorSupport {
  /** The webview implements `drawingBufferColorSpace` and keeps 'display-p3' in it. */
  drawingBufferColorSpace: boolean;
  /** The same for `unpackColorSpace` (recorded only; the viewport leaves it alone). */
  unpackColorSpace: boolean;
  /** The monitor's gamut, as CSS media queries report it. */
  displayGamut: 'rec2020' | 'p3' | 'srgb' | 'unknown';
}

export interface PathResult {
  format: PixelFormat;
  /** The color spaces the canvas kept while this frame was drawn. */
  canvas: { drawing: string; unpack: string };
  /** The engine's bytes at each patch center (RGB). */
  truth: number[][];
  /** The drawing buffer at each patch center, read back (mean RGB, rounded to 0.1). */
  readBack: number[][];
  maxError: number;
  pass: boolean;
}

export interface ColorProbeResult {
  support: ColorSupport;
  paths: PathResult[];
  /** Largest difference between the sRGB and the P3 raw truth: how far a wrong path could be off. */
  srgbVsP3: number;
  error?: string;
}

export function colorSupport(): ColorSupport {
  let drawing = false;
  let unpack = false;
  try {
    const gl = document.createElement('canvas').getContext('webgl2');
    if (gl) {
      drawing = keepsP3(gl, 'drawingBufferColorSpace');
      unpack = keepsP3(gl, 'unpackColorSpace');
      gl.getExtension('WEBGL_lose_context')?.loseContext();
    }
  } catch {
    // Unsupported properties throw in some engines: both stay false.
  }
  const gamut = (g: string) => typeof matchMedia === 'function' && matchMedia(`(color-gamut: ${g})`).matches;
  const displayGamut = gamut('rec2020') ? 'rec2020' : gamut('p3') ? 'p3' : gamut('srgb') ? 'srgb' : 'unknown';
  return { drawingBufferColorSpace: drawing, unpackColorSpace: unpack, displayGamut };
}

/**
 * Whether the context implements `property` and keeps 'display-p3' in it.
 * Reading back alone is not enough: on an engine without the property, the
 * assignment just adds a plain JavaScript property, which reads back fine
 * (seen for `unpackColorSpace` on WKWebView).
 */
export function keepsP3(
  gl: WebGL2RenderingContext,
  property: 'drawingBufferColorSpace' | 'unpackColorSpace',
): boolean {
  if (!(property in Object.getPrototypeOf(gl))) return false;
  gl[property] = 'display-p3';
  return gl[property] === 'display-p3';
}

/** The RGB of raw tiles at a normalized point of the image. */
export function pixelAt(tiles: Tile[], nx: number, ny: number): number[] {
  for (const { header, pixels } of tiles) {
    const x = Math.min(Math.floor(nx * header.imageWidth), header.imageWidth - 1);
    const y = Math.min(Math.floor(ny * header.imageHeight), header.imageHeight - 1);
    const { rect } = header;
    if (x < rect.x || y < rect.y || x >= rect.x + rect.width || y >= rect.y + rect.height) continue;
    const i = ((y - rect.y) * rect.width + (x - rect.x)) * 4;
    return [...pixels.subarray(i, i + 3)];
  }
  throw new Error(`no tile covers ${nx}, ${ny}`);
}

/** Largest per-channel difference over all points. */
export function maxError(a: number[][], b: number[][]): number {
  let max = 0;
  a.forEach((p, i) => p.forEach((v, c) => (max = Math.max(max, Math.abs(v - (b[i]?.[c] ?? NaN))))));
  return Number.isNaN(max) ? Infinity : max;
}

/** Runs one render and resolves with its frame. */
async function renderFrame(engine: Engine, format: PixelFormat, proxy: boolean): Promise<ArrayBuffer> {
  let generation: number | null = null;
  const off = engine.onEvent((e: Event) => {
    if (e.type === 'renderFinished') generation = e.generation;
  });
  try {
    // A preview is only made during a drag, so both paths render as interactive.
    await engine.execute({
      type: 'renderSpike',
      width: SCENE[0],
      height: SCENE[1],
      exposureEv: 0,
      wb: [1, 1, 1],
      format,
      interactive: true,
      proxy,
    });
  } finally {
    off();
  }
  if (generation === null) throw new Error(`no frame for ${format}${proxy ? ' preview' : ''}`);
  return engine.fetchFrame(generation);
}

/** Loads the scene, renders it four ways and reads each back. */
export async function runColorProbe(engine: Engine): Promise<ColorProbeResult> {
  const support = colorSupport();
  const result: ColorProbeResult = { support, paths: [], srgbVsP3: 0 };
  let viewport: WebGLViewport | null = null;
  try {
    viewport = new WebGLViewport(document.createElement('canvas'));
    viewport.resize(CANVAS[0], CANVAS[1], 1);
    await engine.execute({ type: 'loadSpikeImage', source: { type: 'synthetic', width: SCENE[0], height: SCENE[1] } });
    const truths: number[][][] = [];
    for (const format of ['rgba8Srgb', 'rgba8DisplayP3'] as const) {
      const raw = await renderFrame(engine, format, false);
      const rawTiles = decodeFrame(raw);
      const truth = PATCHES.map(([x, y]) => pixelAt(rawTiles, x, y));
      truths.push(truth);
      for (const [bytes, tolerance] of [
        [raw, TOLERANCE.raw],
        [await renderFrame(engine, format, true), TOLERANCE.jpeg],
      ] as const) {
        // Generations only go up, so each frame replaces the last one.
        if (!(await putFrame(viewport, bytes))) throw new Error('the probe frame was dropped as stale');
        const readBack = viewport.readPixels(PATCHES).map((p) => p.map((v) => Math.round(v * 10) / 10));
        const error = maxError(readBack, truth);
        result.paths.push({
          format: decodeFrame(bytes)[0]?.header.format ?? format,
          canvas: viewport.colorSpaces(),
          truth,
          readBack,
          maxError: error,
          pass: error <= tolerance,
        });
      }
    }
    const [srgb = [], p3 = []] = truths;
    result.srgbVsP3 = maxError(srgb, p3);
  } catch (e) {
    result.error = e instanceof Error ? e.message : String(e);
  } finally {
    viewport?.destroy();
  }
  return result;
}
