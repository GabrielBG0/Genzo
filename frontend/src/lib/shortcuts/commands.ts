import { ShortcutRegistry, type CommandSpec } from './registry';

/**
 * Every command the M0 shell knows about. Most are placeholders until their
 * milestone wires them (`enabled` returns false), but they are registered
 * now so menus, tooltips and quick actions read their shortcuts from here.
 */
export const DEFAULT_COMMANDS: CommandSpec[] = [
  // App
  { id: 'app.menu', label: 'App menu', keys: ['Alt'], group: 'App' },
  { id: 'app.search', label: 'Search photos, presets and commands', keys: ['Mod+K'], group: 'App' },
  { id: 'app.shortcuts', label: 'Keyboard shortcuts', keys: ['Mod+/'], group: 'App' },
  { id: 'module.library', label: 'Library', keys: ['G'], group: 'Modules' },
  { id: 'module.develop', label: 'Develop', keys: ['D'], group: 'Modules' },
  { id: 'module.export', label: 'Export', keys: ['Mod+Shift+E'], group: 'Modules' },
  // Edit
  { id: 'edit.undo', label: 'Undo', keys: ['Mod+Z'], group: 'Edit' },
  { id: 'edit.redo', label: 'Redo', keys: ['Mod+Shift+Z'], group: 'Edit' },
  { id: 'settings.copy', label: 'Copy settings', keys: ['Mod+Shift+C'], group: 'Edit' },
  { id: 'settings.paste', label: 'Paste settings', keys: ['Mod+Shift+V'], group: 'Edit' },
  // Develop
  { id: 'develop.beforeAfter', label: 'Before / after', keys: ['\\'], scope: 'develop', group: 'Develop' },
  { id: 'develop.resetAll', label: 'Reset all', keys: ['Mod+Shift+R'], scope: 'develop', group: 'Develop' },
  { id: 'develop.crop', label: 'Crop', keys: ['R'], scope: 'develop', group: 'Develop' },
  { id: 'develop.remove', label: 'Remove', keys: ['Q'], scope: 'develop', group: 'Develop' },
  { id: 'develop.masking', label: 'Masking', keys: ['Shift+W'], scope: 'develop', group: 'Develop' },
  { id: 'develop.clipping', label: 'Show clipping', keys: ['J'], scope: 'develop', group: 'Develop' },
  // View (APP-08)
  { id: 'view.sidePanels', label: 'Hide side panels', keys: ['Tab'], group: 'View' },
  { id: 'view.allPanels', label: 'Hide all panels', keys: ['Shift+Tab'], group: 'View' },
  { id: 'view.filmstrip', label: 'Show filmstrip', keys: ['F6'], group: 'View' },
  { id: 'view.leftPanel', label: 'Show left panel', keys: ['F7'], group: 'View' },
  { id: 'view.rightPanel', label: 'Show right panel', keys: ['F8'], group: 'View' },
];

export function createRegistry(): ShortcutRegistry {
  const registry = new ShortcutRegistry();
  for (const spec of DEFAULT_COMMANDS) {
    registry.register({ enabled: () => false, ...spec });
  }
  return registry;
}
