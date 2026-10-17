/**
 * M0 spike 0.E: the app benchmarks itself. Started with GENZO_AUTOBENCH=FILE,
 * for Linux (WebKitGTK) and macOS (WKWebView), where no DevTools attach exists
 * and scripts/measure-latency.mjs cannot drive the app.
 *
 * The drag protocol copies measure-latency.mjs, so the numbers compare with the
 * Windows result: per case, wait for the first frame, then drag the slider end
 * to end 6 times in 120 steps of about 8 ms, release (the settled render) and
 * pause 600 ms. The difference: input is synthetic. Values go straight to the
 * slider's input handler, so pointer event dispatch is not timed; it is the
 * same start point the probe uses for real input.
 */
import type { EngineInfo } from '$contract/genzo';
import type { Engine } from '../engine/types';
import type { Zoom } from '../viewport/types';
import type { ColorProbeResult } from './colorProbe';

export type AutobenchSlider = 'Exposure' | 'Temp';

export interface AutobenchCase {
  source: string;
  frame: string;
  slider: AutobenchSlider;
  zoom: Zoom;
}

/** What the app exposes to the runner: the same steps a user's drag goes through. */
export interface AutobenchDriver {
  /** Selects the case's source, frame and zoom and loads the image. */
  prepare(c: AutobenchCase): Promise<void>;
  /** Moves the slider to `t` (0 is its left end, 1 its right end) and records an input. */
  input(slider: AutobenchSlider, t: number): void;
  /** The end of a gesture: one full-quality render, not timed. */
  settle(): void;
  /** Clears the probe's samples and the diagnostics. */
  reset(): void;
  /** Input to pixels, as the probe times it (from the oldest waiting input). */
  samples(): number[];
  /** Where the time went since `reset`. */
  diagnostics(): Diagnostics;
}

/**
 * What splits a sample. A probe sample starts at the oldest waiting input, so
 * with input every 8 ms it spans the request it waited behind as well as its
 * own. `sendToDrawMs` times one request alone: `sendToEventMs` (IPC and the
 * engine) plus `eventToDrawMs` (fetch, decode, upload and the next animation
 * frame). `execute` resolves after its event is out, so it overlaps the fetch.
 */
export interface Diagnostics extends TransportTimes {
  /** Engine time (render, readback, encode) of every finished frame. */
  engineMs: number[];
  sendToDrawMs: number[];
  eventToDrawMs: number[];
  framesDrawn: number;
  /** Why the viewport cannot draw, if it cannot. */
  viewportError: string | null;
}

export const PROTOCOL = { drags: 6, steps: 120, stepMs: 8, pauseMs: 600, loadWaitMs: 3000, prepareTimeoutMs: 60_000 };

/**
 * The gate cases are 2560 × 1440 at Fit. 1:1 sends full frames (an open M1
 * item); it is reported, not gated. 48 MP goes last: on a GPU with 2 GB it
 * runs out of memory, which stops the render thread and every case after it.
 */
export const CASES: AutobenchCase[] = [
  { source: 'GPU · 12 MP raw', frame: '2560 × 1440', slider: 'Exposure', zoom: 'fit' },
  { source: 'GPU · 12 MP raw', frame: '2560 × 1440', slider: 'Temp', zoom: 'fit' },
  { source: 'GPU · 12 MP raw', frame: '2560 × 1440', slider: 'Exposure', zoom: 1 },
  { source: 'GPU · 48 MP raw', frame: '2560 × 1440', slider: 'Exposure', zoom: 'fit' },
  { source: 'GPU · 48 MP raw', frame: '2560 × 1440', slider: 'Temp', zoom: 'fit' },
];

export interface Stat {
  n: number;
  p50: number | null;
  p95: number | null;
}

const stat = (xs: number[]): Stat => ({ n: xs.length, p50: percentile(xs, 0.5), p95: percentile(xs, 0.95) });

