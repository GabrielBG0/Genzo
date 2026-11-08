import { SvelteSet } from 'svelte/reactivity';
import type { Event, ExportState } from '$contract/genzo';
import type { Engine } from '../engine/types';

/** The export dialog's values, kept for the session (OUT-01). */
export interface ExportSettings {
  /** The folder exported to last; null before the first export. */
  folder: string | null;
  quality: number;
  size: 'full' | 'long';
  longEdge: number;
}

/** The export the status bar follows: the newest started. */
export interface ExportProgress {
  job: number;
  /** The file's name, for the status bar. */
  name: string;
  rowsDone: number;
  rowsTotal: number;
  state: ExportState;
}

/** What starting an export answered. */
export type Started = { ok: true } | { ok: false; exists: boolean; reason: string };

/** How long "Exported NAME" stays in the status bar. */
export const DONE_FOR_MS = 4000;

/** How long quitting waits for the cancelled exports to delete their temporary files. */
export const QUIT_WAIT_MS = 3000;

/** What quitting during an export asks (OUT-01). */
export const QUIT_QUESTION = 'An export is in progress. Quit anyway?';

/** The smallest long edge the engine takes (DEV-01's smallest crop). */
export const MIN_LONG_EDGE = 32;

/** The folder of `path`, or '' for a bare name. */
export function folderOf(path: string): string {
  const at = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
  return at < 0 ? '' : path.slice(0, at);
}

