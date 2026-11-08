<script lang="ts">
  import type { ZoomState } from '../develop/zoom.svelte';
  import { useRegistry } from '../shortcuts/context';
  import { slidingLens } from './indicator.svelte';

  interface Props {
    zoom: ZoomState;
    /** Toggles that are on, by command: the before, the clipping overlays. */
    on: Record<string, boolean>;
  }
  let { zoom, on }: Props = $props();
  const registry = useRegistry();

  function run(id: string) {
    const cmd = registry.get(id);
    if (cmd.enabled()) cmd.run();
  }

  const TOGGLES = [
    { command: 'develop.beforeAfter', label: 'Before / After' },
    { command: 'develop.clipping', label: 'Clipping' },
  ];
</script>

<!-- The photo toolbar (DEV-01 viewport spec): the flat Overlay surface, no blur. -->
<div class="photo-toolbar overlay" role="toolbar" aria-label="Photo toolbar">
  {#each TOGGLES as t (t.command)}
    <button
      class={['tb', on[t.command] && 'on']}
      aria-pressed={on[t.command] ?? false}
      title={registry.title(t.command)}
      disabled={!registry.get(t.command).enabled()}
      onclick={() => run(t.command)}
    >
      {t.label} <kbd class="mono">{registry.shortcut(t.command)}</kbd>
    </button>
  {/each}
  <span class="divider" aria-hidden="true"></span>
  <span
    class="zoom"
    role="radiogroup"
    aria-label="Zoom"
    {@attach slidingLens(() => (zoom.level === 'fit' ? 0 : 1))}
  >
    <button
      data-segment
      role="radio"
      aria-checked={zoom.level === 'fit'}
      class={[zoom.level === 'fit' && 'on']}
      title={registry.title('view.zoom')}
      onclick={() => zoom.fit()}
    >
      Fit
    </button>
    <button
      data-segment
      role="radio"
      aria-checked={zoom.level === '1:1'}
      class={[zoom.level === '1:1' && 'on']}
      title={registry.title('view.zoom')}
      onclick={() => zoom.zoomIn(null)}
    >
      1:1
    </button>
  </span>
  <span class="readout mono" aria-live="polite">{zoom.readout}</span>
</div>

<style>
  .photo-toolbar {
    position: absolute;
    left: 50%;
    bottom: 16px;
    transform: translateX(-50%);
    height: 42px;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 8px 0 6px;
    border-radius: var(--r-pill);
    white-space: nowrap;
    color: var(--text-body);
    font-size: var(--text-small);
    z-index: 2;
  }

  .tb {
    height: 28px;
    padding: 0 10px;
    border-radius: var(--r-pill);
    color: var(--text-secondary);
  }

  .tb:hover:not(:disabled) {
    background: var(--hover);
  }

  .tb.on {
    color: var(--accent-soft);
    background: var(--toggle-on-bg);
  }

  .tb:disabled {
    color: var(--text-dim);
    cursor: default;
  }

  kbd {
    font-size: var(--text-mono-small);
    color: var(--text-muted);
  }

  .divider {
    width: 1px;
    height: 18px;
    background: var(--line);
  }

  .zoom {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--r-pill);
    background: var(--trough);
  }

  .zoom button {
    height: 26px;
    padding: 0 11px;
    border-radius: var(--r-pill);
    color: var(--text-muted);
  }

  .zoom button.on {
    color: var(--text-on-lens);
  }

  .readout {
    min-width: 64px;
    text-align: right;
    color: var(--text-secondary);
    font-size: var(--text-mono-small);
  }
</style>
