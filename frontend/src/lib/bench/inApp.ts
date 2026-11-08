import type { Change, EngineInfo, Event } from '$contract/genzo';
import type { DevelopSession } from '../develop/session.svelte';
import type { ZoomState } from '../develop/zoom.svelte';
import type { Engine } from '../engine/types';
import { PROTOCOL, runAutobench, type AutobenchDriver, type AutobenchSlider, type Diagnostics } from './autobench';
import { runColorProbe } from './colorProbe';
import { LatencyProbe } from './latency';

/** The cases' synthetic RAW sizes and view sizes, by the names the report keeps. */
const SOURCES: Record<string, [number, number]> = {
  'GPU · 12 MP raw': [4032, 3024],
  'GPU · 48 MP raw': [8064, 6048],
};
const FRAMES: Record<string, [number, number]> = { '2560 × 1440': [2560, 1440], '1280 × 720': [1280, 720] };

/** Against the fake engine (e2e tests), the same steps, shortened. */
const QUICK: typeof PROTOCOL = { drags: 2, steps: 6, stepMs: 8, pauseMs: 40, loadWaitMs: 40, prepareTimeoutMs: 20_000 };

/** What the autobench needs from the app around it. */
export interface BenchHost {
  /** The engine the app uses, already wrapped by `timedEngine` with `diag`. */
  engine: Engine;
  info: EngineInfo;
  session: DevelopSession;
  zoom: ZoomState;
  /** Pins the view the engine renders to a size in device pixels, whatever the window's. */
  pinView(size: [number, number]): void;
  /** What the app says while the bench runs. */
  status(text: string): void;
}

/** A fresh set of diagnostics, for `timedEngine` and the driver. */
export function newDiagnostics(): Diagnostics {
  return {
    engineMs: [],
    executeMs: [],
    fetchMs: [],
    fetchBytes: [],
    sendToDrawMs: [],
    eventToDrawMs: [],
    framesDrawn: 0,
    viewportError: null,
  };
}

/**
 * The autobench in the app (GENZO_AUTOBENCH): each case opens its synthetic
 * RAW through Develop, pins the view, and drags Exposure or Temp through
 * the session's gesture, timed from input to the viewport's draw. Then the
 * color probe, then the report goes to the engine, which writes it.
 * Returns the hooks the app calls: each draw, with its generation.
 */
export function startAutobench(host: BenchHost, diag: Diagnostics): { ondraw: (generation: number, at: number) => void } {
  const { engine, session, zoom } = host;
  /** When each generation's `RenderFinished` arrived. */
  let finishedAt: [generation: number, at: number][] = [];
  let drawnWaiters: (() => void)[] = [];
  let gesture = false;
  let last: Change | null = null;

  // Input goes through the session as a slider's drag does; one request
  // in flight at a time, the latest value waiting (the M0 protocol).
  const probe = new LatencyProbe<Change>((change) => {
    session.change([change], gesture ? 'Update' : 'Start');
    gesture = true;
    return Promise.resolve();
  });

  engine.onEvent((e: Event) => {
    if (e.type !== 'renderFinished') return;
    diag.engineMs.push(e.renderMs);
    finishedAt.push([e.generation, performance.now()]);
  });

  const nextDraw = () => new Promise<void>((resolve) => drawnWaiters.push(resolve));

  function valueOf(slider: AutobenchSlider, t: number): Change {
    if (slider === 'Exposure') return { Exposure: Math.round((-3 + 6 * t) * 100) / 100 };
    const tint = session.shown?.white_balance.tint ?? 0;
    return { WhiteBalance: { mode: 'Custom', temp: Math.round((3000 + 6000 * t) / 10) * 10, tint } };
  }

  const driver: AutobenchDriver = {
    async prepare(c) {
      const source = SOURCES[c.source];
      const frame = FRAMES[c.frame];
      if (!source || !frame) throw new Error(`unknown case ${c.source} at ${c.frame}`);
      // The fake draws its frames in JavaScript: a small view keeps the e2e test quick.
      host.pinView(engine.kind === 'fake' ? [320, 180] : frame);
      if (c.zoom === 'fit') zoom.fit();
      const drawn = nextDraw();
      await session.openSynthetic(source[0], source[1]);
      if (c.zoom !== 'fit') zoom.zoomIn(null);
      await drawn;
    },
    input(slider, t) {
      last = valueOf(slider, t);
      probe.input(last);
    },
    settle() {
      // The end of a gesture: one full-quality render, not timed.
      probe.cancel();
      if (last) session.change([last], 'End');
      gesture = false;
    },
    reset() {
      probe.reset();
      for (const k of ['engineMs', 'executeMs', 'fetchMs', 'fetchBytes', 'eventToDrawMs'] as const) diag[k] = [];
      diag.framesDrawn = 0;
    },
    samples: () => probe.all,
    diagnostics: () => ({ ...diag, sendToDrawMs: probe.sendToDrawMs }),
  };

  void (async () => {
    try {
      const protocol = engine.kind === 'fake' ? QUICK : PROTOCOL;
      const report = await runAutobench(driver, host.info, undefined, protocol, (done, total) => {
        host.status(`Autobench running, case ${Math.min(done + 1, total)} of ${total}. Do not touch or cover this window.`);
      });
      // After the cases, so it cannot change their numbers. It opens its own small scene.
      // The fake sends no JPEG previews and has no colors to check: only a real webview does.
      if (engine.kind !== 'fake') {
        host.status('Autobench: checking colors.');
        report.color = await runColorProbe(engine);
      }
      await engine.execute({ type: 'saveAutobenchReport', report: JSON.stringify(report, null, 2) });
      const ran = report.cases.filter((c) => c.n > 0).length;
      host.status(`Autobench done: ${ran} of ${report.cases.length} cases. The report is saved; the script closes the app.`);
    } catch (e) {
      host.status(`Autobench failed: ${e instanceof Error ? e.message : String(e)}`);
    }
  })();

  return {
    ondraw(generation, at) {
      diag.framesDrawn += 1;
      const finished = finishedAt.find(([g]) => g === generation);
      if (finished) diag.eventToDrawMs.push(at - finished[1]);
      finishedAt = finishedAt.filter(([g]) => g > generation);
      probe.drawn(at);
      const waiters = drawnWaiters;
      drawnWaiters = [];
      for (const w of waiters) w();
    },
  };
}
