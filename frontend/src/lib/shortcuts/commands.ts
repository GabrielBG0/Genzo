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
  { id: 'app.quit', label: 'Quit', keys: ['Mod+Q'], group: 'App' },
  { id: 'help.about', label: 'About Genzo', group: 'Help' },
  { id: 'file.openFolder', label: 'Open folder', keys: ['Mod+O'], group: 'File' },
  { id: 'module.library', label: 'Library', keys: ['G'], group: 'Modules' },
  { id: 'module.develop', label: 'Develop', keys: ['D'], group: 'Modules' },
  { id: 'module.export', label: 'Export', keys: ['Mod+Shift+E'], group: 'Modules' },
  { id: 'export.cancel', label: 'Cancel export', group: 'File' },
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
  // The histogram's corner markers (DEV-03): no key of their own.
  { id: 'develop.clippingShadows', label: 'Shadow clipping', scope: 'develop', group: 'Develop' },
  { id: 'develop.clippingHighlights', label: 'Highlight clipping', scope: 'develop', group: 'Develop' },
  // Basic panel (DEV-02)
  { id: 'develop.treatment', label: 'Color / B&W', keys: ['V'], scope: 'develop', group: 'Develop' },
  { id: 'develop.picker', label: 'White balance picker', keys: ['W'], scope: 'develop', group: 'Develop' },
  { id: 'photo.previous', label: 'Previous photo', keys: ['ArrowLeft'], scope: 'develop', group: 'Photo' },
  { id: 'photo.next', label: 'Next photo', keys: ['ArrowRight'], scope: 'develop', group: 'Photo' },
  // Rotate and flip, and the crop tool's keys (DEV-12)
  { id: 'photo.rotateLeft', label: 'Rotate left', keys: ['Mod+['], scope: 'develop', group: 'Photo' },
  { id: 'photo.rotateRight', label: 'Rotate right', keys: ['Mod+]'], scope: 'develop', group: 'Photo' },
  { id: 'photo.flipHorizontal', label: 'Flip horizontal', scope: 'develop', group: 'Photo' },
  { id: 'photo.flipVertical', label: 'Flip vertical', scope: 'develop', group: 'Photo' },
  { id: 'crop.overlay', label: 'Next crop overlay', keys: ['O'], scope: 'develop', group: 'Develop' },
  { id: 'crop.swap', label: 'Swap the aspect', keys: ['X'], scope: 'develop', group: 'Develop' },
  { id: 'crop.straighten', label: 'Straighten tool', scope: 'develop', group: 'Develop' },
  { id: 'crop.done', label: 'Close', keys: ['Enter'], scope: 'develop', group: 'Develop' },
  // Esc: the picker, the straighten tool, or the crop tool (DEV-02, DEV-12).
  { id: 'develop.pickerCancel', label: 'Cancel', keys: ['Escape'], scope: 'develop', group: 'Develop' },
  // View (APP-08)
  { id: 'view.sidePanels', label: 'Hide side panels', keys: ['Tab'], group: 'View' },
  { id: 'view.allPanels', label: 'Hide all panels', keys: ['Shift+Tab'], group: 'View' },
  { id: 'view.filmstrip', label: 'Show filmstrip', keys: ['F6'], group: 'View' },
  { id: 'view.leftPanel', label: 'Show left panel', keys: ['F7'], group: 'View' },
  { id: 'view.rightPanel', label: 'Show right panel', keys: ['F8'], group: 'View' },
  // The Develop viewport (DEV-01 viewport spec)
  { id: 'view.zoom', label: 'Zoom to 1:1', keys: ['Z'], scope: 'develop', group: 'View' },
  { id: 'view.zoomHold', label: 'Temporary zoom', keys: ['Space'], scope: 'develop', group: 'View' },
  { id: 'view.toolbar', label: 'Hide toolbar', keys: ['T'], scope: 'develop', group: 'View' },
];

export function createRegistry(): ShortcutRegistry {
  const registry = new ShortcutRegistry();
  for (const spec of DEFAULT_COMMANDS) {
    registry.register({ enabled: () => false, ...spec });
  }
  return registry;
}
