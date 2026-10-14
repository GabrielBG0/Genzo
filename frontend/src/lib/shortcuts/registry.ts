/**
 * The shortcut registry: the single source for keyboard handling, menu
 * labels, tooltips and (later) command search and the shortcut sheet.
 * Every command the UI offers is registered here, so its shortcut reads the
 * same everywhere it appears.
 */

export type Platform = 'mac' | 'other';

/** Where a command applies. `global` commands work in every module. */
export type Scope = 'global' | 'develop' | 'library';

export interface CommandSpec {
  id: string;
  label: string;
  /**
   * Key chords such as `Mod+Shift+Z`, `Alt`, `F6` or `\`. `Mod` is Cmd on
   * macOS and Ctrl elsewhere. The first chord is the one shown.
   */
  keys?: string[];
  scope?: Scope;
  /** Grouping for menus and the shortcut sheet. */
  group?: string;
  run?: () => void;
  enabled?: () => boolean;
}

export interface Command extends Required<Pick<CommandSpec, 'id' | 'label' | 'scope' | 'group'>> {
  keys: string[];
  run: () => void;
  enabled: () => boolean;
}

interface Chord {
  mod: boolean;
  shift: boolean;
  alt: boolean;
  key: string;
}

const MODIFIERS = ['Mod', 'Shift', 'Alt'] as const;

function parseChord(chord: string): Chord {
  // `+` alone is a key, so split on `+` only between parts.
  const parts = chord === '+' ? ['+'] : chord.split(/\+(?!$)/);
  const key = parts.pop() ?? '';
  const mods = new Set(parts);
  for (const m of mods) {
    if (!(MODIFIERS as readonly string[]).includes(m)) throw new Error(`unknown modifier ${m} in ${chord}`);
  }
  return {
    mod: mods.has('Mod'),
    shift: mods.has('Shift'),
    alt: mods.has('Alt'),
    key: normalizeKey(key),
  };
}

function normalizeKey(key: string): string {
  return key.length === 1 ? key.toUpperCase() : key;
}

const MAC_SYMBOLS: Record<string, string> = { Mod: '⌘', Shift: '⇧', Alt: '⌥' };

/** Formats a chord the way the handoff shows it: `Ctrl Shift Z` or `⌘⇧Z`. */
export function formatChord(chord: string, platform: Platform): string {
  const c = parseChord(chord);
  const mods: string[] = [];
  if (c.mod) mods.push('Mod');
  if (c.alt) mods.push('Alt');
  if (c.shift) mods.push('Shift');
  if (platform === 'mac') {
    const keyName = c.key === 'Alt' ? '⌥' : c.key;
    return mods.map((m) => MAC_SYMBOLS[m]).join('') + keyName;
  }
  return [...mods.map((m) => (m === 'Mod' ? 'Ctrl' : m)), c.key].join(' ');
}

/** The chord a keyboard event represents, or null for a bare modifier press. */
function eventChord(e: KeyboardEvent, platform: Platform): Chord | null {
  const mod = platform === 'mac' ? e.metaKey : e.ctrlKey;
  // A lone Alt press is a command of its own (the app menu).
  if (e.key === 'Alt') return { mod, shift: e.shiftKey, alt: false, key: 'Alt' };
  if (['Control', 'Meta', 'Shift'].includes(e.key)) return null;
  // Prefer the physical key for letters and digits, so Shift and Alt
  // variants (which change e.key) still match.
  let key = e.key;
  if (/^Key[A-Z]$/.test(e.code)) key = e.code.slice(3);
  else if (/^Digit[0-9]$/.test(e.code)) key = e.code.slice(5);
  else if (e.code === 'Backslash') key = '\\';
  else if (e.code === 'BracketLeft') key = '[';
  else if (e.code === 'BracketRight') key = ']';
  else if (e.code === 'Slash') key = '/';
  return { mod, shift: e.shiftKey, alt: e.altKey, key: normalizeKey(key) };
}

function sameChord(a: Chord, b: Chord): boolean {
  return a.mod === b.mod && a.shift === b.shift && a.alt === b.alt && a.key === b.key;
}

export class ShortcutRegistry {
  private commands = new Map<string, Command>();

  constructor(readonly platform: Platform = detectPlatform()) {}

  register(spec: CommandSpec): Command {
    if (this.commands.has(spec.id)) throw new Error(`command ${spec.id} registered twice`);
    const keys = spec.keys ?? [];
    keys.forEach(parseChord); // validate early
    const cmd: Command = {
      id: spec.id,
      label: spec.label,
      keys,
      scope: spec.scope ?? 'global',
      group: spec.group ?? 'General',
      run: spec.run ?? (() => {}),
      enabled: spec.enabled ?? (() => true),
    };
    for (const other of this.commands.values()) {
      const clash = other.keys.find((k) =>
        keys.some(
          (j) =>
            sameChord(parseChord(k), parseChord(j)) &&
            (other.scope === cmd.scope || other.scope === 'global' || cmd.scope === 'global'),
        ),
      );
      if (clash) throw new Error(`${spec.id} and ${other.id} both use ${clash}`);
    }
    this.commands.set(cmd.id, cmd);
    return cmd;
  }

  /** Replaces what a registered command does, keeping its keys and label. */
  bind(id: string, run: () => void, enabled?: () => boolean): void {
    const cmd = this.get(id);
    cmd.run = run;
    if (enabled) cmd.enabled = enabled;
  }

  get(id: string): Command {
    const cmd = this.commands.get(id);
    if (!cmd) throw new Error(`unknown command ${id}`);
    return cmd;
  }

  has(id: string): boolean {
    return this.commands.has(id);
  }

  all(): Command[] {
    return [...this.commands.values()];
  }

  /** The displayed shortcut of a command, or '' if it has none. */
  shortcut(id: string): string {
    const key = this.get(id).keys[0];
    return key ? formatChord(key, this.platform) : '';
  }

  /** `Label  Shortcut`, for tooltips and aria labels. */
  title(id: string): string {
    const s = this.shortcut(id);
    const label = this.get(id).label;
    return s ? `${label}  ${s}` : label;
  }

  /** The command a key event triggers in `scope`, if any. */
  match(e: KeyboardEvent, scope: Scope): Command | null {
    const chord = eventChord(e, this.platform);
    if (!chord) return null;
    for (const cmd of this.commands.values()) {
      if (cmd.scope !== 'global' && cmd.scope !== scope) continue;
      if (cmd.keys.some((k) => sameChord(parseChord(k), chord))) return cmd;
    }
    return null;
  }

  /** Runs the matching command. Returns true when the event was handled. */
  handle(e: KeyboardEvent, scope: Scope): boolean {
    const target = e.target as HTMLElement | null;
    if (target?.closest('input, textarea, [contenteditable="true"]')) return false;
    const cmd = this.match(e, scope);
    if (!cmd || !cmd.enabled()) return false;
    e.preventDefault();
    cmd.run();
    return true;
  }
}

export function detectPlatform(): Platform {
  if (typeof navigator === 'undefined') return 'other';
  return /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent) ? 'mac' : 'other';
}
