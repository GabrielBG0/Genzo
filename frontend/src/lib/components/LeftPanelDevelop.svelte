<script lang="ts">
  import type { Snippet } from 'svelte';
  import { useRegistry } from '../shortcuts/context';
  import Icon from './Icon.svelte';

  export interface PresetGroup {
    name: string;
    presets: string[];
  }

  export interface HistoryStep {
    label: string;
    value?: string;
  }

  interface Props {
    /** Newest history step, shown at the top and highlighted. */
    latest?: string | null;
    latestValue?: string;
    /** Older steps, newest first. */
    history?: HistoryStep[];
    presetGroups?: PresetGroup[];
    activePreset?: string | null;
    snapshots?: number;
    collections?: number;
    /** Content of the Navigator box: a preview of the photo. */
    navigator?: Snippet;
  }
  let {
    latest = null,
    latestValue = '',
    history = [],
    presetGroups = [],
    activePreset = null,
    snapshots = 0,
    collections = 0,
    navigator,
  }: Props = $props();

  const registry = useRegistry();
  let open = $state<Record<string, boolean>>({ presets: true, history: true });
  // Preset groups start open when they hold the active preset.
  let groupOpen = $state<Record<string, boolean>>({});
  const isGroupOpen = (g: PresetGroup) => groupOpen[g.name] ?? g.presets.includes(activePreset ?? '');

  const copyKey = $derived(registry.shortcut('settings.copy'));
  const pasteKey = $derived(registry.shortcut('settings.paste'));
</script>

<aside class="panel pane" aria-label="Left panel">
  <div class="nav-title">
    <span>NAVIGATOR</span><span class="mono zoom">Fit · Fill · 1:1</span>
  </div>
  <div class="navigator">
    {#if navigator}{@render navigator()}{/if}
  </div>

  {@render section('presets', 'Presets', '+')}
  {#if open.presets}
    <div class="list">
      {#each presetGroups as g (g.name)}
        <button class="row" style:padding-left="8px" onclick={() => (groupOpen[g.name] = !isGroupOpen(g))}>
          <span class="twisty">{isGroupOpen(g) ? '⌄' : '›'}</span><span class="text">{g.name}</span>
          <span class="count mono">{g.presets.length}</span>
        </button>
        {#if isGroupOpen(g)}
          {#each g.presets as p (p)}
            <button class={['row', p === activePreset && 'lens']} style:padding-left="24px">
              <span class="twisty"></span><span class="text">{p}</span>
              <span class="count mono">{p === activePreset ? 'on' : ''}</span>
            </button>
          {/each}
        {/if}
      {/each}
    </div>
  {/if}

  {@render section('snapshots', 'Snapshots', String(snapshots))}

  {@render section('history', 'History')}
  {#if open.history}
    <ol class="history">
      {#if latest}
        <li class="step latest">
          <span class="marker"></span><span class="text">{latest}</span>
          <span class="count mono">{latestValue}</span>
        </li>
      {/if}
      {#each history as step, i (i)}
        <li class={['step', i === history.length - 1 && 'origin']}>
          <span class="marker"></span><span class="text">{step.label}</span>
          <span class="count mono">{step.value ?? ''}</span>
        </li>
      {/each}
    </ol>
  {/if}

  <div class="fill"></div>
  {@render section('collections', 'Collections', String(collections))}
  <div class="actions">
    <button class="pill lens" title={registry.title('settings.copy')}>Copy <span class="hint mono">{copyKey}</span></button>
    <button class="pill lens" title={registry.title('settings.paste')}>Paste <span class="hint mono">{pasteKey}</span></button>
  </div>
</aside>

{#snippet section(id: string, heading: string, meta = '')}
  <button class="section" aria-expanded={open[id] ?? false} onclick={() => (open[id] = !open[id])}>
    <Icon name={open[id] ? 'chevronDown' : 'chevronRight'} size={14} stroke={2} />
    <span class="text">{heading}</span>
    <span class="meta mono">{meta}</span>
  </button>
{/snippet}

<style>
  .panel {
    width: var(--left-w);
    height: 100%;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    color: var(--text-body);
  }

  .nav-title {
    height: 30px;
    flex: none;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 12px;
    font-size: var(--text-caption);
    font-weight: 600;
    letter-spacing: var(--header-tracking);
    color: var(--text-secondary-alt);
  }

  .zoom {
    font-weight: 400;
    letter-spacing: 0;
  }

  .navigator {
    margin: 0 10px 10px;
    height: 146px;
    flex: none;
    position: relative;
    overflow: hidden;
    border-radius: 8px;
    background: var(--photo-backdrop);
  }

  .section {
    height: 26px;
    flex: none;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 8px;
    border-top: 1px solid var(--divider);
    font-size: var(--text-caption);
    font-weight: 600;
    letter-spacing: var(--header-tracking);
    text-transform: uppercase;
    text-align: left;
  }

  .section .text {
    flex: 1;
  }

  .meta {
    font-weight: 400;
    letter-spacing: 0;
    color: var(--text-muted);
  }

  .list {
    padding: 0 6px 6px;
  }

  .row {
    width: 100%;
    height: var(--row-list);
    display: flex;
    align-items: center;
    gap: 6px;
    padding-right: 10px;
    border-radius: var(--r-thumb);
    margin-bottom: 1px;
    border: 1px solid transparent;
    text-align: left;
  }

  .row:hover:not(.lens) {
    background: var(--hover);
  }

  .twisty {
    width: 10px;
    color: var(--text-muted);
  }

  .text {
    flex: 1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .count {
    font-size: var(--text-mono-small);
    color: var(--text-muted);
  }

  .history {
    position: relative;
    list-style: none;
    margin: 0;
    padding: 0 0 4px 20px;
  }

  .history::before {
    content: '';
    position: absolute;
    left: 16px;
    top: 8px;
    bottom: 8px;
    width: 1px;
    background: var(--slider-track);
  }

  .step {
    position: relative;
    height: var(--row-slider);
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 12px 0 10px;
  }

  .step .count {
    color: var(--text-secondary);
  }

  .marker {
    position: absolute;
    left: -7px;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--marker-inactive);
  }

  .latest {
    color: var(--text-on-lens);
  }

  .latest .marker {
    background: var(--accent);
    box-shadow: var(--history-glow);
  }

  .origin {
    color: var(--text-muted);
  }

  .origin .marker {
    border: 1px solid var(--marker-inactive);
    background: var(--bg);
  }

  .fill {
    flex: 1;
  }

  .actions {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 6px;
    padding: 8px 10px 10px;
    border-top: 1px solid var(--divider);
  }

  .pill {
    height: 28px;
    padding: 0 6px;
    border-radius: var(--r-pill);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .hint {
    color: var(--text-on-lens-hint);
    font-size: var(--text-mono-small);
  }
</style>
