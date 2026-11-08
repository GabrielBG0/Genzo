import type { ShortcutRegistry } from './shortcuts/registry';

/** A side of the window with collapsible panels. */
export type Side = 'left' | 'right';

/** The modifiers a panel header's click carries (APP-08's solo mode). */
export interface HeaderClick {
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
}

/**
 * Which parts of the window show, and which panels are open (APP-08). For
 * the session: Tab, Shift Tab, F6, F7 and F8 hide and show the side panels
 * and the filmstrip, and each side has a solo mode in which one panel is
 * open at a time.
 */
export class Layout {
  left = $state(true);
  right = $state(true);
  filmstrip = $state(true);
  /** Each side's solo mode. */
  solo: Record<Side, boolean> = $state({ left: false, right: false });
  /** Each panel's side and whether it is open, by id. */
  private panels: Record<string, { side: Side; open: boolean }> = $state({});

  constructor(private readonly mac = false) {}

  /** Adds a panel, open unless said; a panel added twice keeps its state. */
  register(id: string, side: Side, open = true): void {
    this.panels[id] ??= { side, open };
  }

  isOpen(id: string): boolean {
    return this.panels[id]?.open ?? false;
  }

  /**
   * A click on a panel's header: Alt turns its side's solo mode on or off,
   * Ctrl (Cmd on macOS) opens or closes all of the side's panels, and a
   * plain click opens or closes it, closing the others in solo mode.
   */
  click(id: string, e: HeaderClick): void {
    const panel = this.panels[id];
    if (!panel) return;
    const side = this.sideOf(panel.side);
    if (e.altKey) {
      this.solo[panel.side] = !this.solo[panel.side];
      if (this.solo[panel.side]) for (const [other, p] of side) p.open = other === id;
      return;
    }
    if (this.mac ? e.metaKey : e.ctrlKey) {
      const open = !panel.open;
      for (const [, p] of side) p.open = open;
      return;
    }
    panel.open = !panel.open;
    if (panel.open && this.solo[panel.side]) for (const [other, p] of side) if (other !== id) p.open = false;
  }

  /** Tab: both side panels hide if either shows, else both show. */
  toggleSides(): void {
    const show = !(this.left || this.right);
    this.left = show;
    this.right = show;
  }

  /** Shift Tab: the side panels and the filmstrip, all hidden if any shows, else all shown. */
  toggleAll(): void {
    const show = !(this.left || this.right || this.filmstrip);
    this.left = show;
    this.right = show;
    this.filmstrip = show;
  }

  private sideOf(side: Side): [string, { side: Side; open: boolean }][] {
    return Object.entries(this.panels).filter(([, p]) => p.side === side);
  }
}

/**
 * Binds the View commands to `layout` (APP-08), their labels following it:
 * "Hide left panel" while it shows, "Show left panel" while it is hidden.
 */
export function bindLayout(registry: ShortcutRegistry, layout: Layout, enabled: () => boolean): void {
  const label = (shown: () => boolean, what: string) => () => `${shown() ? 'Hide' : 'Show'} ${what}`;
  registry.bind(
    'view.sidePanels',
    () => layout.toggleSides(),
    enabled,
    label(() => layout.left || layout.right, 'side panels'),
  );
  registry.bind(
    'view.allPanels',
    () => layout.toggleAll(),
    enabled,
    label(() => layout.left || layout.right || layout.filmstrip, 'all panels'),
  );
  registry.bind(
    'view.filmstrip',
    () => (layout.filmstrip = !layout.filmstrip),
    enabled,
    label(() => layout.filmstrip, 'filmstrip'),
  );
  registry.bind('view.leftPanel', () => (layout.left = !layout.left), enabled, label(() => layout.left, 'left panel'));
  registry.bind('view.rightPanel', () => (layout.right = !layout.right), enabled, label(() => layout.right, 'right panel'));
}
