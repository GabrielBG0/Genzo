<script lang="ts">
  import { untrack } from 'svelte';
  import type { EngineInfo } from '$contract/genzo';
  import Filmstrip from './components/Filmstrip.svelte';
  import Histogram from './components/Histogram.svelte';
  import LeftPanelDevelop from './components/LeftPanelDevelop.svelte';
  import PanelHeader from './components/PanelHeader.svelte';
  import PhotoViewport from './components/PhotoViewport.svelte';
  import SliderRow from './components/SliderRow.svelte';
  import StatusBar from './components/StatusBar.svelte';
  import TopBar, { type Module } from './components/TopBar.svelte';
  import type { Engine } from './engine/types';
  import { provideRegistry } from './shortcuts/context';
  import { runAutobench, timedEngine, type AutobenchDriver, type Diagnostics } from './spike/autobench';
  import { runColorProbe } from './spike/colorProbe';
  import { LatencyProbe } from './spike/latency';
  import type { Zoom } from './viewport/types';

  interface Props {
    engine: Engine;
    info: EngineInfo;
  }
  let { engine: given, info }: Props = $props();
  // The autobench times the transport calls; otherwise the engine is used as given.
  const diag: Diagnostics = {
    engineMs: [],
    executeMs: [],
    fetchMs: [],
    fetchBytes: [],
    sendToDrawMs: [],
    eventToDrawMs: [],
    framesDrawn: 0,
    viewportError: null,
  };
  /** When each generation's renderFinished arrived, for the autobench. */
  // Not reactive state: nothing renders from it.
  let finishedAt: [generation: number, at: number][] = [];
  const engine = untrack(() => (info.autobench ? timedEngine(given, diag) : given));

  const registry = provideRegistry();
  let module: Module = $state('develop');

  // M0 spikes 0.D and 0.E: drive the GPU pipeline (or the CPU test pattern)
  // from sliders and time input to pixels on screen.
  const SIZES = { '2560 × 1440': [2560, 1440], '1280 × 720': [1280, 720] } as const;
  const SOURCES = {
    'GPU · 48 MP raw': [8064, 6048],
    'GPU · 12 MP raw': [4032, 3024],
    'Test pattern': null,
  } as const;
  type Source = keyof typeof SOURCES;
  let size: keyof typeof SIZES = $state('2560 × 1440');
  // Picked once at startup: a real engine without a GPU can only draw the test pattern.
  // The autobench starts small: 48 MP can run out of GPU memory, which stops
  // the render thread before the first case.
  function initialSource(): Source {
    if (engine.kind === 'tauri' && info.adapter.kind === 'none') return 'Test pattern';
    return info.autobench ? 'GPU · 12 MP raw' : 'GPU · 48 MP raw';
  }
  let source: Source = $state(initialSource());
  let phase = $state(0);
  let exposure = $state(0);
  let temp = $state(0);
  let zoom: Zoom = $state('fit');
  let stats = $state({ p50: null as number | null, p95: null as number | null, n: 0 });
  let gpuMs: number | null = $state(null);

  const wb = (t: number): [number, number, number] => [1 + t * 0.004, 1, 1 - t * 0.004];

  function render(interactive: boolean): Promise<void> {
    const [width, height] = SIZES[size];
    if (SOURCES[source] === null) {
      return engine.execute({ type: 'renderTestPattern', width, height, phase: phase / 100, format: 'rgba8Srgb' });
    }
    return engine.execute({
      type: 'renderSpike',
      width,
      height,
      exposureEv: exposure,
      wb: wb(temp),
      format: 'rgba8Srgb',
      interactive,
      // While dragging, a half-size JPEG preview is much cheaper to move into
      // the webview. At 1:1 the preview would show at the wrong scale.
      proxy: interactive && zoom === 'fit',
    });
  }

  // While dragging: interactive renders, timed by the probe.
  const probe = new LatencyProbe<null>(() => render(true));
  // At the end of a gesture: one full-quality render, not timed. It may replace
  // a drag render on the render thread, so that drag sample is dropped too.
  function settle() {
    probe.cancel();
    void render(false).catch(() => {});
  }

  function ondraw(generation: number, at: number) {
    if (info.autobench) {
      diag.framesDrawn += 1;
      const finished = finishedAt.find(([g]) => g === generation);
      if (finished) diag.eventToDrawMs.push(at - finished[1]);
      finishedAt = finishedAt.filter(([g]) => g > generation);
    }
    probe.drawn(at);
    stats = { p50: probe.percentile(0.5), p95: probe.percentile(0.95), n: probe.count };
  }

  async function load() {
    probe.reset();
    const dims = SOURCES[source];
    if (dims) await engine.execute({ type: 'loadSpikeImage', source: { type: 'synthetic', width: dims[0], height: dims[1] } });
    probe.input(null);
  }

  // Started with GENZO_AUTOBENCH: drive the sliders like the Windows script
  // does, then hand the report to the engine, which writes it to disk.
  let autobench: string | null = $state(null);
  const driver: AutobenchDriver = {
    async prepare(c) {
      source = c.source as Source;
      size = c.frame as keyof typeof SIZES;
      zoom = c.zoom;
      await load();
    },
    input(slider, t) {
      if (slider === 'Exposure') exposure = Math.round((-3 + 6 * t) * 100) / 100;
      else temp = Math.round(-100 + 200 * t);
      probe.input(null);
    },
    settle,
    reset: () => {
      probe.reset();
      for (const k of ['engineMs', 'executeMs', 'fetchMs', 'fetchBytes', 'eventToDrawMs'] as const) diag[k] = [];
      diag.framesDrawn = 0;
    },
    samples: () => probe.all,
    diagnostics: () => ({ ...diag, sendToDrawMs: probe.sendToDrawMs }),
  };
  async function selfBenchmark() {
    const report = await runAutobench(driver, info, undefined, undefined, (done, total) => {
      autobench = `Autobench running, case ${Math.min(done + 1, total)} of ${total}. Do not touch or cover this window.`;
    });
    // After the cases, so it cannot change their numbers. It loads its own small image.
    autobench = 'Autobench: checking colors.';
    report.color = await runColorProbe(engine);
    await engine.execute({ type: 'saveAutobenchReport', report: JSON.stringify(report, null, 2) });
    autobench = 'Autobench done. The report is saved; the script closes the app.';
  }

  $effect(() => {
    // The first frame, so the viewport is not empty. Untracked: a source
    // change reloads through the select's onchange, not by re-running this.
    untrack(() => {
      const first = load();
      if (info.autobench) {
        void first
          .catch(() => {})
          .then(selfBenchmark)
          .catch((e: unknown) => (autobench = `Autobench failed: ${e instanceof Error ? e.message : String(e)}`));
      }
    });
    return engine.onEvent((e) => {
      if (e.type !== 'renderFinished') return;
      gpuMs = e.gpuMs;
      if (info.autobench) {
        diag.engineMs.push(e.renderMs);
        finishedAt.push([e.generation, performance.now()]);
      }
    });
  });

  const ms = (v: number | null) => (v === null ? '–' : `${v.toFixed(1)} ms`);
  const notice = $derived(info.adapter.kind === 'hardware' ? null : info.adapter.fallbackReason);

  function onkeydown(e: KeyboardEvent) {
    registry.handle(e, module === 'develop' ? 'develop' : 'library');
  }
