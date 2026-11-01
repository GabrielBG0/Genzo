<script lang="ts">
  import { untrack } from 'svelte';
  import type { EngineInfo } from '$contract/genzo';
  import BasicPanel from './components/BasicPanel.svelte';
  import Filmstrip from './components/Filmstrip.svelte';
  import Histogram from './components/Histogram.svelte';
  import LeftPanelDevelop from './components/LeftPanelDevelop.svelte';
  import PhotoViewport from './components/PhotoViewport.svelte';
  import StatusBar from './components/StatusBar.svelte';
  import TopBar, { type Module } from './components/TopBar.svelte';
  import { DevelopSession } from './develop/session.svelte';
  import type { Engine } from './engine/types';
  import { provideRegistry } from './shortcuts/context';

  interface Props {
    engine: Engine;
    info: EngineInfo;
  }
  let { engine, info }: Props = $props();

  const registry = provideRegistry();
  let module: Module = $state('develop');
  const session = untrack(() => new DevelopSession(engine));

  registry.bind(
    'edit.undo',
    () => session.undo(),
    () => session.canUndo,
    () => (session.canUndo && session.edit?.undo ? `Undo ${session.edit.undo}` : 'Undo'),
  );
  registry.bind(
    'edit.redo',
    () => session.redo(),
    () => session.canRedo,
    () => (session.canRedo && session.edit?.redo ? `Redo ${session.edit.redo}` : 'Redo'),
  );
  registry.bind(
    'develop.treatment',
    () => {
      const e = session.shown;
      if (e) session.change([{ Treatment: e.treatment === 'Color' ? 'Monochrome' : 'Color' }]);
    },
    () => session.editable,
  );
  registry.bind(
    'develop.picker',
    () => session.togglePicker(),
    () => session.editable,
  );
  registry.bind(
    'develop.pickerCancel',
    () => (session.picking = false),
    () => session.picking,
  );

  $effect(() => {
    untrack(() => {
      // The fake opens its demo folder; the real engine waits for a folder
      // to be chosen (wired with the real engine).
      if (engine.kind === 'fake') void session.openFolder('demo');
    });
  });

  /** The viewport's size goes to the engine once per animation frame at most. */
  let viewFrame: number | null = null;
  function onresize(width: number, height: number) {
    if (viewFrame !== null) cancelAnimationFrame(viewFrame);
    viewFrame = requestAnimationFrame(() => {
      viewFrame = null;
      void session.setView(width, height);
    });
  }

  const current = $derived(session.strip.photos.find((p) => p.id === session.strip.current) ?? null);
  const items = $derived(session.strip.photos.map((p) => ({ id: String(p.id), name: p.fileName, edited: p.edited })));
  const adapterNotice = $derived(info.adapter.kind === 'hardware' ? null : info.adapter.fallbackReason);

  function onkeydown(e: KeyboardEvent) {
    registry.handle(e, module === 'develop' ? 'develop' : 'library');
  }
</script>

<svelte:window {onkeydown} />

<div class="shell">
  <TopBar {module} catalog={current ? current.fileName : 'No folder open'} onmodule={(m) => (module = m)} />
  <main>
    <div class="left">
      <LeftPanelDevelop />
    </div>
    <div class="center">
      <PhotoViewport
        {engine}
        photo={session.strip.current}
        picking={session.picking}
        onpick={(x, y) => session.pick(x, y)}
        {onresize}
        empty={session.reason ?? 'No photo open'}
      />
    </div>
    <aside class="right pane" aria-label="Right panel">
      <Histogram />
      <BasicPanel {session} presets={info.whiteBalancePresets} />
    </aside>
  </main>
  <Filmstrip
    {items}
    selected={session.strip.current === null ? null : String(session.strip.current)}
    onselect={(id) => void session.select(Number(id))}
  />
  <StatusBar
    mode="Develop"
    info={current ? [current.fileName] : []}
    facts={['sRGB', '8-bit', engine.kind === 'fake' ? 'Fake engine' : info.adapter.name]}
    notice={session.notice ?? session.reason ?? adapterNotice}
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
</style>
