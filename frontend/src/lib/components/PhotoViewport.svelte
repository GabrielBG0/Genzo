<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { ZoomState } from '../develop/zoom.svelte';
  import type { Engine } from '../engine/types';
  import { putFrame } from '../viewport/frame';
  import type { Viewport, Zoom } from '../viewport/types';
  import { WebGLViewport } from '../viewport/webgl';

  interface Props {
    /** Where frames come from. Without one, the empty placeholder shows. */
    engine?: Engine | null;
    /**
     * Develop's zoom and pan (DEV-01 viewport spec): a click zooms, a drag
     * and the wheel pan at 1:1, and the transform follows it. Without one,
     * `zoom` places the frame (the spike).
     */
    view?: ZoomState | null;
    /** A click toggles the zoom: false for a photo that does not render (it stays at Fit). */
    zoomable?: boolean;
    zoom?: Zoom;
    /** Called after a generation's pixels reached the screen. */
    ondraw?: (generation: number, at: number) => void;
    /** Called when the viewport cannot draw at all (no WebGL2). */
    onerror?: (message: string) => void;
    /** The drawing area's size in device pixels, when it changes. */
    onresize?: (width: number, height: number) => void;
    /** The white balance picker is on: a crosshair, and clicks pick. */
    picking?: boolean;
    /** A click on the photo while picking, in the shown image's normalized coordinates. */
    onpick?: (x: number, y: number) => void;
    /** What to say when there is no frame. */
    empty?: string;
    /**
     * The photo shown: frames of other photos are not drawn, and a switch
     * clears the view until the new photo's first frame. Left out, every
     * frame is drawn (the spike).
     */
    photo?: number | null;
    /** A badge on the photo, top left, such as "Before" (WF-02). */
    badge?: string | null;
    /** Over the photo, once it has a frame: the photo toolbar. */
    children?: Snippet;
  }
  let {
    engine = null,
    view = null,
    zoomable = true,
    zoom = 'fit',
    ondraw,
    onerror,
    onresize,
    picking = false,
    onpick,
    empty = 'No photo open',
    photo,
    badge = null,
    children,
  }: Props = $props();

  /** A press that moves less than this, in CSS pixels, is a click. */
  const CLICK_SLOP = 3;

  let host: HTMLDivElement | undefined = $state();
  let canvas: HTMLCanvasElement | undefined = $state();
  let viewport: Viewport | null = null;
  let hasFrame = $state(false);
  let error = $state<string | null>(null);
  let pan = { x: 0, y: 0 };
  /** A press on the photo: where it started, where it is, and whether it moved enough to pan. */
  let press: { x: number; y: number; lastX: number; lastY: number; moved: boolean } | null = $state(null);

  function schedule(generation?: number) {
    requestAnimationFrame(() => {
      viewport?.draw();
      if (generation !== undefined) ondraw?.(generation, performance.now());
    });
  }

  $effect(() => {
    if (!canvas || !host) return;
    try {
      viewport = new WebGLViewport(canvas);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      onerror?.(error);
      return;
    }
    const vp = viewport;
    const ro = new ResizeObserver(([entry]) => {
      if (!entry) return;
      vp.resize(entry.contentRect.width, entry.contentRect.height, devicePixelRatio);
      onresize?.(entry.contentRect.width * devicePixelRatio, entry.contentRect.height * devicePixelRatio);
      schedule();
    });
    ro.observe(host);
    return () => {
      ro.disconnect();
      vp.destroy();
      viewport = null;
    };
  });

  $effect(() => {
    const eng = engine;
    if (!eng) return;
    return eng.onEvent(async (event) => {
      // A render's frame, or a photo's embedded preview while it opens (DEV-01).
      const generation =
        event.type === 'renderFinished' ? event.generation : event.type === 'previewShown' ? event.generation : null;
      if (generation === null || !viewport) return;
      if (generation < viewport.generation) return;
      if (photo !== undefined && 'photo' in event && event.photo !== photo) return;
      const bytes = await eng.fetchFrame(generation);
      if (!viewport) return;
      if (await putFrame(viewport, bytes)) {
        hasFrame = true;
        schedule(generation);
      }
    });
  });

  $effect(() => {
    void photo;
    hasFrame = false;
    viewport?.clear();
  });

  // The transform: Develop's zoom state, or the spike's zoom and pan.
  $effect(() => {
    if (view) {
      // At Fit the held frame is centered as it comes: before an edit reply
      // names the new photo's size, the old size would stretch it.
      viewport?.setTransform(view.level === 'fit' ? { zoom: 'fit' } : { zoom: 1, origin: view.origin, shown: view.shown });
    } else {
      if (zoom === 'fit') pan = { x: 0, y: 0 };
      viewport?.setTransform({ zoom, panX: pan.x, panY: pan.y });
    }
    schedule();
  });

  /** `e`'s position in device pixels from the view's top left. */
  function devicePoint(e: MouseEvent): { x: number; y: number } | null {
    if (!host) return null;
    const r = host.getBoundingClientRect();
    return { x: (e.clientX - r.left) * devicePixelRatio, y: (e.clientY - r.top) * devicePixelRatio };
  }

  function onpointerdown(e: PointerEvent) {
    // The toolbar's buttons over the photo keep their clicks.
    if (e.button !== 0 || (e.target instanceof Element && e.target.closest('button, [role="toolbar"]'))) return;
    press = { x: e.clientX, y: e.clientY, lastX: e.clientX, lastY: e.clientY, moved: false };
    if (!view && zoom !== 'fit' && !picking) pan = { x: pan.x - e.clientX, y: pan.y - e.clientY };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function onpointermove(e: PointerEvent) {
    if (view) view.pointer = devicePoint(e);
    if (!press) return;
    if (!press.moved && Math.hypot(e.clientX - press.x, e.clientY - press.y) > CLICK_SLOP) press.moved = true;
    if (!press.moved) return;
    if (view) {
      view.pan((e.clientX - press.lastX) * devicePixelRatio, (e.clientY - press.lastY) * devicePixelRatio);
    } else if (zoom !== 'fit' && !picking) {
      viewport?.setTransform({ zoom, panX: pan.x + e.clientX, panY: pan.y + e.clientY });
      schedule();
    }
    press.lastX = e.clientX;
    press.lastY = e.clientY;
  }

  function onpointerup(e: PointerEvent) {
    const p = press;
    press = null;
    if (!p) return;
    if (!view && zoom !== 'fit' && !picking) pan = { x: pan.x + e.clientX, y: pan.y + e.clientY };
    if (p.moved) return;
    // A click: the picker picks; otherwise Develop's zoom toggles at the point.
    const at = devicePoint(e);
    if (picking) {
      const point = view ? at && view.normalizedAt(at) : at && viewport?.imagePoint(at.x / devicePixelRatio, at.y / devicePixelRatio);
      if (point) onpick?.(point[0], point[1]);
    } else if (view && hasFrame && zoomable) {
      view.toggle(at);
    }
  }

  function onwheel(e: WheelEvent) {
    if (!view || view.level !== '1:1') return;
    e.preventDefault();
    // Lines and pages as pixels; Shift turns a plain wheel sideways.
    const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? 400 : 1;
    const [dx, dy] = e.shiftKey && e.deltaX === 0 ? [e.deltaY, 0] : [e.deltaX, e.deltaY];
    view.pan(-dx * unit * devicePixelRatio, -dy * unit * devicePixelRatio);
  }

  const panning = $derived(view?.level === '1:1' && !picking);
</script>

<!-- A click zooms or picks; Z, Space and W do the same from the keyboard. -->
<div
  class={['viewport', picking && 'picking', panning && 'pan', panning && press?.moved && 'grabbing', view && zoomable && !picking && view.level === 'fit' && hasFrame && 'zoomable']}
  bind:this={host}
  role="img"
  aria-label="Photo"
  {onpointerdown}
  {onpointermove}
  {onpointerup}
  onpointercancel={() => (press = null)}
  onpointerleave={() => view && (view.pointer = null)}
  {onwheel}
>
  <canvas bind:this={canvas} class={[!hasFrame && 'hidden']}></canvas>
  {#if error}
    <div class="placeholder">Cannot draw photos: {error}</div>
  {:else if !hasFrame}
    <div class="placeholder">{empty}</div>
  {/if}
  {#if badge && hasFrame}<span class="badge" role="status">{badge}</span>{/if}
  {#if hasFrame}{@render children?.()}{/if}
</div>

<style>
  .viewport {
    position: relative;
    width: 100%;
    height: 100%;
    overflow: hidden;
    border-radius: var(--r-photo);
    background: var(--photo-backdrop);
    touch-action: none;
  }

  .zoomable {
    cursor: zoom-in;
  }

  .pan {
    cursor: grab;
  }

  .grabbing {
    cursor: grabbing;
  }

  .picking {
    cursor: crosshair;
  }

  .hidden {
    visibility: hidden;
  }

  canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    display: block;
  }

  /* The flat Overlay surface: no blur over the photo. */
  .badge {
    position: absolute;
    top: 10px;
    left: 10px;
    padding: 2px 9px;
    border-radius: var(--r-pill);
    background: var(--overlay-bg);
    border: 1px solid var(--overlay-border-color);
    color: var(--text-body);
    font-size: var(--text-small);
    font-weight: 500;
    pointer-events: none;
  }

  .placeholder {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: var(--text-muted);
  }
</style>
