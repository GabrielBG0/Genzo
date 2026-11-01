import {
  CONTRACT_VERSION,
  TILE_SIZE,
  type AdapterInfo,
  type Change,
  type Command,
  type CommandResult,
  type Edit,
  type Event,
  type Handshake,
  type HandshakeResult,
  type Histogram,
  type Phase,
  type PhotoEdit,
  type PixelFormat,
  type Query,
  type QueryResult,
  type Strip,
  type StripPhoto,
  type TileHeader,
  type WhiteBalancePreset,
} from '$contract/genzo';
import type { EditKind } from './edit';
import { PhotoEdits } from './history';
import { encodeTile } from './tiles';
import type { Engine } from './types';

const ADAPTER: AdapterInfo = {
  kind: 'none',
  name: 'Fake engine',
  backend: 'none',
  fallbackReason: 'Running against the fake engine',
};

const DONE: CommandResult = { type: 'done' };

/** The engine's presets (`genzo_color::white_balance::Preset`), as its info gives them. */
export const PRESETS: WhiteBalancePreset[] = [
  { name: 'Daylight', temp: 5501.4990234375, tint: 10.010603904724121 },
  { name: 'Cloudy', temp: 6503.0107421875, tint: 9.78622055053711 },
  { name: 'Shade', temp: 7505.99853515625, tint: 9.63988208770752 },
  { name: 'Tungsten', temp: 2855.75537109375, tint: 0.008021079935133457 },
  { name: 'Fluorescent', temp: 4224.54150390625, tint: 5.864230632781982 },
  { name: 'Flash', temp: 5499.98388671875, tint: 0.000011306194210192189 },
];

/** A photo of the fake's folder: what the strip shows, and what it renders. */
export interface FakePhoto {
  photo: StripPhoto;
  /** Null for a photo that does not render. */
  kind: EditKind | null;
  width: number;
  height: number;
}

/**
 * The folder every path opens in the fake: three RAW files, a JPEG and a
 * file Genzo cannot decode.
 */
export function demoFolder(): FakePhoto[] {
  const photo = (
    id: number,
    fileName: string,
    minute: number,
    [iso, time, f, mm]: [number, number, number, number],
  ): StripPhoto => ({
    id,
    fileName,
    captureTime: { year: 2026, month: 10, day: 12, hour: 17, minute, second: 4, nanos: 0 },
    shooting: {
      iso,
      exposureTime: { num: 1, den: time },
      fNumber: { num: Math.round(f * 10), den: 10 },
      focalLength: { num: mm, den: 1 },
    },
    kind: 'renders',
    reason: null,
    edited: false,
  });
  return [
    {
      photo: photo(1, 'DSC_0412.NEF', 2, [100, 125, 8, 24]),
      kind: { raw: true, asShotTemp: 5400, asShotTint: 4 },
      width: 6048,
      height: 4032,
    },
    {
      photo: photo(2, 'DSC_0413.NEF', 3, [400, 60, 4, 50]),
      kind: { raw: true, asShotTemp: 3200, asShotTint: -6 },
      width: 6048,
      height: 4032,
    },
    {
      photo: photo(3, 'IMG_2201.DNG', 9, [64, 2000, 1.8, 6]),
      kind: { raw: true, asShotTemp: 6500, asShotTint: 10 },
      width: 3024,
      height: 4032,
    },
    {
      photo: photo(4, 'harbor.jpg', 15, [200, 250, 5.6, 35]),
      kind: { raw: false },
      width: 4000,
      height: 3000,
    },
    {
      photo: {
        ...photo(5, 'DSC_0420.NEF', 21, [100, 125, 8, 24]),
        kind: 'unsupported',
        reason: 'Nikon High Efficiency RAW is not supported yet',
      },
      kind: null,
      width: 0,
      height: 0,
    },
  ];
}

/** Same compatibility rule as ContractVersion::is_compatible_with in Rust. */
export function isCompatible(a: string, b: string): boolean {
  const parse = (s: string) => {
    const m = /^(\d+)\.(\d+)\.(\d+)$/.exec(s.trim());
    return m ? [Number(m[1]), Number(m[2])] : null;
  };
  const x = parse(a);
  const y = parse(b);
  if (!x || !y || x[0] !== y[0]) return false;
  return x[0] !== 0 || x[1] === y[1];
}