/** The file name of `path`. */
export function nameOf(path: string): string {
  return path.slice(Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\')) + 1);
}

/** `folder` and `name` joined with the separator `folder` already uses. */
export function join(folder: string, name: string): string {
  if (!folder) return name;
  const sep = folder.includes('\\') && !folder.includes('/') ? '\\' : '/';
  return folder.endsWith(sep) ? folder + name : folder + sep + name;
}

/** `path` with `.jpg` added unless it ends in `.jpg` or `.jpeg`, any case. */
export function withJpg(path: string): string {
  return /\.jpe?g$/i.test(path) ? path : `${path}.jpg`;
}

/** A photo's export name: its file name with `.jpg` for its extension. */
export function exportName(fileName: string): string {
  const dot = fileName.lastIndexOf('.');
  return `${dot > 0 ? fileName.slice(0, dot) : fileName}.jpg`;
}

/** The engine's error text: Tauri rejects with a string, the fake with an Error. */
function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/**
 * The app's side of OUT-01's export: the dialog's remembered values, the
 * suggested destination, starting and cancelling, and the status bar's
 * text from `ExportProgress`.
 */
export class Exports {
  settings: ExportSettings = $state({ folder: null, quality: 85, size: 'full', longEdge: 2048 });
  current: ExportProgress | null = $state(null);
  /** Each job's destination. Not state: nothing renders from it. */
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  private paths = new Map<number, string>();
  /** Jobs queued or running, the one the status bar follows or not. */
  private active = new SvelteSet<number>();
  /** Jobs that ended: an answer arriving after their last event adds none. */
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  private ended = new Set<number>();
  private clear: ReturnType<typeof setTimeout> | null = null;
  private readonly unsubscribe: () => void;

  constructor(private readonly engine: Engine) {
    this.unsubscribe = engine.onEvent((e) => this.onEvent(e));
  }

  dispose(): void {
    this.unsubscribe();
    if (this.clear !== null) clearTimeout(this.clear);
  }

  /** Whether an export is queued or running. */
  get busy(): boolean {
    const s = this.current?.state.type;
    return s === 'queued' || s === 'running';
  }

  /** Whether any export is queued or running: quitting asks first (OUT-01). */
  get running(): boolean {
    return this.active.size > 0;
  }

  /**
   * How far the followed export is, 0 to 1, while it is queued or running;
   * null otherwise. The Export quick action's ring (OUT-01).
   */
  get progress(): number | null {
    const c = this.current;
    if (!c || !this.busy) return null;
    return c.state.type === 'running' && c.rowsTotal > 0 ? Math.min(1, c.rowsDone / c.rowsTotal) : 0;
  }

  /**
   * Where a photo's export goes unless the user chooses: the folder
   * exported to last, else the photo's own; its name with `.jpg`.
   */
  suggested(fileName: string, photoFolder: string): string {
    return join(this.settings.folder ?? photoFolder, exportName(fileName));
  }

  /**
   * Exports `photo` to `path` with the dialog's values. `replace` only for
   * the save dialog's own path, which asked already (OUT-01).
   */
  async start(photo: number, path: string, replace: boolean): Promise<Started> {
    const { quality, size, longEdge } = this.settings;
    try {
      const r = await this.engine.execute({
        type: 'exportJpeg',
        photo,
        path,
        longEdge: size === 'long' ? longEdge : null,
        quality,
        replace,
      });
      if (r.type !== 'export') throw new Error(`exportJpeg answered ${r.type}`);
      this.settings.folder = folderOf(path) || this.settings.folder;
      this.paths.set(r.job, path);
      if (!this.ended.has(r.job)) this.active.add(r.job);
      // Its events may have come before the answer.
      if (this.current?.job !== r.job) {
        this.follow({ job: r.job, name: nameOf(path), rowsDone: 0, rowsTotal: 0, state: { type: 'queued' } });
      }
      return { ok: true };
    } catch (e) {
      const reason = message(e);
      return { ok: false, exists: reason.endsWith('already exists'), reason };
    }
  }

  /** Cancels the export the status bar follows. */
  async cancel(): Promise<void> {
    if (!this.current || !this.busy) return;
    await this.engine.execute({ type: 'cancelExport', job: this.current.job });
  }

  /**
   * Cancels every export, then waits until they have ended (each deletes
   * its temporary file), at most `waitMs`: quitting (OUT-01).
   */
  async cancelAll(waitMs = QUIT_WAIT_MS): Promise<void> {
    const jobs = [...this.active];
    await Promise.allSettled(jobs.map((job) => this.engine.execute({ type: 'cancelExport', job })));
    const deadline = Date.now() + waitMs;
    while (this.active.size > 0 && Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 20));
    }
  }

  /**
   * Whether the window may close: yes with no export; during one, if the
   * user says so, after cancelling it (OUT-01).
   */
  async allowQuit(): Promise<boolean> {
    if (!this.running) return true;
    if (!(await this.engine.ask(QUIT_QUESTION, 'Quit', 'Keep exporting'))) return false;
    await this.cancelAll();
    return true;
  }

  /** The status bar's text for the export it follows, if any (APP-07). */
  get status(): string | null {
    const c = this.current;
    if (!c) return null;
    switch (c.state.type) {
      case 'queued':
        return `Waiting to export ${c.name}`;
      case 'running': {
        const pct = c.rowsTotal > 0 ? Math.floor((100 * c.rowsDone) / c.rowsTotal) : 0;
        return `Exporting ${c.name}, ${pct} %`;
      }
      case 'finished':
        return c.state.notes.length > 0 ? `Exported ${c.name}. ${c.state.notes.join('. ')}` : `Exported ${c.name}`;
      case 'failed':
        return `Export failed: ${c.state.reason}`;
      case 'cancelled':
        return 'Export cancelled';
    }
  }

  private follow(p: ExportProgress) {
    if (this.clear !== null) {
      clearTimeout(this.clear);
      this.clear = null;
    }
    this.current = p;
    if (p.state.type === 'finished' || p.state.type === 'cancelled') {
      this.clear = setTimeout(() => {
        if (this.current?.job === p.job) this.current = null;
      }, DONE_FOR_MS);
    }
  }

  private onEvent(e: Event) {
    if (e.type !== 'exportProgress') return;
    const live = e.state.type === 'queued' || e.state.type === 'running';
    if (live && !this.ended.has(e.job)) {
      this.active.add(e.job);
    } else if (!live) {
      this.ended.add(e.job);
      this.active.delete(e.job);
    }
    // A newer export takes the status bar; an older one takes it back only
    // while it runs and the newer has ended (cancelled while queued, say).
    if (this.current && e.job < this.current.job && (this.busy || !live)) return;
    this.follow({
      job: e.job,
      name: nameOf(this.paths.get(e.job) ?? e.path),
      rowsDone: e.rowsDone,
      rowsTotal: e.rowsTotal,
      state: e.state,
    });
  }
}
