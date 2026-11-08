import type { TileHeader } from '$contract/genzo';
import type { Tile } from '../engine/tiles';

/** A decoded JPEG tile: the drag preview arrives this way. */
export interface DecodedTile {
  header: TileHeader;
  image: ImageBitmap;
}

/** `fit` scales the image into the viewport; a number is device pixels per image pixel. */
export type Zoom = 'fit' | number;

export interface ViewTransform {
  /** Device pixels per shown-image pixel, or `fit`. */
  zoom: Zoom;
  /** The view's top left in shown-image pixels; the image is centered without one. */
  origin?: { x: number; y: number };
  /**
   * The shown image's size in pixels (`PhotoEdit.shown`); the held Fit
   * frame's size without one. Frames are scaled to it, so a Fit frame and
   * 1:1 tiles line up.
   */
  shown?: { width: number; height: number };
  /** Pan offset in CSS pixels from the centered position (the spike). */
  panX?: number;
  panY?: number;
}

/**
 * Draws photo pixels. The default implementation is the WebGL2 tile canvas;
 * the fallback ladder (proxy, codec, native surface) swaps the implementation
 * behind this interface, never the rest of the app.
 */
export interface Viewport {
  /** Newest generation drawn so far. */
  readonly generation: number;
  /**
   * Uploads tiles. Tiles older than the newest generation are dropped.
   * Returns false when the whole batch was stale.
   */
  putTiles(tiles: Tile[]): boolean;
  /** Same as `putTiles`, for tiles the page has already decoded from JPEG. */
  putImages(tiles: DecodedTile[]): boolean;
  setTransform(t: ViewTransform): void;
  /** Drops every frame held, until the next one (a photo switch). */
  clear(): void;
  /** CSS size of the drawing area and the device pixel ratio. */
  resize(width: number, height: number, dpr: number): void;
  /** Draws now. Called on the next animation frame by the owner. */
  draw(): void;
  /**
   * The point of the shown image under (x, y), CSS pixels from the drawing
   * area's top left, normalized to 0..1; null off the image or before any.
   */
  imagePoint(x: number, y: number): [number, number] | null;
  destroy(): void;
}
