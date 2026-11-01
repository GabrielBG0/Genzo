import type { Change, ColorSpace, Command, CommandResult, Edit, Phase, PhotoEdit, Strip } from '$contract/genzo';
import { applyChange, type EditKind } from '../engine/edit';
import type { Engine } from '../engine/types';

/** Commands that change the open photo's edit and answer with it. */
type EditCommand = Extract<Command, { type: 'applyEditDelta' | 'resetParam' | 'pickWhiteBalance' | 'undo' | 'redo' }>;

/**
 * Develop's state on the app side: the strip, the open photo's edit, and
 * the commands that change it. The engine owns the edit (DEV-01); this
 * keeps the newest reply and the values a drag shows before it answers.
 *
 * - **Replies in order.** The shell runs each command on a thread of its
 *   own, so two sent in a row could reach the engine swapped: a gesture's
 *   *end* before its last *update* would leave a gesture open. Edit
 *   commands therefore go one at a time; while one is in flight, the
 *   newest *update* waits and replaces any older one (latest wins).
 * - **The newest revision wins.** A reply is taken only when its revision
 *   is above the photo's newest seen (contract 0.5.0); queries and photo
 *   switches can still answer late.
 * - **Optimistic values.** A gesture's changes show at once, applied to the
 *   newest edit, and stay until every edit command sent has answered, so
 *   a reply that predates the latest move cannot snap a slider back.
 */
export class DevelopSession {
  strip: Strip = $state({ photos: [], current: null, direction: 'next' });
  /** The engine's newest edit of the open photo. */
  edit: PhotoEdit | null = $state(null);
  /** Why the open photo does not render, for the status bar. */
  reason: string | null = $state(null);
  /** The last refusal (a pick on a clipped area), for the status bar. */
  notice: string | null = $state(null);
  /** The white balance picker is on (DEV-02, W). */
  picking = $state(false);
  /** A gesture is in progress: Undo and Redo wait for its end (APP-10). */
  inGesture = $state(false);
  private optimistic: Edit | null = $state(null);

  /** The edit the panel shows: the optimistic one during a gesture. */
  readonly shown: Edit | null = $derived(this.edit ? (this.optimistic ?? this.edit.edit) : null);

  /** The newest revision seen of each photo's edit. Not state: nothing renders from it. */
  private seen: Record<number, number> = {};
  private queue: EditCommand[] = [];
  private inFlight = false;
  private frame: number | null = null;
  private pendingUpdate: Change[] | null = null;

  constructor(
    private readonly engine: Engine,
    private readonly space: ColorSpace = 'srgb',
  ) {}

  get photo(): number | null {
    return this.strip.current;
  }

  /** Whether the open photo can be edited: it renders. */
  get editable(): boolean {
    return this.edit !== null && this.edit.photo === this.strip.current;
  }

  get canUndo(): boolean {
    return this.editable && !this.inGesture && this.edit?.undo != null;
  }

  get canRedo(): boolean {
    return this.editable && !this.inGesture && this.edit?.redo != null;
  }

  async openFolder(path: string): Promise<void> {
    const r = await this.engine.execute({ type: 'openFolder', path });
    if (r.type !== 'folder') throw new Error(`openFolder answered ${r.type}`);
    this.strip = r.strip;
    this.opened(r.edit, r.reason);
  }

  async select(photo: number): Promise<void> {
    if (photo === this.strip.current) return;
    this.endGestureLocally();
    this.strip.current = photo;
    this.edit = null;
    this.reason = null;
    this.notice = null;
    this.picking = false;
    const r = await this.engine.execute({ type: 'selectPhoto', photo });
    if (r.type !== 'selected') throw new Error(`selectPhoto answered ${r.type}`);
    // A later switch wins over this one.
    if (this.strip.current !== photo) return;
    this.opened(r.edit, r.reason);
  }

  /** The viewport's size in device pixels, at Fit. */
  setView(width: number, height: number): Promise<CommandResult> {
    return this.engine.execute({
      type: 'setView',
      width: Math.max(1, Math.round(width)),
      height: Math.max(1, Math.round(height)),
      view: { type: 'fit' },
      space: this.space,
    });
  }

