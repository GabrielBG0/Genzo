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
    onclose?: () => void;
  }
  let { items, label = '', width = 260, onclose }: Props = $props();
  const registry = useRegistry();

  function shortcutOf(item: Exclude<MenuItem, { separator: true }>): string {
    if (item.shortcut !== undefined) return item.shortcut;
    return item.command && registry.has(item.command) ? registry.shortcut(item.command) : '';
  }

  function select(item: Exclude<MenuItem, { separator: true }>) {
    if (item.disabled || item.submenu) return;
    if (item.onselect) item.onselect();
    else if (item.command) registry.get(item.command).run();
    onclose?.();
  }
</script>

<div class="menu" role="menu" aria-label={label} style:width="{width}px">
  {#each items as item, i (i)}
    {#if item.separator}
      <div class="sep" role="separator"></div>
    {:else}
      <button
        class={['item', item.hot && 'lens', item.danger && 'danger']}
        role={item.check === undefined ? 'menuitem' : 'menuitemcheckbox'}
        aria-checked={item.check === undefined ? undefined : item.check === 'mixed' ? 'mixed' : item.check}
        aria-haspopup={item.submenu ? 'menu' : undefined}
        disabled={item.disabled}
        onclick={() => select(item)}
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

  .sep {
    height: 1px;
    margin: 4px 8px;
    background: var(--line);
  }
</style>
