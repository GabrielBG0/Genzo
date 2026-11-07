<script lang="ts" module>
  import type { ThumbImage } from '../develop/thumbnails.svelte';

  export interface FilmstripItem {
    id: string;
    name: string;
    /** The thumbnail; none yet draws a neutral cell. */
    thumb?: ThumbImage | null;
    /** The file has no thumbnail: its cell shows the file name. */
    noThumb?: boolean;
    edited?: boolean;
    /** The file went missing after listing: dimmed (LIB-09, States). */
    missing?: boolean;
  }

  /** Cell width, gap and the strip's side padding, in CSS px (the handoff's Filmstrip). */
  export const CELL = 108;
  export const GAP = 8;
  export const PAD = 10;
  const PITCH = CELL + GAP;
  /** Cells drawn beyond each visible edge. */
  const OVERSCAN = 4;

  /**
   * The cells at least partly visible with the strip scrolled to `scroll`
   * and `width` wide: every cell before it is measured.
   */
  export function visibleRange(scroll: number, width: number, count: number): { first: number; last: number } {
    if (count === 0) return { first: 0, last: -1 };
    if (width <= 0) return { first: 0, last: count - 1 };
    // Cell i spans [PAD + i * PITCH, PAD + i * PITCH + CELL).
    const first = Math.floor((scroll - PAD - CELL) / PITCH) + 1;
    const last = Math.ceil((scroll + width - PAD) / PITCH) - 1;
    return { first: Math.max(0, Math.min(count - 1, first)), last: Math.max(0, Math.min(count - 1, last)) };
  }
</script>

<script lang="ts">
  import Thumb from './Thumb.svelte';

  interface Props {
    items: FilmstripItem[];
    selected?: string | null;
    onselect?: (id: string) => void;
    /** The visible cells changed: their indices, first and last. */
    onvisible?: (first: number, last: number) => void;
    /** Shown when there are no items. */
    message?: string;
    /** A shortcut shown beside the message. */
    shortcut?: string;
  }
  let {
    items,
    selected = null,
    onselect,
    onvisible,
    message = 'No photos. Open a folder to start.',
    shortcut = '',
  }: Props = $props();

  let strip: HTMLElement | undefined = $state();
  let scroll = $state(0);
  let width = $state(0);

  const range = $derived(visibleRange(scroll, width, items.length));
  const drawn = $derived(
    width <= 0
      ? range
      : { first: Math.max(0, range.first - OVERSCAN), last: Math.min(items.length - 1, range.last + OVERSCAN) },
  );
  const cells = $derived(items.slice(drawn.first, drawn.last + 1).map((item, i) => ({ item, index: drawn.first + i })));

  $effect(() => {
    onvisible?.(range.first, range.last);
  });

  // Keeps the current photo in view, centered when it had left it. Only on
  // a change of photo, width or count: a thumbnail arriving changes `items`,
  // and must not pull back a strip the user scrolled away.
  const at = $derived(items.findIndex((i) => i.id === selected));
  $effect(() => {
    if (!strip || width <= 0 || at < 0) return;
    const left = PAD + at * PITCH;
    if (left < strip.scrollLeft || left + CELL > strip.scrollLeft + width) {
      strip.scrollLeft = left - (width - CELL) / 2;
    }
  });

  function onwheel(e: WheelEvent) {
    // A mouse wheel scrolls the strip sideways.
    if (!strip || e.deltaX !== 0 || e.deltaY === 0) return;
    strip.scrollLeft += e.deltaY;
    e.preventDefault();
  }
</script>

<section
  class="filmstrip pane"
  aria-label="Filmstrip"
  bind:this={strip}
  bind:clientWidth={width}
  onscroll={() => (scroll = strip?.scrollLeft ?? 0)}
  {onwheel}
>
  {#if items.length === 0}
    <p class="empty">
      {message}{#if shortcut}<kbd>{shortcut}</kbd>{/if}
    </p>
  {:else}
    <div class="track" style:width="{PAD * 2 + items.length * PITCH - GAP}px">
      {#each cells as { item, index } (item.id)}
        <button
          class={['frame', item.id === selected && 'current', item.missing && 'missing']}
          style:left="{PAD + index * PITCH}px"
          aria-current={item.id === selected}
          title={item.name}
          onclick={() => onselect?.(item.id)}
        >
          <Thumb image={item.thumb?.image ?? null} p3={item.thumb?.p3 ?? false} alt={item.name} />
          {#if item.noThumb}<span class="name">{item.name}</span>{/if}
          {#if item.edited}<span class="edited" aria-label="Edited">●</span>{/if}
        </button>
      {/each}
    </div>
  {/if}
</section>

<style>
  .filmstrip {
    height: var(--filmstrip-h);
    position: relative;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
  }

  .empty {
    height: 100%;
    margin: 0;
    display: flex;
    gap: 10px;
    align-items: center;
    justify-content: center;
    color: var(--text-muted);
  }

  kbd {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-dim);
  }

  .track {
    position: relative;
    height: 100%;
  }

  .frame {
    position: absolute;
    top: calc(50% - 35px);
    width: 108px;
    height: 70px;
    border-radius: var(--r-thumb);
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

  .missing {
    opacity: 0.35;
  }

  .current :global(.thumb) {
    border-radius: var(--r-thumb);
    overflow: hidden;
  }

  .name {
    position: absolute;
    inset: auto 6px 6px;
    font-size: 10px;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: left;
  }

  .edited {
    position: absolute;
    right: 5px;
    top: 3px;
    font-size: 8px;
    color: var(--accent);
  }
</style>
