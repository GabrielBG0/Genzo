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
    // A newer export takes the status bar; an older one's end does not.
    if (this.current && e.job < this.current.job) return;
    this.follow({
      job: e.job,
      name: nameOf(this.paths.get(e.job) ?? e.path),
      rowsDone: e.rowsDone,
      rowsTotal: e.rowsTotal,
      state: e.state,
    });
  }
}
