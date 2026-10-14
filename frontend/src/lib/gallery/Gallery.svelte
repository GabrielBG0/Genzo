<script lang="ts">
  import Filmstrip from '../components/Filmstrip.svelte';
  import GridCell from '../components/GridCell.svelte';
  import Histogram from '../components/Histogram.svelte';
  import LeftPanelDevelop from '../components/LeftPanelDevelop.svelte';
  import LeftPanelLibrary from '../components/LeftPanelLibrary.svelte';
  import LibraryToolbar, { type LibraryView } from '../components/LibraryToolbar.svelte';
  import Menu from '../components/Menu.svelte';
  import PanelHeader from '../components/PanelHeader.svelte';
  import PhotoViewport from '../components/PhotoViewport.svelte';
  import SliderRow from '../components/SliderRow.svelte';
  import StatusBar from '../components/StatusBar.svelte';
  import Thumb from '../components/Thumb.svelte';
  import TopBar, { type Module } from '../components/TopBar.svelte';
  import { provideRegistry } from '../shortcuts/context';
  import { sampleHistogram, sampleThumb } from './fixtures';

  // The Storybook-style gallery (0.F step 4). Each block names the handoff's
  // component code, so it can be compared with "Open full size" there.
  provideRegistry();

  let module: Module = $state('develop');
  let view: LibraryView = $state('grid');
  let exposure = $state(0.35);
  let contrast = $state(0);
  let temp = $state(5600);
  let tint = $state(8);
  let highlights = $state(-38);
  let toneOpen = $state(false);
  let filmSelected = $state('f3');
  let libraryActive = $state('day2');

  const film = Array.from({ length: 12 }, (_, i) => ({
    id: `f${i}`,
    name: `IMG_${4090 + i}.dng`,
    thumb: sampleThumb(i),
    edited: [0, 2, 3, 6, 9].includes(i),
  }));
</script>

