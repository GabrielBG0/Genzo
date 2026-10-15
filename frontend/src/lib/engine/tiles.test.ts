import { describe, expect, it } from 'vitest';
import type { TileHeader } from '$contract/genzo';
import { decodeFrame, encodeTile, isJpeg, TileDecodeError } from './tiles';

const header: TileHeader = {
  generation: 0x10203040506070, // below 2^53, as all generations are
  rect: { x: 256, y: 512, width: 3, height: 2 },
  mipLevel: 1,
  format: 'rgba8DisplayP3',
  imageWidth: 1280,
  imageHeight: 720,
};

describe('tile protocol', () => {
  it('matches the Rust layout byte for byte', () => {
    // Same layout as layout_is_stable in backend/crates/model/src/frame.rs.
    const msg = encodeTile(header, new Uint8Array(24));
    expect(msg.length).toBe(44 + 24);
    expect([...msg.slice(0, 4)].map((c) => String.fromCharCode(c)).join('')).toBe('GZT1');
    const view = new DataView(msg.buffer);
    expect(view.getUint16(4, true)).toBe(44);
    expect(msg[6]).toBe(2);
    expect(msg[7]).toBe(1);
    expect(view.getBigUint64(8, true)).toBe(0x10203040506070n);
    expect(view.getUint32(40, true)).toBe(24);
  });

  it('round-trips several tiles in one frame', () => {
    const a = encodeTile(header, new Uint8Array(24).fill(7));
    const b = encodeTile({ ...header, rect: { x: 0, y: 0, width: 1, height: 1 } }, new Uint8Array([1, 2, 3, 4]));
    const frame = new Uint8Array(a.length + b.length);
    frame.set(a);
    frame.set(b, a.length);
    const tiles = decodeFrame(frame.buffer);
    expect(tiles).toHaveLength(2);
    expect(tiles[0]?.header).toEqual(header);
    expect([...(tiles[0]?.pixels ?? [])]).toEqual(new Array(24).fill(7));
    expect([...(tiles[1]?.pixels ?? [])]).toEqual([1, 2, 3, 4]);
  });

  it('carries JPEG payloads of any length', () => {
    // Same bytes as jpeg_payload_has_its_own_length in backend/crates/model/src/frame.rs.
    const jpeg = new Uint8Array([0xff, 0xd8, 0xff, 0xd9, 7]);
    const jpegHeader: TileHeader = { ...header, format: 'jpegSrgb' };
    const msg = encodeTile(jpegHeader, jpeg);
    expect(msg[6]).toBe(3);
    const [tile] = decodeFrame(msg.slice().buffer);
    expect(tile?.header).toEqual(jpegHeader);
    expect([...(tile?.pixels ?? [])]).toEqual([...jpeg]);
    expect(isJpeg('jpegDisplayP3')).toBe(true);
    expect(isJpeg('rgba8Srgb')).toBe(false);
  });

  it('rejects damaged frames', () => {
    const msg = encodeTile(header, new Uint8Array(24));
    expect(() => decodeFrame(msg.slice(0, 10).buffer)).toThrow(TileDecodeError);
    expect(() => decodeFrame(msg.slice(0, msg.length - 1).buffer)).toThrow(TileDecodeError);
    const bad = msg.slice();
    bad[0] = 0;
    expect(() => decodeFrame(bad.buffer)).toThrow(/magic/);
  });
});
