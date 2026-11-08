<script lang="ts">
  import type { Clipping, Histogram } from '$contract/genzo';
  import { clippedChannels, clippedShare, plotScale } from '../develop/histogram';
  import { useRegistry } from '../shortcuts/context';
  import Icon from './Icon.svelte';
  import { slidingLens } from './indicator.svelte';
  import type { IconName } from './icons';

  export type Tool = 'edit' | 'crop' | 'remove' | 'masking';
  export type Side = 'shadows' | 'highlights';

  interface Props {
    /** The shown photo's counts (DEV-03), or none: an empty plot. */
    histogram?: Histogram | null;
    /** EXIF row: ISO, focal length, aperture, shutter. */
    exif?: string[];
    /** Which J overlays are on: their markers have a ring. */
    overlay?: { shadows: boolean; highlights: boolean };
    /** The overlays can be turned on: a photo that renders is open. */
    canOverlay?: boolean;
    tool?: Tool;
    /** Hide the tool strip. */
    notools?: boolean;
    ontool?: (t: Tool) => void;
    onoverlay?: (side: Side) => void;
  }
  let {
    histogram = null,
    exif = [],
    overlay = { shadows: false, highlights: false },
    canOverlay = true,
    tool = 'edit',
    notools = false,
    ontool,
    onoverlay,
  }: Props = $props();

  const registry = useRegistry();
  // Red eye is Later, so the strip has four tools.
  const tools: { id: Tool; icon: IconName; label: string; command?: string }[] = [
    { id: 'edit', icon: 'edit', label: 'Edit' },
    { id: 'crop', icon: 'crop', label: 'Crop', command: 'develop.crop' },
    { id: 'remove', icon: 'remove', label: 'Remove', command: 'develop.remove' },
    { id: 'masking', icon: 'masking', label: 'Masking', command: 'develop.masking' },
  ];

  const W = 280;
  const H = 84;
  const scale = $derived(histogram ? plotScale(histogram) : 0);

  /** A channel's filled area over its 256 bins; bins taller than the scale are cut at the top. */
  function area(bins: number[]): string {
    if (scale === 0 || bins.length < 2) return '';
    const step = W / (bins.length - 1);
    const pts = bins.map((n, i) => `${(i * step).toFixed(1)} ${(H - Math.min(1, n / scale) * (H - 4)).toFixed(1)}`);
    return `M0 ${H} L${pts.join(' L')} L${W} ${H}Z`;
  }

  const markers: { side: Side; label: string; points: string }[] = [
    { side: 'shadows', label: 'Shadow clipping', points: '0,0 9,0 0,9' },
    { side: 'highlights', label: 'Highlight clipping', points: '0,0 9,0 9,9' },
  ];

  function clippingOf(side: Side): Clipping | null {
    return histogram ? histogram[side] : null;
  }

  function title(side: Side, label: string): string {
    const c = clippingOf(side);
    const s = registry.shortcut('develop.clipping');
    const head = s ? `${label}  ${s}` : label;
    return c && histogram ? `${head}\n${clippedShare(c, histogram.pixels)}` : head;
  }
</script>

<div class="histogram">
  <div class="graph">
    {#if histogram && scale > 0}
      <svg viewBox="0 0 {W} {H}" preserveAspectRatio="none" aria-label="Histogram" role="img">
        <path class="blue" d={area(histogram.blue)} />
        <path class="green" d={area(histogram.green)} />
        <path class="red" d={area(histogram.red)} />
      </svg>
    {/if}
    {#each markers as m (m.side)}
      {@const lit = clippingOf(m.side)}
      {@const channels = lit && lit.pixels > 0 ? clippedChannels(lit) : []}
      <button
        class={['clip', m.side, overlay[m.side] && 'on']}
        title={title(m.side, m.label)}
        aria-label={m.label}
        aria-pressed={overlay[m.side]}
        data-channels={channels.join(' ')}
        disabled={!canOverlay}
        onclick={() => onoverlay?.(m.side)}
      >
        <svg viewBox="0 0 9 9" aria-hidden="true">
          {#if channels.length === 0}
            <polygon class="off" points={m.points} />
          {:else if channels.length === 3}
            <polygon class="all" points={m.points} />
          {:else}
            <!-- One channel shows its own color, two their mix. -->
            {#each channels as ch (ch)}<polygon class={ch} points={m.points} />{/each}
          {/if}
        </svg>
      </button>
    {/each}
  </div>
  <div class="exif mono">
    {#each exif as e, i (i)}<span>{e}</span>{/each}
  </div>
</div>

{#if !notools}
  <div class="tools" role="toolbar" aria-label="Tools" {@attach slidingLens(() => tools.findIndex((t) => t.id === tool))}>
    {#each tools as t (t.id)}
      {@const title = t.command ? registry.title(t.command) : t.label}
      <button
        data-segment
        class={['tool', t.id === tool && 'on']}
        aria-label={title}
        {title}
        aria-pressed={t.id === tool}
        onclick={() => ontool?.(t.id)}
      >
        <Icon name={t.icon} size={16} stroke={1.7} />
      </button>
    {/each}
  </div>
{/if}

<style>
  .histogram {
    padding: 10px 10px 8px;
    color: var(--text-body);
  }

  .graph {
    height: 84px;
    position: relative;
    border-radius: 8px;
    background: var(--well);
    box-shadow: var(--well-shadow);
  }

  .graph > svg {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }

  path.red {
    fill: var(--hist-red);
    opacity: 0.4;
  }

  path.green {
    fill: var(--hist-green);
    opacity: 0.4;
  }

  path.blue {
    fill: var(--hist-blue);
    opacity: 0.45;
  }

  .clip {
    position: absolute;
    top: 3px;
    width: 13px;
    height: 13px;
    padding: 2px;
    border-radius: 3px;
  }

  .clip.shadows {
    left: 3px;
  }

  .clip.highlights {
    right: 3px;
  }

  /* The selection indicator: the overlay is on. */
  .clip.on {
    box-shadow: 0 0 0 1.5px var(--selection-ring);
  }

  .clip svg {
    display: block;
    width: 9px;
    height: 9px;
    isolation: isolate;
  }

  polygon {
    mix-blend-mode: screen;
  }

  polygon.off {
    fill: var(--clip-off);
  }

  polygon.all {
    fill: var(--clip-all);
  }

  polygon.red {
    fill: var(--hist-red);
  }

  polygon.green {
    fill: var(--hist-green);
  }

  polygon.blue {
    fill: var(--hist-blue);
  }

  .exif {
    display: flex;
    justify-content: space-between;
    margin-top: 6px;
    min-height: 15px;
    font-size: var(--text-mono-small);
    color: var(--text-secondary);
  }

  .tools {
    display: flex;
    gap: 2px;
    margin: 0 10px 8px;
    padding: 2px;
    border-radius: var(--r-pill);
    background: var(--trough-soft);
  }

  .tool.on {
    color: var(--text-on-lens);
  }

  .tool {
    flex: 1;
    height: 28px;
    border-radius: var(--r-pill);
    display: grid;
    place-items: center;
    border: 1px solid transparent;
    color: var(--text-secondary-alt);
  }
</style>