  /**
   * A gesture's step. *Start* and *end* go at once (an *end* replaces an
   * update still waiting); updates go once per animation frame at most.
   * A delta without a gesture is an *end* alone.
   */
  change(changes: Change[], phase: Phase = 'End'): void {
    const photo = this.photo;
    if (photo === null || !this.editable) return;
    this.optimistic = changes.reduce((e, c) => applyChange(e, c, kindOf(e)), this.shown as Edit);
    this.inGesture = phase !== 'End';
    if (phase === 'Update') {
      this.pendingUpdate = changes;
      this.frame ??= requestAnimationFrame(() => {
        this.frame = null;
        const pending = this.pendingUpdate;
        this.pendingUpdate = null;
        if (pending && this.photo === photo) this.send({ type: 'applyEditDelta', photo, changes: pending, phase });
      });
      return;
    }
    this.cancelUpdate();
    this.send({ type: 'applyEditDelta', photo, changes, phase });
  }

  /** Resets one control, or the whole edit with `null` (APP-10: one step). */
  reset(param: Parameters<typeof resetCommand>[1]): void {
    if (this.photo === null || !this.editable) return;
    this.send(resetCommand(this.photo, param));
  }

  undo(): void {
    if (this.canUndo) this.send({ type: 'undo', photo: this.photo as number });
  }

  redo(): void {
    if (this.canRedo) this.send({ type: 'redo', photo: this.photo as number });
  }

  togglePicker(): void {
    this.picking = this.editable && !this.picking;
  }

  /** A click on the photo while the picker is on, in the shown image's normalized coordinates. */
  pick(x: number, y: number): void {
    if (!this.picking || this.photo === null) return;
    this.send({ type: 'pickWhiteBalance', photo: this.photo, x, y });
  }

  /** Takes a photo's edit if it is the newest seen; true when taken. */
  accept(edit: PhotoEdit): boolean {
    if (edit.revision < (this.seen[edit.photo] ?? -1)) return false;
    this.seen[edit.photo] = edit.revision;
    const entry = this.strip.photos.find((p) => p.id === edit.photo);
    if (entry) entry.edited = edit.edited;
    if (edit.photo === this.strip.current) this.edit = edit;
    return true;
  }

  /** Asks the engine for the open photo's edit again. */
  async refresh(): Promise<void> {
    const photo = this.photo;
    if (photo === null) return;
    const r = await this.engine.query({ type: 'getEdit', photo });
    if (r.type === 'edit' && r.edit) this.accept(r.edit);
  }

  private opened(edit: PhotoEdit | null, reason: string | null) {
    this.reason = reason;
    if (edit) this.accept(edit);
    else this.edit = null;
  }

  private send(command: EditCommand) {
    this.queue.push(command);
    void this.drain();
  }

  private async drain() {
    if (this.inFlight) return;
    this.inFlight = true;
    try {
      for (let next = this.queue.shift(); next; next = this.queue.shift()) {
        try {
          const r = await this.engine.execute(next);
          if (r.type === 'edit') {
            this.accept(r.edit);
            if (next.type === 'pickWhiteBalance' && r.edit.photo === this.photo) {
              this.notice = r.refused;
              if (!r.refused) this.picking = false;
            } else if (r.refused) {
              this.notice = r.refused;
            }
          }
        } catch (e) {
          this.notice = e instanceof Error ? e.message : String(e);
        }
      }
    } finally {
      this.inFlight = false;
      // Every command sent has answered: the engine's edit is the truth,
      // clamped as it clamps.
      if (!this.inGesture && this.frame === null) this.optimistic = null;
    }
  }

  private cancelUpdate() {
    if (this.frame !== null) cancelAnimationFrame(this.frame);
    this.frame = null;
    this.pendingUpdate = null;
  }

  /** A switch mid-gesture: the engine is told nothing more of it. */
  private endGestureLocally() {
    this.cancelUpdate();
    this.inGesture = false;
    this.optimistic = null;
  }
}

function resetCommand(photo: number, param: Extract<Command, { type: 'resetParam' }>['param']): EditCommand {
  return { type: 'resetParam', photo, param };
}

/**
 * The photo's kind as far as the app can tell from an edit: enough to
 * clamp an optimistic value. As Shot's own Temp and Tint stay as the edit
 * has them until the engine answers.
 */
function kindOf(e: Edit): EditKind {
  return e.profile === 'Embedded'
    ? { raw: false }
    : { raw: true, asShotTemp: e.white_balance.temp, asShotTint: e.white_balance.tint };
}
