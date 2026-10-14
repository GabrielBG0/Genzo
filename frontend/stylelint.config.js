// Colors come from the design tokens. Only tokens.css may spell one out.
export default {
  customSyntax: 'postcss-html',
  ignoreFiles: ['src/styles/tokens.css', 'dist/**'],
  rules: {
    'color-no-hex': true,
    'color-named': 'never',
    'function-disallowed-list': [
      ['rgb', 'rgba', 'hsl', 'hsla', 'hwb', 'lab', 'lch', 'oklab', 'oklch', 'color'],
      { message: 'Use a color token from src/styles/tokens.css.' },
    ],
  },
  overrides: [{ files: ['**/*.css'], customSyntax: 'postcss' }],
};
