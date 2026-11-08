<script lang="ts">
  import { untrack } from 'svelte';
  import type { EngineInfo } from '$contract/genzo';
  import BasicPanel from './components/BasicPanel.svelte';
  import ExportDialog from './components/ExportDialog.svelte';
  import Filmstrip from './components/Filmstrip.svelte';
  import Histogram from './components/Histogram.svelte';
  import LeftPanelDevelop from './components/LeftPanelDevelop.svelte';
  import PhotoViewport from './components/PhotoViewport.svelte';
  import StatusBar from './components/StatusBar.svelte';
  import TopBar, { type Module } from './components/TopBar.svelte';
  import { exifRow } from './develop/exif';
  import { Exports } from './develop/export.svelte';
  import { DevelopSession } from './develop/session.svelte';
  import { Thumbnails } from './develop/thumbnails.svelte';
  import { bindLayout, Layout } from './layout.svelte';
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
  const thumbnails = untrack(() => new Thumbnails(engine));
  const exports = untrack(() => new Exports(engine));
  /** Which panels show (APP-08), for the session. */
  const layout = new Layout(registry.platform === 'mac');
  bindLayout(registry, layout, () => !exportOpen);
  /** The export dialog (OUT-01) is open. */
  let exportOpen = $state(false);

  /** File › Open folder (LIB-09): the system's folder dialog. */
  async function openFolder() {
    const path = await engine.chooseFolder();
    if (path !== null) await session.openFolder(path);
  }

  registry.bind(
    'file.openFolder',
    () => void openFolder(),
    () => !exportOpen,
  );
  registry.bind(
    'module.export',
    () => (exportOpen = true),
    () => session.editable,
  );

  registry.bind(
    'export.cancel',
    () => void exports.cancel(),
    () => exports.busy,
  );

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
    'develop.beforeAfter',
    () => session.toggleBefore(),
    () => session.editable,
  );
  registry.bind(
    'develop.picker',
    () => session.togglePicker(),
    () => session.editable,
  );
  registry.bind(
    'develop.clipping',
    () => session.toggleClipping(),
    () => session.editable,
  );
  registry.bind(
    'develop.clippingShadows',
    () => session.toggleClipping('shadows'),
    () => session.editable,
  );
  registry.bind(
    'develop.clippingHighlights',
    () => session.toggleClipping('highlights'),
    () => session.editable,
  );
  const at = $derived(session.strip.photos.findIndex((p) => p.id === session.strip.current));
  registry.bind(
    'photo.previous',
    () => void session.step('previous'),
    () => at > 0,
  );
  registry.bind(
    'photo.next',
    () => void session.step('next'),
    () => at >= 0 && at < session.strip.photos.length - 1,
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
    // Quitting during an export asks first, and cancels it (OUT-01).
    const stopClose = engine.onCloseRequested(() => exports.allowQuit());
    return () => {
      stopClose();
      session.dispose();
      thumbnails.dispose();
      exports.dispose();
    };
  });

  function onmodule(m: Module) {
    // In M1 the Export tab opens the export dialog (APP-01, OUT-01).
    if (m === 'export') {
      if (session.editable) exportOpen = true;
      return;
    }
    // Leaving Develop turns the clipping overlays off (DEV-03, APP-01).
    if (m !== 'develop') session.clearClipping();
    module = m;
  }

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
  const items = $derived(
    session.strip.photos.map((p) => {
      const thumb = thumbnails.get(p.id);
      return {
        id: String(p.id),
        name: p.fileName,
        edited: p.edited,
        thumb: typeof thumb === 'object' ? thumb : null,
        noThumb: thumb === 'none',
        missing: p.kind === 'missing',
      };
    }),
  );

  // The strip's thumbnails, the visible cells first (LIB-09).
  let visible = $state({ first: 0, last: -1 });
  const ids = $derived(session.strip.photos.map((p) => p.id));
  $effect(() => thumbnails.show(ids, visible.first, visible.last));

  const folderName = $derived(session.folder?.split(/[\\/]/).filter(Boolean).pop() ?? null);
  const stripMessage = $derived(
    session.folder === null ? 'Open a folder' : `No photos in this folder: ${folderName ?? session.folder}`,
  );
  const adapterNotice = $derived(info.adapter.kind === 'hardware' ? null : info.adapter.fallbackReason);

  function onkeydown(e: KeyboardEvent) {
    // The dialog takes its own keys; no command runs under it.
    if (exportOpen) return;
    registry.handle(e, module === 'develop' ? 'develop' : 'library');
  }
</script>

<svelte:window {onkeydown} />

<div
  class="shell"
  style:grid-template-rows="var(--topbar-h) minmax(0, 1fr){layout.filmstrip ? ' var(--filmstrip-h)' : ''} var(--statusbar-h)"
>
  <TopBar
    {module}
    catalog={current ? current.fileName : 'No folder open'}
    progress={{ 'module.export': exports.progress }}
    on={{ 'develop.beforeAfter': session.before }}
    {onmodule}
  />
  <!-- A hidden panel stays mounted, so it shows again as it was (APP-08). -->
  <main
    style:grid-template-columns="{layout.left ? 'var(--left-w) ' : ''}minmax(0, 1fr){layout.right ? ' var(--right-w)' : ''}"
  >
    <div class="left" hidden={!layout.left}>
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
        badge={session.before ? 'Before' : null}
      />
    </div>
    <aside class="right pane" aria-label="Right panel" hidden={!layout.right}>
      <Histogram
        histogram={session.histogram}
        exif={current ? exifRow(current.shooting) : []}
        overlay={session.overlay}
        canOverlay={session.editable}
        onoverlay={(side) => session.toggleClipping(side)}
      />
      <BasicPanel {session} presets={info.whiteBalancePresets} {layout} />
    </aside>
  </main>
  <div class="strip" hidden={!layout.filmstrip}>
    <Filmstrip
      {items}
      selected={session.strip.current === null ? null : String(session.strip.current)}
      onselect={(id) => void session.select(Number(id))}
      onvisible={(first, last) => (visible = { first, last })}
      message={stripMessage}
      shortcut={session.folder === null ? registry.shortcut('file.openFolder') : ''}
    />
  </div>
  <StatusBar
    mode="Develop"
    info={current ? [current.fileName] : []}
    facts={['sRGB', '8-bit', engine.kind === 'fake' ? 'Fake engine' : info.adapter.name]}
    notice={exports.status ?? session.notice ?? session.reason ?? adapterNotice}
    noticeMenu={exports.busy ? [{ label: 'Cancel export', command: 'export.cancel' }] : null}
  />
  {#if exportOpen && current && session.editable}
    <ExportDialog
      {engine}
      {exports}
      photo={current.id}
      fileName={current.fileName}
      photoFolder={session.folder ?? ''}
      onclose={() => (exportOpen = false)}
    />
  {/if}
</div>

<style>
  .shell {
    height: 100%;
    min-height: 600px;
    display: grid;
    gap: var(--gap);
    padding: 0 var(--pad);
  }

  main {
    display: grid;
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

  .strip {
    display: grid;
    min-height: 0;
  }

  [hidden] {
    display: none !important;
  }
</style>