/**
 * A TypeScript stand-in for the engine. Implements the same contract, with a
 * synthetic tile source, so screens can be built and tested without Rust.
 * Develop's commands act on a demo folder (`demoFolder`), with each photo's
 * edit and history kept as the engine keeps them (`history.ts`).
 */
export class FakeEngine implements Engine {
  readonly kind = 'fake';
  private listeners = new Set<(e: Event) => void>();
  private frames = new Map<number, ArrayBuffer>();
  private generation = 0;
  private spikeSize: [number, number] | null = null;
  /** Reported in the engine info; tests set it to run the autobench. */
  autobench = false;
  /** The last report sent with `saveAutobenchReport`. */
  autobenchReport: string | null = null;
  /**
   * How long each reply takes: ms, or a promise the test resolves. Tests
   * set it to deliver replies out of order, as the shell can: the answer
   * is made at once (the edit, the render), only its delivery waits.
   */
  latency: (message: Command | Query) => number | Promise<void> = () => 0;
  /** Every command executed, oldest first, for tests. */
  readonly sent: Command[] = [];
  private folder: FakePhoto[] = [];
  private strip: Strip = { photos: [], current: null, direction: 'next' };
  private edits = new Map<number, PhotoEdits>();
  private view = { width: 0, height: 0 };
  private histograms = new Map<number, { generation: number; histogram: Histogram }>();

  constructor(private readonly contractVersion = CONTRACT_VERSION) {}

  async handshake(hello: Handshake): Promise<HandshakeResult> {
    return {
      compatible: isCompatible(hello.contractVersion, this.contractVersion),
      engineContractVersion: this.contractVersion,
      engineVersion: '0.0.0-fake',
    };
  }

  async query(query: Query): Promise<QueryResult> {
    const result = this.answer(query);
    await this.deliver(query);
    return result;
  }

  private answer(query: Query): QueryResult {
    switch (query.type) {
      case 'getEngineInfo':
        return {
          type: 'engineInfo',
          info: {
            engineVersion: '0.0.0-fake',
            contractVersion: this.contractVersion,
            adapter: ADAPTER,
            autobench: this.autobench,
            whiteBalancePresets: structuredClone(PRESETS),
          },
        };
      case 'listFolderPhotos':
        return { type: 'folderPhotos', strip: structuredClone(this.strip) };
      case 'getEdit':
        return { type: 'edit', edit: this.edits.get(query.photo)?.state() ?? null };
      case 'getHistogram': {
        const h = this.histograms.get(query.photo);
        return { type: 'histogram', counted: h ? { photo: query.photo, ...structuredClone(h) } : null };
      }
    }
  }

  async execute(command: Command): Promise<CommandResult> {
    this.sent.push(structuredClone(command));
    const result = this.run(command);
    await this.deliver(command);
    return result;
  }

  private async deliver(message: Command | Query) {
    const delay = this.latency(message);
    if (typeof delay !== 'number') await delay;
    else if (delay > 0) await new Promise((resolve) => setTimeout(resolve, delay));
  }

