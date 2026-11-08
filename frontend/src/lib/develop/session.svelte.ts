import { SvelteMap } from 'svelte/reactivity';
import type {
  Change,
  ClippingPreview,
  ColorSpace,
  Command,
  CommandResult,
  Edit,
  Event,
  Histogram,
  Phase,
  PhotoEdit,
  Strip,
} from '$contract/genzo';
import { applyChange, type EditKind } from '../engine/edit';
import type { Engine } from '../engine/types';
import { tokenRgba, type Rgba8 } from './tokens';

/** Commands that change the open photo's edit and answer with it. */
type EditCommand = Extract<
  Command,
  { type: 'applyEditDelta' | 'resetParam' | 'pickWhiteBalance' | 'undo' | 'redo' | 'showBefore' }
>;
/**
 * Every command that goes in order with the edits: the clipping preview
 * belongs to the gesture it is sent in, so it must not overtake its start.
 */
type Queued = EditCommand | Extract<Command, { type: 'setClippingOverlay' | 'setClippingPreview' }>;

/** The J overlays' colors, from the token set (DEV-03). */
export interface ClipColors {
  shadow: Rgba8;
  highlight: Rgba8;
}

export type ClipSide = 'shadows' | 'highlights';

/** How long a refusal stays in the status bar (APP-07). */
export const NOTICE_MS = 6000;

const tokenClipColors = (): ClipColors => ({
  shadow: tokenRgba('--clip-shadow'),
  highlight: tokenRgba('--clip-highlight'),
});

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
 * - **A switch waits.** It ends a gesture in progress with its latest
 *   value, and goes after every edit command sent, so it cannot overtake
 *   an *end* and leave the photo it leaves mid-gesture.
 * - **The histogram** (DEV-03) is the newest count of the open photo,
 *   by generation, from `HistogramReady`. The J overlays are the
 *   session's, not a photo's: they stay on across switches.
 */
export class DevelopSession {
  strip: Strip = $state({ photos: [], current: null, direction: 'next' });
  /** The folder opened last; none before the first (LIB-09, States). */
  folder: string | null = $state(null);
  /** The engine's newest edit of the open photo. */
  edit: PhotoEdit | null = $state(null);
  /** Why the open photo does not render, for the status bar. */
  reason: string | null = $state(null);
  /** The last refusal (a pick on a clipped area), for the status bar; it goes after `NOTICE_MS` (APP-07). */
  notice: string | null = $state(null);
  /** What each photo shows so far since it was opened: its embedded preview, or a render. */
  private shownState = new SvelteMap<number, 'preview' | 'rendered'>();
  private noticeTimer: ReturnType<typeof setTimeout> | null = null;
  /** The white balance picker is on (DEV-02, W). */
  picking = $state(false);
  /** A gesture is in progress: Undo and Redo wait for its end (APP-10). */
  inGesture = $state(false);
  /** The open photo's newest counts, or none yet. */
  histogram: Histogram | null = $state.raw(null);
  /** Which J overlays are on. */
  overlay = $state({ shadows: false, highlights: false });
  private optimistic: Edit | null = $state(null);
  /** The before asked for, until every command sent has answered (WF-02). */
  private wantBefore: boolean | null = $state(null);

  /** The edit the panel shows: the optimistic one during a gesture. */
  readonly shown: Edit | null = $derived(this.edit ? (this.optimistic ?? this.edit.edit) : null);

  /** The newest revision seen of each photo's edit. Not state: nothing renders from it. */
  private seen: Record<number, number> = {};
  private queue: Queued[] = [];
  private inFlight = false;
  private frame: number | null = null;
  private pendingUpdate: Change[] | null = null;
  /** The gesture's latest changes, for an *end* a switch sends. */
  private gestureChanges: Change[] | null = null;
  private idlers: (() => void)[] = [];
  /** The generation of the counts shown; a lower one is older. */
  private histogramGeneration = -1;
  /** The Alt preview sent for the gesture in progress. */
  private previewing: ClippingPreview = 'none';
  private readonly unsubscribe: () => void;

  constructor(
    private readonly engine: Engine,
    /** The viewport's color space, as the engine renders it. */
    readonly space: ColorSpace = 'srgb',
    private readonly clipColors: () => ClipColors = tokenClipColors,
  ) {
    this.unsubscribe = engine.onEvent((e) => this.onEvent(e));
  }

  dispose(): void {
    this.unsubscribe();
    if (this.noticeTimer !== null) clearTimeout(this.noticeTimer);
  }