</script>

<svelte:window {onkeydown} />

<div class="shell">
  <TopBar {module} catalog="M0 spike" onmodule={(m) => (module = m)} />
  <main>
    <div class="left">
      <LeftPanelDevelop />
    </div>
    <div class="center">
      <PhotoViewport {engine} {zoom} {ondraw} onerror={(m) => (diag.viewportError = m)} />
    </div>
    <aside class="right pane" aria-label="Right panel">
      <Histogram />
      <PanelHeader heading="Latency spike" meta="M0 · 0.D, 0.E" edited={stats.n > 0} />
      {#if SOURCES[source] === null}
        <SliderRow label="Phase" bind:value={phase} min={0} max={100} oninput={() => probe.input(null)} />
      {:else}
        <SliderRow
          label="Exposure"
          bind:value={exposure}
          min={-3}
          max={3}
          step={0.01}
          oninput={() => probe.input(null)}
          onchange={settle}
        />
        <SliderRow label="Temp" bind:value={temp} track="temp" oninput={() => probe.input(null)} onchange={settle} />
      {/if}
      <dl class="readout mono">
        <dt>Source</dt>
        <dd>
          <select bind:value={source} onchange={() => void load()}>
            {#each Object.keys(SOURCES) as s (s)}<option>{s}</option>{/each}
          </select>
        </dd>
        <dt>Frame</dt>
        <dd>
          <select bind:value={size} onchange={() => probe.reset()}>
            {#each Object.keys(SIZES) as s (s)}<option>{s}</option>{/each}
          </select>
        </dd>
        <dt>Zoom</dt>
        <dd>
          <select bind:value={zoom}>
            <option value="fit">Fit</option>
            <option value={1}>1:1</option>
          </select>
        </dd>
        <dt>Input → pixels p50</dt>
        <dd>{ms(stats.p50)}</dd>
        <dt>Input → pixels p95</dt>
        <dd class={[stats.p95 !== null && stats.p95 > 50 && 'over']}>{ms(stats.p95)}</dd>
        <dt>GPU, last frame</dt>
        <dd>{ms(gpuMs)}</dd>
        <dt>Samples</dt>
        <dd>{stats.n}</dd>
      </dl>
      <p class="note" role="status">
        {autobench ?? 'Gate: under 50 ms p95 at 2560 px. Drag the slider to collect samples.'}
      </p>
    </aside>
  </main>
  <Filmstrip items={[]} />
  <StatusBar
    mode="Develop"
    info={[`Engine ${info.engineVersion}`, `Contract ${info.contractVersion}`]}
    facts={['sRGB', '8-bit', engine.kind === 'fake' ? 'Fake engine' : info.adapter.name]}
    {notice}
  />
</div>

<style>
  .shell {
    height: 100%;
    min-height: 600px;
    display: grid;
    grid-template-rows: var(--topbar-h) minmax(0, 1fr) var(--filmstrip-h) var(--statusbar-h);
    gap: var(--gap);
    padding: 0 var(--pad);
  }

  main {
    display: grid;
    grid-template-columns: var(--left-w) minmax(0, 1fr) var(--right-w);
    gap: var(--gap);
    min-height: 0;
  }

  .left,
  .center {
    min-height: 0;
  }

  .right {
    min-height: 0;
    overflow: auto;
  }

  .readout {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 4px 10px;
    margin: 8px 12px 0 26px;
    font-size: var(--text-mono-small);
  }

  dt {
    color: var(--text-muted);
  }

  dd {
    margin: 0;
    text-align: right;
    color: var(--text-strong);
  }

  dd.over {
    color: var(--danger);
  }

  select {
    font: inherit;
    color: var(--text-body);
    background: var(--dialog);
    border: 1px solid var(--field);
    border-radius: 6px;
  }

  .note {
    margin: 10px 12px 0 26px;
    color: var(--text-muted);
    font-size: var(--text-small);
  }
</style>
