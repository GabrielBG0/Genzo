<script lang="ts">
  import {
    clampAngle,
    corners,
    fromLocal,
    gripAt,
    handles,
    moved,
    overlayLines,
    ratioOf,
    resized,
    straightenAngle,
    turnedBy,
    type Frame,
    type Grip,
    type Point,
  } from '../develop/crop';
  import type { CropTool } from '../develop/cropTool.svelte';
  import type { ZoomState } from '../develop/zoom.svelte';

  interface Props {
    tool: CropTool;
    /** The view's size, in device pixels. */
    zoom: ZoomState;
  }
  let { tool, zoom }: Props = $props();

  /** How near a handle a press grabs it, in CSS pixels. */
  const REACH = 10;

  /**
   * The whole oriented photo at Fit in the view (the tool shows it so):
   * device pixels per photo pixel, and its top left in photo pixels.
   */
  const place = $derived.by(() => {
    const s = tool.size;
    const v = zoom.view;
    if (s.width <= 0 || v.width <= 0) return null;
    const z = Math.min(v.width / s.width, v.height / s.height);
    return { z, ox: (s.width - v.width / z) / 2, oy: (s.height - v.height / z) / 2 };
  });
  const dpr = typeof devicePixelRatio === 'number' ? devicePixelRatio : 1;

  /** A photo point on screen, in CSS pixels. */
  function screen(p: Point): Point {
    if (!place) return { x: 0, y: 0 };
    return { x: ((p.x - place.ox) * place.z) / dpr, y: ((p.y - place.oy) * place.z) / dpr };
  }

  /** A pointer on the photo, in its pixels. */
  function photoAt(e: PointerEvent, el: Element): Point {
    const r = el.getBoundingClientRect();
    const z = place?.z ?? 1;
    return { x: (place?.ox ?? 0) + ((e.clientX - r.left) * dpr) / z, y: (place?.oy ?? 0) + ((e.clientY - r.top) * dpr) / z };
  }

  const frame = $derived(tool.frame);
  const poly = $derived(frame ? corners(frame).map(screen) : []);
  const points = $derived(poly.map((p) => `${p.x},${p.y}`).join(' '));
  const lines = $derived.by(() => {
    if (!frame) return [];
    const { x, y } = overlayLines(tool.overlay);
    const at = (u: number, v: number) => screen(fromLocal(frame, { x: (u - 0.5) * frame.w, y: (v - 0.5) * frame.h }));
    return [...x.map((u) => [at(u, 0), at(u, 1)]), ...y.map((v) => [at(0, v), at(1, v)])];
  });
  const grips = $derived(frame ? handles(frame).map((h) => ({ grip: h.grip, at: screen(h.at) })) : []);

  /** A drag: what it grabbed, the frame and point it started from, and whether it has sent its start. */
  let drag: { grip: Grip; start: Frame; from: Point; started: boolean } | null = null;
  /** The straighten tool's line, on screen. */
  let line: { a: Point; b: Point; from: Point } | null = $state(null);
  /** The grip under the pointer, for the cursor. */
  let hover: Grip = $state('outside');

  function onpointerdown(e: PointerEvent) {
    if (e.button !== 0 || !frame) return;
    e.stopPropagation();
    const el = e.currentTarget as Element;
    el.setPointerCapture(e.pointerId);
    const p = photoAt(e, el);
    if (tool.straightening) {
      const s = screen(p);
      line = { a: s, b: s, from: p };
      return;
    }
    const reach = (REACH * dpr) / (place?.z ?? 1);
    drag = { grip: gripAt(frame, p, reach), start: frame, from: p, started: false };
  }

  function onpointermove(e: PointerEvent) {
    const el = e.currentTarget as Element;
    const p = photoAt(e, el);
    if (line) {
      line = { ...line, b: screen(p) };
      return;
    }
    if (!drag) {
      if (frame) hover = gripAt(frame, p, (REACH * dpr) / (place?.z ?? 1));
      return;
    }
    const d = drag;
    let next: Frame;
    if (d.grip === 'inside') next = moved(d.start, { x: p.x - d.from.x, y: p.y - d.from.y }, tool.size);
    else if (d.grip === 'outside') {
      const deg = clampAngle((d.start.a * 180) / Math.PI + turnedBy(d.start, d.from, p));
      next = { ...d.start, a: (deg * Math.PI) / 180 };
    } else next = resized(d.start, d.grip, p, tool.locked ? ratioOf(d.start) : null);
    tool.sendFrame(next, d.started ? 'Update' : 'Start');
    d.started = true;
  }

  function onpointerup(e: PointerEvent) {
    const el = e.currentTarget as Element;
    if (line) {
      const p = photoAt(e, el);
      const long = Math.hypot(p.x - line.from.x, p.y - line.from.y) * ((place?.z ?? 1) / dpr) > 8;
      if (long) tool.setAngle(straightenAngle(line.from, p), 'End');
      line = null;
      tool.straightening = false;
      return;
    }
    const d = drag;
    drag = null;
    if (d?.started && frame) tool.sendFrame(frame, 'End');
  }

  const CURSORS: Record<Grip, string> = {
    nw: 'nwse-resize',
    se: 'nwse-resize',
    ne: 'nesw-resize',
    sw: 'nesw-resize',
    n: 'ns-resize',
    s: 'ns-resize',
    e: 'ew-resize',
    w: 'ew-resize',
    inside: 'move',
    outside: 'alias',
  };
</script>

<!-- The crop frame (DEV-12): drags resize, move or turn it; the panel and keys do the rest. -->
<svg
  class="crop-overlay"
  role="application"
  aria-label="Crop frame"
  style:cursor={tool.straightening ? 'crosshair' : CURSORS[hover]}
  {onpointerdown}
  {onpointermove}
  {onpointerup}
  onpointercancel={() => {
    drag = null;
    line = null;
  }}
>
  {#if frame && place}
    <path class="dim" fill-rule="evenodd" d="M-1,-1 H100000 V100000 H-1 Z M{points.replaceAll(' ', ' L')} Z" />
    {#each lines as [a, b], i (i)}
      <line class="guide" x1={a?.x} y1={a?.y} x2={b?.x} y2={b?.y} />
    {/each}
    <polygon class="frame" {points} />
    {#each grips as g (g.grip)}
      <rect class="handle" x={g.at.x - 4.5} y={g.at.y - 4.5} width="9" height="9" data-grip={g.grip} />
    {/each}
    {#if line}
      <line class="straighten" x1={line.a.x} y1={line.a.y} x2={line.b.x} y2={line.b.y} />
    {/if}
  {/if}
</svg>

<style>
  .crop-overlay {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    z-index: 1;
    touch-action: none;
  }

  .dim {
    fill: var(--crop-dim);
  }

  .frame {
    fill: none;
    stroke: var(--text-strong);
    stroke-width: 1;
  }

  .guide {
    stroke: var(--text-strong);
    stroke-opacity: 0.5;
    stroke-width: 1;
  }

  .handle {
    fill: var(--text-strong);
    stroke: var(--bg);
    stroke-width: 1;
  }

  .straighten {
    stroke: var(--accent);
    stroke-width: 1.5;
  }
</style>
