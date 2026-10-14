<script lang="ts">
  import { useRegistry } from '../shortcuts/context';
  import Icon from './Icon.svelte';
  import type { IconName } from './icons';

  export type Tool = 'edit' | 'crop' | 'remove' | 'masking';

  export interface HistogramData {
    /** Per-channel bin heights, each normalized to 0..1. */
    r: number[];
    g: number[];
    b: number[];
  }

  interface Props {
    data?: HistogramData | null;
    /** EXIF row: ISO, focal length, aperture, shutter. */
    exif?: string[];
    clip?: { shadow: boolean; highlight: boolean };
    tool?: Tool;
    /** Hide the tool strip. */
    notools?: boolean;
    ontool?: (t: Tool) => void;
    onclip?: (which: 'shadow' | 'highlight') => void;
  }
  let {
    data = null,
    exif = [],
    clip = { shadow: false, highlight: false },
    tool = 'edit',
    notools = false,
    ontool,
    onclip,
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

  function area(bins: number[]): string {
    if (bins.length < 2) return '';
    const step = W / (bins.length - 1);
    const pts = bins.map((v, i) => `${(i * step).toFixed(1)} ${(H - Math.min(1, Math.max(0, v)) * (H - 4)).toFixed(1)}`);
    return `M0 ${H} L${pts.join(' L')} L${W} ${H}Z`;
  }
</script>

<div class="histogram">
  <div class="graph">
    {#if data}
      <svg viewBox="0 0 {W} {H}" preserveAspectRatio="none" aria-label="Histogram" role="img">
        <path class="b" d={area(data.b)} />
        <path class="g" d={area(data.g)} />
        <path class="r" d={area(data.r)} />
      </svg>
    {/if}
    <button
      class={['clip', 'left', clip.shadow && 'on']}
      title="Shadow clipping  {registry.shortcut('develop.clipping')}"
      aria-pressed={clip.shadow}
      aria-label="Shadow clipping"
      onclick={() => onclip?.('shadow')}
    ></button>
    <button
      class={['clip', 'right', clip.highlight && 'on']}
      title="Highlight clipping  {registry.shortcut('develop.clipping')}"
      aria-pressed={clip.highlight}
      aria-label="Highlight clipping"
      onclick={() => onclip?.('highlight')}
    ></button>
  </div>
  <div class="exif mono">
    {#each exif as e, i (i)}<span>{e}</span>{/each}
  </div>
</div>

{#if !notools}
  <div class="tools" role="toolbar" aria-label="Tools">
    {#each tools as t (t.id)}
      {@const title = t.command ? registry.title(t.command) : t.label}
      <button
        class={['tool', t.id === tool && 'lens']}
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

  svg {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }

  .r {
    fill: var(--hist-red);
    opacity: 0.4;
  }

  .g {
    fill: var(--hist-green);
    opacity: 0.4;
  }

  .b {
    fill: var(--hist-blue);
    opacity: 0.45;
  }

  .clip {
    position: absolute;
    top: 5px;
    width: 0;
    height: 0;
    border-top: 9px solid var(--clip-off);
  }

  .clip.left {
    left: 5px;
    border-right: 9px solid transparent;
  }

  .clip.right {
    right: 5px;
    border-left: 9px solid transparent;
  }

  .clip.left.on {
    border-top-color: var(--clip-shadow);
  }

  .clip.right.on {
    border-top-color: var(--clip-highlight);
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
