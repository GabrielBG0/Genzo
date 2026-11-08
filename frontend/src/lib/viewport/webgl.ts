import type { TileHeader } from '$contract/genzo';
import type { Tile } from '../engine/tiles';
import type { DecodedTile, ViewTransform, Viewport } from './types';

const VERTEX = `#version 300 es
in vec2 a_pos;
uniform vec2 u_scale;
uniform vec2 u_offset;
out vec2 v_uv;
void main() {
  v_uv = a_pos;
  vec2 p = a_pos * u_scale + u_offset;
  gl_Position = vec4(p.x * 2.0 - 1.0, 1.0 - p.y * 2.0, 0.0, 1.0);
}`;

const FRAGMENT = `#version 300 es
precision mediump float;
in vec2 v_uv;
uniform sampler2D u_tex;
out vec4 color;
void main() { color = texture(u_tex, v_uv); }`;

function compile(gl: WebGL2RenderingContext, type: number, src: string): WebGLShader {
  const s = gl.createShader(type);
  if (!s) throw new Error('createShader failed');
  gl.shaderSource(s, src);
  gl.compileShader(s);
  if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s) ?? 'shader');
  return s;
}

/** Where the view is on the shown image: its zoom (device pixels per shown pixel) and top left. */
export interface Placement {
  zoom: number;
  originX: number;
  originY: number;
  shownWidth: number;
  shownHeight: number;
}

/**
 * The view on the shown image for transform `t`, with a held frame of size
 * `held` (the shown image's size when the transform names none) and a
 * canvas of CSS size `canvas` at its device pixel ratio.
 */
export function placement(
  t: ViewTransform,
  held: { width: number; height: number },
  canvas: { width: number; height: number; dpr: number },
): Placement {
  const cw = canvas.width * canvas.dpr;
  const ch = canvas.height * canvas.dpr;
  const shown = t.shown && t.shown.width > 0 && t.shown.height > 0 ? t.shown : held;
  const zoom = t.zoom === 'fit' ? Math.min(cw / shown.width, ch / shown.height) : t.zoom;
  const originX = (t.origin?.x ?? (shown.width - cw / zoom) / 2) - ((t.panX ?? 0) * canvas.dpr) / zoom;
  const originY = (t.origin?.y ?? (shown.height - ch / zoom) / 2) - ((t.panY ?? 0) * canvas.dpr) / zoom;
  return { zoom, originX, originY, shownWidth: shown.width, shownHeight: shown.height };
}

/**
 * A layer held for drawing: a texture covering `rect` of a frame whose
 * image is `image` in size (a Fit frame's is smaller than the shown
 * image; a 1:1 frame's is the shown image's).
 */
interface Layer {
  texture: WebGLTexture;
  image: { width: number; height: number };
  rect: { x: number; y: number; width: number; height: number };
  /** The texture's allocated size; zero when the layer holds nothing. */
  size: { width: number; height: number };
}

/**
 * The WebGL2 tile canvas. It holds two layers, both moved by uniforms only
 * (DEV-01 viewport spec): the Fit frame, the backdrop at any zoom, and the
 * newest 1:1 frame's tiles over it, placed by their rect in the shown
 * image. A frame whose tiles cover its whole image is a Fit frame; one
 * that covers part of it is a 1:1 frame.
 */
export class WebGLViewport implements Viewport {
  generation = 0;
  private gl: WebGL2RenderingContext;
  private program: WebGLProgram;
  private base: Layer;
  private detail: Layer;
  private transform: ViewTransform = { zoom: 'fit' };
  private size = { width: 1, height: 1, dpr: 1 };
  private uScale: WebGLUniformLocation | null;
  private uOffset: WebGLUniformLocation | null;

