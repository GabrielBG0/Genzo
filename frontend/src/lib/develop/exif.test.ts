import { describe, expect, it } from 'vitest';
import type { Shooting } from '$contract/genzo';
import { exifRow } from './exif';

const NONE: Shooting = { iso: null, exposureTime: null, fNumber: null, focalLength: null };
const r = (num: number, den = 1) => ({ num, den });

describe('the EXIF row (DEV-03, criterion 8)', () => {
  const time = (num: number, den = 1) => exifRow({ ...NONE, exposureTime: r(num, den) });
  const f = (num: number, den = 1) => exifRow({ ...NONE, fNumber: r(num, den) });
  const mm = (num: number, den = 1) => exifRow({ ...NONE, focalLength: r(num, den) });

  it('writes exposure times under a second as a rounded fraction, longer ones in seconds', () => {
    expect(time(1, 250)).toEqual(['1/250 s']);
    expect(time(1, 8000)).toEqual(['1/8000 s']);
    expect(time(3, 10)).toEqual(['1/3 s']);
    expect(time(13, 10)).toEqual(['1.3 s']);
    expect(time(30)).toEqual(['30 s']);
    // 1/1.25 would round to 1/1.
    expect(time(4, 5)).toEqual(['0.8 s']);
    expect(time(2, 3)).toEqual(['1/2 s']);
  });

  it('writes the f-number with one decimal, none when it is .0', () => {
    expect(f(18, 10)).toEqual(['f/1.8']);
    expect(f(8)).toEqual(['f/8']);
    expect(f(11)).toEqual(['f/11']);
  });

  it('writes the focal length whole, with one decimal below 10 mm', () => {
    expect(mm(686, 100)).toEqual(['6.9 mm']);
    expect(mm(24)).toEqual(['24 mm']);
    expect(mm(105)).toEqual(['105 mm']);
  });

  it('keeps the order ISO, focal length, aperture, time, and leaves out what is missing or 0', () => {
    expect(exifRow({ iso: 100, exposureTime: r(1, 125), fNumber: r(8), focalLength: r(24) })).toEqual([
      'ISO 100',
      '24 mm',
      'f/8',
      '1/125 s',
    ]);
    expect(exifRow({ iso: 65535, exposureTime: r(0, 1), fNumber: r(0, 1), focalLength: null })).toEqual(['ISO 65535']);
    expect(exifRow(NONE)).toEqual([]);
  });
});
