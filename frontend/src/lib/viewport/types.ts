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
  zoom: Zoom;
  /** Pan offset in CSS pixels from the centered position. */
  panX: number;
  panY: number;
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