<div class="gallery">
  <h1>Component gallery</h1>
  <p class="lead">
    Shared components from the UI handoff, built on the design tokens. Compare each with the
    handoff's "Open full size" view.
  </p>

  <section>
    <h2>C1 · Top bar <code>module</code></h2>
    <div class="frame" style:width="1440px">
      <TopBar {module} catalog="Iceland_2026" onmodule={(m) => (module = m)} />
    </div>
  </section>

  <div class="row">
    <section>
      <h2>C2 · Left panel · Develop <code>latest, latestValue</code></h2>
      <div class="frame" style:height="714px">
        <LeftPanelDevelop
          latest="Shadows"
          latestValue="+24"
          history={[
            { label: 'Highlights', value: '-38' },
            { label: 'Exposure', value: '+0.35' },
            { label: 'Preset: Velvia 50' },
            { label: 'Lens profile on' },
            { label: 'Import', value: '4/14' },
          ]}
          presetGroups={[
            { name: 'Film', presets: ['Portra 400', 'Ektar 100', 'Tri-X 400', 'Velvia 50'] },
            { name: 'Landscape', presets: Array.from({ length: 8 }, (_, i) => `Landscape ${i + 1}`) },
            { name: 'User presets', presets: Array.from({ length: 12 }, (_, i) => `Preset ${i + 1}`) },
          ]}
          activePreset="Velvia 50"
          snapshots={3}
          collections={14}
        >
          {#snippet navigator()}<Thumb image={sampleThumb(0, 230, 146)} alt="Navigator" />{/snippet}
        </LeftPanelDevelop>
      </div>
    </section>

    <section>
      <h2>C3 · Left panel · Library <code>active</code></h2>
      <div class="frame" style:height="714px">
        <LeftPanelLibrary
          active={libraryActive}
          onselect={(id) => (libraryActive = id)}
          sections={[
            {
              id: 'catalog',
              heading: 'Catalog',
              items: [
                { id: 'all', name: 'All photographs', icon: 'photos', count: 5016 },
                { id: 'quick', name: 'Quick Collection', icon: 'star', count: 12 },
                { id: 'prev', name: 'Previous import', icon: 'clock', count: 248 },
              ],
            },
            {
              id: 'folders',
              heading: 'Folders',
              items: [
                { id: 'hd', name: 'Macintosh HD', icon: 'drive', expanded: true },
                { id: '2026', name: '2026', icon: 'folder', count: 1904, depth: 1, expanded: true },
                { id: 'ice', name: '04 Iceland', icon: 'folder', count: 248, depth: 2, expanded: true },
                { id: 'day1', name: 'Day 1 Reykjavik', icon: 'folder', count: 71, depth: 3 },
                { id: 'day2', name: 'Day 2 Vik', icon: 'folder', count: 96, depth: 3 },
                { id: 'day3', name: 'Day 3 Jokulsarlon', icon: 'folder', count: 81, depth: 3 },
                { id: 'camp', name: '09 Campinas', icon: 'folder', count: 133, depth: 2, expanded: false },
              ],
            },
            {
              id: 'collections',
              heading: 'Collections',
              items: [
                { id: 'port', name: 'Portfolio', icon: 'collectionSet', expanded: true },
                { id: 'sel', name: 'Iceland selects', icon: 'collection', count: 38, depth: 1 },
                { id: 'prints', name: 'Prints 2026', icon: 'collection', count: 14, depth: 1 },
              ],
            },
          ]}
        />
      </div>
    </section>

    <div class="stack">
      <section>
        <h2>C4 · Histogram + tool strip <code>tool, clip, notools</code></h2>
        <div class="frame" style:width="304px">
          <Histogram
            data={sampleHistogram()}
            exif={['ISO 100', '24 mm', 'f/8', '1/125 s']}
            clip={{ shadow: false, highlight: true }}
          />
        </div>
      </section>

      <section>
        <h2>C5 · Panel header <code>heading, meta, edited, open</code></h2>
        <div class="frame" style:width="304px">
          <PanelHeader heading="Basic" edited meta="Color" />
          <PanelHeader heading="Tone curve" open={toneOpen} ontoggle={() => (toneOpen = !toneOpen)} />
        </div>
      </section>

      <section>
        <h2>C6 · Slider row <code>label, value, from, track, dim, hot</code></h2>
        <div class="frame" style:width="304px">
          <SliderRow label="Temp" bind:value={temp} min={2000} max={50000} defaultValue={5600} from="left" track="temp" />
          <SliderRow label="Tint" bind:value={tint} track="tint" />
          <SliderRow label="Exposure" bind:value={exposure} min={-5} max={5} step={0.05} />
          <SliderRow label="Contrast" bind:value={contrast} />
          <SliderRow label="Highlights" bind:value={highlights} hot />
        </div>
      </section>

      <section>
        <h2>C11 · Menu</h2>
        <Menu
          label="Grid photo"
          items={[
            { label: 'Open in Develop', command: 'module.develop' },
            { label: 'Set rating', submenu: true, hot: true },
            { label: 'Set flag', submenu: true },
            { label: 'Quick Collection', check: 'mixed', shortcut: 'B' },
            { separator: true },
            { label: 'Remove photo…', danger: true, shortcut: 'Del' },
          ]}
        />
      </section>
    </div>
  </div>

  <section>
    <h2>C7 · Grid cell <code>idx, stars, flag, label, selected, badge</code></h2>
    <div class="cells">
      <div style:width="180px"><GridCell idx={409} stars={4} flag="pick" label="green" thumb={sampleThumb(0)} /></div>
      <div style:width="180px"><GridCell idx={410} stars={5} label="blue" thumb={sampleThumb(1)} badge="edited" /></div>
      <div style:width="180px"><GridCell idx={411} stars={3} selected thumb={sampleThumb(3)} badge="⧉ 3" /></div>
      <div style:width="180px"><GridCell idx={412} flag="reject" thumb={sampleThumb(4)} /></div>
    </div>
  </section>

  <section>
    <h2>C8 · Library toolbar <code>view, extra</code></h2>
    <div class="frame" style:width="860px">
      <LibraryToolbar {view} extra="96 photos · 1 selected" onview={(v) => (view = v)} />
    </div>
  </section>

  <section>
    <h2>C9 · Filmstrip <code>selected</code></h2>
    <div style:width="1440px">
      <Filmstrip items={film} selected={filmSelected} onselect={(id) => (filmSelected = id)} />
    </div>
  </section>

  <section>
    <h2>C10 · Status bar <code>mode, info</code></h2>
    <div class="frame" style:width="1440px">
      <StatusBar
        mode="Develop"
        info={['96 photos · 1 selected', '14 adjustments · 2 masks']}
        facts={['Display P3', '16-bit', 'Previews 96/96', 'Catalog saved']}
      />
    </div>
  </section>

  <section>
    <h2>C12 · Photo viewport placeholder</h2>
    <div style:width="600px" style:height="400px"><PhotoViewport /></div>
  </section>
</div>

<style>
  .gallery {
    padding: 24px;
    height: 100%;
    overflow: auto;
    user-select: text;
  }

  h1 {
    margin: 0 0 4px;
    font-size: 22px;
    color: var(--text-strong);
  }

  h2 {
    margin: 0 0 8px;
    font-size: var(--text-small);
    font-weight: 600;
    color: var(--text-secondary);
  }

  code {
    font-family: var(--font-mono);
    font-weight: 400;
    color: var(--text-muted);
  }

  .lead {
    margin: 0 0 24px;
    color: var(--text-muted);
  }

  section {
    margin-bottom: 28px;
  }

  .frame {
    outline: 1px dashed var(--divider);
  }

  .row {
    display: flex;
    gap: 24px;
    align-items: flex-start;
  }

  .stack {
    display: flex;
    flex-direction: column;
  }

  .cells {
    display: flex;
    gap: 12px;
  }
</style>
