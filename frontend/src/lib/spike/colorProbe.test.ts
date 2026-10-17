import { describe, expect, it } from 'vitest';
import type { TileHeader } from '$contract/genzo';
import type { Tile } from '../engine/tiles';
import { PATCHES, maxError, pixelAt } from './colorProbe';

function tile(x: number, y: number, width: number, height: number, rgb: (x: number, y: number) => number[]): Tile {
  const header: TileHeader = {
    generation: 1,
    rect: { x, y, width, height },
    mipLevel: 0,
    format: 'rgba8DisplayP3',
    imageWidth: 8,
    imageHeight: 4,
  };
  const pixels = new Uint8Array(width * height * 4);
  for (let j = 0; j < height; j++)
    for (let i = 0; i < width; i++) pixels.set([...rgb(x + i, y + j), 255], (j * width + i) * 4);
  return { header, pixels };
}

describe('color probe', () => {
  it('samples the patch centers of the 6 × 2 grid', () => {
    expect(PATCHES).toHaveLength(12);
    expect(PATCHES[0]).toEqual([1 / 12, 0.5]);
    expect(PATCHES[11]).toEqual([11 / 12, 0.7]);
  });

  it('finds a pixel in whichever tile covers it', () => {
    const at = (x: number, y: number) => [x, y, 7];
    const tiles = [tile(0, 0, 4, 4, at), tile(4, 0, 4, 4, at)];
    expect(pixelAt(tiles, 0, 0)).toEqual([0, 0, 7]);
    expect(pixelAt(tiles, 5 / 8, 3 / 4)).toEqual([5, 3, 7]);
    // The right and bottom edges clamp to the last pixel.
    expect(pixelAt(tiles, 1, 1)).toEqual([7, 3, 7]);
  });

  it('reports the largest channel error, or Infinity when points are missing', () => {
    expect(maxError([[10, 20, 30], [0, 0, 0]], [[11, 17, 30], [0, 0, 2]])).toBe(3);
    expect(maxError([[1, 2, 3]], [])).toBe(Infinity);
  });
});