  /**
   * The status bar's preview status of the photo shown (APP-07): null with
   * none, else what its viewport shows.
   */
  get previewStatus(): 'Opening' | 'Embedded preview' | 'No preview' | 'Interactive' | 'Settled' | null {
    const photo = this.strip.current;
    if (photo === null) return null;
    if (this.inGesture) return 'Interactive';
    const shown = this.shownState.get(photo);
    if (shown === 'rendered' && this.editable) return 'Settled';
    if (shown === 'preview') return 'Embedded preview';
    return this.reason ? 'No preview' : 'Opening';
  }

  /** A refusal or an error, said in the status bar until the next success or `NOTICE_MS`. */
  private say(text: string | null) {
    if (this.noticeTimer !== null) clearTimeout(this.noticeTimer);
    this.noticeTimer = null;
    this.notice = text;
    if (text !== null) {
      this.noticeTimer = setTimeout(() => {
        this.noticeTimer = null;
        if (this.notice === text) this.notice = null;
      }, NOTICE_MS);
    }
  }

  get photo(): number | null {
    return this.strip.current;
  }

  /** Whether the open photo can be edited: it renders. */
  get editable(): boolean {
    return this.edit !== null && this.edit.photo === this.strip.current;
  }

  /** The viewport shows the photo's before, its defaults (WF-02). */
  get before(): boolean {
    return this.editable && (this.wantBefore ?? this.edit?.before ?? false);
  }

  get canUndo(): boolean {
    return this.editable && !this.inGesture && this.edit?.undo != null;
  }

  get canRedo(): boolean {
    return this.editable && !this.inGesture && this.edit?.redo != null;
  }

  async openFolder(path: string): Promise<void> {
    // Every photo opens anew: its preview, then its first render.
    this.shownState.clear();
    const r = await this.engine.execute({ type: 'openFolder', path });
    if (r.type !== 'folder') throw new Error(`openFolder answered ${r.type}`);
    this.folder = path;
    this.strip = r.strip;
    this.clearHistogram();
    this.opened(r.edit, r.reason);
    await this.fetchHistogram();
  }

  /**
   * Selects the photo after or before the current one (← and →). Nothing
   * at either end: the strip does not wrap.
   */
  async step(direction: 'next' | 'previous'): Promise<void> {
    const at = this.strip.photos.findIndex((p) => p.id === this.strip.current);
    const next = this.strip.photos[direction === 'next' ? at + 1 : at - 1];
    if (at < 0 || !next) return;
    await this.select(next.id);
  }

  async select(photo: number): Promise<void> {
    if (photo === this.strip.current) return;
    if (this.inGesture && this.gestureChanges) this.change(this.gestureChanges, 'End');
    this.cancelUpdate();
    this.inGesture = false;
    this.optimistic = null;
    this.strip.current = photo;
    this.edit = null;
    this.reason = null;
    this.say(null);
    this.shownState.delete(photo);
    this.picking = false;
    this.previewing = 'none';
    this.clearHistogram();
    await this.idle();
    // A later switch went first.
    if (this.strip.current !== photo) return;
    try {
      const r = await this.engine.execute({ type: 'selectPhoto', photo });
      if (r.type !== 'selected') throw new Error(`selectPhoto answered ${r.type}`);
      // A later switch wins over this one.
      if (this.strip.current !== photo) return;
      this.opened(r.edit, r.reason);
      await this.fetchHistogram();
    } catch (e) {
      // Said in the status bar, as a failed edit command is.
      if (this.strip.current === photo) this.say(e instanceof Error ? e.message : String(e));
    }
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
    // A change shows the after (WF-02).
    if (this.before) this.wantBefore = false;
    this.inGesture = phase !== 'End';
    this.gestureChanges = this.inGesture ? changes : null;
    // The engine ends the preview with the gesture.
    if (!this.inGesture) this.previewing = 'none';
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
    if (this.before) this.wantBefore = false;
    this.send(resetCommand(this.photo, param));
  }

  undo(): void {
    if (!this.canUndo) return;
    if (this.before) this.wantBefore = false;
    this.send({ type: 'undo', photo: this.photo as number });
  }

  redo(): void {
    if (!this.canRedo) return;
    if (this.before) this.wantBefore = false;
    this.send({ type: 'redo', photo: this.photo as number });
  }

