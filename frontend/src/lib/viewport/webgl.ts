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

/** Image placement in normalized canvas coordinates (0..1). */
export function placement(
  t: ViewTransform,
  image: { width: number; height: number },
  canvas: { width: number; height: number; dpr: number },
): { scaleX: number; scaleY: number; offsetX: number; offsetY: number } {
  const cw = canvas.width * canvas.dpr;
  const ch = canvas.height * canvas.dpr;
  const s =
    t.zoom === 'fit' ? Math.min(cw / image.width, ch / image.height) : t.zoom;
  const w = (image.width * s) / cw;
  const h = (image.height * s) / ch;
  return {
    scaleX: w,
    scaleY: h,
    offsetX: (1 - w) / 2 + (t.panX * canvas.dpr) / cw,
    offsetY: (1 - h) / 2 + (t.panY * canvas.dpr) / ch,
  };
}

/** The WebGL2 tile canvas. Zoom and pan only change uniforms. */
export class WebGLViewport implements Viewport {
  generation = 0;
  private gl: WebGL2RenderingContext;
  private program: WebGLProgram;
  private texture: WebGLTexture;
  private image = { width: 0, height: 0 };
  private transform: ViewTransform = { zoom: 'fit', panX: 0, panY: 0 };
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
    const tex = gl.createTexture();
    this.texture = tex;
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
  }

  putTiles(tiles: Tile[]): boolean {
    const gl = this.gl;
    let accepted = false;
    for (const { header, pixels } of tiles) {
      if (!this.accept(header)) continue;
      accepted = true;
      const { x, y, width, height } = header.rect;
      gl.texSubImage2D(gl.TEXTURE_2D, 0, x, y, width, height, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
    }
    return accepted;
  }

  putImages(tiles: DecodedTile[]): boolean {
    const gl = this.gl;
    let accepted = false;
    for (const { header, image } of tiles) {
      if (!this.accept(header)) continue;
      accepted = true;
      gl.texSubImage2D(gl.TEXTURE_2D, 0, header.rect.x, header.rect.y, gl.RGBA, gl.UNSIGNED_BYTE, image);
    }
    return accepted;
  }

  /** Drops stale tiles; otherwise prepares the texture for the tile's frame. */
  private accept(header: TileHeader): boolean {
    const gl = this.gl;
    if (header.generation < this.generation) return false;
    if (header.generation > this.generation) this.generation = header.generation;
    this.setColorSpace(header.format === 'rgba8DisplayP3' || header.format === 'jpegDisplayP3' ? 'display-p3' : 'srgb');
    if (header.imageWidth !== this.image.width || header.imageHeight !== this.image.height) {
      this.image = { width: header.imageWidth, height: header.imageHeight };
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, header.imageWidth, header.imageHeight, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
    }
    return true;
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
    this.transform = t;
  }

  resize(width: number, height: number, dpr: number): void {
    this.size = { width: Math.max(1, width), height: Math.max(1, height), dpr };
    this.canvas.width = Math.round(this.size.width * dpr);
    this.canvas.height = Math.round(this.size.height * dpr);
  }

  draw(): void {
    const gl = this.gl;
    gl.viewport(0, 0, this.canvas.width, this.canvas.height);
    gl.clearColor(0, 0, 0, 0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    if (this.image.width === 0) return;
    const p = placement(this.transform, this.image, this.size);
    gl.useProgram(this.program);
    gl.uniform2f(this.uScale, p.scaleX, p.scaleY);
    gl.uniform2f(this.uOffset, p.offsetX, p.offsetY);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
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

  /** The color spaces the webview actually kept, after `setColorSpace`. */
  colorSpaces(): { drawing: string; unpack: string } {
    return { drawing: String(this.gl.drawingBufferColorSpace), unpack: String(this.gl.unpackColorSpace) };
  }

  destroy(): void {
    this.gl.deleteTexture(this.texture);
    this.gl.deleteProgram(this.program);
  }
}
