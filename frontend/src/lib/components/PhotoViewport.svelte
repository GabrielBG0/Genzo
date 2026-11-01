<script lang="ts">
  import type { Engine } from '../engine/types';
  import { putFrame } from '../viewport/frame';
  import type { Viewport, Zoom } from '../viewport/types';
  import { WebGLViewport } from '../viewport/webgl';

  interface Props {
    /** Where frames come from. Without one, the empty placeholder shows. */
    engine?: Engine | null;
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
  }
  let {
    engine = null,
    zoom = 'fit',
    ondraw,
    onerror,
    onresize,
    picking = false,
    onpick,
    empty = 'No photo open',
    photo,
  }: Props = $props();

  let host: HTMLDivElement | undefined = $state();
  let canvas: HTMLCanvasElement | undefined = $state();
  let viewport: Viewport | null = null;
  let hasFrame = $state(false);
  let error = $state<string | null>(null);
  let pan = { x: 0, y: 0 };
  let dragFrom: { x: number; y: number } | null = null;

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
      if (event.type !== 'renderFinished' || !viewport) return;
      if (event.generation < viewport.generation) return;
      if (photo !== undefined && event.photo !== photo) return;
      const bytes = await eng.fetchFrame(event.generation);
      if (!viewport) return;
      if (await putFrame(viewport, bytes)) {
        hasFrame = true;
        schedule(event.generation);
      }
    });
  });

  $effect(() => {
    void photo;
    hasFrame = false;
  });

  $effect(() => {
    if (zoom === 'fit') pan = { x: 0, y: 0 };
    viewport?.setTransform({ zoom, panX: pan.x, panY: pan.y });
    schedule();
  });

  function onclick(e: MouseEvent) {
    if (!picking || !host) return;
    const r = host.getBoundingClientRect();
    const point = viewport?.imagePoint(e.clientX - r.left, e.clientY - r.top);
    if (point) onpick?.(point[0], point[1]);
  }

  function onpointerdown(e: PointerEvent) {
    if (zoom === 'fit' || picking) return;
    dragFrom = { x: e.clientX - pan.x, y: e.clientY - pan.y };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function onpointermove(e: PointerEvent) {
    if (!dragFrom) return;
    pan = { x: e.clientX - dragFrom.x, y: e.clientY - dragFrom.y };
    viewport?.setTransform({ zoom, panX: pan.x, panY: pan.y });
    schedule();
  }
</script>

<!-- The picker's click is pointer-only; W and Esc work from the keyboard. -->
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<div
  class={['viewport', picking && 'picking']}
  bind:this={host}
  role="img"
  aria-label="Photo"
  {onclick}
  {onpointerdown}
  {onpointermove}
  onpointerup={() => (dragFrom = null)}
>
  <canvas bind:this={canvas} class={[!hasFrame && 'hidden']}></canvas>
  {#if error}
    <div class="placeholder">Cannot draw photos: {error}</div>
  {:else if !hasFrame}
    <div class="placeholder">{empty}</div>
  {/if}
</div>

<style>
  .viewport {
    position: relative;
    width: 100%;
    height: 100%;
    overflow: hidden;
    border-radius: var(--r-photo);
    background: var(--photo-backdrop);
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

  .placeholder {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: var(--text-muted);
  }
</style>
