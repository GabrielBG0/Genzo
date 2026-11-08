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

  function enabledItems(): HTMLButtonElement[] {
    return menuEl ? [...menuEl.querySelectorAll<HTMLButtonElement>('button.item:not(:disabled)')] : [];
  }

  $effect(() => {
    if (autofocus) enabledItems()[0]?.focus();
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

<div class="menu" role="menu" tabindex="-1" aria-label={label} style:width="{width}px" bind:this={menuEl} {onkeydown}>
  {#if header}
    <div class="header" role="presentation">
      <span>{header.name}</span><span class="header-value mono">{header.value}</span>
    </div>
  {/if}
  {#each items as item, i (i)}
    {#if item.separator}
      <div class="sep" role="separator"></div>
    {:else}
      <button
        class={['item', item.hot && 'lens', item.danger && 'danger']}
        role={item.check === undefined ? 'menuitem' : 'menuitemcheckbox'}
        aria-checked={item.check === undefined ? undefined : item.check === 'mixed' ? 'mixed' : item.check}
        aria-haspopup={item.submenu ? 'menu' : undefined}
        aria-expanded={item.submenu ? (item.hot ?? false) : undefined}
        disabled={item.disabled}
        data-index={i}
        onclick={(e) => select(item, i, e.currentTarget)}
        onpointerenter={(e) => (item.submenu ? onsubmenu?.(i, e.currentTarget, false) : onpoint?.(i))}
      >
        <span class="check">{item.check === true ? '✓' : item.check === 'mixed' ? '–' : ''}</span>
        <span class="label">{item.label}</span>
        <span class="shortcut mono">{shortcutOf(item)}</span>
        <span class="sub">{item.submenu ? '›' : ''}</span>
      </button>
    {/if}
  {/each}
</div>

<style>
  .menu {
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

  .item:hover:not(:disabled, .lens) {
    background: var(--lens-bg);
    border-color: var(--lens-border-color);
    box-shadow: var(--lens-shadow);
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
    background: var(--lens-bg);
    border-color: var(--lens-border-color);
    color: var(--text-on-lens);
  }

  .sep {
    height: 1px;
    margin: 4px 8px;
    background: var(--line);
  }
</style>
