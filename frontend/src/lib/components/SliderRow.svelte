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
    /**
     * `edit` changes the photo: a glass thumb on a recessed track, the
     * detent and fine adjust (DEV-02). `aux` changes a view or a tool: an
     * opaque pill with a neutral fill (the handoff's control decisions).
     */
    kind?: 'edit' | 'aux';
    /** A tick at 0 that a drag catches on: bipolar edit sliders by default (DEV-02). */
    detent?: boolean;
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
    /**
     * Alt goes down (true) or up (false) during a pointer drag: the tone
     * sliders' clipping preview (DEV-03). The drag's end ends it on its own.
     */
    onalt?: ((held: boolean) => void) | undefined;
    /** A right-click, or the context-menu key: the slider's menu (APP-06), with the value as shown. */
    oncontextmenu?: (e: MouseEvent, shown: string) => void;
  }

  let {
    label,
    kind = 'edit',
    detent,
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
    onalt,
    oncontextmenu,
  }: Props = $props();

  /** The detent catches within this share of the track's width (DEV-02). */
  const DETENT_REACH = 0.015;
  /** Shift during a drag moves at this share of the pointer's speed (DEV-02). */
  const FINE = 0.1;
  /** The thumb's way home after a reset, in ms (DEV-02). */
  const RESET_MS = 200;

  let trackEl: HTMLDivElement | undefined = $state();
  let handleEl: HTMLDivElement | undefined = $state();
  /**
   * A drag's anchor: the pointer's x and the thumb's place (0 to 1) when it
   * was set, and the speed from there (1, or `FINE` with Shift). The thumb
   * is at `t + (x − x0) / width × speed`, so pressing or releasing Shift
   * re-anchors and the thumb never jumps.
   */
  let anchor = { x: 0, t: 0, speed: 1 };
  /** The thumb's place during a drag, unclamped. */
  let dragT = 0;
  /** The drag holds at the detent. */
  let caught = $state(false);
  /** Shift is held during the drag: fine adjust. */
  let fine = $state(false);
  /** The thumb animates home after a reset. */
  let resetting = $state(false);
  let resetTimer: ReturnType<typeof setTimeout> | null = null;
  /** The last press was on the handle. The track captures the pointer, so a double-click lands there. */
  let onHandle = false;
  let fieldEl: HTMLInputElement | undefined = $state();
  let dragging = $state(false);
  /** An arrow key is held: one gesture until it is released. */
  let keyHeld = false;
  /** Alt is held during the drag, as last told to `onalt`. */
  let altHeld = false;
  let typing = $state(false);
  let typed = $state('');

  const toT = (v: number) => (scale ? scale.toT(v) : (v - min) / (max - min));
  const pos = $derived(Math.min(1, Math.max(0, toT(Math.min(max, Math.max(min, value))))));
  const origin = $derived(from === 'center' ? toT(0) : 0);
  const hasDetent = $derived(detent ?? (kind === 'edit' && min < 0 && max > 0));
  const zeroT = $derived(toT(0));
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

  /** The value at place `t` on the track. */
  function valueAtT(t: number): number {
    const c = Math.min(1, Math.max(0, t));
    return clampStep(scale ? scale.fromT(c) : min + c * (max - min));
  }

  /** The pointer's place on the track, 0 to 1, unclamped. */
  function pointerT(clientX: number): number {
    if (!trackEl) return pos;
    const r = trackEl.getBoundingClientRect();
    return r.width > 0 ? (clientX - r.left) / r.width : 0;
  }

  /** The thumb's place for the pointer at `clientX`, through the anchor, and the detent. */
  function dragTo(clientX: number): number {
    const width = trackEl?.getBoundingClientRect().width ?? 0;
    dragT = anchor.t + (width > 0 ? ((clientX - anchor.x) / width) * anchor.speed : 0);
    caught = hasDetent && !fine && Math.abs(dragT - zeroT) <= DETENT_REACH;
    return caught ? 0 : valueAtT(dragT);
  }

  /** Shift went down or up mid-drag: the speed changes from where the thumb is. */
  function setFine(held: boolean, clientX: number) {
    if (kind !== 'edit' || held === fine) return;
    fine = held;
    anchor = { x: clientX, t: caught ? zeroT : dragT, speed: held ? FINE : 1 };
  }

  function set(v: number): boolean {
    if (v === value) return false;
    value = v;
    return true;
  }

  function onpointerdown(e: PointerEvent) {
    if (disabled || e.button !== 0) return;
    dragging = true;
    stopReset();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    // A press on the track jumps there; one on the handle takes it as it is.
    onHandle = handleEl !== undefined && e.target === handleEl;
    // Shift held from the press: fine from the start, and no catch.
    fine = kind === 'edit' && e.shiftKey;
    anchor = { x: e.clientX, t: onHandle ? pos : pointerT(e.clientX), speed: fine ? FINE : 1 };
    dragT = anchor.t;
    if (!onHandle) set(dragTo(e.clientX));
    onstart?.(value);
    alt(e.altKey);
  }

  function onpointermove(e: PointerEvent) {
    if (!dragging) return;
    alt(e.altKey);
    setFine(e.shiftKey, e.clientX);
    if (set(dragTo(e.clientX))) oninput?.(value);
  }

  function onpointerup() {
    if (!dragging) return;
    dragging = false;
    altHeld = false;
    caught = false;
    fine = false;
    onchange?.(value);
  }

  function alt(held: boolean) {
    if (!onalt || held === altHeld) return;
    altHeld = held;
    onalt(held);
  }

  /** Alt pressed or released without moving the pointer; Shift is read on the next move. */
  function onwindowkey(e: KeyboardEvent) {
    if (!dragging || !onalt || e.key !== 'Alt') return;
    // Alt alone would move the focus to the window menu.
    e.preventDefault();
    alt(e.type === 'keydown');
  }

  function stopReset() {
    if (resetTimer !== null) clearTimeout(resetTimer);
    resetTimer = null;
    resetting = false;
  }

  function reset(e: MouseEvent) {
    e.stopPropagation();
    if (disabled) return;
    // The value goes at once; only the thumb takes its 200 ms home.
    stopReset();
    resetting = true;
    resetTimer = setTimeout(stopReset, RESET_MS);
    if (onreset) {
      onreset();
      return;
    }
    set(defaultValue);
    onchange?.(value);
  }

  function onkeydown(e: KeyboardEvent) {
    if (disabled || typing) return;
    // The context-menu key and Shift F10 open the slider's menu (APP-06),
    // at the slider: not every platform sends `contextmenu` for them.
    if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) {
      e.preventDefault();
      e.stopPropagation();
      (e.currentTarget as HTMLElement).dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true }));
      return;
    }
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

