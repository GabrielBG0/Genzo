<script lang="ts">
  import { useRegistry } from '../shortcuts/context';

  export type LibraryView = 'grid' | 'loupe' | 'compare' | 'survey';

  interface Props {
    view: LibraryView;
    /** Text at the right end, e.g. a selection count. */
    extra?: string;
    sort?: string;
    onview?: (v: LibraryView) => void;
  }
  let { view, extra = '', sort = 'Capture time', onview }: Props = $props();
  const registry = useRegistry();

  // People (O) is Later, so it is not offered.
  const views: { id: LibraryView; label: string; key: string }[] = [
    { id: 'grid', label: 'Grid', key: 'G' },
    { id: 'loupe', label: 'Loupe', key: 'E' },
    { id: 'compare', label: 'Compare', key: 'C' },
    { id: 'survey', label: 'Survey', key: 'N' },
  ];
  // Library view commands are registered in M2; until then show the keys from the handoff.
  const keyOf = (v: { id: LibraryView; key: string }) =>
    registry.has(`library.${v.id}`) ? registry.shortcut(`library.${v.id}`) : v.key;
</script>

<div class="toolbar" role="toolbar" aria-label="Library toolbar">
  <span class="segments">
    {#each views as v (v.id)}
      <button class={['seg', v.id === view && 'lens']} aria-pressed={v.id === view} onclick={() => onview?.(v.id)}>
        {v.label} <span class="key mono">{keyOf(v)}</span>
      </button>
    {/each}
  </span>
  <span class="caption">Sort</span><span>{sort} ⌄</span>
  <span class="spacer"></span>
  <span class="extra">{extra}</span>
</div>

<style>
  .toolbar {
    height: 42px;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 12px;
    border-top: 1px solid var(--divider);
    color: var(--text-body);
    white-space: nowrap;
    overflow: hidden;
  }

  .segments {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--r-pill);
    background: var(--trough-soft);
  }

  .seg {
    height: 26px;
    padding: 0 10px;
    border-radius: var(--r-pill);
    border: 1px solid transparent;
    color: var(--text-secondary-alt);
  }

  .key {
    font-size: var(--text-mono-small);
    opacity: 0.6;
  }

  .caption {
    color: var(--text-muted);
  }

  .spacer {
    flex: 1;
  }

  .extra {
    color: var(--text-secondary);
  }
</style>
