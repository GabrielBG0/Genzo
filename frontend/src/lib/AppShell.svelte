<script lang="ts">
  import { untrack } from 'svelte';
  import type { EngineInfo } from '$contract/genzo';
  import AboutDialog from './components/AboutDialog.svelte';
  import AppMenu from './components/AppMenu.svelte';
  import BasicPanel from './components/BasicPanel.svelte';
  import ExportDialog from './components/ExportDialog.svelte';
  import Filmstrip from './components/Filmstrip.svelte';
  import Histogram from './components/Histogram.svelte';
  import LeftPanelDevelop from './components/LeftPanelDevelop.svelte';
  import PhotoToolbar from './components/PhotoToolbar.svelte';
  import PhotoViewport from './components/PhotoViewport.svelte';
  import StatusBar from './components/StatusBar.svelte';
  import TopBar, { type Module } from './components/TopBar.svelte';
  import { exifRow } from './develop/exif';
  import { Exports } from './develop/export.svelte';
  import { chooseNotice } from './develop/status';
  import { DevelopSession } from './develop/session.svelte';
  import { Thumbnails } from './develop/thumbnails.svelte';
  import { ZoomState } from './develop/zoom.svelte';
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
  /** The viewport's zoom and pan (DEV-01 viewport spec), for the session. */
  const zoom = new ZoomState();
  /** The photo toolbar shows (T), for the session. */
  let toolbarShown = $state(true);
  /** Space is down: its key repeats are not new presses. */
  let spaceDown = false;
  /** The export dialog (OUT-01) is open. */
  let exportOpen = $state(false);
  /** About Genzo (APP-02) is open. */
  let aboutOpen = $state(false);
  /** The app menu (APP-02) is open, its top left here. */
  let menuAt: { x: number; y: number } | null = $state(null);
  /** A dialog takes the keys: no command runs under it. */
  const inDialog = $derived(exportOpen || aboutOpen);
  bindLayout(registry, layout, () => !inDialog);

  /** Opens the app menu below the logo, or closes it. */
  function toggleMenu() {
    if (menuAt) {
      menuAt = null;
      return;
    }
    const logo = document.querySelector('.app-menu')?.getBoundingClientRect();
    menuAt = { x: logo?.left ?? 8, y: (logo?.bottom ?? 40) + 4 };
  }
  registry.bind('app.menu', toggleMenu, () => !inDialog);
  registry.bind('app.quit', () => engine.closeWindow(), () => true);
  registry.bind('help.about', () => (aboutOpen = true), () => !inDialog);

  /** File › Open folder (LIB-09): the system's folder dialog. */
  async function openFolder() {
    const path = await engine.chooseFolder();
    if (path !== null) await session.openFolder(path);
  }

  registry.bind(
    'file.openFolder',
    () => void openFolder(),
    () => !inDialog,
  );
  registry.bind(
    'module.export',
    () => (exportOpen = true),
    () => session.editable,
  );

  // D goes to Develop; Library waits for M2 (APP-01).
  registry.bind(
    'module.develop',
    () => onmodule('develop'),
    () => !inDialog,
  );
  registry.bind(
    'module.library',
    () => onmodule('library'),
    () => false,
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
    'develop.resetAll',
    () => session.reset(null),
    () => session.editable,
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
  // The viewport (DEV-01 viewport spec): Z, Space held, T.
  registry.bind(
    'view.zoom',
    () => zoom.toggle(zoom.pointer),
    () => session.editable && !inDialog,
    () => (zoom.level === 'fit' ? 'Zoom to 1:1' : 'Zoom to Fit'),
  );
  registry.bind(
    'view.zoomHold',
    () => {
      if (spaceDown) return;
      spaceDown = true;
      zoom.hold(zoom.pointer);
    },
    () => session.editable && !inDialog,
  );
  registry.bind(
    'view.toolbar',
    () => (toolbarShown = !toolbarShown),
    () => !inDialog,
    () => (toolbarShown ? 'Hide toolbar' : 'Show toolbar'),
  );
  function releaseSpace() {
    spaceDown = false;
    zoom.release();
  }

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

  function onresize(width: number, height: number) {
    zoom.resize(width, height);
  }

  /** What the viewport shows goes to the engine once per animation frame at most, the last one always. */
  let viewFrame: number | null = null;
  $effect(() => {
    const { width, height } = zoom.view;
    const request = zoom.request;
    // Again once a switch has answered: the engine opens each photo at Fit.
    void session.edit?.photo;
    if (width === 0 || height === 0) return;
    if (viewFrame !== null) cancelAnimationFrame(viewFrame);
    viewFrame = requestAnimationFrame(() => {
      viewFrame = null;
      void session.setView(width, height, request);
    });
  });

  // The shown image's size comes with each edit reply; a new photo opens on its center.
  let sizedPhoto: number | null = null;
  $effect(() => {
    const e = session.edit;
    if (!e) return;
    untrack(() => {
      zoom.setShown(e.shown.width, e.shown.height, e.photo !== sizedPhoto);
      sizedPhoto = e.photo;
    });
  });
  // A photo that does not render shows at Fit (its embedded preview); one opening keeps the zoom.
  $effect(() => {
    if (session.reason !== null) untrack(() => zoom.fit());
  });

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

  // The status bar (APP-07).
  const notice = $derived(
    chooseNotice({
      exporting: exports.status,
      refusal: session.notice,
      reason: session.reason,
      notes: session.editable ? (session.edit?.notes ?? []) : [],
      adapter: adapterNotice,
    }),
  );
  const statusInfo = $derived(
    session.folder === null
      ? []
      : [
          `${session.strip.photos.length} ${session.strip.photos.length === 1 ? 'photo' : 'photos'}`,
          ...(current ? [current.fileName] : []),
        ],
  );
  const PREVIEW_TITLES: Record<string, string> = {
    Opening: 'The photo is opening',
    'Embedded preview': "The file's embedded preview: not rendered by Genzo",
    'No preview': 'This file has no embedded preview to show',
    Interactive: 'A quick preview while you drag; full quality when you let go',
    Settled: 'Rendered at full quality',
  };
  const statusFacts = $derived([
    ...(session.previewStatus
      ? [{ text: session.previewStatus, title: PREVIEW_TITLES[session.previewStatus] ?? '' }]
      : []),
    session.space === 'displayP3'
      ? { text: 'Display P3', title: 'Frames are rendered in Display P3' }
      : { text: 'sRGB', title: 'Frames are rendered in sRGB' },
    { text: '8-bit', title: 'Frames are shown with 8 bits per channel' },
    engine.kind === 'fake'
      ? { text: 'Fake engine', title: 'The browser build: a fake engine, no GPU' }
      : { text: info.adapter.name, title: 'The GPU rendering the photos' },
  ]);

  /**
   * Alt opens the app menu only on its own (APP-02): pressed and released
   * with no other key or click between, so Alt-drag and Alt-click keep
   * their meaning.
   */
  let altAlone = false;
  /** A pointer button is down: Alt now belongs to a drag or a click. */
  let pressing = false;

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Alt') {
      // In a text field Alt is the field's (Alt codes on Windows).
      const typing = e.target instanceof Element && e.target.closest('input, textarea, [contenteditable="true"]');
      altAlone = !typing && !pressing && (!e.repeat || altAlone);
      if (altAlone) e.preventDefault();
      return;
    }
    altAlone = false;
    // A dialog, or the app menu, takes its own keys; no command runs under it.
    if (inDialog || menuAt) return;
    registry.handle(e, module === 'develop' ? 'develop' : 'library');
  }

  function onkeyup(e: KeyboardEvent) {
    // Space up ends a temporary zoom, wherever focus went meanwhile.
    if (e.code === 'Space') releaseSpace();
    if (e.key !== 'Alt') return;
    const alone = altAlone;
    altAlone = false;
    if (!alone) return;
    e.preventDefault();
    const cmd = registry.get('app.menu');
    if (cmd.enabled()) cmd.run();
  }

  const menuChecks = $derived({
    'view.sidePanels': layout.left && layout.right,
    'view.allPanels': layout.left && layout.right && layout.filmstrip,
    'view.filmstrip': layout.filmstrip,
    'view.leftPanel': layout.left,
    'view.rightPanel': layout.right,
    'view.toolbar': toolbarShown,
    'develop.beforeAfter': session.before,
    'develop.picker': session.picking,
    'develop.clipping':
      session.overlay.shadows && session.overlay.highlights
        ? true
        : session.overlay.shadows || session.overlay.highlights
          ? ('mixed' as const)
          : false,
  });
