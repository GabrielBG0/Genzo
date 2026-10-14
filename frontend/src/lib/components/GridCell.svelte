<script lang="ts">
  import Thumb, { type RgbaImage } from './Thumb.svelte';

  export type Flag = 'pick' | 'reject' | null;
  export type ColorLabel = 'red' | 'yellow' | 'green' | 'blue' | 'purple' | null;

  interface Props {
    idx: number;
    stars?: number;
    flag?: Flag;
    label?: ColorLabel;
    selected?: boolean;
    /** Short text in the corner badge, e.g. "edited" or "⧉ 3" for a stack. */
    badge?: string | null;
    thumb?: RgbaImage | null;
  }
  let {
    idx,
    stars = 0,
    flag = null,
    label = null,
    selected = false,
    badge = null,
    thumb = null,
  }: Props = $props();
</script>

<div class={['cell', selected && 'selected', flag === 'reject' && 'rejected']} aria-selected={selected} role="gridcell">
  <div class="photo">
    <Thumb image={thumb} alt="Photo {idx}" />
    <span class="idx mono">{idx}</span>
    {#if badge}<span class="badge">{badge}</span>{/if}
  </div>
  <div class="meta">
    <span class="flag" aria-label={flag ?? 'unflagged'}>{flag === 'pick' ? '⚑' : flag === 'reject' ? '✕' : ''}</span>
    <span class="stars" aria-label="{stars} stars"
      >{'★'.repeat(stars)}<span class="off">{'★'.repeat(5 - stars)}</span></span
    >
    <span class="spacer"></span>
    <span class={['label', label]} aria-label={label ? `${label} label` : 'no label'}></span>
  </div>
</div>

<style>
  .cell {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 6px;
    border-radius: var(--r-pane);
    color: var(--text-body);
    font-size: var(--text-small);
    background: var(--cell-bg);
    border: 1px solid var(--cell-border);
  }

  .selected {
    background: var(--cell-selected-bg);
    border-color: var(--cell-selected-border-color);
    box-shadow: var(--cell-selected-shadow);
  }

  .rejected .photo {
    opacity: 0.35;
  }

  .photo {
    position: relative;
    aspect-ratio: 3 / 2;
    border-radius: var(--r-thumb);
    overflow: hidden;
  }

  .idx {
    position: absolute;
    left: 6px;
    top: 4px;
    font-size: 10px;
    color: var(--thumb-index);
  }

  .badge {
    position: absolute;
    right: 5px;
    top: 5px;
    padding: 0 6px;
    border-radius: var(--r-pill);
    background: var(--badge-bg);
    color: var(--text-on-lens);
    font-size: 10px;
    line-height: 16px;
  }

  .meta {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 16px;
  }

  .flag {
    width: 12px;
    color: var(--text-strong);
    font-size: var(--text-caption);
  }

  .stars {
    letter-spacing: 1px;
    color: var(--rating);
    font-size: var(--text-mono-small);
  }

  .off {
    color: var(--rating-empty);
  }

  .spacer {
    flex: 1;
  }

  .label {
    width: 9px;
    height: 9px;
    border-radius: 50%;
  }

  .label.red {
    background: var(--label-red);
  }

  .label.yellow {
    background: var(--label-yellow);
  }

  .label.green {
    background: var(--label-green);
  }

  .label.blue {
    background: var(--label-blue);
  }

  .label.purple {
    background: var(--label-purple);
  }
</style>
