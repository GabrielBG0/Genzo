import { describe, expect, it } from 'vitest';
import { createRegistry } from './shortcuts/commands';
import { bindLayout, Layout } from './layout.svelte';

const click = { altKey: false, ctrlKey: false, metaKey: false };
const alt = { ...click, altKey: true };
const ctrl = { ...click, ctrlKey: true };

function threePanels(mac = false) {
  const layout = new Layout(mac);
  layout.register('basic', 'right');
  layout.register('curve', 'right', false);
  layout.register('detail', 'right', false);
  layout.register('navigator', 'left');
  return layout;
}

describe('Layout (APP-08)', () => {
  it('Tab hides both side panels if either shows, and shows both if both are hidden', () => {
    const layout = new Layout();
    layout.toggleSides();
    expect([layout.left, layout.right, layout.filmstrip]).toEqual([false, false, true]);
    layout.toggleSides();
    expect([layout.left, layout.right]).toEqual([true, true]);
    layout.left = false;
    layout.toggleSides();
    expect([layout.left, layout.right]).toEqual([false, false]);
  });

  it('Shift Tab hides the side panels and the filmstrip, and shows them all again', () => {
    const layout = new Layout();
    layout.toggleAll();
    expect([layout.left, layout.right, layout.filmstrip]).toEqual([false, false, false]);
    layout.toggleAll();
    expect([layout.left, layout.right, layout.filmstrip]).toEqual([true, true, true]);
    layout.left = false;
    layout.right = false;
    layout.toggleAll();
    expect(layout.filmstrip).toBe(false);
  });

  it('a click opens or closes a panel; registering again keeps its state', () => {
    const layout = threePanels();
    layout.click('basic', click);
    expect(layout.isOpen('basic')).toBe(false);
    layout.register('basic', 'right');
    expect(layout.isOpen('basic')).toBe(false);
    expect(layout.isOpen('unknown')).toBe(false);
  });

  it('solo mode: Alt-click leaves only that panel open, and opening another closes it', () => {
    const layout = threePanels();
    layout.click('curve', click);
    expect(layout.isOpen('curve')).toBe(true);
    layout.click('curve', alt);
    expect(layout.solo.right).toBe(true);
    expect(layout.solo.left).toBe(false);
    expect([layout.isOpen('basic'), layout.isOpen('curve'), layout.isOpen('detail')]).toEqual([false, true, false]);
    layout.click('detail', click);
    expect([layout.isOpen('curve'), layout.isOpen('detail')]).toEqual([false, true]);
    // The other side is not touched.
    expect(layout.isOpen('navigator')).toBe(true);
    // Off again: opening one leaves the others.
    layout.click('detail', alt);
    layout.click('basic', click);
    expect([layout.isOpen('basic'), layout.isOpen('detail')]).toEqual([true, true]);
  });

  it('Ctrl-click opens all of a side if the panel is closed, and closes all if it is open', () => {
    const layout = threePanels();
    layout.click('curve', ctrl);
    expect([layout.isOpen('basic'), layout.isOpen('curve'), layout.isOpen('detail')]).toEqual([true, true, true]);
    layout.click('basic', ctrl);
    expect([layout.isOpen('basic'), layout.isOpen('curve'), layout.isOpen('detail')]).toEqual([false, false, false]);
    expect(layout.isOpen('navigator')).toBe(true);
    // Cmd on macOS; Ctrl there is a plain click.
    const mac = threePanels(true);
    mac.click('curve', { ...click, metaKey: true });
    expect(mac.isOpen('detail')).toBe(true);
    mac.click('basic', ctrl);
    expect([mac.isOpen('basic'), mac.isOpen('detail')]).toEqual([false, true]);
  });
});

describe('bindLayout (APP-08)', () => {
  it('runs the View commands, their labels following the layout', () => {
    const registry = createRegistry();
    const layout = new Layout();
    let enabled = true;
    bindLayout(registry, layout, () => enabled);
    expect(registry.label('view.leftPanel')).toBe('Hide left panel');
    registry.get('view.leftPanel').run();
    expect(layout.left).toBe(false);
    expect(registry.label('view.leftPanel')).toBe('Show left panel');
    expect(registry.label('view.sidePanels')).toBe('Hide side panels');
    registry.get('view.rightPanel').run();
    expect(registry.label('view.sidePanels')).toBe('Show side panels');
    registry.get('view.filmstrip').run();
    expect(registry.label('view.filmstrip')).toBe('Show filmstrip');
    expect(registry.label('view.allPanels')).toBe('Show all panels');
    registry.get('view.allPanels').run();
    expect([layout.left, layout.right, layout.filmstrip]).toEqual([true, true, true]);
    // Under a dialog: disabled.
    enabled = false;
    expect(registry.get('view.sidePanels').enabled()).toBe(false);
  });
});