<svelte:window onkeydown={onwindowkey} onkeyup={onwindowkey} />

<div
  class={[
    'row',
    kind,
    changed && 'changed',
    (hot || dragging) && 'hot',
    dragging && 'held',
    fine && 'fine',
    resetting && 'resetting',
    disabled && 'disabled',
  ]}
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
      if (!disabled && !dragging && !typing) oncontextmenu(e, shown);
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
    ondblclick={(e) => onHandle && reset(e)}
  >
    {#if hasDetent}
      <div class={['detent', caught && 'caught']} style:left="{zeroT * 100}%"></div>
    {/if}
    {#if track === 'plain'}
      <div
        class="fill"
        style:left="{Math.min(origin, pos) * 100}%"
        style:width="{Math.abs(pos - origin) * 100}%"
      ></div>
    {/if}
    <div class="handle" bind:this={handleEl} style:left="{pos * 100}%">
      {#if kind === 'edit'}<span class="line"></span>{/if}
    </div>
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
    height: 4px;
    border-radius: 2px;
    background: var(--slider-track);
    box-shadow: var(--slider-track-inset);
    cursor: pointer;
    touch-action: none;
  }

  .row:hover .track.plain,
  .held .track.plain {
    background: var(--slider-track-hover);
  }

  /* A taller hit area than the track. */
  .track::before {
    content: '';
    position: absolute;
    inset: -8px 0;
  }

  /* Temp and Tint: their gradients, softer at rest (the handoff, 2026-11-05). */
  .track.temp {
    background: var(--track-temp);
  }

  .track.tint {
    background: var(--track-tint);
  }

  .track.temp,
  .track.tint {
    opacity: 0.72;
  }

  .row:hover .track.temp,
  .row:hover .track.tint,
  .held .track.temp,
  .held .track.tint {
    opacity: 1;
  }

  .fill {
    position: absolute;
    top: 0;
    height: 100%;
    border-radius: 2px;
    background: var(--slider-fill);
  }

  /* The detent: a tick at 0, lit while a drag holds there (DEV-02). */
  .detent {
    position: absolute;
    top: 50%;
    width: 1px;
    height: 8px;
    margin-left: -0.5px;
    transform: translateY(-50%);
    background: var(--slider-detent);
  }

  .detent.caught {
    background: var(--slider-detent-hit);
  }

  /* Edit: the glass thumb, the only glass here (CLAUDE.md: blur on slider handles only). */
  .handle {
    position: absolute;
    top: 50%;
    width: 22px;
    height: 14px;
    transform: translate(-50%, -50%);
    border-radius: var(--r-thumb-glass);
    background: var(--thumb-glass-bg);
    border: 1px solid var(--thumb-glass-border);
    box-shadow: var(--thumb-glass-shadow);
    backdrop-filter: blur(1.5px);
    display: grid;
    place-items: center;
    transition:
      width var(--t-lens) var(--spring),
      height var(--t-lens) var(--spring);
  }

  .row:hover .handle {
    border-color: var(--thumb-glass-border-hover);
  }

  .line {
    width: 1.5px;
    height: 8px;
    border-radius: 1px;
    background: var(--thumb-line);
  }

  .held .handle {
    width: 24px;
    height: 16px;
    background: var(--thumb-glass-bg-held);
    border-color: var(--thumb-glass-border-held);
  }

  .held .line {
    height: 10px;
  }

  .held .value {
    color: var(--accent-soft);
  }

  .fine .handle {
    width: 18px;
  }

  .fine .line {
    background: var(--accent-soft);
  }

  .row:focus-visible .handle {
    box-shadow:
      var(--thumb-glass-shadow),
      0 0 0 2px var(--bg),
      0 0 0 3.5px var(--accent);
  }

  /* A reset animates home; a drag never does (DEV-02). */
  .resetting .handle {
    transition: left 200ms var(--ease-out);
  }

  .resetting .fill {
    transition:
      left 200ms var(--ease-out),
      width 200ms var(--ease-out);
  }

  /* Aux: an opaque pill on a thinner track, a neutral fill, no detent. */
  .aux .track {
    height: 3px;
    background: var(--aux-track);
    box-shadow: none;
  }

  .aux .fill {
    background: var(--aux-fill);
  }

  .aux .handle {
    width: 16px;
    height: 11px;
    border-radius: var(--r-pill);
    background: var(--slider-handle-bg);
    border: 1px solid var(--slider-handle-border-color);
    box-shadow: var(--slider-handle-shadow), var(--slider-handle-ring);
    backdrop-filter: none;
  }

  .aux.row:focus-visible .handle {
    box-shadow:
      var(--slider-handle-shadow),
      0 0 0 2px var(--bg),
      0 0 0 3.5px var(--accent);
  }

  @media (prefers-reduced-motion: reduce) {
    .handle,
    .resetting .handle,
    .resetting .fill {
      transition: none;
    }
  }
</style>
