import type { Crop } from '$contract/genzo';

/**
 * The crop tool's geometry (DEV-12), in the oriented photo's pixels: the
 * crop is DEV-01's (center and size normalized to the oriented photo, the
 * angle in degrees, clockwise, about its center in pixel space). The
 * engine fits whatever it gets (`geometry::fit`); these keep the tool's
 * requests sensible, so a move does not shrink the crop.
 */

export interface Point {
  x: number;
  y: number;
}

/** The oriented photo's size in pixels. */
export interface Size {
  width: number;
  height: number;
}

/** A handle of the frame, or a drag inside or outside it. */
export type Grip = 'nw' | 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w' | 'inside' | 'outside';

export const MAX_ANGLE = 45;
/** The smallest crop, in pixels on each side (DEV-01). */
export const MIN_CROP = 32;

/** The crop's frame in pixels: its center, size, and the angle in radians. */
export interface Frame {
  cx: number;
  cy: number;
  w: number;
  h: number;
  /** Radians, clockwise. */
  a: number;
}

export function frameOf(c: Crop, s: Size): Frame {
  return { cx: c.cx * s.width, cy: c.cy * s.height, w: c.w * s.width, h: c.h * s.height, a: (c.angle * Math.PI) / 180 };
}

export function cropOf(f: Frame, s: Size): Crop {
  return {
    cx: f.cx / s.width,
    cy: f.cy / s.height,
    w: Math.min(1, f.w / s.width),
    h: Math.min(1, f.h / s.height),
    angle: (f.a * 180) / Math.PI,
  };
}

/** A point in the frame's own axes, from its center, to the photo's pixels. */
export function fromLocal(f: Frame, p: Point): Point {
  const [sin, cos] = [Math.sin(f.a), Math.cos(f.a)];
  return { x: f.cx + cos * p.x - sin * p.y, y: f.cy + sin * p.x + cos * p.y };
}

/** A point of the photo, in pixels, in the frame's own axes from its center. */
export function toLocal(f: Frame, p: Point): Point {
  const [sin, cos] = [Math.sin(f.a), Math.cos(f.a)];
  const [dx, dy] = [p.x - f.cx, p.y - f.cy];
  return { x: cos * dx + sin * dy, y: -sin * dx + cos * dy };
}

/** The frame's corners in the photo's pixels: top left, top right, bottom right, bottom left. */
export function corners(f: Frame): Point[] {
  return [
    [-1, -1],
    [1, -1],
    [1, 1],
    [-1, 1],
  ].map(([u = 0, v = 0]) => fromLocal(f, { x: (u * f.w) / 2, y: (v * f.h) / 2 }));
}

/** The eight handles, at the frame's corners and edge midpoints, in the photo's pixels. */
export function handles(f: Frame): { grip: Grip; at: Point }[] {
  const at: [Grip, number, number][] = [
    ['nw', -1, -1],
    ['n', 0, -1],
    ['ne', 1, -1],
    ['e', 1, 0],
    ['se', 1, 1],
    ['s', 0, 1],
    ['sw', -1, 1],
    ['w', -1, 0],
  ];
  return at.map(([grip, u, v]) => ({ grip, at: fromLocal(f, { x: (u * f.w) / 2, y: (v * f.h) / 2 }) }));
}

/** What a press at `p` grabs: a handle within `reach` pixels, the inside, or the outside. */
export function gripAt(f: Frame, p: Point, reach: number): Grip {
  for (const h of handles(f)) if (Math.hypot(h.at.x - p.x, h.at.y - p.y) <= reach) return h.grip;
  const l = toLocal(f, p);
  return Math.abs(l.x) <= f.w / 2 && Math.abs(l.y) <= f.h / 2 ? 'inside' : 'outside';
}

/** The half extents, along the photo's axes, of a frame `w` × `h` at angle `a`. */
function extents(w: number, h: number, a: number): Point {
  const [sin, cos] = [Math.abs(Math.sin(a)), Math.abs(Math.cos(a))];
  return { x: (cos * w + sin * h) / 2, y: (sin * w + cos * h) / 2 };
}

/** The frame moved so it stays inside the photo, its size kept. */
export function keepInside(f: Frame, s: Size): Frame {
  const e = extents(f.w, f.h, f.a);
  const clamp = (c: number, half: number, side: number) => (half * 2 >= side ? side / 2 : Math.min(Math.max(c, half), side - half));
  return { ...f, cx: clamp(f.cx, e.x, s.width), cy: clamp(f.cy, e.y, s.height) };
}

/** The frame moved by `d` pixels of the photo, kept inside. */
export function moved(f: Frame, d: Point, s: Size): Frame {
  return keepInside({ ...f, cx: f.cx + d.x, cy: f.cy + d.y }, s);
}

/**
 * The frame after dragging `grip` to the photo point `p`: the opposite
 * corner or edge stays. With `ratio` (width over height), a corner keeps
 * it, and an edge resizes the other side about the center.
 */