  constructor(private canvas: HTMLCanvasElement) {
    const gl = canvas.getContext('webgl2', { alpha: true, antialias: false, premultipliedAlpha: true });
    if (!gl) throw new Error('WebGL2 is not available');
    this.gl = gl;
    const program = gl.createProgram();
    gl.attachShader(program, compile(gl, gl.VERTEX_SHADER, VERTEX));
    gl.attachShader(program, compile(gl, gl.FRAGMENT_SHADER, FRAGMENT));
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(program) ?? 'link');
    this.program = program;
    gl.useProgram(program);
    const buf = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, buf);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([0, 0, 1, 0, 0, 1, 1, 1]), gl.STATIC_DRAW);
    const loc = gl.getAttribLocation(program, 'a_pos');
    gl.enableVertexAttribArray(loc);
    gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
    this.uScale = gl.getUniformLocation(program, 'u_scale');
    this.uOffset = gl.getUniformLocation(program, 'u_offset');
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    this.base = this.layer(gl.LINEAR);
    // 1:1 tiles sit on whole device pixels: nearest keeps them exact.
    this.detail = this.layer(gl.NEAREST);
  }

  private layer(filter: number): Layer {
    const gl = this.gl;
    const texture = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, filter);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, filter);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    return { texture, image: { width: 0, height: 0 }, rect: { x: 0, y: 0, width: 0, height: 0 }, size: { width: 0, height: 0 } };
  }

  putTiles(tiles: Tile[]): boolean {
    const gl = this.gl;
    const layer = this.accept(tiles.map((t) => t.header));
    if (!layer) return false;
    for (const { header, pixels } of tiles) {
      const { x, y, width, height } = header.rect;
      gl.texSubImage2D(gl.TEXTURE_2D, 0, x - layer.rect.x, y - layer.rect.y, width, height, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
    }
    return true;
  }

  putImages(tiles: DecodedTile[]): boolean {
    const gl = this.gl;
    const layer = this.accept(tiles.map((t) => t.header));
    if (!layer) return false;
    for (const { header, image } of tiles) {
      const { x, y } = header.rect;
      gl.texSubImage2D(gl.TEXTURE_2D, 0, x - layer.rect.x, y - layer.rect.y, gl.RGBA, gl.UNSIGNED_BYTE, image);
    }
    return true;
  }

  /**
   * Takes one frame's tiles, unless they are older than the newest frame
   * drawn: the layer they go to, its texture bound and sized for them.
   */
  private accept(headers: TileHeader[]): Layer | null {
    const gl = this.gl;
    const first = headers[0];
    if (!first) return null;
    const generation = Math.max(...headers.map((h) => h.generation));
    if (generation < this.generation) return null;
    this.generation = generation;
    this.setColorSpace(first.format === 'rgba8DisplayP3' || first.format === 'jpegDisplayP3' ? 'display-p3' : 'srgb');
    const image = { width: first.imageWidth, height: first.imageHeight };
    // The tiles' bounding box, and whether they cover the whole image.
    let [x0, y0, x1, y1, area] = [Infinity, Infinity, 0, 0, 0];
    for (const { rect } of headers) {
      x0 = Math.min(x0, rect.x);
      y0 = Math.min(y0, rect.y);
      x1 = Math.max(x1, rect.x + rect.width);
      y1 = Math.max(y1, rect.y + rect.height);
      area += rect.width * rect.height;
    }
    const whole = x0 === 0 && y0 === 0 && x1 === image.width && y1 === image.height && area >= image.width * image.height;
    const layer = whole ? this.base : this.detail;
    // A Fit frame is the newer render: the 1:1 tiles held are of an older one.
    if (whole) this.dropDetail();
    layer.image = image;
    layer.rect = { x: x0, y: y0, width: x1 - x0, height: y1 - y0 };
    gl.bindTexture(gl.TEXTURE_2D, layer.texture);
    // A 1:1 frame starts transparent, so the Fit frame shows where it has no tile.
    if (!whole || layer.size.width !== layer.rect.width || layer.size.height !== layer.rect.height) {
      layer.size = { width: layer.rect.width, height: layer.rect.height };
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, layer.size.width, layer.size.height, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
    }
    return layer;
  }

  private dropDetail() {
    this.detail.image = { width: 0, height: 0 };
    this.detail.size = { width: 0, height: 0 };
  }

  private setColorSpace(space: PredefinedColorSpace) {
    // The webview converts to the monitor profile (spec: v1 color policy).
    // `unpackColorSpace` stays sRGB: every frame's bytes are already encoded
    // in its space and must upload unchanged. Raw tiles ignore it, but a
    // decoded JPEG preview counts as sRGB, and with 'display-p3' here WebView2
    // converts a P3 preview to P3 a second time, which desaturates it (found
    // by the color probe, up to 15 of 255).
    if (this.gl.drawingBufferColorSpace !== space) this.gl.drawingBufferColorSpace = space;
  }

  setTransform(t: ViewTransform): void {
    // Back at Fit, the 1:1 tiles go (DEV-01 viewport spec).
    if (t.zoom === 'fit') this.dropDetail();
    this.transform = t;
  }

  clear(): void {
    this.base.image = { width: 0, height: 0 };
    this.base.size = { width: 0, height: 0 };
    this.dropDetail();
  }

  resize(width: number, height: number, dpr: number): void {
    this.size = { width: Math.max(1, width), height: Math.max(1, height), dpr };
    this.canvas.width = Math.round(this.size.width * dpr);
    this.canvas.height = Math.round(this.size.height * dpr);
  }

  /** The view on the shown image now, or null before a frame. */
  private where(): Placement | null {
    const held = this.base.size.width > 0 ? this.base.image : this.detail.size.width > 0 ? this.detail.image : null;
    return held ? placement(this.transform, held, this.size) : null;
  }

  draw(): void {
    const gl = this.gl;
    gl.viewport(0, 0, this.canvas.width, this.canvas.height);
    gl.clearColor(0, 0, 0, 0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    const p = this.where();
    if (!p) return;
    gl.useProgram(this.program);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    for (const layer of [this.base, this.detail]) {
      if (layer.size.width === 0) continue;
      // The layer's rect in shown pixels: its frame scaled to the shown image.
      const sx = p.shownWidth / layer.image.width;
      const sy = p.shownHeight / layer.image.height;
      const x = (layer.rect.x * sx - p.originX) * p.zoom;
      const y = (layer.rect.y * sy - p.originY) * p.zoom;
      gl.bindTexture(gl.TEXTURE_2D, layer.texture);
      gl.uniform2f(
        this.uScale,
        (layer.rect.width * sx * p.zoom) / this.canvas.width,
        (layer.rect.height * sy * p.zoom) / this.canvas.height,
      );
      gl.uniform2f(this.uOffset, x / this.canvas.width, y / this.canvas.height);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    }
  }

  imagePoint(x: number, y: number): [number, number] | null {
    const p = this.where();
    if (!p) return null;
    const u = (p.originX + (x * this.size.dpr) / p.zoom) / p.shownWidth;
    const v = (p.originY + (y * this.size.dpr) / p.zoom) / p.shownHeight;
    return u >= 0 && u <= 1 && v >= 0 && v <= 1 ? [u, v] : null;
  }

  /**
   * Draws, then reads the drawing buffer back: the mean RGB of the
   * (2r+1)² square around each point (normalized canvas coordinates, top
   * left 0,0). The values are in the drawing buffer's color space, before
   * the webview converts to the monitor. For the color probe.
   */
  readPixels(points: [number, number][], r = 2): [number, number, number][] {
    const gl = this.gl;
    this.draw();
    const side = 2 * r + 1;
    const px = new Uint8Array(side * side * 4);
    return points.map(([nx, ny]) => {
      const x = Math.round(nx * this.canvas.width) - r;
      // WebGL rows start at the bottom.
      const y = Math.round((1 - ny) * this.canvas.height) - r;
      gl.readPixels(x, y, side, side, gl.RGBA, gl.UNSIGNED_BYTE, px);
      let [r0, g0, b0] = [0, 0, 0];
      for (let i = 0; i < px.length; i += 4) {
        r0 += px[i] ?? 0;
        g0 += px[i + 1] ?? 0;
        b0 += px[i + 2] ?? 0;
      }
      const n = side * side;
      return [r0 / n, g0 / n, b0 / n];
    });
  }

  /**
   * The color spaces the webview actually kept, after `setColorSpace`, or
   * 'unsupported' where it does not implement the property (assigning one
   * then only adds a plain JavaScript property, which reads back anyway).
   */
  colorSpaces(): { drawing: string; unpack: string } {
    const proto: object = Object.getPrototypeOf(this.gl);
    const read = (p: 'drawingBufferColorSpace' | 'unpackColorSpace') =>
      p in proto ? String(this.gl[p]) : 'unsupported';
    return { drawing: read('drawingBufferColorSpace'), unpack: read('unpackColorSpace') };
  }

  destroy(): void {
    this.gl.deleteTexture(this.base.texture);
    this.gl.deleteTexture(this.detail.texture);
    this.gl.deleteProgram(this.program);
  }
}
