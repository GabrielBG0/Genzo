import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import type { Clipping, Histogram as Counts } from '$contract/genzo';
import { clippedShare, plotScale } from '../develop/histogram';
import { parseColor } from '../develop/tokens';
import Histogram from './Histogram.svelte';

const NONE: Clipping = { pixels: 0, red: false, green: false, blue: false };

/** Counts with the given bins set, the same in each channel unless overridden. */
function counts(bins: Record<number, number>, extra: Partial<Counts> = {}): Counts {
  const channel = () => Array.from({ length: 256 }, (_, i) => bins[i] ?? 0);
  const pixels = Object.values(bins).reduce((a, b) => a + b, 0);
  return { red: channel(), green: channel(), blue: channel(), highlights: NONE, shadows: NONE, pixels, ...extra };
}

const marker = (name: string) => screen.getByRole('button', { name });

describe('Histogram (DEV-03)', () => {
  it('fills the plot with the tallest bin from 1 to 254; bins 0 and 255 are cut', () => {
    expect(plotScale(counts({ 0: 9000, 40: 120, 200: 300, 255: 5000 }))).toBe(300);
    // Only the ends hold pixels: the taller of them fills it.
    expect(plotScale(counts({ 0: 70, 255: 90 }))).toBe(90);
    expect(plotScale(counts({}))).toBe(0);
    // The scale is shared by the channels.
    const h = counts({ 100: 10 });
    h.green[30] = 400;
    expect(plotScale(h)).toBe(400);
  });

  it('draws three areas cut at the top, and nothing with no pixels counted', () => {
    const { container, rerender } = render(Histogram, { histogram: counts({ 0: 9000, 128: 100 }), notools: true });
    const path = container.querySelector('path.red')?.getAttribute('d') ?? '';
    // Bin 0 is at the top (y = 4), not above it.
    expect(path.startsWith('M0 84 L0.0 4.0 L')).toBe(true);
    expect(container.querySelectorAll('path')).toHaveLength(3);
    rerender({ histogram: counts({}) });
    expect(container.querySelector('svg[aria-label="Histogram"]')).toBeNull();
    rerender({ histogram: null });
    expect(container.querySelector('svg[aria-label="Histogram"]')).toBeNull();
  });

  it('lights each marker in its clipped channels, and says the share of pixels', () => {
    const h = counts(
      { 0: 10, 128: 990 },
      {
        highlights: { pixels: 4, red: true, green: true, blue: false },
        shadows: { pixels: 10, red: true, green: true, blue: true },
      },
    );
    render(Histogram, { histogram: h, notools: true });
    expect(marker('Highlight clipping')).toHaveAttribute('data-channels', 'red green');
    expect(marker('Shadow clipping')).toHaveAttribute('data-channels', 'red green blue');
    // Three channels are white, not a blend of their tints.
    expect(marker('Shadow clipping').querySelectorAll('polygon')).toHaveLength(1);
    expect(marker('Shadow clipping').querySelector('polygon.all')).not.toBeNull();
    expect(marker('Highlight clipping').querySelectorAll('polygon.red, polygon.green')).toHaveLength(2);
    expect(marker('Highlight clipping').title).toBe('Highlight clipping  J\n0.4 % of pixels');
    expect(marker('Shadow clipping').title).toBe('Shadow clipping  J\n1.0 % of pixels');
    expect(clippedShare({ ...NONE, pixels: 1 }, 1e6)).toBe('under 0.1 % of pixels');
  });

  it('keeps the markers dim when nothing clips, and with no counts', () => {
    const { container, rerender } = render(Histogram, { histogram: counts({ 128: 5 }), notools: true });
    expect(marker('Highlight clipping')).toHaveAttribute('data-channels', '');
    expect(container.querySelectorAll('polygon.off')).toHaveLength(2);
    rerender({ histogram: null });
    expect(marker('Shadow clipping').title).toBe('Shadow clipping  J');
    expect(container.querySelectorAll('polygon.off')).toHaveLength(2);
  });

  it('rings a marker whose overlay is on, and turns it with a click', async () => {
    const onoverlay = vi.fn();
    render(Histogram, { overlay: { shadows: false, highlights: true }, onoverlay, notools: true });
    expect(marker('Highlight clipping')).toHaveAttribute('aria-pressed', 'true');
    expect(marker('Highlight clipping')).toHaveClass('on');
    expect(marker('Shadow clipping')).toHaveAttribute('aria-pressed', 'false');
    await fireEvent.click(marker('Shadow clipping'));
    expect(onoverlay).toHaveBeenCalledWith('shadows');
  });

  it('keeps the EXIF row its height when empty', () => {
    const { container } = render(Histogram, { exif: [], notools: true });
    expect(container.querySelector('.exif')?.children).toHaveLength(0);
  });

  it('reads the overlay colors from token values', () => {
    expect(parseColor('#ff6b6b')).toEqual([255, 107, 107, 255]);
    expect(parseColor('#5b9dff80')).toEqual([91, 157, 255, 128]);
    expect(parseColor('#abc')).toEqual([170, 187, 204, 255]);
    expect(parseColor('rgb(91, 157, 255)')).toEqual([91, 157, 255, 255]);
    expect(parseColor('rgba(0, 0, 0, 0.5)')).toEqual([0, 0, 0, 128]);
    expect(parseColor('var(--x)')).toBeNull();
  });
});
