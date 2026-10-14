<script lang="ts">
  interface Props {
    label: string;
    value: number;
    min?: number;
    max?: number;
    defaultValue?: number;
    step?: number;
    /** Where the fill starts: the center for bipolar sliders, else the left. */
    from?: 'center' | 'left';
    /** Track style: plain, or a color gradient for Temp and Tint. */
    track?: 'plain' | 'temp' | 'tint';
    /** Forces the dim (unchanged) look, as in the handoff's `dim` prop. */
    dim?: boolean;
    /** Forces the hot (hovered or dragged) look, as in the handoff's `hot` prop. */
    hot?: boolean;
    disabled?: boolean;
    format?: (v: number) => string;
    /** Every change while dragging. The value shown is optimistic. */
    oninput?: (v: number) => void;
    /** The end of a gesture: pointer up, a key press or a reset. */
    onchange?: (v: number) => void;
    oncontextmenu?: (e: MouseEvent) => void;
  }

  let {
    label,
    value = $bindable(),
    min = -100,
    max = 100,
    defaultValue = 0,
    step = 1,
    from = min < 0 && max > 0 ? 'center' : 'left',
    track = 'plain',
    dim,
    hot = false,
    disabled = false,
    format = (v: number) => (Number.isInteger(step) ? String(Math.round(v)) : v.toFixed(2)),
    oninput,
    onchange,
    oncontextmenu,
  }: Props = $props();

  let trackEl: HTMLDivElement | undefined = $state();
  let dragging = $state(false);

  const pos = $derived((Math.min(max, Math.max(min, value)) - min) / (max - min));
  const origin = $derived(from === 'center' ? (0 - min) / (max - min) : 0);
  const changed = $derived(dim === undefined ? value !== defaultValue : !dim);
  const shown = $derived(formatSigned(value));

  function formatSigned(v: number): string {
    const s = format(v);
    return from === 'center' && v > 0 ? `+${s}` : s;
  }

  function clampStep(v: number): number {
    const snapped = Math.round((v - min) / step) * step + min;
    return Math.min(max, Math.max(min, Number(snapped.toFixed(6))));
  }

  function valueAt(clientX: number): number {
    if (!trackEl) return value;
    const r = trackEl.getBoundingClientRect();
    const t = r.width > 0 ? (clientX - r.left) / r.width : 0;
    return clampStep(min + Math.min(1, Math.max(0, t)) * (max - min));
  }

  function set(v: number) {
    if (v === value) return;
    value = v;
    oninput?.(v);
  }

  function onpointerdown(e: PointerEvent) {
    if (disabled || e.button !== 0) return;
    dragging = true;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    set(valueAt(e.clientX));
  }

  function onpointermove(e: PointerEvent) {
    if (dragging) set(valueAt(e.clientX));
  }

  function onpointerup() {
    if (!dragging) return;
    dragging = false;
    onchange?.(value);
  }

  function reset() {
    if (disabled) return;
    set(defaultValue);
    onchange?.(value);
  }

  function onkeydown(e: KeyboardEvent) {
    if (disabled) return;
    const big = e.shiftKey ? 10 : 1;
    const delta =
      e.key === 'ArrowRight' || e.key === 'ArrowUp'
        ? step * big
        : e.key === 'ArrowLeft' || e.key === 'ArrowDown'
          ? -step * big
          : 0;
    if (delta === 0) return;
    e.preventDefault();
    e.stopPropagation();
    set(clampStep(value + delta));
    onchange?.(value);
  }
</script>

<div
  class={['row', changed && 'changed', (hot || dragging) && 'hot', disabled && 'disabled']}
  role="slider"
  tabindex={disabled ? -1 : 0}
  aria-label={label}
  aria-valuemin={min}
  aria-valuemax={max}
  aria-valuenow={value}
  aria-valuetext={shown}
  aria-disabled={disabled}
  ondblclick={reset}
  {onkeydown}
  oncontextmenu={(e) => {
    if (oncontextmenu) {
      e.preventDefault();
      oncontextmenu(e);
    }
  }}
>
  <span class="label">{label}</span>
  <!-- The slider role and keyboard handling sit on the row; the track only takes the pointer. -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class={['track', track]}
    bind:this={trackEl}
    {onpointerdown}
    {onpointermove}
    {onpointerup}
    onpointercancel={onpointerup}
  >
    {#if track === 'plain'}
      <div
        class="fill"
        style:left="{Math.min(origin, pos) * 100}%"
        style:width="{Math.abs(pos - origin) * 100}%"
      ></div>
    {/if}
    <div class="handle" style:left="{pos * 100}%"></div>
  </div>
  <span class="value mono">{shown}</span>
</div>

<style>
  .row {
    display: grid;
    grid-template-columns: var(--slider-label-w) minmax(0, 1fr) var(--slider-value-w);
    gap: 10px;
    align-items: center;
    height: var(--row-slider);
    padding: 0 12px 0 26px;
    font-size: var(--text-base);
    color: var(--text-muted);
    outline: none;
  }

  .row:focus-visible .handle {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }

  .label {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .value {
    font-size: var(--text-mono);
    text-align: right;
    color: var(--text-dim);
  }

  .changed .label,
  .changed .value,
  .hot .label {
    color: var(--text-strong);
  }

  .disabled {
    opacity: 0.5;
  }

  .track {
    position: relative;
    height: 3px;
    border-radius: 2px;
    background: var(--slider-track);
    cursor: pointer;
    touch-action: none;
  }

  /* A taller hit area than the 3 px track. */
  .track::before {
    content: '';
    position: absolute;
    inset: -8px 0;
  }

  .track.temp {
    background: var(--track-temp);
  }

  .track.tint {
    background: var(--track-tint);
  }

  .fill {
    position: absolute;
    top: 0;
    height: 3px;
    border-radius: 2px;
    background: var(--slider-fill);
  }

  .handle {
    position: absolute;
    top: -5px;
    width: 18px;
    height: 13px;
    margin-left: -9px;
    border-radius: var(--r-pill);
    background: var(--slider-handle-bg);
    border: 1px solid var(--slider-handle-border-color);
    box-shadow: var(--slider-handle-shadow);
    pointer-events: none;
  }
</style>
