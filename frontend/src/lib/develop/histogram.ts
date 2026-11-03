import type { Clipping, Histogram } from '$contract/genzo';

export type Channel = 'red' | 'green' | 'blue';

/**
 * The count that fills the plot (DEV-03): the tallest bin from 1 to 254
 * of any channel, so a clipped spike at 0 or 255 is cut at the top
 * instead of flattening the rest. When those are all empty, the tallest
 * of bins 0 and 255. Zero with no pixels counted.
 */
export function plotScale(h: Histogram): number {
  let inner = 0;
  let ends = 0;
  for (const bins of [h.red, h.green, h.blue]) {
    for (let i = 0; i < bins.length; i++) {
      const n = bins[i] ?? 0;
      if (i === 0 || i === 255) ends = Math.max(ends, n);
      else inner = Math.max(inner, n);
    }
  }
  return inner > 0 ? inner : ends;
}

/** The channels that clip on one side, in the order they mix. */
export function clippedChannels(c: Clipping): Channel[] {
  return (['red', 'green', 'blue'] as const).filter((ch) => c[ch]);
}

/** "0.4 % of pixels"; a share under 0.05 % that is not zero reads "under 0.1 %". */
export function clippedShare(c: Clipping, pixels: number): string {
  if (pixels === 0 || c.pixels === 0) return '0 % of pixels';
  const share = (100 * c.pixels) / pixels;
  return share < 0.05 ? 'under 0.1 % of pixels' : `${share.toFixed(1)} % of pixels`;
}