export interface Breakdown {
  framesDrawn: number;
  viewportError: string | null;
  engine: Stat;
  sendToDraw: Stat;
  sendToEvent: Stat;
  eventToDraw: Stat;
  execute: Stat;
  fetch: Stat;
  fetchKB: Stat;
}

export function breakdown(d: Diagnostics): Breakdown {
  const eventToDraw = stat(d.eventToDrawMs);
  const sendToDraw = stat(d.sendToDrawMs);
  // Medians do not subtract exactly, so this is the median request minus the median second half.
  const minus = (a: number | null, b: number | null) => (a === null || b === null ? null : Math.max(0, a - b));
  return {
    framesDrawn: d.framesDrawn,
    viewportError: d.viewportError,
    engine: stat(d.engineMs),
    sendToDraw,
    sendToEvent: { n: sendToDraw.n, p50: minus(sendToDraw.p50, eventToDraw.p50), p95: minus(sendToDraw.p95, eventToDraw.p95) },
    eventToDraw,
    execute: stat(d.executeMs),
    fetch: stat(d.fetchMs),
    fetchKB: stat(d.fetchBytes.map((b) => Math.round(b / 1024))),
  };
}

export interface CaseResult extends AutobenchCase {
  n: number;
  p50: number | null;
  p95: number | null;
  /** Input to pixels of every sample, in ms. */
  samples: number[];
  /** Where the time went; see {@link Diagnostics}. Null when the case failed before the drag. */
  breakdown: Breakdown | null;
  /** Uncaught errors in the page during the case (first few). */
  pageErrors: string[];
  /**
   * The page was hidden at some point during the case (minimized, covered or
   * the screen off, as far as the webview reports it). Its samples are then
   * suspect: hidden pages draw late or not at all.
   */
  pageHidden: boolean;
  error?: string;
}

export interface AutobenchReport {
  kind: 'genzo-autobench';
  version: 1;
  protocol: typeof PROTOCOL;
  input: 'synthetic';
  startedAt: string;
  durationMs: number;
  engine: EngineInfo;
  page: { userAgent: string; devicePixelRatio: number; window: [number, number]; webgl: string | null };
  cases: CaseResult[];
  /** The color check, run after the cases (0.E step 5). Absent in reports from before it existed. */
  color?: ColorProbeResult;
}

/** Round trips through the transport, recorded by {@link timedEngine}. */
export interface TransportTimes {
  /** `execute` calls: IPC there and back plus the engine's work, in ms. */
  executeMs: number[];
  /** `fetchFrame` calls: moving a frame's bytes into the page, in ms. */
  fetchMs: number[];
  fetchBytes: number[];
}

/** Wraps an engine to time its transport calls. */
export function timedEngine(engine: Engine, times: TransportTimes): Engine {
  return {
    kind: engine.kind,
    handshake: (hello) => engine.handshake(hello),
    query: (q) => engine.query(q),
    onEvent: (listener) => engine.onEvent(listener),
    async execute(command) {
      const t0 = performance.now();
      await engine.execute(command);
      if (command.type === 'renderSpike' && command.interactive) times.executeMs.push(performance.now() - t0);
    },
    async fetchFrame(generation) {
      const t0 = performance.now();
      const bytes = await engine.fetchFrame(generation);
      times.fetchMs.push(performance.now() - t0);
      times.fetchBytes.push(bytes.byteLength);
      return bytes;
    },
  };
}

/** Same rule as LatencyProbe.percentile. */
export function percentile(samples: number[], p: number): number | null {
  if (samples.length === 0) return null;
  const sorted = [...samples].sort((a, b) => a - b);
  return sorted[Math.round((sorted.length - 1) * p)] ?? null;
}

/**
 * The renderer WebGL runs on, which may be a different GPU from the engine's.
 * WebKit masks it (WebKitGTK reports "Apple GPU"), so the bench kit also
 * records the OpenGL and EGL renderers from outside the app.
 */