</script>

<svelte:window
  {onkeydown}
  {onkeyup}
  onpointerdown={() => {
    altAlone = false;
    pressing = true;
  }}
  onpointerup={() => (pressing = false)}
  onpointercancel={() => (pressing = false)}
  onblur={() => {
    altAlone = false;
    pressing = false;
    releaseSpace();
  }}
/>

<div
  class="shell"
  style:grid-template-rows="var(--topbar-h) minmax(0, 1fr){layout.filmstrip ? ' var(--filmstrip-h)' : ''} var(--statusbar-h)"
>
  <TopBar
    onmenu={toggleMenu}
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
        view={zoom}
        zoomable={session.editable}
      >
        {#if toolbarShown && session.editable}
          <PhotoToolbar
            {zoom}
            on={{
              'develop.beforeAfter': session.before,
              'develop.clipping': session.overlay.shadows || session.overlay.highlights,
            }}
          />
        {/if}
      </PhotoViewport>
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
    info={statusInfo}
    facts={statusFacts}
    notice={notice?.text ?? null}
    noticeTitle={notice?.title}
    noticeMenu={exports.busy ? [{ label: 'Cancel export', command: 'export.cancel' }] : null}
  />
  {#if menuAt}
    <AppMenu folder={folderName} at={menuAt} checks={menuChecks} onclose={() => (menuAt = null)} />
  {/if}
  {#if aboutOpen}
    <AboutDialog {info} onclose={() => (aboutOpen = false)} />
  {/if}
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
