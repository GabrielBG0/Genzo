import {
  CONTRACT_VERSION,
  TILE_SIZE,
  type AdapterInfo,
  type Command,
  type Event,
  type Handshake,
  type HandshakeResult,
  type Query,
  type QueryResult,
  type TileHeader,
} from '$contract/genzo';
import { encodeTile } from './tiles';
import type { Engine } from './types';

const ADAPTER: AdapterInfo = {
  kind: 'none',
  name: 'Fake engine',
  backend: 'none',
  fallbackReason: 'Running against the fake engine',
};

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
 */
export class FakeEngine implements Engine {
  readonly kind = 'fake';
  private listeners = new Set<(e: Event) => void>();
  private frames = new Map<number, ArrayBuffer>();
  private generation = 0;

  constructor(private readonly contractVersion = CONTRACT_VERSION) {}

  async handshake(hello: Handshake): Promise<HandshakeResult> {
    return {
      compatible: isCompatible(hello.contractVersion, this.contractVersion),
      engineContractVersion: this.contractVersion,
      engineVersion: '0.0.0-fake',
    };
  }

  async query(query: Query): Promise<QueryResult> {
    switch (query.type) {
      case 'getEngineInfo':
        return {
          type: 'engineInfo',
          info: {
            engineVersion: '0.0.0-fake',
            contractVersion: this.contractVersion,
            adapter: ADAPTER,
          },
        };
    }
  }

  async execute(command: Command): Promise<void> {
    switch (command.type) {
      case 'renderTestPattern': {
        const started = performance.now();
        const generation = ++this.generation;
        const { width, height, phase, format } = command;
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
        this.frames.set(generation, concat(parts));
        this.frames.delete(generation - 4);
        this.emit({
          type: 'renderFinished',
          generation,
          tileCount: parts.length,
          renderMs: performance.now() - started,
        });
        return;
      }
    }
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
