<script lang="ts" module>
  export type MenuItem =
    | { separator: true }
    | {
        separator?: false;
        label: string;
        /** Registered command; supplies the shortcut and the action. */
        command?: string;
        /** Explicit shortcut text, for items without a command. */
        shortcut?: string;
        /** true, false, or 'mixed' for a selection with mixed states ("–"). */
        check?: boolean | 'mixed';
        submenu?: boolean;
        hot?: boolean;
        danger?: boolean;
        disabled?: boolean;
        onselect?: () => void;
      };
</script>

<script lang="ts">
  import { useRegistry } from '../shortcuts/context';
  import { fadeOut } from './fadeOut';
  import { slidingLens } from './indicator.svelte';

  interface Props {
    items: MenuItem[];
    label?: string;
    width?: number;
    /** A header naming the target and its value (APP-06's Decided style). */
    header?: { name: string; value: string } | null;
    /** Focus the first enabled item when shown, and move with ↑ ↓; Esc closes. */
    autofocus?: boolean;
    /** Esc, or an item chosen when `onchosen` is not given. */
    onclose?: () => void;
    /** After an item ran (the app menu closes as a whole; APP-02). */
    onchosen?: () => void;
    /**
     * A submenu item is opened: clicked, pointed at, or → or Enter on it
     * (`byKey`). `anchor` is its button, to place the submenu beside.
     */
    onsubmenu?: (index: number, anchor: HTMLElement, byKey: boolean) => void;
    /** The pointer is on an item that is not a submenu. */
    onpoint?: (index: number) => void;
    /** ← in the menu: a submenu goes back to its parent. */
    onleft?: () => void;
  }
  let {
    items,
    label = '',
    width = 260,
    header = null,
    autofocus = false,
    onclose,
    onchosen,
    onsubmenu,
    onpoint,
    onleft,
  }: Props = $props();
  const registry = useRegistry();
  let menuEl: HTMLDivElement | undefined = $state();

  /** Items under the pointer and with focus, by index in `items`. */
  let pointed: number | null = $state(null);
  let focused: number | null = $state(null);
  /**
   * The lit item, where the one highlight sits: the one pointed at, else an
   * open submenu's item, else the focused one.
   */
  const lit = $derived.by(() => {
    const hot = items.findIndex((it) => !it.separator && it.hot);
    const at = pointed ?? (hot >= 0 ? hot : null) ?? focused;
    if (at === null) return null;
    const item = items[at];
    return item && !item.separator && !item.disabled ? at : null;
  });
  /** The lit item's place among the buttons, for the highlight. */
  const litButton = $derived(lit === null ? null : items.slice(0, lit).filter((it) => !it.separator).length);

  function indexOf(e: Event): number | null {
    const el = e.target instanceof Element ? e.target.closest<HTMLElement>('button.item') : null;
    return el ? Number(el.dataset.index) : null;
  }

  function enabledItems(): HTMLButtonElement[] {
    return menuEl ? [...menuEl.querySelectorAll<HTMLButtonElement>('button.item:not(:disabled)')] : [];
  }

  $effect(() => {
    if (autofocus) enabledItems()[0]?.focus();
  });

  // Closes with a 90 ms fade, of a copy (`fadeOut`).
  $effect(() => {
    const el = menuEl;
    return () => fadeOut(el);
  });

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      onclose?.();
      return;
    }
    if (e.key === 'ArrowLeft' && onleft) {
      e.preventDefault();
      e.stopPropagation();
      onleft();
      return;
    }
    if (e.key === 'ArrowRight' || e.key === 'Enter') {
      const at = Number((document.activeElement as HTMLElement | null)?.dataset.index);
      const item = items[at];
      if (item && !item.separator && item.submenu && !item.disabled) {
        e.preventDefault();
        e.stopPropagation();
        onsubmenu?.(at, document.activeElement as HTMLElement, true);
      }
      return;
    }
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    e.preventDefault();
    e.stopPropagation();
    const all = enabledItems();
    if (all.length === 0) return;
    const at = all.indexOf(document.activeElement as HTMLButtonElement);
    const next = e.key === 'ArrowDown' ? (at + 1) % all.length : (at - 1 + all.length) % all.length;
    all[next]?.focus();
  }

  /**
   * A shortcut's modifiers and its key, shown apart (APP-02's Decided
   * style): "Ctrl Shift" and "E", or "⌘⇧" and "E".
   */
  function splitShortcut(s: string): { mods: string; key: string } {
    if (s.includes(' ')) {
      const at = s.lastIndexOf(' ');
      return { mods: s.slice(0, at + 1), key: s.slice(at + 1) };
    }
    const m = /^([⌘⇧⌥⌃]*)(.*)$/u.exec(s);
    return { mods: m?.[1] ?? '', key: m?.[2] ?? s };
  }

  function shortcutOf(item: Exclude<MenuItem, { separator: true }>): string {
    if (item.shortcut !== undefined) return item.shortcut;
    return item.command && registry.has(item.command) ? registry.shortcut(item.command) : '';
  }

  function select(item: Exclude<MenuItem, { separator: true }>, i: number, el: HTMLElement) {
    if (item.disabled) return;
    if (item.submenu) {
      onsubmenu?.(i, el, false);
      return;
    }
    // Closed first: a command may open a dialog that takes focus.
    (onchosen ?? onclose)?.();
    if (item.onselect) item.onselect();
    else if (item.command) registry.get(item.command).run();
  }
