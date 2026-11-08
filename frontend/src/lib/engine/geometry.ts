import type { Crop, Edit, Size } from '$contract/genzo';

/**
 * The edit's geometry for the fake engine, as `pipeline::geometry` has it
 * (DEV-01, *Crop and rotation coordinates*). The fake's files are all
 * stored upright.
 */

/** The smallest crop, in pixels on each side (DEV-01). */
const MIN_CROP = 32;
const MAX_ANGLE = 45;

export const FULL_CROP: Crop = { cx: 0.5, cy: 0.5, w: 1, h: 1, angle: 0 };

function isFull(c: Crop): boolean {
  return c.cx === 0.5 && c.cy === 0.5 && c.w === 1 && c.h === 1 && c.angle === 0;
}

/** The stored size in the edit's orientation. */
export function orientedSize(edit: Edit, stored: [number, number]): [number, number] {
  return edit.quarter_turns % 2 === 1 ? [stored[1], stored[0]] : stored;
}

/**
 * The sizes an edit reply names (`PhotoEdit.oriented`, `PhotoEdit.shown`),
 * as `PhotoEdits::sizes` computes them: the stored size in the edit's
 * orientation, and the crop's size in whole pixels, at least 1.
 */
export function sizes(edit: Edit, stored: [number, number]): { oriented: Size; shown: Size } {
  const [w, h] = orientedSize(edit, stored);
  const c = edit.crop;
  return {
    oriented: { width: w, height: h },
    shown: isFull(c)
      ? { width: w, height: h }
      : { width: Math.max(1, Math.round(c.w * w)), height: Math.max(1, Math.round(c.h * h)) },
  };
}

/** The crop after one more quarter turn clockwise: `Geometry::turned`. */
export function turnedCrop(c: Crop, flipped: boolean): Crop {
  return flipped
    ? { cx: c.cy, cy: 1 - c.cx, w: c.h, h: c.w, angle: c.angle }
    : { cx: 1 - c.cy, cy: c.cx, w: c.h, h: c.w, angle: c.angle };
}

/** The crop after a horizontal flip: `Geometry::flipped`. */
export function flippedCrop(c: Crop): Crop {
  return { ...c, cx: 1 - c.cx, angle: -c.angle };
}

/** Its four corners in oriented pixels: `Crop::corners`. */
function corners(c: Crop, [w, h]: [number, number]): [number, number][] {
  const [cw, ch] = [c.w * w, c.h * h];
  const center = [c.cx * w, c.cy * h] as const;
  const [sin, cos] = [Math.sin((c.angle * Math.PI) / 180), Math.cos((c.angle * Math.PI) / 180)];
  return [
    [-0.5, -0.5],
    [0.5, -0.5],
    [0.5, 0.5],
    [-0.5, 0.5],
  ].map(([u = 0, v = 0]) => {
    const [dx, dy] = [u * cw, v * ch];
    return [center[0] + cos * dx - sin * dy, center[1] + sin * dx + cos * dy];
  });
}

/**
 * The crop as the engine accepts it on an oriented image of `size`:
 * `geometry::fit`. The angle clamped, a rectangle that leaves the image
 * shrunk about its center, none under the minimum or off the image.
 */
export function fitCrop(crop: Crop, size: [number, number]): Crop | null {
  if (![crop.cx, crop.cy, crop.w, crop.h, crop.angle].every(Number.isFinite)) return null;
  const [w, h] = size;
  const c: Crop = {
    ...crop,
    angle: Math.min(MAX_ANGLE, Math.max(-MAX_ANGLE, crop.angle)),
    w: Math.min(1, crop.w),
    h: Math.min(1, crop.h),
  };
  if (c.cx < 0 || c.cx > 1 || c.cy < 0 || c.cy > 1) return null;
  const center = [c.cx * w, c.cy * h];
  let s = 1;
  for (const corner of corners(c, size)) {
    [w, h].forEach((limit, axis) => {
      const k = (corner[axis] ?? 0) - (center[axis] ?? 0);
      if (k > 0) s = Math.min(s, (limit - (center[axis] ?? 0)) / k);
      else if (k < 0) s = Math.min(s, (center[axis] ?? 0) / -k);
    });
  }
  const fitted = { ...c, w: c.w * s, h: c.h * s };
  const min = (side: number) => Math.min(MIN_CROP, side) - 1e-9;
  return fitted.w * w >= min(w) && fitted.h * h >= min(h) ? fitted : null;
}

/**
 * Where a pixel of the shown image (`x`, `y`, of a `shown` size) comes
 * from, normalized on the stored photo: through the crop and its angle,
 * then back through the flip and the quarter turns. Null outside the
 * photo. The fake draws its scene through it.
 */
export function storedPoint(edit: Edit, stored: [number, number], shown: [number, number], x: number, y: number): [number, number] | null {
  const oriented = orientedSize(edit, stored);
  const c = isFull(edit.crop) ? FULL_CROP : edit.crop;
  const [cw, ch] = [c.w * oriented[0], c.h * oriented[1]];
  // The pixel's place in the crop, from its center, in oriented pixels.
  const dx = ((x + 0.5) / shown[0] - 0.5) * cw;
  const dy = ((y + 0.5) / shown[1] - 0.5) * ch;
  const [sin, cos] = [Math.sin((c.angle * Math.PI) / 180), Math.cos((c.angle * Math.PI) / 180)];
  let a = (c.cx * oriented[0] + cos * dx - sin * dy) / oriented[0];
  let b = (c.cy * oriented[1] + sin * dx + cos * dy) / oriented[1];
  if (a < 0 || a > 1 || b < 0 || b > 1) return null;
  // The flip applies after the turns: undo it first.
  if (edit.flip_horizontal) a = 1 - a;
  // Each clockwise turn took (a, b) from (b, 1 − a); undo them.
  for (let i = 0; i < edit.quarter_turns % 4; i++) [a, b] = [b, 1 - a];
  return [a, b];
}
