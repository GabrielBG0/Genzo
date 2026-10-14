<script lang="ts">
  import Icon from './Icon.svelte';
  import type { IconName } from './icons';

  export interface TreeItem {
    id: string;
    name: string;
    icon: IconName;
    count?: number;
    depth?: number;
    /** undefined for leaves. */
    expanded?: boolean;
  }

  export interface TreeSection {
    id: string;
    heading: string;
    items: TreeItem[];
  }

  interface Props {
    /** Id of the active source (folder, collection). */
    active?: string | null;
    sections?: TreeSection[];
    onselect?: (id: string) => void;
    onimport?: () => void;
    onexport?: () => void;
  }
  let { active = null, sections = [], onselect, onimport, onexport }: Props = $props();

  let filter = $state('');
  const visible = (item: TreeItem) =>
    filter.trim() === '' || item.name.toLowerCase().includes(filter.trim().toLowerCase());
  const fmt = new Intl.NumberFormat('en-US');
</script>

<aside class="panel pane" aria-label="Catalog">
  <label class="filter">
    <Icon name="search" size={13} />
    <input bind:value={filter} placeholder="Filter folders and collections" />
  </label>

  {#each sections as s (s.id)}
    <div class="section">
      <Icon name="chevronDown" size={14} stroke={2} /><span class="heading">{s.heading}</span><span class="add">+</span>
    </div>
    <div class="list">
      {#each s.items.filter(visible) as item (item.id)}
        <button
          class={['row', item.id === active && 'lens']}
          style:padding-left="{8 + (item.depth ?? 0) * 14}px"
          aria-current={item.id === active}
          onclick={() => onselect?.(item.id)}
        >
          <span class="twisty">{item.expanded === undefined ? '' : item.expanded ? '⌄' : '›'}</span>
          <Icon name={item.icon} size={13} />
          <span class="text">{item.name}</span>
          <span class="count mono">{item.count === undefined ? '' : fmt.format(item.count)}</span>
        </button>
      {/each}
    </div>
  {/each}

  <div class="fill"></div>
  <div class="actions">
    <button class="pill lens" onclick={() => onimport?.()}>Import…</button>
    <button class="pill lens" onclick={() => onexport?.()}>Export…</button>
  </div>
</aside>

<style>
  .panel {
    width: var(--left-w);
    height: 100%;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    color: var(--text-body);
  }

  .filter {
    margin: 8px 8px 4px;
    height: 26px;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 10px;
    border-radius: var(--r-pill);
    border: 1px solid var(--field);
    color: var(--text-muted);
  }

  input {
    flex: 1;
    min-width: 0;
    background: none;
    border: 0;
    outline: none;
    color: var(--text-body);
    font: inherit;
    font-size: var(--text-small);
  }

  input::placeholder {
    color: var(--text-muted);
  }

  .section {
    height: 26px;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 12px 0 8px;
    margin-top: 4px;
    border-top: 1px solid var(--divider);
    font-size: var(--text-caption);
    font-weight: 600;
    letter-spacing: var(--header-tracking);
    text-transform: uppercase;
  }

  .heading {
    flex: 1;
  }

  .add {
    color: var(--text-muted);
    font-weight: 400;
  }

  .list {
    padding: 0 6px;
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
    color: var(--text-body);
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
    border-radius: var(--r-pill);
  }
</style>
