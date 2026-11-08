<script lang="ts">
  import { useRegistry } from '../shortcuts/context';
  import Menu, { type MenuItem } from './Menu.svelte';

  interface Props {
    /** The open folder's name, the menu's header; null before one. */
    folder: string | null;
    /** Where the menu's top left corner goes, in the window. */
    at: { x: number; y: number };
    /** Checks by command id: the View parts shown, toggles that are on. */
    checks: Record<string, boolean | 'mixed'>;
    onclose: () => void;
  }
  let { folder, at, checks, onclose }: Props = $props();
  const registry = useRegistry();

  /** A registered command as an item: its label, shortcut and state from the registry. */
  function cmd(id: string, label?: string): MenuItem {
    return {
      label: label ?? registry.label(id),
      command: id,
      disabled: !registry.get(id).enabled(),
      ...(id in checks ? { check: checks[id] ?? false } : {}),
    };
  }
  const sep: MenuItem = { separator: true };

  const SUBMENUS: { label: string; items: () => MenuItem[] }[] = [
    {
      label: 'File',
      items: () => [
        cmd('file.openFolder', 'Open folder…'),
        cmd('module.export', 'Export…'),
        ...(registry.get('export.cancel').enabled() ? [cmd('export.cancel')] : []),
      ],
    },
    { label: 'Edit', items: () => [cmd('edit.undo'), cmd('edit.redo')] },
    { label: 'Photo', items: () => [cmd('photo.previous'), cmd('photo.next')] },
    {
      label: 'Develop',
      items: () => [
        cmd('develop.beforeAfter'),
        cmd('develop.resetAll'),
        sep,
        cmd('develop.treatment'),
        cmd('develop.picker'),
        cmd('develop.clipping'),
      ],
    },
    {
      label: 'View',
      items: () => [
        cmd('view.sidePanels', 'Side panels'),
        cmd('view.allPanels', 'All panels'),
        cmd('view.filmstrip', 'Filmstrip'),
        cmd('view.leftPanel', 'Left panel'),
        cmd('view.rightPanel', 'Right panel'),
      ],
    },
    { label: 'Help', items: () => [cmd('help.about', 'About Genzo')] },
  ];
  /** Where the submenus start in the main menu's items. */
  const FIRST_SUB = 3;

  /** An open submenu: its index in `SUBMENUS`, where it goes, and whether the keyboard opened it. */
  interface OpenSub {
    index: number;
    x: number;
    y: number;
    byKey: boolean;
  }
  let sub: OpenSub | null = $state(null);
  let mainEl: HTMLDivElement | undefined = $state();

  const main: MenuItem[] = $derived([
    cmd('file.openFolder', 'Open folder…'),
    cmd('module.export', 'Export…'),
    sep,
    ...SUBMENUS.map((s, i): MenuItem => ({ label: s.label, submenu: true, hot: sub?.index === i })),
    sep,
    cmd('app.quit'),
  ]);
  const subMenu = $derived(SUBMENUS[(sub as OpenSub | null)?.index ?? -1]);
  const subItems: MenuItem[] = $derived(subMenu?.items() ?? []);

  function openSub(i: number, anchor: HTMLElement, byKey: boolean) {
    const r = anchor.getBoundingClientRect();
    // Beside its item, the menus' padding (5 px) lined up.
    sub = { index: i - FIRST_SUB, x: r.right + 9, y: r.top - 6, byKey };
  }

  /** ← or Esc in a submenu: back to its item in the main menu. */
  function back() {
    if (!sub) return;
    const index = sub.index + FIRST_SUB;
    sub = null;
    mainEl?.querySelector<HTMLElement>(`button[data-index="${index}"]`)?.focus();
  }

  function outside(e: PointerEvent) {
    if (!(e.target instanceof Element && e.target.closest('.appmenu-pop, .app-menu'))) onclose();
  }
</script>

<svelte:window onpointerdown={outside} />

<div class="appmenu-pop" style:left="{at.x}px" style:top="{at.y}px" bind:this={mainEl}>
  <Menu
    items={main}
    label="App menu"
    width={240}
    header={{ name: folder ?? 'No folder open', value: '' }}
    autofocus
    {onclose}
    onchosen={onclose}
    onsubmenu={openSub}
    onpoint={() => (sub = null)}
  />
</div>
{#if sub}
  {#key sub.index}
    <div class="appmenu-pop" style:left="{sub.x}px" style:top="{sub.y}px">
      <Menu
        items={subItems}
        label={subMenu?.label ?? ''}
        width={260}
        autofocus={sub.byKey}
        onclose={back}
        onchosen={onclose}
        onleft={back}
      />
    </div>
  {/key}
{/if}

<style>
  .appmenu-pop {
    position: fixed;
    z-index: 30;
  }
</style>