export function webglRenderer(): string | null {
  try {
    const gl = document.createElement('canvas').getContext('webgl2');
    if (!gl) return null;
    const ext = gl.getExtension('WEBGL_debug_renderer_info');
    const name = String(gl.getParameter(ext ? ext.UNMASKED_RENDERER_WEBGL : gl.RENDERER));
    gl.getExtension('WEBGL_lose_context')?.loseContext();
    return name;
  } catch {
    return null;
  }
}

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

function timeout<T>(p: Promise<T>, ms: number, what: string): Promise<T> {
  return Promise.race([p, sleep(ms).then(() => Promise.reject(new Error(`${what} took over ${ms} ms`)))]);
}

/** Runs every case; a failing case is recorded and the next one runs. */
export async function runAutobench(
  driver: AutobenchDriver,
  engine: EngineInfo,
  cases: AutobenchCase[] = CASES,
  protocol = PROTOCOL,
  onprogress: (done: number, total: number) => void = () => {},
): Promise<AutobenchReport> {
  const started = performance.now();
  const startedAt = new Date().toISOString();
  const results: CaseResult[] = [];
  // A frame that fails between the engine and the screen fails in an async
  // event handler, where nothing reports it. Catch those here.
  let pageErrors: string[] = [];
  const note = (message: string) => {
    if (pageErrors.length < 5 && !pageErrors.includes(message)) pageErrors.push(message);
  };
  const onerror = (e: ErrorEvent) => note(e.message);
  const onrejection = (e: PromiseRejectionEvent) => note(e.reason instanceof Error ? e.reason.message : String(e.reason));
  // Reset per case; the listener marks it when the page goes hidden.
  const visibility = { hidden: false };
  const onvisibility = () => {
    if (document.hidden) visibility.hidden = true;
  };
  window.addEventListener('error', onerror);
  window.addEventListener('unhandledrejection', onrejection);
  document.addEventListener('visibilitychange', onvisibility);
  for (const [i, c] of cases.entries()) {
    onprogress(i, cases.length);
    pageErrors = [];
    visibility.hidden = document.hidden;
    try {
      await timeout(driver.prepare(c), protocol.prepareTimeoutMs, `loading ${c.source}`);
      await sleep(protocol.loadWaitMs);
      driver.reset();
      for (let d = 0; d < protocol.drags; d++) {
        for (let s = 0; s <= protocol.steps; s++) {
          const t = s / protocol.steps;
          driver.input(c.slider, d % 2 === 0 ? t : 1 - t);
          await sleep(protocol.stepMs);
        }
        driver.settle();
        await sleep(protocol.pauseMs);
      }
      const samples = driver.samples();
      results.push({
        ...c,
        n: samples.length,
        p50: percentile(samples, 0.5),
        p95: percentile(samples, 0.95),
        samples,
        breakdown: breakdown(driver.diagnostics()),
        pageErrors,
        pageHidden: visibility.hidden || document.hidden,
      });
    } catch (e) {
      results.push({
        ...c,
        n: 0,
        p50: null,
        p95: null,
        samples: [],
        breakdown: null,
        pageErrors,
        pageHidden: visibility.hidden || document.hidden,
        error: e instanceof Error ? e.message : String(e),
      });
    }
  }
  window.removeEventListener('error', onerror);
  window.removeEventListener('unhandledrejection', onrejection);
  document.removeEventListener('visibilitychange', onvisibility);
  onprogress(cases.length, cases.length);
  return {
    kind: 'genzo-autobench',
    version: 1,
    protocol,
    input: 'synthetic',
    startedAt,
    durationMs: performance.now() - started,
    engine,
    page: {
      userAgent: navigator.userAgent,
      devicePixelRatio: window.devicePixelRatio,
      window: [window.innerWidth, window.innerHeight],
      webgl: webglRenderer(),
    },
    cases: results,
  };
}
