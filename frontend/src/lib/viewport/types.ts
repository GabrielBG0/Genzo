import type { Tile } from '../engine/tiles';

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
  setTransform(t: ViewTransform): void;
  /** CSS size of the drawing area and the device pixel ratio. */
  resize(width: number, height: number, dpr: number): void;
  /** Draws now. Called on the next animation frame by the owner. */
  draw(): void;
  destroy(): void;
}
