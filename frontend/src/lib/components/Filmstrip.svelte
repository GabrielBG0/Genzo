<script lang="ts">
  import Thumb, { type RgbaImage } from './Thumb.svelte';

  export interface FilmstripItem {
    id: string;
    name: string;
    thumb?: RgbaImage | null;
    edited?: boolean;
  }

  interface Props {
    items: FilmstripItem[];
    selected?: string | null;
    onselect?: (id: string) => void;
  }
  let { items, selected = null, onselect }: Props = $props();
</script>

<section class="filmstrip pane" aria-label="Filmstrip">
  {#if items.length === 0}
    <span class="empty">No photos. Open a folder to start.</span>
  {/if}
  {#each items as item (item.id)}
    <button
      class={['frame', item.id === selected && 'current']}
      aria-current={item.id === selected}
      title={item.name}
      onclick={() => onselect?.(item.id)}
    >
      <Thumb image={item.thumb ?? null} alt={item.name} />
      {#if item.edited}<span class="edited" aria-label="Edited">●</span>{/if}
    </button>
  {/each}
</section>

<style>
  .filmstrip {
    height: var(--filmstrip-h);
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 0 10px;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
  }

  .empty {
    color: var(--text-muted);
    padding-left: 4px;
  }

  .frame {
    flex: none;
    width: 108px;
    height: 70px;
    border-radius: var(--r-thumb);
    position: relative;
    overflow: hidden;
    opacity: 0.72;
  }

  .frame:hover {
    opacity: 0.9;
  }

  .current {
    opacity: 1;
    overflow: visible;
    box-shadow:
      0 0 0 2px var(--selection-ring),
      var(--thumb-lift);
  }

  .current :global(.thumb) {
    border-radius: var(--r-thumb);
    overflow: hidden;
  }

  .edited {
    position: absolute;
    right: 5px;
    top: 3px;
    font-size: 8px;
    color: var(--accent);
  }
</style>