  /** \ and the quick action: the before or the after (WF-02). Nothing is recorded. */
  toggleBefore(): void {
    if (this.photo === null || !this.editable) return;
    const before = !this.before;
    this.wantBefore = before;
    this.send({ type: 'showBefore', photo: this.photo, before });
  }

  togglePicker(): void {
    this.picking = this.editable && !this.picking;
  }

  /** A click on the photo while the picker is on, in the shown image's normalized coordinates. */
  pick(x: number, y: number): void {
    if (!this.picking || this.photo === null) return;
    this.send({ type: 'pickWhiteBalance', photo: this.photo, x, y });
  }

  /**
   * J (no side) turns both overlays on, or both off when both are on; a
   * marker turns its own. Nothing for a photo that does not render.
   */
  toggleClipping(side?: ClipSide): void {
    if (!this.editable) return;
    const { shadows, highlights } = this.overlay;
    const on = !(shadows && highlights);
    this.setOverlay(
      side === undefined
        ? { shadows: on, highlights: on }
        : { shadows: side === 'shadows' ? !shadows : shadows, highlights: side === 'highlights' ? !highlights : highlights },
    );
  }

  /** Leaving Develop turns both overlays off (APP-01). */
  clearClipping(): void {
    if (this.overlay.shadows || this.overlay.highlights) this.setOverlay({ shadows: false, highlights: false });
  }

  /**
   * Alt during a tone slider's drag: shows only what clips, until Alt goes
   * up or the gesture ends. Outside a gesture there is none.
   */
  preview(preview: ClippingPreview): void {
    if (!this.inGesture || preview === this.previewing) return;
    this.previewing = preview;
    this.send({ type: 'setClippingPreview', preview });
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

  private setOverlay(overlay: { shadows: boolean; highlights: boolean }) {
    let colors: ClipColors;
    try {
      colors = this.clipColors();
    } catch (e) {
      this.say(e instanceof Error ? e.message : String(e));
      return;
    }
    this.overlay = overlay;
    this.send({ type: 'setClippingOverlay', ...overlay, shadowColor: colors.shadow, highlightColor: colors.highlight });
  }

  private onEvent(e: Event) {
    if (e.type === 'histogramReady') this.takeHistogram(e.photo, e.generation, e.histogram);
    else if (e.type === 'previewShown' && e.generation !== null && this.shownState.get(e.photo) !== 'rendered') {
      this.shownState.set(e.photo, 'preview');
    } else if (e.type === 'renderFinished' && e.photo !== null) {
      this.shownState.set(e.photo, 'rendered');
    }
  }

  /** Takes counts of the open photo newer than those shown. */
  private takeHistogram(photo: number, generation: number, histogram: Histogram) {
    if (photo !== this.strip.current || generation <= this.histogramGeneration) return;
    this.histogramGeneration = generation;
    this.histogram = histogram;
  }

  private clearHistogram() {
    this.histogram = null;
    this.histogramGeneration = -1;
  }

  /** The engine's latest counts, for counts sent before the reply (a photo already shown). */
  private async fetchHistogram() {
    const photo = this.photo;
    if (photo === null || this.histogram) return;
    const r = await this.engine.query({ type: 'getHistogram', photo });
    if (r.type === 'histogram' && r.counted) this.takeHistogram(r.counted.photo, r.counted.generation, r.counted.histogram);
  }

  private opened(edit: PhotoEdit | null, reason: string | null) {
    this.reason = reason;
    this.wantBefore = null;
    if (edit) this.accept(edit);
    else this.edit = null;
  }

  /** Resolves once every edit command sent has answered. */
  private idle(): Promise<void> {
    if (!this.inFlight && this.queue.length === 0) return Promise.resolve();
    return new Promise((resolve) => this.idlers.push(resolve));
  }

  private send(command: Queued) {
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
              if (!r.refused) this.picking = false;
            }
            // A refusal is said; a success clears the last one (APP-07).
            if (r.edit.photo === this.photo) this.say(r.refused);
          }
        } catch (e) {
          this.say(e instanceof Error ? e.message : String(e));
        }
      }
    } finally {
      this.inFlight = false;
      // Every command sent has answered: the engine's edit is the truth,
      // clamped as it clamps.
      if (!this.inGesture && this.frame === null) this.optimistic = null;
      this.wantBefore = null;
      for (const resolve of this.idlers.splice(0)) resolve();
    }
  }

  private cancelUpdate() {
    if (this.frame !== null) cancelAnimationFrame(this.frame);
    this.frame = null;
    this.pendingUpdate = null;
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