  private run(command: Command): CommandResult {
    switch (command.type) {
      case 'renderTestPattern': {
        const { width, height, phase, format } = command;
        this.renderPattern(width, height, phase, format);
        return DONE;
      }
      case 'loadSpikeImage': {
        // The fake has no GPU: it remembers the size and renders the test pattern.
        const { width, height } = command.source;
        this.spikeSize = [width & ~1, height & ~1];
        this.emit({ type: 'spikeImageLoaded', width: this.spikeSize[0], height: this.spikeSize[1] });
        return DONE;
      }
      case 'renderSpike': {
        if (!this.spikeSize) throw new Error('no image loaded');
        const [w, h] = fit(this.spikeSize, [command.width, command.height]);
        // Exposure moves the pattern, so a stale frame is easy to spot.
        this.renderPattern(w, h, (command.exposureEv + 3) / 6, command.format);
        return DONE;
      }
      case 'saveAutobenchReport': {
        if (!this.autobench) throw new Error('not in autobench mode');
        this.autobenchReport = command.report;
        return DONE;
      }
      // Every path opens the demo folder, its first photo selected. Its
      // ids are the same each time, so edits are kept, as the engine keeps
      // them for a file it has seen this session.
      case 'openFolder': {
        this.endGesture();
        this.folder = demoFolder();
        for (const p of this.folder) p.photo.edited = this.edits.get(p.photo.id)?.state().edited ?? false;
        this.strip = { photos: this.folder.map((p) => p.photo), current: null, direction: 'next' };
        const first = this.folder[0]?.photo.id;
        const selected = first === undefined ? null : this.select(first);
        return {
          type: 'folder',
          strip: structuredClone(this.strip),
          edit: selected?.edit ?? null,
          reason: selected?.reason ?? null,
        };
      }
      case 'setView':
        this.view = { width: command.width, height: command.height };
        this.renderPhoto(false);
        return DONE;
      case 'setClippingOverlay':
      case 'setClippingPreview':
        return DONE;
      case 'selectPhoto': {
        const { edit, reason } = this.select(command.photo);
        return { type: 'selected', photo: command.photo, edit, reason };
      }
      case 'applyEditDelta':
        return this.edited(command.photo, (p) => p.apply(command.changes, command.phase), command.phase);
      case 'resetParam': {
        const change: Change = command.param === null ? 'ResetAll' : { Reset: command.param };
        return this.edited(command.photo, (p) => p.apply([change], 'End'), 'End');
      }
      case 'pickWhiteBalance':
        return this.pick(command.photo, command.x, command.y);
      case 'undo':
        return this.edited(command.photo, (p) => p.undo(), 'End');
      case 'redo':
        return this.edited(command.photo, (p) => p.redo(), 'End');
    }
  }

  /** Switches to `id`, as `Engine::select_photo` does, and renders it. */
  private select(id: number): { edit: PhotoEdit | null; reason: string | null } {
    const index = this.folder.findIndex((p) => p.photo.id === id);
    const fake = this.folder[index];
    if (!fake) throw new Error(`photo ${id} is not in the strip`);
    if (id !== this.strip.current) this.endGesture();
    const previous = this.folder.findIndex((p) => p.photo.id === this.strip.current);
    if (previous >= 0 && index !== previous) this.strip.direction = index > previous ? 'next' : 'previous';
    this.strip.current = id;
    if (!fake.kind) return { edit: null, reason: fake.photo.reason };
    let edits = this.edits.get(id);
    if (!edits) {
      edits = new PhotoEdits(id, fake.kind);
      this.edits.set(id, edits);
    }
    this.renderPhoto(false);
    return { edit: edits.state(), reason: null };
  }

  /**
   * Ends the open photo's gesture, recording its step as its *end* would,
   * before that photo goes: mirrors `Session::end_gesture`.
   */
  private endGesture() {
    const open = this.strip.current;
    const p = open === null ? undefined : this.edits.get(open);
    if (!p?.inGesture()) return;
    p.apply([], 'End');
    const entry = this.strip.photos.find((s) => s.id === open);
    if (entry) entry.edited = p.state().edited;
  }

  /**
   * Acts on photo `photo`'s edit and renders it, as `Engine::apply_to`
   * does: a photo not open changes nothing, and one never opened is an
   * error.
   */
  private edited(photo: number, act: (p: PhotoEdits) => boolean, phase: Phase): CommandResult {
    const p = this.edits.get(photo);
    if (!p) throw new Error(`photo ${photo} was not opened this session`);
    const open = this.strip.current === photo;
    const recorded = open ? act(p) : false;
    const state = p.state();
    const entry = this.strip.photos.find((s) => s.id === photo);
    if (entry) entry.edited = state.edited;
    if (open) this.renderPhoto(phase !== 'End');
    return { type: 'edit', edit: state, recorded, refused: null };
  }

  /**
   * The picker, as `Engine::pick_white_balance` answers it. The fake has
   * no pixels to sample: the top tenth of the photo counts as clipped, and
   * elsewhere the point gives a white, warmer to the right.
   */
  private pick(photo: number, x: number, y: number): CommandResult {
    const p = this.edits.get(photo);
    if (!p || this.strip.current !== photo || p.inGesture()) return this.edited(photo, () => false, 'End');
    if (y < 0.1) {
      return {
        type: 'edit',
        edit: p.state(),
        recorded: false,
        refused: "Can't set white balance here: the area is clipped",
      };
    }
    const mode = 'Custom' as const;
    const tint = Math.round(40 * y - 20);
    const change: Change = p.kind.raw
      ? { WhiteBalance: { mode, temp: Math.round(3000 + 5000 * x), tint } }
      : { WhiteBalance: { mode, temp: Math.round(80 * x - 40), tint } };
    return this.edited(photo, (q) => q.apply([change], 'End', 'White balance: picked'), 'End');
  }

