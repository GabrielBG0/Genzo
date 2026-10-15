// M0 step 0.E: measure slider input to pixels in the real app and print the
// probe's p50/p95. Attaches to the app's webview over the Chrome DevTools
// Protocol, then drags a slider with real mouse events.
//
// Windows (WebView2):
//   $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=9222"
//   ..\shell\target\release\genzo-shell.exe          # or the built app
//   node scripts/measure-latency.mjs [--source "GPU · 12 MP raw"] [--frame "2560 × 1440"]
//                                    [--slider Exposure] [--drags 6] [--port 9222] [--breakdown 1]
// --breakdown times the IPC call (`execute`) and the frame fetch inside the page.
//
// Not for macOS (WKWebView) or Linux (WebKitGTK), which have no CDP: read the
// numbers off the app's readout panel there.
import { chromium } from '@playwright/test';

const args = new Map();
for (let i = 2; i < process.argv.length; i += 2) args.set(process.argv[i].replace(/^--/, ''), process.argv[i + 1]);
const port = args.get('port') ?? '9222';
const slider = args.get('slider') ?? 'Exposure';
const drags = Number(args.get('drags') ?? 6);

const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
const page = browser.contexts().flatMap((c) => c.pages())[0];
if (!page) throw new Error('no page found in the webview');
await page.bringToFront();

if (args.get('breakdown')) {
  // Tauri's IPC and the frame scheme both go through fetch, so timing fetch
  // (until the body is read) splits the path: ipc = the `execute` round trip
  // (render, readback, tiles), frame = moving the pixels to the page.
  await page.evaluate(() => {
    const t = (window.__breakdown = { ipc: [], frame: [] });
    const f = window.fetch.bind(window);
    window.fetch = async (u, o) => {
      const kind = String(u).includes('genzo-frame') ? 'frame' : String(u).includes('ipc') ? 'ipc' : null;
      const t0 = performance.now();
      const r = await f(u, o);
      if (!kind) return r;
      for (const m of ['arrayBuffer', 'json', 'text']) {
        const orig = r[m].bind(r);
        r[m] = async () => {
          const body = await orig();
          t[kind].push([performance.now() - t0, body.byteLength ?? String(body).length]);
          return body;
        };
      }
      return r;
    };
  });
}

const readout = async (label) => {
  const text = await page.locator('dl.readout dt', { hasText: label }).locator('xpath=following-sibling::dd[1]').innerText();
  return text.trim();
};

const select = async (title, value) => {
  const sel = page.locator('dl.readout dt', { hasText: title }).locator('xpath=following-sibling::dd[1]//select');
  if (value) await sel.selectOption(value);
  return sel.inputValue();
};

const source = await select('Source', args.get('source'));
const frame = await select('Frame', args.get('frame'));
await page.waitForTimeout(3000); // load the image, first render

// The slider's track element: the row whose label matches.
const row = page.locator('.row', { hasText: slider }).first();
const track = row.locator('[class*=track]').first();
const box = await track.boundingBox();
if (!box) throw new Error(`slider "${slider}" not found`);
const y = box.y + box.height / 2;

// A reset is not exposed: the probe keeps its last 200 samples, so drag enough
// to fill them with this run only.
for (let d = 0; d < drags; d++) {
  await page.mouse.move(box.x + 4, y);
  await page.mouse.down();
  const steps = 120;
  for (let i = 0; i <= steps; i++) {
    const t = d % 2 === 0 ? i / steps : 1 - i / steps;
    await page.mouse.move(box.x + 4 + t * (box.width - 8), y);
    await page.waitForTimeout(8); // about 120 Hz of pointer events
  }
  await page.mouse.up();
  await page.waitForTimeout(600);
}

const pct = (a, p) => (a.length ? [...a].sort((x, y) => x - y)[Math.round((a.length - 1) * p)] : null);
let breakdown;
if (args.get('breakdown')) {
  const b = await page.evaluate(() => window.__breakdown);
  const sum = (rows) => ({ n: rows.length, p50: pct(rows.map((x) => x[0]), 0.5), p95: pct(rows.map((x) => x[0]), 0.95), bytes: rows[0]?.[1] ?? null });
  breakdown = { ipcMs: sum(b.ipc), frameFetchMs: sum(b.frame) };
}
console.log(JSON.stringify({
  breakdown,
  source, frame, slider,
  p50: await readout('p50'),
  p95: await readout('p95'),
  gpuLastFrame: await readout('GPU, last frame'),
  samples: await readout('Samples'),
  userAgent: await page.evaluate(() => navigator.userAgent),
}, null, 2));
await browser.close();
