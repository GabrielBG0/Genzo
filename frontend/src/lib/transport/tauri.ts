// The only module that touches the Tauri API (architecture rule, enforced by
// eslint). Everything else talks to the `Engine` interface.

import { convertFileSrc, invoke, isTauri as tauriPresent } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type {
  Command,
  Event,
  Handshake,
  HandshakeResult,
  Query,
  QueryResult,
} from '$contract/genzo';
import type { Engine } from '../engine/types';

/** Event name and URI scheme; must match shell/src/main.rs. */
const EVENT = 'genzo://event';
const FRAME_SCHEME = 'genzo-frame';

/** Whether the app runs inside the Tauri shell rather than a plain browser. */
export function isTauri(): boolean {
  return tauriPresent();
}

export class TauriEngine implements Engine {
  readonly kind = 'tauri';

  handshake(hello: Handshake): Promise<HandshakeResult> {
    return invoke('handshake', { hello });
  }

  query(query: Query): Promise<QueryResult> {
    return invoke('query', { query });
  }

  execute(command: Command): Promise<void> {
    return invoke('execute', { command });
  }

  onEvent(listener: (event: Event) => void): () => void {
    const stop = listen<Event>(EVENT, (e) => listener(e.payload));
    return () => void stop.then((f) => f());
  }

  async fetchFrame(generation: number): Promise<ArrayBuffer> {
    // Pixels travel over a custom URI scheme as raw bytes, never as JSON.
    const res = await fetch(convertFileSrc(String(generation), FRAME_SCHEME));
    if (!res.ok) throw new Error(`frame ${generation}: HTTP ${res.status}`);
    return res.arrayBuffer();
  }
}