export function resized(f: Frame, grip: Grip, p: Point, ratio: number | null): Frame {
  if (grip === 'inside' || grip === 'outside') return f;
  const l = toLocal(f, p);
  // The frame's edges in its own axes.
  let [left, right, top, bottom] = [-f.w / 2, f.w / 2, -f.h / 2, f.h / 2];
  const west = grip.includes('w');
  const east = grip.includes('e');
  const north = grip.startsWith('n');
  const south = grip.startsWith('s');
  if (west) left = Math.min(l.x, right - MIN_CROP);
  if (east) right = Math.max(l.x, left + MIN_CROP);
  if (north) top = Math.min(l.y, bottom - MIN_CROP);
  if (south) bottom = Math.max(l.y, top + MIN_CROP);
  let [w, h] = [right - left, bottom - top];
  if (ratio !== null) {
    if ((west || east) && (north || south)) {
      // A corner: the larger of the two sizes the pointer asks for wins.
      if (w / h > ratio) h = w / ratio;
      else w = h * ratio;
      if (west) left = right - w;
      else right = left + w;
      if (north) top = bottom - h;
      else bottom = top + h;
    } else if (west || east) {
      h = w / ratio;
      [top, bottom] = [-h / 2, h / 2];
    } else {
      w = h * ratio;
      [left, right] = [-w / 2, w / 2];
    }
  }
  const center = fromLocal(f, { x: (left + right) / 2, y: (top + bottom) / 2 });
  return { cx: center.x, cy: center.y, w: right - left, h: bottom - top, a: f.a };
}

/** The angle, in degrees, of the pointer at `p` around the frame's center, from where the drag began at `from`. */
export function turnedBy(f: Frame, from: Point, p: Point): number {
  const a0 = Math.atan2(from.y - f.cy, from.x - f.cx);
  const a1 = Math.atan2(p.y - f.cy, p.x - f.cx);
  let d = ((a1 - a0) * 180) / Math.PI;
  if (d > 180) d -= 360;
  if (d < -180) d += 360;
  return d;
}

/** Degrees clamped to the straighten range, rounded to the slider's 0.1° step. */
export function clampAngle(deg: number): number {
  return Math.round(Math.min(MAX_ANGLE, Math.max(-MAX_ANGLE, deg)) * 10) / 10;
}

/**
 * The largest frame of `ratio` (width over height) about `f`'s center at
 * its angle, inside the photo.
 */
export function withRatio(f: Frame, ratio: number, s: Size): Frame {
  const [sin, cos] = [Math.abs(Math.sin(f.a)), Math.abs(Math.cos(f.a))];
  // Height h, width ratio·h: its half extents must reach no edge.
  const hx = (cos * ratio + sin) / 2;
  const hy = (sin * ratio + cos) / 2;
  const room = { x: Math.min(f.cx, s.width - f.cx), y: Math.min(f.cy, s.height - f.cy) };
  const h = Math.max(MIN_CROP, Math.min(room.x / hx, room.y / hy));
  return { ...f, w: h * ratio, h };
}

/** The frame's ratio, width over height. */
export function ratioOf(f: Frame): number {
  return f.w / f.h;
}

/**
 * A preset's ratio for this frame: landscape or portrait as the frame is
 * (a square frame takes the ratio as written).
 */
export function oriented(ratio: number, f: Frame): number {
  const wide = f.w >= f.h;
  return wide === ratio >= 1 ? ratio : 1 / ratio;
}

/**
 * The straighten tool: the angle, in degrees, that makes the line from `a`
 * to `b` level, or plumb when it is closer to vertical.
 */
export function straightenAngle(a: Point, b: Point): number {
  let deg = (Math.atan2(b.y - a.y, b.x - a.x) * 180) / Math.PI;
  // A line has no direction: within ±90°.
  if (deg > 90) deg -= 180;
  if (deg < -90) deg += 180;
  if (deg > 45) deg -= 90;
  else if (deg < -45) deg += 90;
  return clampAngle(deg);
}

/** The crop's size in pixels, as the panel shows it. */
export function pixelSize(c: Crop, s: Size): Size {
  return { width: Math.round(c.w * s.width), height: Math.round(c.h * s.height) };
}

export type Overlay = 'thirds' | 'golden' | 'grid' | 'none';
export const OVERLAYS: Overlay[] = ['thirds', 'golden', 'grid', 'none'];

/** The overlay's lines inside the frame, as fractions of its width (`x`) or height (`y`). */
export function overlayLines(o: Overlay): { x: number[]; y: number[] } {
  switch (o) {
    case 'thirds':
      return { x: [1 / 3, 2 / 3], y: [1 / 3, 2 / 3] };
    case 'golden': {
      const g = 1 / ((1 + Math.sqrt(5)) / 2);
      return { x: [1 - g, g], y: [1 - g, g] };
    }
    case 'grid': {
      const at = [1, 2, 3, 4, 5, 6, 7].map((i) => i / 8);
      return { x: at, y: at };
    }
    case 'none':
      return { x: [], y: [] };
  }
}

/** The aspect presets: the label and the ratio, or `original` and `free`. */
export const ASPECTS: { label: string; ratio: number | 'original' | 'free' }[] = [
  { label: 'Original', ratio: 'original' },
  { label: 'Free', ratio: 'free' },
  { label: '1:1', ratio: 1 },
  { label: '4:5', ratio: 5 / 4 },
  { label: '5:7', ratio: 7 / 5 },
  { label: '3:2', ratio: 3 / 2 },
  { label: '16:9', ratio: 16 / 9 },
];
