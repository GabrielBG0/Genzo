import {
  PIXEL_FORMAT_CODES,
  TILE_HEADER_LEN,
  TILE_MAGIC,
  type PixelFormat,
  type TileHeader,
} from '$contract/genzo';

/** One decoded tile: its header and a view onto its pixels (no copy). */
export interface Tile {
  header: TileHeader;
  pixels: Uint8Array;
}

export class TileDecodeError extends Error {}

/** Whether a tile's payload is a JPEG (a drag preview) rather than raw RGBA. */
export function isJpeg(format: PixelFormat): boolean {
  return format === 'jpegSrgb' || format === 'jpegDisplayP3';
}

const FORMAT_BY_CODE = new Map<number, PixelFormat>(
  Object.entries(PIXEL_FORMAT_CODES).map(([name, code]) => [code, name as PixelFormat]),
);
const MAGIC = [...TILE_MAGIC].map((c) => c.charCodeAt(0));

/**
 * Splits a frame (concatenated tile messages) into tiles. The layout is
 * documented in backend/crates/model/src/frame.rs.
 */
export function decodeFrame(buffer: ArrayBuffer): Tile[] {
  const view = new DataView(buffer);
  const tiles: Tile[] = [];
  let offset = 0;
  while (offset < buffer.byteLength) {
    if (buffer.byteLength - offset < TILE_HEADER_LEN) {
      throw new TileDecodeError('truncated tile header');
    }
    for (let i = 0; i < 4; i++) {
      if (view.getUint8(offset + i) !== MAGIC[i]) throw new TileDecodeError('bad tile magic');
    }
    const headerLen = view.getUint16(offset + 4, true);
    const code = view.getUint8(offset + 6);
    const format = FORMAT_BY_CODE.get(code);
    if (!format) throw new TileDecodeError(`unknown pixel format ${code}`);
    const u32 = (o: number) => view.getUint32(offset + o, true);
    const header: TileHeader = {
      generation: Number(view.getBigUint64(offset + 8, true)),
      rect: { x: u32(16), y: u32(20), width: u32(24), height: u32(28) },
      mipLevel: view.getUint8(offset + 7),
      format,
      imageWidth: u32(32),
      imageHeight: u32(36),
    };
    const payloadLen = u32(40);
    if (!isJpeg(format) && payloadLen !== header.rect.width * header.rect.height * 4) {
      throw new TileDecodeError('tile payload length does not match its rectangle');
    }
    const start = offset + headerLen;
    if (start + payloadLen > buffer.byteLength) throw new TileDecodeError('truncated tile payload');
    tiles.push({ header, pixels: new Uint8Array(buffer, start, payloadLen) });
    offset = start + payloadLen;
  }
  return tiles;
}

/** Encodes tiles the way the engine does. Used by the fake engine and tests. */
export function encodeTile(header: TileHeader, pixels: Uint8Array): Uint8Array {
  const out = new Uint8Array(TILE_HEADER_LEN + pixels.byteLength);
  const view = new DataView(out.buffer);
  MAGIC.forEach((b, i) => view.setUint8(i, b));
  view.setUint16(4, TILE_HEADER_LEN, true);
  view.setUint8(6, PIXEL_FORMAT_CODES[header.format]);
  view.setUint8(7, header.mipLevel);
  view.setBigUint64(8, BigInt(header.generation), true);
  const { x, y, width, height } = header.rect;
  [x, y, width, height, header.imageWidth, header.imageHeight, pixels.byteLength].forEach((v, i) =>
    view.setUint32(16 + i * 4, v, true),
  );
  out.set(pixels, TILE_HEADER_LEN);
  return out;
}
