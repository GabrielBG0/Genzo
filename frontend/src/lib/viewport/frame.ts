import { decodeFrame, isJpeg } from '../engine/tiles';
import type { Viewport } from './types';

/**
 * Uploads one fetched frame to the viewport. Returns false when the whole
 * frame was stale. Shared by the photo viewport and the color probe, so the
 * probe checks the path the app draws with.
 */
export async function putFrame(viewport: Viewport, bytes: ArrayBuffer): Promise<boolean> {
  const tiles = decodeFrame(bytes);
  if (!tiles.some((t) => isJpeg(t.header.format))) return viewport.putTiles(tiles);
  // A drag preview: JPEG bands, decoded by the webview in parallel.
  // The bitmaps keep the file's pixel values (no color conversion).
  const images = await Promise.all(
    tiles.map(async ({ header, pixels }) => ({
      header,
      image: await createImageBitmap(new Blob([pixels.slice()], { type: 'image/jpeg' }), {
        colorSpaceConversion: 'none',
      }),
    })),
  );
  const drawn = viewport.putImages(images);
  for (const { image } of images) image.close();
  return drawn;
}