  /**
   * Renders the open photo fitted to the view, as a scene its edit moves:
   * exposure brightens it, Temp and Tint tint it, B&W grays it. A settled
   * frame's histogram follows it, as the engine sends them.
   */
  private renderPhoto(interactive: boolean) {
    const id = this.strip.current;
    const fake = this.folder.find((p) => p.photo.id === id);
    const edits = id === null ? undefined : this.edits.get(id);
    if (id === null || !fake?.kind || !edits || this.view.width === 0) return;
    const started = performance.now();
    const [width, height] = fit([fake.width, fake.height], [this.view.width, this.view.height]);
    const generation = ++this.generation;
    const look = lookOf(edits.current(), fake.kind);
    const histogram = emptyHistogram();
    const parts: Uint8Array[] = [];
    for (let y = 0; y < height; y += TILE_SIZE) {
      for (let x = 0; x < width; x += TILE_SIZE) {
        const rect = { x, y, width: Math.min(TILE_SIZE, width - x), height: Math.min(TILE_SIZE, height - y) };
        const header: TileHeader = {
          generation,
          rect,
          mipLevel: 0,
          format: 'rgba8Srgb',
          imageWidth: width,
          imageHeight: height,
        };
        const pixels = sceneTile(width, height, rect, look);
        count(histogram, pixels);
        parts.push(encodeTile(header, pixels));
      }
    }
    this.keepFrame(generation, parts);
    this.emit({
      type: 'renderFinished',
      photo: id,
      generation,
      tileCount: parts.length,
      renderMs: performance.now() - started,
      gpuMs: null,
    });
    if (interactive) return;
    this.histograms.set(id, { generation, histogram });
    this.emit({ type: 'histogramReady', photo: id, generation, histogram: structuredClone(histogram) });
  }

  private renderPattern(width: number, height: number, phase: number, format: PixelFormat) {
    const started = performance.now();
    const generation = ++this.generation;
    const parts: Uint8Array[] = [];
    for (let y = 0; y < height; y += TILE_SIZE) {
      for (let x = 0; x < width; x += TILE_SIZE) {
        const rect = {
          x,
          y,
          width: Math.min(TILE_SIZE, width - x),
          height: Math.min(TILE_SIZE, height - y),
        };
        const header: TileHeader = {
          generation,
          rect,
          mipLevel: 0,
          format,
          imageWidth: width,
          imageHeight: height,
        };
        parts.push(encodeTile(header, testPatternTile(width, height, rect, phase)));
      }
    }
    this.keepFrame(generation, parts);
    this.emit({
      type: 'renderFinished',
      photo: null,
      generation,
      tileCount: parts.length,
      renderMs: performance.now() - started,
      gpuMs: null,
    });
  }

  private keepFrame(generation: number, parts: Uint8Array[]) {
    this.frames.set(generation, concat(parts));
    this.frames.delete(generation - 4);
  }

