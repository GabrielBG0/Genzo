import { describe, expect, it } from 'vitest';
import {
  clampAngle,
  corners,
  cropOf,
  frameOf,
  gripAt,
  moved,
  oriented,
  pixelSize,
  resized,
  straightenAngle,
  toLocal,
  turnedBy,
  withRatio,
  type Frame,
} from './crop';

const S = { width: 6000, height: 4000 };
const near = (a: number, b: number, d = 1e-6) => expect(Math.abs(a - b)).toBeLessThan(d);

/** A 3000 × 2000 frame centered on the photo. */
function frame(angle = 0): Frame {
  return frameOf({ cx: 0.5, cy: 0.5, w: 0.5, h: 0.5, angle }, S);
}

describe('crop geometry (DEV-12, criterion 2)', () => {
  it('converts between the normalized crop and pixels', () => {
    const f = frame(10);
    expect(f).toMatchObject({ cx: 3000, cy: 2000, w: 3000, h: 2000 });
    const c = cropOf(f, S);
    near(c.angle, 10);
    near(c.w, 0.5);
    expect(pixelSize(c, S)).toEqual({ width: 3000, height: 2000 });
  });

  it('finds the handles, the inside and the outside, at any angle', () => {
    const f = frame(30);
    const [nw, , se] = corners(f);
    if (!nw || !se) throw new Error('corners');
    expect(gripAt(f, nw, 20)).toBe('nw');
    expect(gripAt(f, se, 20)).toBe('se');
    expect(gripAt(f, { x: 3000, y: 2000 }, 20)).toBe('inside');
    expect(gripAt(f, { x: 10, y: 10 }, 20)).toBe('outside');
  });

  it('resizes from the opposite corner, freely or keeping the ratio', () => {
    const f = frame();
    // The bottom right corner, from (4500, 3000) to (5000, 3200).
    const free = resized(f, 'se', { x: 5000, y: 3200 }, null);
    expect(free).toMatchObject({ cx: 3250, cy: 2100, w: 3500, h: 2200 });
    const locked = resized(f, 'se', { x: 5000, y: 3200 }, 1.5);
    near(locked.w / locked.h, 1.5);
    // The top left corner stays.
    near(locked.cx - locked.w / 2, 1500);
    near(locked.cy - locked.h / 2, 1000);
  });

  it('resizes an edge, and with the ratio the other side about the center', () => {
    const f = frame();
    const free = resized(f, 'e', { x: 5000, y: 0 }, null);
    expect(free).toMatchObject({ w: 3500, h: 2000, cy: 2000 });
    const locked = resized(f, 'e', { x: 5000, y: 0 }, 1.5);
    near(locked.h, 3500 / 1.5);
    near(locked.cy, 2000);
  });

  it('resizes in the frame’s own axes when it is turned', () => {
    const f = frame(90);
    // Turned a quarter, the frame's east edge points down the photo.
    const r = resized(f, 'e', { x: 3000, y: 4000 }, null);
    near(r.w, 3500);
    near(r.h, 2000);
    near(toLocal(f, { x: r.cx, y: r.cy }).x, 250);
  });

  it('never makes a side under 32 px', () => {
    const r = resized(frame(), 'se', { x: 0, y: 0 }, null);
    expect(r.w).toBe(32);
    expect(r.h).toBe(32);
  });

  it('moves inside the photo only', () => {
    const f = frame();
    expect(moved(f, { x: 100, y: -50 }, S)).toMatchObject({ cx: 3100, cy: 1950 });
    expect(moved(f, { x: 10000, y: 10000 }, S)).toMatchObject({ cx: 4500, cy: 3000 });
  });

  it('turns by the pointer’s angle around the center', () => {
    const f = frame();
    near(turnedBy(f, { x: 4000, y: 2000 }, { x: 3000, y: 3000 }), 90);
    expect(clampAngle(52.34)).toBe(45);
    expect(clampAngle(-1.234)).toBe(-1.2);
  });

  it('fits a ratio about the center, and orients it as the frame is', () => {
    const square = withRatio(frame(), 1, S);
    expect(square.w).toBe(square.h);
    // The center at (3000, 2000): the room is 2000 up and down.
    near(square.h, 4000);
    expect(oriented(3 / 2, frame())).toBe(1.5);
    expect(oriented(3 / 2, { ...frame(), w: 1000, h: 2000 })).toBeCloseTo(2 / 3);
    // At an angle the frame is smaller, so its corners stay inside.
    const turned = withRatio(frame(20), 1.5, S);
    for (const c of corners(turned)) {
      expect(c.x).toBeGreaterThanOrEqual(-1e-6);
      expect(c.y).toBeLessThanOrEqual(4000 + 1e-6);
    }
  });

  it('straightens a line to level, or to plumb when it is closer to vertical', () => {
    expect(straightenAngle({ x: 0, y: 0 }, { x: 100, y: 5 })).toBe(2.9);
    expect(straightenAngle({ x: 100, y: 5 }, { x: 0, y: 0 })).toBe(2.9);
    expect(straightenAngle({ x: 0, y: 0 }, { x: 5, y: 100 })).toBe(-2.9);
    expect(straightenAngle({ x: 0, y: 0 }, { x: -5, y: 100 })).toBe(2.9);
  });
});