</script>

<div
  class="menu"
  role="menu"
  tabindex="-1"
  aria-label={label}
  style:width="{width}px"
  bind:this={menuEl}
  {onkeydown}
  onpointerleave={() => (pointed = null)}
  onfocusin={(e) => (focused = indexOf(e))}
  onfocusout={() => (focused = null)}
  {@attach slidingLens(() => litButton, 'menu')}
>
  {#if header}
    <div class="header" role="presentation">
      <span>{header.name}</span><span class="header-value mono">{header.value}</span>
    </div>
  {/if}
  {#each items as item, i (i)}
    {#if item.separator}
      <div class="sep" role="separator"></div>
    {:else}
      {@const sc = splitShortcut(shortcutOf(item))}
      <button
        data-segment
        class={['item', i === lit && 'lit', item.danger && 'danger']}
        role={item.check === undefined ? 'menuitem' : 'menuitemcheckbox'}
        aria-checked={item.check === undefined ? undefined : item.check === 'mixed' ? 'mixed' : item.check}
        aria-haspopup={item.submenu ? 'menu' : undefined}
        aria-expanded={item.submenu ? (item.hot ?? false) : undefined}
        disabled={item.disabled}
        data-index={i}
        onclick={(e) => select(item, i, e.currentTarget)}
        onpointerenter={(e) => {
          pointed = item.disabled ? null : i;
          if (item.submenu) onsubmenu?.(i, e.currentTarget, false);
          else onpoint?.(i);
        }}
      >
        <span class="check">{item.check === true ? '✓' : item.check === 'mixed' ? '–' : ''}</span>
        <span class="label">{item.label}</span>
        <span class="shortcut mono"><span class="mods">{sc.mods}</span>{sc.key}</span>
        <span class="sub">{item.submenu ? '›' : ''}</span>
      </button>
    {/if}
  {/each}
</div>

<style>
  /* Opens from its anchor (the top left corner) in 140 ms. */
  .menu {
    transform-origin: top left;
    animation: menu-open var(--t-menu-in) ease-out;
    max-width: calc(100vw - 48px);
    padding: 5px;
    border-radius: var(--r-menu);
    background: var(--dialog-raised);
    border: 1px solid var(--menu-border-color);
    box-shadow: var(--menu-shadow);
    color: var(--text-body);
    font-size: var(--text-base);
  }

  .item {
    width: 100%;
    height: var(--row-menu);
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 10px 0 8px;
    border-radius: var(--r-menu-item);
    border: 1px solid transparent;
    text-align: left;
  }

  /* The highlight is the menu's one sliding lens; the lit item's text follows it. */
  .item {
    transition: color 50ms ease-out;
  }

  .item.lit {
    color: var(--text-on-lens);
  }

  .item:disabled {
    color: var(--text-dim);
    cursor: default;
  }

  .danger {
    color: var(--danger);
  }

  .check {
    width: 14px;
    flex: none;
    text-align: center;
    font-size: var(--text-caption);
    color: var(--accent-soft);
  }

  .label {
    flex: 1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .shortcut {
    font-size: var(--text-mono-small);
    color: var(--text-muted);
    white-space: nowrap;
  }

  .mods {
    color: var(--text-dim);
  }

  .sub {
    width: 10px;
    flex: none;
    color: var(--text-muted);
  }

  .header {
    display: flex;
    gap: 8px;
    padding: 4px 10px 6px 30px;
    font-size: var(--text-caption);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-muted);
  }

  .header-value {
    color: var(--accent-soft);
    text-transform: none;
  }

  .item:focus-visible {
    outline: none;
  }

  @keyframes menu-open {
    from {
      opacity: 0;
      transform: translateY(-4px) scale(0.96);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .menu {
      animation: none;
    }
  }

  /* Inset to the labels (APP-02's Decided style). */
  .sep {
    height: 1px;
    margin: 4px 8px 4px 30px;
    background: var(--line);
  }
</style>
