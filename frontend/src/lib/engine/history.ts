import type { Change, Edit, Phase, PhotoEdit } from '$contract/genzo';
import { applyChange, defaults, deltaLabel, isEdited, sameEdit, type EditKind } from './edit';

interface Step {
  label: string;
  edit: Edit;
}

/**
 * One photo's edit and history, for the fake engine: mirrors
 * `genzo_engine::history::PhotoEdits` (APP-10).
 * - A delta between a gesture's *start* and *end* changes the edit shown,
 *   not the history; its *end* records one step.
 * - A change that leaves the edit as it was records nothing.
 * - Undo and redo do nothing during a gesture; a new step drops the redo
 *   steps; `revision` counts every delta, undo and redo.
 */
export class PhotoEdits {
  private readonly base: Edit;
  private steps: Step[] = [];
  private position = 0;
  private gesture: Step | null = null;
  private revision = 0;

  constructor(
    readonly photo: number,
    readonly kind: EditKind,
  ) {
    this.base = defaults(kind);
  }

  committed(): Edit {
    return this.steps[this.position - 1]?.edit ?? this.base;
  }

  current(): Edit {
    return this.gesture?.edit ?? this.committed();
  }

  inGesture(): boolean {
    return this.gesture !== null;
  }

  undoLabel(): string | null {
    if (this.gesture) return null;
    return this.steps[this.position - 1]?.label ?? null;
  }

  redoLabel(): string | null {
    if (this.gesture) return null;
    return this.steps[this.position]?.label ?? null;
  }

  /** Applies `changes` in `phase`; returns whether a step was recorded. */
  apply(changes: Change[], phase: Phase, label?: string): boolean {
    let edit = this.current();
    for (const c of changes) edit = applyChange(edit, c, this.kind);
    const stepLabel = this.gesture?.label ?? label ?? deltaLabel(changes);
    this.revision += 1;
    if (phase !== 'End') {
      this.gesture = { label: stepLabel, edit };
      return false;
    }
    this.gesture = null;
    if (sameEdit(edit, this.committed())) return false;
    this.steps.length = this.position;
    this.steps.push({ label: stepLabel, edit });
    this.position += 1;
    return true;
  }

  undo(): boolean {
    if (this.undoLabel() === null) return false;
    this.position -= 1;
    this.revision += 1;
    return true;
  }

  redo(): boolean {
    if (this.redoLabel() === null) return false;
    this.position += 1;
    this.revision += 1;
    return true;
  }

  /** The contract's view of it: mirrors `photo_edit` in `genzo_engine::messages`. */
  state(): PhotoEdit {
    const edit = structuredClone(this.current());
    return {
      photo: this.photo,
      edit,
      edited: isEdited(edit, defaults(this.kind)),
      undo: this.undoLabel(),
      redo: this.redoLabel(),
      revision: this.revision,
    };
  }
}
