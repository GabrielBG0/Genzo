import js from '@eslint/js';
import svelte from 'eslint-plugin-svelte';
import globals from 'globals';
import ts from 'typescript-eslint';

// The one file allowed to touch the Tauri API (architecture rule).
const TRANSPORT = 'src/lib/transport/tauri.ts';

export default ts.config(
  { ignores: ['dist/', 'node_modules/', 'test-results/', 'playwright-report/'] },
  js.configs.recommended,
  ...ts.configs.strict,
  ...svelte.configs.recommended,
  {
    languageOptions: { globals: { ...globals.browser, ...globals.node } },
  },
  {
    files: ['**/*.svelte', '**/*.svelte.ts'],
    languageOptions: { parserOptions: { parser: ts.parser, extraFileExtensions: ['.svelte'] } },
  },
  {
    rules: {
      'no-restricted-imports': [
        'error',
        {
          patterns: [
            {
              group: ['@tauri-apps/*'],
              message: `Only ${TRANSPORT} may import the Tauri API. Go through the Engine interface.`,
            },
            {
              group: ['**/backend/**', '**/shell/**'],
              message: 'The frontend imports no backend or shell code; use $contract.',
            },
          ],
        },
      ],
    },
  },
  {
    files: [TRANSPORT],
    rules: { 'no-restricted-imports': 'off' },
  },
);