  onEvent(listener: (event: Event) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  async fetchFrame(generation: number): Promise<ArrayBuffer> {
    const frame = this.frames.get(generation);
    if (!frame) throw new Error(`frame ${generation} is gone`);
    return frame;
  }

  private emit(event: Event) {
    // Async, like real engine events.
    queueMicrotask(() => this.listeners.forEach((l) => l(event)));
  }
}

/** Mirrors genzo_pipeline::gpu::spike::fit: fit inside bounds, never enlarge. */
export function fit([w, h]: [number, number], [bw, bh]: [number, number]): [number, number] {
  const s = Math.min(bw / w, bh / h, 1);
  return [Math.max(1, Math.round(w * s)), Math.max(1, Math.round(h * s))];
}

interface Look {
  gain: number;
  rgb: [number, number, number];
  mono: boolean;
}

function lookOf(edit: Edit, kind: EditKind): Look {
  const wb = edit.white_balance;
  // How far white balance moves from as shot, about -1 to 1.
  const warm = kind.raw ? (1e6 / kind.asShotTemp - 1e6 / wb.temp) / 200 : wb.temp / 100;
  const green = (kind.raw ? wb.tint - kind.asShotTint : wb.tint) / 150;
  return {
    gain: 2 ** edit.exposure * (1 + edit.contrast / 400),
    rgb: [1 + 0.3 * warm + 0.1 * green, 1 - 0.2 * green, 1 - 0.3 * warm + 0.1 * green],
    mono: edit.treatment === 'Monochrome',
  };
}

/** A sky over ground with a gray card, under the photo's look. */
function sceneTile(
  width: number,
  height: number,
  rect: { x: number; y: number; width: number; height: number },
  look: Look,
): Uint8Array {
  const out = new Uint8Array(rect.width * rect.height * 4);
  let i = 0;
  for (let y = rect.y; y < rect.y + rect.height; y++) {
    const v = y / Math.max(1, height);
    for (let x = rect.x; x < rect.x + rect.width; x++) {
      const u = x / Math.max(1, width);
      const card = Math.abs(u - 0.5) < 0.12 && Math.abs(v - 0.62) < 0.1;
      const [r0, g0, b0] = card
        ? [0.45, 0.45, 0.45]
        : v < 0.55
          ? [0.35 + 0.3 * u, 0.5 + 0.2 * v, 0.8]
          : [0.35, 0.3 + 0.2 * u, 0.2];
      const [gr, gg, gb] = look.rgb;
      const [r, g, b] = [r0 * gr * look.gain, g0 * gg * look.gain, b0 * gb * look.gain];
      const gray = 0.3 * r + 0.59 * g + 0.11 * b;
      for (const c of look.mono ? [gray, gray, gray] : [r, g, b]) {
        out[i++] = Math.round(255 * Math.min(1, Math.max(0, c)));
      }
      out[i++] = 255;
    }
  }
  return out;
}

function emptyHistogram(): Histogram {
  const clipping = () => ({ pixels: 0, red: false, green: false, blue: false });
  return {
    red: Array<number>(256).fill(0),
    green: Array<number>(256).fill(0),
    blue: Array<number>(256).fill(0),
    highlights: clipping(),
    shadows: clipping(),
    pixels: 0,
  };
}

/** Counts RGBA8 pixels: a pixel clips on a side when any channel does. */
function count(h: Histogram, rgba: Uint8Array) {
  for (let i = 0; i < rgba.length; i += 4) {
    const r = rgba[i] ?? 0;
    const g = rgba[i + 1] ?? 0;
    const b = rgba[i + 2] ?? 0;
    h.red[r] = (h.red[r] ?? 0) + 1;
    h.green[g] = (h.green[g] ?? 0) + 1;
    h.blue[b] = (h.blue[b] ?? 0) + 1;
    h.pixels += 1;
    if (r === 255 || g === 255 || b === 255) {
      h.highlights.pixels += 1;
      h.highlights.red ||= r === 255;
      h.highlights.green ||= g === 255;
      h.highlights.blue ||= b === 255;
    }
    if (r === 0 || g === 0 || b === 0) {
      h.shadows.pixels += 1;
      h.shadows.red ||= r === 0;
      h.shadows.green ||= g === 0;
      h.shadows.blue ||= b === 0;
    }
  }
}

/** Mirrors genzo_pipeline::test_pattern::render_tile. */
function testPatternTile(
  width: number,
  height: number,
  rect: { x: number; y: number; width: number; height: number },
  phase: number,
): Uint8Array {
  const out = new Uint8Array(rect.width * rect.height * 4);
  const shift = Math.floor(Math.min(1, Math.max(0, phase)) * 255);
  let i = 0;
  for (let y = rect.y; y < rect.y + rect.height; y++) {
    const g = Math.floor((y * 255) / Math.max(1, height));
    for (let x = rect.x; x < rect.x + rect.width; x++) {
      out[i++] = (Math.floor((x * 255) / Math.max(1, width)) + shift) % 256;
      out[i++] = g;
      out[i++] = (Math.floor(x / 64) + Math.floor(y / 64)) % 2 === 0 ? 200 : 60;
      out[i++] = 255;
    }
  }
  return out;
}

function concat(parts: Uint8Array[]): ArrayBuffer {
  const total = parts.reduce((n, p) => n + p.byteLength, 0);
  const out = new Uint8Array(total);
  let o = 0;
  for (const p of parts) {
    out.set(p, o);
    o += p.byteLength;
  }
  return out.buffer;
}
