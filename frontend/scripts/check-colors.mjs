// Fails when a color literal appears outside the token file. Covers inline
// styles in Svelte markup and colors in TypeScript, which stylelint misses.
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';

const ROOT = new URL('../src', import.meta.url).pathname;
const ALLOWED = new Set(['styles/tokens.css']);
const COLOR = /#[0-9a-fA-F]{3,8}\b|\b(?:rgba?|hsla?|oklch|oklab)\(/g;

const problems = [];
function walk(dir) {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) walk(path);
    else if (/\.(svelte|ts|css)$/.test(name) && !/\.test\.ts$/.test(name)) check(path);
  }
}
function check(path) {
  const rel = relative(ROOT, path);
  if (ALLOWED.has(rel)) return;
  readFileSync(path, 'utf8')
    .split('\n')
    .forEach((line, i) => {
      // Comments may mention colors.
      const code = line.replace(/\/\/.*$|\/\*.*?\*\//g, '');
      // `#` followed by hex digits inside an id or href is not a color.
      for (const m of code.matchAll(COLOR)) {
        const before = code.slice(0, m.index);
        if (/(?:href|id|url\()\s*=?\s*["'(]?$/.test(before)) continue;
        problems.push(`src/${rel}:${i + 1}: color literal "${m[0]}"; use a token`);
      }
    });
}
walk(ROOT);
if (problems.length) {
  console.error(problems.join('\n'));
  process.exit(1);
}
