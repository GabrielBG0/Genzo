<script lang="ts">
  import { useRegistry } from '../shortcuts/context';
  import Icon from './Icon.svelte';
  import type { IconName } from './icons';

  export type Module = 'library' | 'develop' | 'export';

  interface QuickAction {
    command: string;
    icon: IconName;
  }

  interface Props {
    module: Module;
    catalog?: string;
    /** Library is disabled until M2. */
    libraryEnabled?: boolean;
    /** Groups of quick actions, separated by dividers (APP-03). */
    quickActions?: QuickAction[][];
    onmodule?: (m: Module) => void;
    onmenu?: () => void;
    onsearch?: () => void;
  }

  const DEVELOP_ACTIONS: QuickAction[][] = [
    [
      { command: 'edit.undo', icon: 'undo' },
      { command: 'edit.redo', icon: 'redo' },
    ],
    [
      { command: 'develop.beforeAfter', icon: 'beforeAfter' },
      { command: 'develop.resetAll', icon: 'reset' },
    ],
    [{ command: 'module.export', icon: 'export' }],
  ];

  let {
    module,
    catalog = 'No folder open',
    libraryEnabled = false,
    quickActions = DEVELOP_ACTIONS,
    onmodule,
    onmenu,
    onsearch,
  }: Props = $props();

  const registry = useRegistry();
  const modules: { id: Module; command: string }[] = [
    { id: 'library', command: 'module.library' },
    { id: 'develop', command: 'module.develop' },
    { id: 'export', command: 'module.export' },
  ];

  function run(id: string) {
    const cmd = registry.get(id);
    if (cmd.enabled()) cmd.run();
  }
</script>

<header class="topbar">
  <button
    class="app-menu"
    aria-label={registry.title('app.menu')}
    title={registry.title('app.menu')}
    onclick={() => onmenu?.()}
  >
    <span class="logo lens"><Icon name="logo" stroke={2} /></span>
    <span class="name">Genzo</span>
    <span class="catalog">{catalog}</span>
    <Icon name="chevronDown" size={11} stroke={2.4} />
  </button>

  <div class="quick" role="toolbar" aria-label="Quick actions">
    {#each quickActions as group, g (g)}
      {#if g > 0}<span class="sep"></span>{/if}
      {#each group as action (action.command)}
        {@const cmd = registry.get(action.command)}
        <button
          class="qa"
          aria-label={registry.title(action.command)}
          title={registry.title(action.command)}
          disabled={!cmd.enabled()}
          onclick={() => run(action.command)}
        >
          <Icon name={action.icon} />
        </button>
      {/each}
    {/each}
  </div>

  <span class="spacer"></span>
  <button class="search pane" title={registry.title('app.search')} onclick={() => onsearch?.()}>
    <Icon name="search" />
    <span class="placeholder">Search photos, presets and commands</span>
    <kbd class="mono">{registry.shortcut('app.search')}</kbd>
  </button>
  <span class="spacer"></span>

  <nav aria-label="Modules">
    {#each modules as m (m.id)}
      <button
        class={['tab', m.id === module && 'lens']}
        aria-current={m.id === module ? 'page' : undefined}
        title={registry.title(m.command)}
        disabled={m.id === 'library' && !libraryEnabled}
        onclick={() => onmodule?.(m.id)}
      >
        {registry.get(m.command).label}
      </button>
    {/each}
  </nav>
</header>

<style>
  .topbar {
    height: var(--topbar-h);
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 10px;
    background: var(--bg);
    color: var(--text-body);
  }

  .app-menu {
    flex: none;
    height: 34px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 8px 0 4px;
    border-radius: 10px;
    border: 1px solid transparent;
    color: var(--text-muted);
  }

  .app-menu:hover {
    background: var(--hover);
  }

  .logo {
    width: 26px;
    height: 26px;
    border-radius: 8px;
    display: grid;
    place-items: center;
  }

  .name {
    font-weight: 600;
    color: var(--text-strong);
  }

  .catalog {
    color: var(--text-muted);
  }

  .quick {
    flex: none;
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 2px;
    border-radius: var(--r-pill);
    background: var(--trough);
  }

  .qa {
    height: 28px;
    min-width: 30px;
    padding: 0 8px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--r-pill);
    border: 1px solid transparent;
    color: var(--text-body);
  }

  .qa:hover:not(:disabled) {
    background: var(--hover);
  }

  .qa:disabled {
    color: var(--text-dim);
    cursor: default;
  }

  .sep {
    width: 1px;
    height: 16px;
    margin: 0 3px;
    background: var(--separator);
  }

  .spacer {
    flex: 1;
  }

  .search {
    flex: 0 1 380px;
    min-width: 160px;
    height: 30px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 10px;
    border-radius: var(--r-pill);
    color: var(--text-secondary);
  }

  .placeholder {
    flex: 1;
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  kbd {
    font-size: var(--text-caption);
    padding: 1px 6px;
    border: 1px solid var(--separator);
    border-radius: var(--r-pill);
  }

  nav {
    flex: none;
    display: flex;
    gap: 2px;
  }

  .tab {
    height: 28px;
    padding: 0 11px;
    border-radius: var(--r-pill);
    border: 1px solid transparent;
    color: var(--text-muted);
  }

  .tab:disabled {
    color: var(--text-dim);
    cursor: default;
  }
</style>
