import { describe, expect, it } from 'vitest';
import { ZoomState } from './zoom.svelte';

/** A 6000 × 4000 photo in a 1500 × 1200 view. */
function zoom(): ZoomState {
  const z = new ZoomState();
  z.resize(1500, 1200);
  z.setShown(6000, 4000, true);
  return z;
}

describe('ZoomState (DEV-01 viewport, criterion 1)', () => {
  it('fits the photo, centered, and says its percent', () => {
    const z = zoom();
    expect(z.zoom).toBe(0.25);
    expect(z.origin).toEqual({ x: 0, y: -400 });
    expect(z.readout).toBe('Fit 25%');
    expect(z.request).toEqual({ type: 'fit' });
  });

  it('zooms to 1:1 keeping the point under the pointer', () => {
    const z = zoom();
    // (300, 500) in the view is photo pixel (1200, 1600) at Fit.
    expect(z.imageAt({ x: 300, y: 500 })).toEqual({ x: 1200, y: 1600 });
    z.zoomIn({ x: 300, y: 500 });
    expect(z.level).toBe('1:1');
    expect(z.imageAt({ x: 300, y: 500 })).toEqual({ x: 1200, y: 1600 });
    expect(z.request).toEqual({ type: 'region', zoom: 1, x: 900, y: 1100 });
    expect(z.readout).toBe('1:1');
  });

  it('clamps the pan to the edges and keeps whole pixels', () => {
    const z = zoom();
    z.zoomIn(null);
    expect(z.origin).toEqual({ x: 2250, y: 1400 });
    z.pan(0.4, 0);
    expect(Number.isInteger(z.origin.x)).toBe(true);
    z.pan(100000, 100000);
    expect(z.origin).toEqual({ x: 0, y: 0 });
    z.pan(-100000, -100000);
    expect(z.origin).toEqual({ x: 4500, y: 2800 });
    // A zoom at the corner clamps too.
    z.fit();
    z.zoomIn({ x: 0, y: 0 });
    expect(z.origin).toEqual({ x: 0, y: 0 });
  });

  it('centers a side smaller than the view, which does not pan', () => {
    const z = new ZoomState();
    z.resize(1500, 1200);
    z.setShown(1001, 800, true);
    expect(z.readout).toBe('Fit 150%');
    z.zoomIn(null);
    // -249.5 rounds to a whole pixel.
    expect(z.origin).toEqual({ x: -249, y: -200 });
    z.pan(50, 50);
    expect(z.origin).toEqual({ x: -249, y: -200 });
  });

  it('keeps the center across a resize, and a fraction across a crop', () => {
    const z = zoom();
    z.zoomIn({ x: 300, y: 500 });
    const before = z.imageAt({ x: 750, y: 600 });
    z.resize(1000, 800);
    expect(z.imageAt({ x: 500, y: 400 })).toEqual(before);
    z.setShown(3000, 2000, false);
    expect(z.imageAt({ x: 500, y: 400 })).toEqual({ x: before.x / 2, y: before.y / 2 });
    // A new photo opens on its center.
    z.setShown(6000, 4000, true);
    expect(z.imageAt({ x: 500, y: 400 })).toEqual({ x: 3000, y: 2000 });
  });

  it('holds 1:1 for Space and returns to Fit on release', () => {
    const z = zoom();
    z.hold({ x: 10, y: 10 });
    expect(z.level).toBe('1:1');
    z.release();
    expect(z.level).toBe('fit');
    // At 1:1 already, Space does nothing and its release keeps 1:1.
    z.zoomIn(null);
    z.hold(null);
    z.release();
    expect(z.level).toBe('1:1');
  });

  it('maps the picker through the transform at both levels', () => {
    const z = zoom();
    expect(z.normalizedAt({ x: 750, y: 600 })).toEqual([0.5, 0.5]);
    // The letterbox is off the photo.
    expect(z.normalizedAt({ x: 750, y: 50 })).toBeNull();
    z.zoomIn({ x: 750, y: 600 });
    expect(z.normalizedAt({ x: 750, y: 600 })).toEqual([0.5, 0.5]);
    expect(z.normalizedAt({ x: 0, y: 0 })).toEqual([2250 / 6000, 1400 / 4000]);
  });
});
