// The only module that touches the Tauri API (architecture rule, enforced by
// eslint). Everything else talks to the `Engine` interface.

import { convertFileSrc, invoke, isTauri as tauriPresent } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { ask, open, save } from '@tauri-apps/plugin-dialog';
import type {
  Command,
  CommandResult,
  Event,
  Handshake,
  HandshakeResult,
  Query,
  QueryResult,
} from '$contract/genzo';
import type { Engine } from '../engine/types';

/** Event name and URI schemes; must match shell/src/main.rs. */
const EVENT = 'genzo://event';
const FRAME_SCHEME = 'genzo-frame';
const THUMB_SCHEME = 'genzo-thumb';

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

  execute(command: Command): Promise<CommandResult> {
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

  async chooseFolder(): Promise<string | null> {
    const picked = await open({ directory: true, multiple: false });
    return typeof picked === 'string' ? picked : null;
  }

  chooseSavePath(suggested: string): Promise<string | null> {
    return save({ defaultPath: suggested, filters: [{ name: 'JPEG', extensions: ['jpg', 'jpeg'] }] });
  }

  ask(message: string, ok: string, cancel: string): Promise<boolean> {
    return ask(message, { title: 'Genzo', kind: 'warning', okLabel: ok, cancelLabel: cancel });
  }

  onCloseRequested(allow: () => Promise<boolean>): () => void {
    // Not prevented, the window is destroyed (core:window:allow-destroy).
    const stop = getCurrentWindow().onCloseRequested(async (e) => {
      // A handler that throws (a missing permission, say) must not leave a
      // window that cannot close: an error lets it close.
      let ok = true;
      try {
        ok = await allow();
      } catch (err) {
        console.error('close handler failed', err);
      }
      if (!ok) e.preventDefault();
    });
    return () => void stop.then((f) => f());
  }

  closeWindow(): void {
    // The window's close: `onCloseRequested` asks first (core:window:allow-close).
    void getCurrentWindow().close();
  }

  async fetchThumbnail(photo: number): Promise<ArrayBuffer> {
    const res = await fetch(convertFileSrc(String(photo), THUMB_SCHEME));
    if (!res.ok) throw new Error(`thumbnail ${photo}: HTTP ${res.status}`);
    return res.arrayBuffer();
  }
}
