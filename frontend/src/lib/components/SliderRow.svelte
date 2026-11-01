<script lang="ts" module>
  /** Maps a value to its place on the track, 0 to 1, and back. Linear by default. */
  export interface SliderScale {
    toT(v: number): number;
    fromT(t: number): number;
  }

  /**
   * Reads a typed value (DEV-02): a unit suffix is ignored and a comma is a
   * decimal point, so "5400K" and "+0,35" read. Null when it does not parse.
   */
  export function parseTyped(text: string): number | null {
    const m = /^\s*([+\-−]?)(\d+(?:[.,]\d*)?|[.,]\d+)\s*[a-zA-Z%°]*\s*$/.exec(text);
    if (!m) return null;
    const v = Number((m[2] ?? '').replace(',', '.'));
    return m[1] === '-' || m[1] === '−' ? -v : v;
  }
</script>

<script lang="ts">
  import { tick } from 'svelte';

  interface Props {
    label: string;
    value: number;
    min?: number;
    max?: number;
    defaultValue?: number;
    step?: number;
    /** The step with Shift held: ten steps unless given. */
    shiftStep?: number;
    /** Where the fill starts: the center for bipolar sliders, else the left. */
    from?: 'center' | 'left';
    /** Track style: plain, or a color gradient for Temp and Tint. */
    track?: 'plain' | 'temp' | 'tint';
    /** How values sit along the track (Temp is linear in mireds). */
    scale?: SliderScale | undefined;
    /** Forces the dim (unchanged) look, as in the handoff's `dim` prop. */
    dim?: boolean;
    /** Forces the hot (hovered or dragged) look, as in the handoff's `hot` prop. */
    hot?: boolean;
    disabled?: boolean;
    format?: (v: number) => string;
    /** A gesture starts: pointer down, or the first press of an arrow key. */
    onstart?: (v: number) => void;
    /** Every change while dragging or holding a key. The value shown is optimistic. */
    oninput?: (v: number) => void;
    /**
     * The end of a gesture (pointer up, key up), or a change that is a
     * gesture on its own: a typed value, or a reset without `onreset`.
     */
    onchange?: (v: number) => void;
    /** Double-click on the label or the handle. Without it, the slider goes to `defaultValue`. */
    onreset?: () => void;
    oncontextmenu?: (e: MouseEvent) => void;
  }

  let {
    label,
    value = $bindable(),
    min = -100,
    max = 100,
    defaultValue = 0,
    step = 1,
    shiftStep = step * 10,
    from = min < 0 && max > 0 ? 'center' : 'left',
    track = 'plain',
    scale,
    dim,
    hot = false,
    disabled = false,
    format = (v: number) => (Number.isInteger(step) ? String(Math.round(v)) : v.toFixed(2)),
    onstart,
    oninput,
    onchange,
    onreset,
    oncontextmenu,
  }: Props = $props();

  let trackEl: HTMLDivElement | undefined = $state();
  let fieldEl: HTMLInputElement | undefined = $state();
  let dragging = $state(false);
  /** An arrow key is held: one gesture until it is released. */
  let keyHeld = false;
  let typing = $state(false);
  let typed = $state('');

  const toT = (v: number) => (scale ? scale.toT(v) : (v - min) / (max - min));
  const pos = $derived(Math.min(1, Math.max(0, toT(Math.min(max, Math.max(min, value))))));
  const origin = $derived(from === 'center' ? toT(0) : 0);
  const changed = $derived(dim === undefined ? value !== defaultValue : !dim);
  const shown = $derived(formatSigned(value));

  function formatSigned(v: number): string {
    const s = format(v);
    return from === 'center' && Number(s) > 0 ? `+${s}` : s;
  }

  function clampStep(v: number): number {
    const snapped = Math.round((v - min) / step) * step + min;
    return Math.min(max, Math.max(min, Number(snapped.toFixed(6))));
  }

  function valueAt(clientX: number): number {
    if (!trackEl) return value;
    const r = trackEl.getBoundingClientRect();
    const t = Math.min(1, Math.max(0, r.width > 0 ? (clientX - r.left) / r.width : 0));
    return clampStep(scale ? scale.fromT(t) : min + t * (max - min));
  }

  function set(v: number): boolean {
    if (v === value) return false;
    value = v;
    return true;
  }

  function onpointerdown(e: PointerEvent) {
    if (disabled || e.button !== 0) return;
    dragging = true;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    set(valueAt(e.clientX));
    onstart?.(value);
  }

  function onpointermove(e: PointerEvent) {
    if (dragging && set(valueAt(e.clientX))) oninput?.(value);
  }

  function onpointerup() {
    if (!dragging) return;
    dragging = false;
    onchange?.(value);
  }

  function reset(e: MouseEvent) {
    e.stopPropagation();
    if (disabled) return;
    if (onreset) {
      onreset();
      return;
    }
    set(defaultValue);
    onchange?.(value);
  }

  function onkeydown(e: KeyboardEvent) {
    if (disabled || typing) return;
    const big = e.shiftKey ? shiftStep : step;
    const delta =
      e.key === 'ArrowRight' || e.key === 'ArrowUp'
        ? big
        : e.key === 'ArrowLeft' || e.key === 'ArrowDown'
          ? -big
          : 0;
    if (delta === 0) return;
    e.preventDefault();
    e.stopPropagation();
    const moved = set(clampStep(value + delta));
    if (!keyHeld) {
      keyHeld = true;
      onstart?.(value);
    } else if (moved) {
      oninput?.(value);
    }
  }

  function onkeyup(e: KeyboardEvent) {
    if (!keyHeld || !['ArrowRight', 'ArrowUp', 'ArrowLeft', 'ArrowDown'].includes(e.key)) return;
    keyHeld = false;
    onchange?.(value);
  }

  async function startTyping(e: MouseEvent) {
    e.stopPropagation();
    if (disabled) return;
    typed = shown;
    typing = true;
    await tick();
    fieldEl?.select();
  }

  function finishTyping(apply: boolean) {
    if (!typing) return;
    typing = false;
    if (!apply) return;
    const v = parseTyped(typed);
    // A value that does not parse is refused; the old one stays.
    if (v === null) return;
    set(Math.min(max, Math.max(min, v)));
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
  {onkeydown}
  {onkeyup}
  onblur={() => {
    if (keyHeld) {
      keyHeld = false;
      onchange?.(value);
    }
  }}
  oncontextmenu={(e) => {
    if (oncontextmenu) {
      e.preventDefault();
      oncontextmenu(e);
    }
  }}
>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <span class="label" ondblclick={reset}>{label}</span>
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
    <div class="handle" style:left="{pos * 100}%" ondblclick={reset}></div>
  </div>
  {#if typing}
    <input
      class="value field mono"
      aria-label="{label} value"
      bind:this={fieldEl}
      bind:value={typed}
      onkeydown={(e) => {
        e.stopPropagation();
        if (e.key === 'Enter') finishTyping(true);
        else if (e.key === 'Escape') finishTyping(false);
      }}
      onblur={() => finishTyping(false)}
    />
  {:else}
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <span class="value mono" onclick={startTyping}>{shown}</span>
  {/if}
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
    cursor: text;
  }

  .field {
    width: 100%;
    min-width: 0;
    padding: 0 2px;
    font: inherit;
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    color: var(--text-strong);
    background: var(--well);
    border: 1px solid var(--field);
    border-radius: 4px;
    outline: none;
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
  }
</style>
