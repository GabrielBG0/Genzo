import { describe, expect, it, vi } from 'vitest';
import { createRegistry, DEFAULT_COMMANDS } from './commands';
import { formatChord, ShortcutRegistry } from './registry';

function key(init: KeyboardEventInit & { code?: string }): KeyboardEvent {
  return new KeyboardEvent('keydown', init);
}

describe('formatChord', () => {
  it('formats like the handoff', () => {
    expect(formatChord('Mod+Shift+Z', 'other')).toBe('Ctrl Shift Z');
    expect(formatChord('Mod+Shift+Z', 'mac')).toBe('⌘⇧Z');
    expect(formatChord('Mod+K', 'other')).toBe('Ctrl K');
    expect(formatChord('F6', 'other')).toBe('F6');
    expect(formatChord('\\', 'other')).toBe('\\');
  });
});

describe('ShortcutRegistry', () => {
  it('registers every default command without clashes', () => {
    const r = createRegistry();
    expect(r.all()).toHaveLength(DEFAULT_COMMANDS.length);
    expect(r.title('edit.undo')).toBe('Undo  Ctrl Z');
  });

  it('refuses duplicate ids and clashing keys', () => {
    const r = new ShortcutRegistry('other');
    r.register({ id: 'a', label: 'A', keys: ['Mod+Z'] });
    expect(() => r.register({ id: 'a', label: 'A' })).toThrow(/twice/);
    expect(() => r.register({ id: 'b', label: 'B', keys: ['Mod+Z'] })).toThrow(/both use/);
  });

  it('lets different scopes share a key', () => {
    const r = new ShortcutRegistry('other');
    r.register({ id: 'd', label: 'D', keys: ['R'], scope: 'develop' });
    expect(() => r.register({ id: 'l', label: 'L', keys: ['R'], scope: 'library' })).not.toThrow();
  });

  it('matches events by physical key and scope', () => {
    const r = new ShortcutRegistry('other');
    const undo = vi.fn();
    const crop = vi.fn();
    r.register({ id: 'undo', label: 'Undo', keys: ['Mod+Z'], run: undo });
    r.register({ id: 'crop', label: 'Crop', keys: ['R'], scope: 'develop', run: crop });
    expect(r.handle(key({ key: 'z', code: 'KeyZ', ctrlKey: true }), 'library')).toBe(true);
    expect(undo).toHaveBeenCalledOnce();
    expect(r.handle(key({ key: 'r', code: 'KeyR' }), 'library')).toBe(false);
    expect(r.handle(key({ key: 'r', code: 'KeyR' }), 'develop')).toBe(true);
    expect(crop).toHaveBeenCalledOnce();
    // Shift changes e.key but not the physical key.
    expect(r.match(key({ key: 'Z', code: 'KeyZ', ctrlKey: true, shiftKey: true }), 'develop')).toBeNull();
  });

  it('uses Cmd as Mod on macOS', () => {
    const r = new ShortcutRegistry('mac');
    r.register({ id: 'undo', label: 'Undo', keys: ['Mod+Z'] });
    expect(r.match(key({ key: 'z', code: 'KeyZ', metaKey: true }), 'develop')?.id).toBe('undo');
    expect(r.match(key({ key: 'z', code: 'KeyZ', ctrlKey: true }), 'develop')).toBeNull();
  });

  it('skips disabled commands and text fields', () => {
    const r = new ShortcutRegistry('other');
    const run = vi.fn();
    r.register({ id: 'x', label: 'X', keys: ['X'], run, enabled: () => false });
    expect(r.handle(key({ key: 'x', code: 'KeyX' }), 'develop')).toBe(false);
    r.bind('x', run, () => true);
    const input = document.createElement('input');
    document.body.append(input);
    const e = new KeyboardEvent('keydown', { key: 'x', code: 'KeyX', bubbles: true });
    let handled: boolean | undefined;
    input.addEventListener('keydown', (ev) => (handled = r.handle(ev, 'develop')));
    input.dispatchEvent(e);
    expect(handled).toBe(false);
    expect(run).not.toHaveBeenCalled();
  });
});
