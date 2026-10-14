import type { RgbaImage } from '../components/Thumb.svelte';
import type { HistogramData } from '../components/Histogram.svelte';

type Rgb = [number, number, number];

/**
 * Sample thumbnails for the gallery: the handoff's dusk-gradient stand-ins,
 * rendered as pixel data (what the engine will deliver), not as UI colors.
 */
const PALETTES: [Rgb, Rgb, Rgb, Rgb][] = [
  [[22, 33, 61], [201, 119, 92], [44, 40, 64], [18, 24, 41]],
  [[29, 59, 79], [143, 179, 194], [43, 74, 58], [15, 26, 20]],
  [[42, 47, 58], [154, 164, 178], [74, 79, 87], [26, 28, 32]],
  [[58, 42, 74], [224, 154, 106], [59, 44, 58], [21, 16, 24]],
  [[15, 34, 54], [90, 139, 176], [42, 63, 82], [11, 21, 32]],
  [[45, 58, 42], [200, 195, 154], [77, 90, 58], [26, 32, 21]],
  [[64, 32, 40], [229, 160, 138], [58, 42, 48], [20, 12, 16]],
  [[26, 31, 46], [123, 134, 168], [38, 43, 61], [14, 16, 24]],
  [[34, 49, 58], [216, 199, 160], [51, 70, 80], [18, 26, 30]],
  [[20, 38, 46], [79, 163, 165], [32, 58, 60], [10, 20, 21]],
];

const lerp = (a: Rgb, b: Rgb, t: number): Rgb => [
  a[0] + (b[0] - a[0]) * t,
  a[1] + (b[1] - a[1]) * t,
  a[2] + (b[2] - a[2]) * t,
];

export function sampleThumb(i: number, width = 120, height = 80): RgbaImage {
  const palette = PALETTES[i % PALETTES.length];
  if (!palette) throw new Error('no palettes');
  const [top, horizon, below, bottom] = palette;
  const data = new Uint8ClampedArray(width * height * 4);
  for (let y = 0; y < height; y++) {
    const t = y / (height - 1);
    const c = t < 0.58 ? lerp(top, horizon, t / 0.58) : t < 0.6 ? horizon : lerp(below, bottom, (t - 0.6) / 0.4);
    for (let x = 0; x < width; x++) data.set([c[0], c[1], c[2], 255], (y * width + x) * 4);
  }
  return { width, height, data };
}

export function sampleHistogram(bins = 64): HistogramData {
  const bump = (center: number, spread: number, i: number) =>
    Math.exp(-(((i / (bins - 1) - center) / spread) ** 2));
  const channel = (a: number, b: number) =>
    Array.from({ length: bins }, (_, i) => 0.85 * bump(a, 0.18, i) + 0.5 * bump(b, 0.1, i));
  return { r: channel(0.55, 0.85), g: channel(0.45, 0.7), b: channel(0.3, 0.6) };
}
