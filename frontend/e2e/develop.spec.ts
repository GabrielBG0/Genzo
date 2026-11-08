import { expect, test, type Page } from '@playwright/test';

/** The Develop screen against the fake engine, its demo folder open. */
async function develop(page: Page) {
  await page.goto('/?engine=fake');
  await expect(page.getByRole('slider', { name: 'Exposure' })).toHaveAttribute('aria-valuetext', '0.00');
  await expect(page.getByText('No photo open')).toBeHidden();
}

const undo = (page: Page) => page.getByRole('toolbar', { name: 'Quick actions' }).getByRole('button', { name: /^Undo/ });
const redo = (page: Page) => page.getByRole('toolbar', { name: 'Quick actions' }).getByRole('button', { name: /^Redo/ });

/** Drags a slider's track from one fraction of its width to another. */
async function drag(page: Page, name: string, from: number, to: number) {
  const track = page.getByRole('slider', { name }).locator('.track');
  const box = await track.boundingBox();
  if (!box) throw new Error(`no track for ${name}`);
  const y = box.y + box.height / 2;
  await page.mouse.move(box.x + box.width * from, y);
  await page.mouse.down();
  for (let i = 1; i <= 10; i++) await page.mouse.move(box.x + box.width * (from + ((to - from) * i) / 10), y);
  await page.mouse.up();
}

test('a drag is one step that Ctrl Z undoes and Ctrl Shift Z redoes (APP-10)', async ({ page }) => {
  await develop(page);
  await expect(undo(page)).toBeDisabled();
  await expect(redo(page)).toBeDisabled();
  await drag(page, 'Exposure', 0.5, 0.7);
  const exposure = page.getByRole('slider', { name: 'Exposure' });
  await expect(exposure).toHaveAttribute('aria-valuetext', '+2.00');
  await expect(undo(page)).toBeEnabled();
  // The tooltip names the step (APP-10).
  await expect(undo(page)).toHaveAttribute('title', 'Undo Exposure  Ctrl Z');
  await page.keyboard.press('Control+z');
  await expect(exposure).toHaveAttribute('aria-valuetext', '0.00');
  await expect(undo(page)).toBeDisabled();
  await expect(redo(page)).toHaveAttribute('title', 'Redo Exposure  Ctrl Shift Z');
  await page.keyboard.press('Control+Shift+z');
  await expect(exposure).toHaveAttribute('aria-valuetext', '+2.00');
  await expect(redo(page)).toBeDisabled();
});

test('a focused text field keeps its own undo', async ({ page }) => {
  await develop(page);
  await drag(page, 'Contrast', 0.5, 0.6);
  const contrast = page.getByRole('slider', { name: 'Contrast' });
  await expect(contrast).toHaveAttribute('aria-valuetext', '+20');
  await contrast.getByText('+20').click();
  const field = page.getByRole('textbox', { name: 'Contrast value' });
  await field.fill('35');
  await page.keyboard.press('Control+z');
  await page.keyboard.press('Escape');
  await expect(contrast).toHaveAttribute('aria-valuetext', '+20');
});

test('V switches the treatment and hides Vibrance and Saturation', async ({ page }) => {
  await develop(page);
  await page.keyboard.press('v');
  await expect(page.getByRole('radio', { name: /B&W/ })).toHaveAttribute('aria-checked', 'true');
  await expect(page.getByRole('slider', { name: 'Vibrance' })).toBeHidden();
  await page.keyboard.press('v');
  await expect(page.getByRole('slider', { name: 'Vibrance' })).toBeVisible();
});

test('W turns the picker on, a click on the photo picks, Esc cancels', async ({ page }) => {
  await develop(page);
  const picker = page.getByRole('button', { name: 'White balance picker' });
  await page.keyboard.press('w');
  await expect(picker).toHaveAttribute('aria-pressed', 'true');
  await page.keyboard.press('Escape');
  await expect(picker).toHaveAttribute('aria-pressed', 'false');
  await page.keyboard.press('w');
  const photo = page.getByRole('img', { name: 'Photo' });
  const box = await photo.boundingBox();
  if (!box) throw new Error('no photo');
  // Off center: the fake picks 5500 K there, which is the Flash preset.
  await page.mouse.click(box.x + box.width * 0.3, box.y + box.height * 0.6);
  await expect(picker).toHaveAttribute('aria-pressed', 'false');
  await expect(page.getByRole('combobox', { name: 'White balance preset' })).toHaveValue('Custom');
  await expect(page.getByRole('slider', { name: 'Temp' })).toHaveClass(/changed/);
});

test('each photo keeps its own edit; the filmstrip marks edited photos', async ({ page }) => {
  await develop(page);
  await drag(page, 'Shadows', 0.5, 0.6);
  const strip = page.getByRole('region', { name: 'Filmstrip' });
  const frames = strip.getByRole('button');
  await expect(frames.nth(0).getByLabel('Edited')).toBeVisible();
  await frames.nth(1).click();
  await expect(frames.nth(1)).toHaveAttribute('aria-current', 'true');
  await expect(page.getByRole('slider', { name: 'Shadows' })).toHaveAttribute('aria-valuetext', '0');
  await expect(page.getByRole('slider', { name: 'Temp' })).toHaveAttribute('aria-valuetext', '3200');
  await frames.nth(4).click();
  await expect(page.getByText('Nikon High Efficiency RAW is not supported yet').first()).toBeVisible();
  await expect(page.getByRole('slider', { name: 'Shadows' })).toHaveAttribute('aria-disabled', 'true');
  await frames.nth(0).click();
  await expect(page.getByRole('slider', { name: 'Shadows' })).toHaveAttribute('aria-valuetext', '+20');
});

test('a double-click on the handle resets as one step, without a jump first', async ({ page }) => {
  await develop(page);
  const exposure = page.getByRole('slider', { name: 'Exposure' });
  await exposure.getByText('0.00').click();
  await page.getByRole('textbox', { name: 'Exposure value' }).fill('1');
  await page.keyboard.press('Enter');
  await expect(exposure).toHaveAttribute('aria-valuetext', '+1.00');
  const handle = await exposure.locator('.handle').boundingBox();
  if (!handle) throw new Error('no handle');
  // Off the handle's center: a press there must not move the value.
  await page.mouse.dblclick(handle.x + handle.width / 2 + 5, handle.y + handle.height / 2);
  await expect(exposure).toHaveAttribute('aria-valuetext', '0.00');
  await page.keyboard.press('Control+z');
  await expect(exposure).toHaveAttribute('aria-valuetext', '+1.00');
});

const marker = (page: Page, name: 'Shadow clipping' | 'Highlight clipping') =>
  page.getByRole('button', { name, exact: true });

test('the histogram shows the photo, its EXIF row, and lights a marker when it clips (DEV-03)', async ({ page }) => {
  await develop(page);
  await expect(page.getByRole('img', { name: 'Histogram' })).toBeVisible();
  await expect(page.locator('.exif span')).toHaveText(['ISO 100', '24 mm', 'f/8', '1/125 s']);
  await expect(marker(page, 'Highlight clipping')).toHaveAttribute('data-channels', '');
  await drag(page, 'Exposure', 0.5, 0.7);
  await expect(marker(page, 'Highlight clipping')).not.toHaveAttribute('data-channels', '');
  await expect(marker(page, 'Highlight clipping')).toHaveAttribute('title', /^Highlight clipping {2}J\n[\d.]+ % of pixels$/);
  // A photo that does not render: no plot, J does nothing.
  await page.getByRole('region', { name: 'Filmstrip' }).getByRole('button').nth(4).click();
  await expect(page.getByRole('img', { name: 'Histogram' })).toBeHidden();
  await expect(marker(page, 'Shadow clipping')).toBeDisabled();
});

test('J turns both overlays on, a marker one; they stay on across photos', async ({ page }) => {
  await develop(page);
  await page.keyboard.press('j');
  await expect(marker(page, 'Shadow clipping')).toHaveAttribute('aria-pressed', 'true');
  await expect(marker(page, 'Highlight clipping')).toHaveAttribute('aria-pressed', 'true');
  await marker(page, 'Shadow clipping').click();
  await expect(marker(page, 'Shadow clipping')).toHaveAttribute('aria-pressed', 'false');
  await page.getByRole('region', { name: 'Filmstrip' }).getByRole('button').nth(1).click();
  await expect(page.getByRole('slider', { name: 'Temp' })).toHaveAttribute('aria-valuetext', '3200');
  await expect(marker(page, 'Highlight clipping')).toHaveAttribute('aria-pressed', 'true');
  await page.keyboard.press('j');
  await expect(marker(page, 'Shadow clipping')).toHaveAttribute('aria-pressed', 'true');
  await page.keyboard.press('j');
  await expect(marker(page, 'Highlight clipping')).toHaveAttribute('aria-pressed', 'false');
});

/**
 * Reads a screenshot in the page: the share of pure black pixels, and the
 * RGB at a point given as fractions of its size.
 */
async function readShot(page: Page, png: Buffer, at = { x: 0.5, y: 0.5 }) {
  return page.evaluate(
    async ({ data, at }) => {
      const img = new Image();
      img.src = `data:image/png;base64,${data}`;
      await img.decode();
      const canvas = document.createElement('canvas');
      canvas.width = img.width;
      canvas.height = img.height;
      const ctx = canvas.getContext('2d');
      if (!ctx) throw new Error('no 2d context');
      ctx.drawImage(img, 0, 0);
      const px = ctx.getImageData(0, 0, img.width, img.height).data;
      let black = 0;
      for (let i = 0; i < px.length; i += 4) if (px[i] === 0 && px[i + 1] === 0 && px[i + 2] === 0) black++;
      const i = 4 * (Math.floor(at.y * (img.height - 1)) * img.width + Math.floor(at.x * (img.width - 1)));
      return { black: black / (px.length / 4), rgb: [px[i], px[i + 1], px[i + 2]] as number[] };
    },
    { data: png.toString('base64'), at },
  );
}

const photo = (page: Page) => page.getByRole('img', { name: 'Photo' });

/** The share of the photo's pixels that are pure black. */
async function blackShare(page: Page): Promise<number> {
  return (await readShot(page, await photo(page).screenshot())).black;
}

const near = (got: number[], want: number[]) => got.every((c, i) => Math.abs(c - (want[i] ?? 0)) <= 2);

test('a marker whose three channels clip is white, and J paints the clipped photo in the token red', async ({
  page,
}) => {
  await develop(page);
  // +2 EV: every pixel of the fake's scene clips, the sky in all three channels.
  await drag(page, 'Exposure', 0.5, 0.7);
  await expect(marker(page, 'Highlight clipping')).toHaveAttribute('data-channels', 'red green blue');
  // The triangle's center, a little inside its corner.
  const box = await marker(page, 'Highlight clipping').boundingBox();
  if (!box) throw new Error('no marker');
  const shot = await page.screenshot({ clip: { x: box.x + box.width / 2, y: box.y + 3, width: 2, height: 2 } });
  const corner = (await readShot(page, shot, { x: 0, y: 0 })).rgb;
  expect(corner).toEqual([255, 255, 255]);
  await page.keyboard.press('j');
  // Highlights win over shadows: label.red, read from the token.
  await expect
    .poll(async () => near((await readShot(page, await photo(page).screenshot())).rgb, [255, 107, 107]))
    .toBe(true);
});

test('Alt while dragging Exposure shows only what clips, until Alt goes up', async ({ page }) => {
  await develop(page);
  const track = page.getByRole('slider', { name: 'Exposure' }).locator('.track');
  const box = await track.boundingBox();
  if (!box) throw new Error('no track');
  const y = box.y + box.height / 2;
  await page.mouse.move(box.x + box.width * 0.5, y);
  await page.mouse.down();
  // +1 EV: the sky clips, the rest does not.
  for (let i = 1; i <= 5; i++) await page.mouse.move(box.x + box.width * (0.5 + 0.02 * i), y);
  expect(await blackShare(page)).toBe(0);
  await page.keyboard.down('Alt');
  // Not clipped shows black.
  await expect.poll(() => blackShare(page)).toBeGreaterThan(0.2);
  await page.keyboard.up('Alt');
  await expect.poll(() => blackShare(page)).toBe(0);
  await page.keyboard.down('Alt');
  await expect.poll(() => blackShare(page)).toBeGreaterThan(0.2);
  // The end of the drag ends the preview, Alt still held.
  await page.mouse.up();
  await expect.poll(() => blackShare(page)).toBe(0);
  await page.keyboard.up('Alt');
});

test('the filmstrip shows thumbnails, the file name without one, and ← → move through it (LIB-09)', async ({
  page,
}) => {
  await develop(page);
  const strip = page.getByRole('region', { name: 'Filmstrip' });
  const frames = strip.getByRole('button');
  await expect(frames).toHaveCount(5);
  // Four photos with a thumbnail drawn, the unsupported one with its name.
  for (let i = 0; i < 4; i++) await expect(frames.nth(i).locator('canvas')).toBeVisible();
  await expect(frames.nth(4).locator('canvas')).toHaveCount(0);
  await expect(frames.nth(4).getByText('DSC_0420.NEF')).toBeVisible();

  await expect(frames.nth(0)).toHaveAttribute('aria-current', 'true');
  await page.keyboard.press('ArrowLeft');
  await expect(frames.nth(0)).toHaveAttribute('aria-current', 'true');
  await page.keyboard.press('ArrowRight');
  await expect(frames.nth(1)).toHaveAttribute('aria-current', 'true');
  await expect(page.getByRole('slider', { name: 'Temp' })).toHaveAttribute('aria-valuetext', '3200');
  await page.keyboard.press('ArrowRight');
  await expect(frames.nth(2)).toHaveAttribute('aria-current', 'true');
  await page.keyboard.press('ArrowLeft');
  await expect(frames.nth(1)).toHaveAttribute('aria-current', 'true');

  // On a focused slider the arrows move the slider, not the photo.
  const exposure = page.getByRole('slider', { name: 'Exposure' });
  await exposure.focus();
  await page.keyboard.press('ArrowRight');
  await expect(frames.nth(1)).toHaveAttribute('aria-current', 'true');
  await expect(exposure).not.toHaveAttribute('aria-valuetext', '0.00');
});

test('Export: Ctrl Shift E, the file, replace, Esc, the quick action and the tab (OUT-01)', async ({ page }) => {
  await develop(page);
  const dialog = page.getByRole('dialog', { name: 'Export' });
  const status = page.locator('footer');
  await page.keyboard.press('Control+Shift+E');
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText('demo/DSC_0412.jpg')).toBeVisible();
  await expect(dialog.getByText('All EXIF metadata, including location.', { exact: false })).toBeVisible();
  await dialog.getByRole('button', { name: 'Export', exact: true }).click();
  await expect(dialog).toBeHidden();
  await expect(status.getByText('Exported DSC_0412.jpg')).toBeVisible();

  // Again to the same file: asked first, then replaced.
  await page.keyboard.press('Control+Shift+E');
  await page.keyboard.press('Enter');
  await expect(dialog.getByText('DSC_0412.jpg already exists. Replace it?')).toBeVisible();
  await dialog.getByRole('button', { name: 'Replace' }).click();
  await expect(dialog).toBeHidden();

  // Esc closes; the quick action and the tab open it.
  await page.getByRole('toolbar', { name: 'Quick actions' }).getByRole('button', { name: /^Export/ }).click();
  await expect(dialog).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
  await page.getByRole('button', { name: 'Export', exact: true }).first().click();
  await expect(dialog).toBeVisible();
  // A long edge under 32 px is refused before it is sent.
  await dialog.getByRole('radio', { name: /Long edge/ }).check();
  await dialog.getByLabel('Long edge in pixels').fill('20');
  await expect(dialog.getByRole('alert')).toHaveText('The long edge is 32 px at least.');
  await expect(dialog.getByRole('button', { name: 'Export', exact: true })).toBeDisabled();
  await page.keyboard.press('Escape');

  // A photo that does not render cannot be exported.
  await page.getByRole('region', { name: 'Filmstrip' }).getByRole('button').nth(4).click();
  await expect(page.getByRole('slider', { name: 'Shadows' })).toHaveAttribute('aria-disabled', 'true');
  await page.keyboard.press('Control+Shift+E');
  await expect(dialog).toBeHidden();

  // Ctrl O opens a folder through the system dialog (the fake's: demo).
  await page.keyboard.press('Control+O');
  await expect(page.getByRole('region', { name: 'Filmstrip' }).getByRole('button').nth(0)).toHaveAttribute(
    'aria-current',
    'true',
  );
});

test('Export in progress: the ring, and cancelling from the status bar (OUT-01)', async ({ page }) => {
  await page.goto('/?engine=fake&exportStep=400');
  await expect(page.getByText('No photo open')).toBeHidden();
  const status = page.locator('footer');
  const ring = page.getByRole('toolbar', { name: 'Quick actions' }).getByRole('progressbar', { name: 'Export progress' });
  await expect(ring).toBeHidden();
  await page.keyboard.press('Control+Shift+E');
  await page.getByRole('dialog', { name: 'Export' }).getByRole('button', { name: 'Export', exact: true }).click();
  await expect(ring).toBeVisible();
  await expect(ring).not.toHaveAttribute('aria-valuenow', '0');
  const progress = status.getByRole('button', { name: /^Exporting DSC_0412\.jpg, \d+ %$/ });
  await progress.click();
  // Esc closes the menu; clicking again opens it.
  await expect(page.getByRole('menu', { name: 'Export' })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('menu', { name: 'Export' })).toBeHidden();
  await progress.click();
  await page.getByRole('menuitem', { name: 'Cancel export' }).click();
  await expect(status.getByText('Export cancelled')).toBeVisible();
  await expect(ring).toBeHidden();
  await expect(page.getByRole('menu', { name: 'Export' })).toBeHidden();
});

test('Before / after: \\ and the quick action toggle the before; a drag shows the after (WF-02)', async ({ page }) => {
  await develop(page);
  const badge = page.getByRole('img', { name: 'Photo' }).getByRole('status');
  const quick = page.getByRole('toolbar', { name: 'Quick actions' }).getByRole('button', { name: /^Before \/ after/ });
  await expect(quick).toHaveAttribute('aria-pressed', 'false');
  await expect(quick).toHaveAttribute('title', 'Before / after  \\');
  await drag(page, 'Exposure', 0.5, 0.7);
  await page.keyboard.press('\\');
  await expect(badge).toHaveText('Before');
  await expect(quick).toHaveAttribute('aria-pressed', 'true');
  // Nothing recorded: the panel keeps the edit, and Undo still names it.
  await expect(page.getByRole('slider', { name: 'Exposure' })).toHaveAttribute('aria-valuetext', '+2.00');
  await expect(undo(page)).toHaveAttribute('title', 'Undo Exposure  Ctrl Z');
  await quick.click();
  await expect(badge).toBeHidden();
  await expect(quick).toHaveAttribute('aria-pressed', 'false');
  // A drag turns the before off.
  await page.keyboard.press('\\');
  await expect(badge).toBeVisible();
  await drag(page, 'Contrast', 0.5, 0.6);
  await expect(badge).toBeHidden();
  // In a text field, \ is typed, not the command.
  await page.getByRole('slider', { name: 'Contrast' }).getByText('+20').click();
  await expect(page.getByRole('textbox', { name: 'Contrast value' })).toBeFocused();
  await page.keyboard.press('\\');
  await expect(badge).toBeHidden();
});

test('The slider menu: reset, copy a value and paste it on another photo (APP-06)', async ({ page }) => {
  await develop(page);
  const exposure = page.getByRole('slider', { name: 'Exposure' });
  const menu = page.getByRole('menu', { name: 'Exposure slider' });
  await drag(page, 'Exposure', 0.5, 0.7);
  await exposure.click({ button: 'right' });
  await expect(menu).toBeVisible();
  await expect(menu.getByText('+2.00 EV')).toBeVisible();
  await expect(menu.getByRole('menuitem', { name: /Reset Exposure/ })).toContainText('double-click');
  await expect(menu.getByRole('menuitem', { name: 'Paste value' })).toBeDisabled();
  await page.keyboard.press('Escape');
  await expect(menu).toBeHidden();

  // Copy, then paste on the next photo: one step there.
  await exposure.click({ button: 'right' });
  await menu.getByRole('menuitem', { name: 'Copy value' }).click();
  await expect(menu).toBeHidden();
  const frames = page.getByRole('region', { name: 'Filmstrip' }).getByRole('button');
  await frames.nth(1).click();
  await expect(frames.nth(1)).toHaveAttribute('aria-current', 'true');
  await expect(exposure).toHaveAttribute('aria-valuetext', '0.00');
  await exposure.click({ button: 'right' });
  await menu.getByRole('menuitem', { name: 'Paste value' }).click();
  await expect(exposure).toHaveAttribute('aria-valuetext', '+2.00');
  await expect(undo(page)).toHaveAttribute('title', 'Undo Exposure  Ctrl Z');
  // Another slider cannot take it.
  await page.getByRole('slider', { name: 'Contrast' }).click({ button: 'right' });
  await expect(page.getByRole('menu', { name: 'Contrast slider' }).getByRole('menuitem', { name: 'Paste value' })).toBeDisabled();
  await page.keyboard.press('Escape');

  // Reset, as a double-click does.
  await exposure.click({ button: 'right' });
  await menu.getByRole('menuitem', { name: /Reset Exposure/ }).click();
  await expect(exposure).toHaveAttribute('aria-valuetext', '0.00');
  await expect(undo(page)).toHaveAttribute('title', 'Undo Reset Exposure  Ctrl Z');

  // From the keyboard: Shift F10, the first item focused, ↓ and Enter.
  const highlights = page.getByRole('slider', { name: 'Highlights' });
  await highlights.focus();
  await page.keyboard.press('Shift+F10');
  const hmenu = page.getByRole('menu', { name: 'Highlights slider' });
  await expect(hmenu.getByRole('menuitem', { name: /Reset Highlights/ })).toBeFocused();
  await page.keyboard.press('ArrowDown');
  await expect(hmenu.getByRole('menuitem', { name: 'Copy value' })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(hmenu).toBeHidden();

  // Temp on a RAW file is Kelvin: it pastes into RAW files only.
  const temp = page.getByRole('slider', { name: 'Temp' });
  const tmenu = page.getByRole('menu', { name: 'Temp slider' });
  await temp.click({ button: 'right' });
  await expect(tmenu.getByText(/^\d+ K$/)).toBeVisible();
  await tmenu.getByRole('menuitem', { name: 'Copy value' }).click();
  await frames.nth(2).click();
  await temp.click({ button: 'right' });
  await expect(tmenu.getByRole('menuitem', { name: 'Paste value' })).toBeEnabled();
  await page.keyboard.press('Escape');
  await frames.nth(3).click();
  await expect(frames.nth(3)).toHaveAttribute('aria-current', 'true');
  await temp.click({ button: 'right' });
  await expect(tmenu.getByRole('menuitem', { name: 'Paste value' })).toBeDisabled();
  await page.keyboard.press('Escape');

  // A photo that does not render: its sliders open no menu.
  await frames.nth(4).click();
  await expect(page.getByRole('slider', { name: 'Shadows' })).toHaveAttribute('aria-disabled', 'true');
  await exposure.click({ button: 'right', force: true });
  await expect(menu).toBeHidden();
});

test('Panels: Tab, Shift Tab, F6, F7 and F8 hide and show them (APP-08)', async ({ page }) => {
  await develop(page);
  const left = page.locator('.left');
  const right = page.getByRole('complementary', { name: 'Right panel' });
  const strip = page.getByRole('region', { name: 'Filmstrip' });
  const photo = page.getByRole('img', { name: 'Photo' });
  const width = async () => (await photo.boundingBox())?.width ?? 0;
  const before = await width();

  // Nothing focused: Tab hides both side panels, and the photo grows.
  await page.locator('body').click({ position: { x: 5, y: 5 } });
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  await page.keyboard.press('Tab');
  await expect(left).toBeHidden();
  await expect(right).toBeHidden();
  await expect(strip).toBeVisible();
  await expect.poll(width).toBeGreaterThan(before + 200);
  await page.keyboard.press('Tab');
  await expect(right).toBeVisible();
  await expect(left).toBeVisible();
  await expect.poll(width).toBeLessThan(before + 1);

  // A focused slider keeps Tab for focus.
  const exposure = page.getByRole('slider', { name: 'Exposure' });
  await exposure.focus();
  await page.keyboard.press('Tab');
  await expect(right).toBeVisible();
  await expect(page.getByRole('slider', { name: 'Contrast' })).toBeFocused();

  // F7, F8 and F6 work with a slider focused.
  await page.keyboard.press('F8');
  await expect(right).toBeHidden();
  await page.keyboard.press('F7');
  await expect(left).toBeHidden();
  await page.keyboard.press('F6');
  await expect(strip).toBeHidden();
  await page.keyboard.press('F6');
  await page.keyboard.press('F7');
  await expect(strip).toBeVisible();
  await expect(left).toBeVisible();
  await page.keyboard.press('F8');
  // The right panel shows again as it was: Basic still open.
  await expect(right).toBeVisible();
  await expect(page.getByRole('button', { name: /^Basic/ })).toHaveAttribute('aria-expanded', 'true');

  // Shift Tab, nothing focused: the side panels and the filmstrip; the bars stay.
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  await page.keyboard.press('Shift+Tab');
  await expect(left).toBeHidden();
  await expect(right).toBeHidden();
  await expect(strip).toBeHidden();
  await expect(page.getByRole('toolbar', { name: 'Quick actions' })).toBeVisible();
  await expect(page.locator('footer')).toBeVisible();
  await page.keyboard.press('Shift+Tab');
  await expect(strip).toBeVisible();
  await expect(right).toBeVisible();
});

test('The app menu: the logo and Alt alone open it; submenus, keys, View checks, About (APP-02)', async ({ page }) => {
  await develop(page);
  const menu = page.getByRole('menu', { name: 'App menu' });
  // The logo opens it, with focus on the first item; Esc closes.
  await page.getByRole('button', { name: /^App menu/ }).click();
  await expect(menu).toBeVisible();
  await expect(menu.getByRole('menuitem', { name: /Open folder/ })).toBeFocused();
  await expect(menu.getByRole('menuitem', { name: /Export/ })).toContainText('Ctrl Shift E');
  await page.keyboard.press('Escape');
  await expect(menu).toBeHidden();

  // Alt alone opens and closes it; Alt during a drag does not.
  await page.keyboard.press('Alt');
  await expect(menu).toBeVisible();
  await page.keyboard.press('Alt');
  await expect(menu).toBeHidden();
  const track = page.getByRole('slider', { name: 'Shadows' }).locator('.track');
  const box = await track.boundingBox();
  if (!box) throw new Error('no track');
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.keyboard.down('Alt');
  await page.mouse.move(box.x + box.width * 0.6, box.y + box.height / 2);
  await page.keyboard.up('Alt');
  await page.mouse.up();
  await expect(menu).toBeHidden();

  // Keys: ↓ to Export…, ↓↓ to File, → opens it, ← comes back.
  await page.keyboard.press('Alt');
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('ArrowDown');
  const file = menu.getByRole('menuitem', { name: 'File' });
  await expect(file).toBeFocused();
  await page.keyboard.press('ArrowRight');
  const fileMenu = page.getByRole('menu', { name: 'File' });
  await expect(fileMenu.getByRole('menuitem', { name: /Open folder/ })).toBeFocused();
  await page.keyboard.press('ArrowLeft');
  await expect(fileMenu).toBeHidden();
  await expect(file).toBeFocused();

  // Edit › Undo names the step and undoes it.
  await page.keyboard.press('Escape');
  await drag(page, 'Exposure', 0.5, 0.7);
  await page.getByRole('button', { name: /^App menu/ }).click();
  await menu.getByRole('menuitem', { name: 'Edit' }).hover();
  const edit = page.getByRole('menu', { name: 'Edit' });
  await expect(edit.getByRole('menuitem', { name: /Undo Exposure/ })).toContainText('Ctrl Z');
  await expect(edit.getByRole('menuitem', { name: /^Redo/ })).toBeDisabled();
  await edit.getByRole('menuitem', { name: /Undo Exposure/ }).click();
  await expect(menu).toBeHidden();
  await expect(page.getByRole('slider', { name: 'Exposure' })).toHaveAttribute('aria-valuetext', '0.00');

  // View: checks follow the layout; Filmstrip hides it.
  await page.getByRole('button', { name: /^App menu/ }).click();
  await menu.getByRole('menuitem', { name: 'View' }).hover();
  const view = page.getByRole('menu', { name: 'View' });
  const filmstrip = view.getByRole('menuitemcheckbox', { name: /Filmstrip/ });
  await expect(filmstrip).toHaveAttribute('aria-checked', 'true');
  await filmstrip.click();
  await expect(page.getByRole('region', { name: 'Filmstrip' })).toBeHidden();
  await page.getByRole('button', { name: /^App menu/ }).click();
  await menu.getByRole('menuitem', { name: 'View' }).hover();
  await expect(view.getByRole('menuitemcheckbox', { name: /Filmstrip/ })).toHaveAttribute('aria-checked', 'false');
  await page.keyboard.press('Escape');
  await page.keyboard.press('F6');

  // Develop › Reset all; Help › About Genzo.
  await page.getByRole('button', { name: /^App menu/ }).click();
  await menu.getByRole('menuitem', { name: 'Help' }).hover();
  await page.getByRole('menu', { name: 'Help' }).getByRole('menuitem', { name: 'About Genzo' }).click();
  const about = page.getByRole('dialog', { name: 'Genzo' });
  await expect(about).toContainText('Engine contract');
  await expect(about).toContainText('GNU GPL v3');
  await page.keyboard.press('Escape');
  await expect(about).toBeHidden();
});

test('Quit (Ctrl Q) closes the window, cancelling an export first (APP-02, OUT-01)', async ({ page }) => {
  await page.goto('/?engine=fake&exportStep=400');
  await expect(page.getByText('No photo open')).toBeHidden();
  await page.keyboard.press('Control+Shift+E');
  await page.getByRole('dialog', { name: 'Export' }).getByRole('button', { name: 'Export', exact: true }).click();
  await expect(page.locator('footer').getByRole('button', { name: /^Exporting/ })).toBeVisible();
  await page.keyboard.press('Control+q');
  await expect(page.locator('footer').getByText('Export cancelled')).toBeVisible();
});

test('The status bar: where you are, what shows, and why (APP-07)', async ({ page }) => {
  await develop(page);
  const bar = page.locator('footer');
  await expect(bar).toContainText('Develop');
  await expect(bar.getByText('5 photos')).toBeVisible();
  await expect(bar.getByText('DSC_0412.NEF')).toBeVisible();
  await expect(bar.getByText('Settled')).toHaveAttribute('title', 'Rendered at full quality');
  await expect(bar.getByText('sRGB')).toHaveAttribute('title', 'Frames are rendered in sRGB');
  await expect(bar.getByText('8-bit')).toBeVisible();
  await expect(bar.getByText('Fake engine', { exact: true })).toBeVisible();

  // An unsupported photo: its embedded preview drawn, and why.
  const frames = page.getByRole('region', { name: 'Filmstrip' }).getByRole('button');
  await frames.nth(4).click();
  await expect(bar.getByRole('status')).toHaveText('Nikon High Efficiency RAW is not supported yet');
  await expect(bar.getByText('Embedded preview')).toBeVisible();
  await expect(page.getByRole('img', { name: 'Photo' }).getByText('Nikon High Efficiency')).toBeHidden();

  // Back on a photo that renders: settled, the reason gone.
  await frames.nth(0).click();
  await expect(bar.getByText('Settled')).toBeVisible();
  await expect(bar.getByText('Nikon High Efficiency RAW is not supported yet')).toBeHidden();
  // A photo with a note: said over the adapter notice, gone on the next photo.
  const note = 'Lens corrections from the file are not applied yet';
  await frames.nth(2).click();
  await expect(bar.getByRole('status')).toHaveText(note);
  await frames.nth(3).click();
  await expect(bar.getByText(note)).toBeHidden();
});

test('One indicator slides to the chosen tab or segment; the menu has one highlight (APP-01, APP-03)', async ({
  page,
}) => {
  await develop(page);
  const modules = page.getByRole('navigation', { name: 'Modules' });
  const lensOver = async (
    group: ReturnType<Page['locator']>,
    name: string | RegExp,
    role: 'button' | 'radio' | 'menuitem' = 'button',
  ) => {
    const lens = await group.locator('.sliding-lens').boundingBox();
    const item = await group.getByRole(role, { name }).boundingBox();
    if (!lens || !item) throw new Error('no box');
    expect(Math.abs(lens.x - item.x)).toBeLessThan(1);
    expect(Math.abs(lens.width - item.width)).toBeLessThan(1);
  };
  // One lens in the tab bar, on Develop; Library waits for M2, and D keeps Develop.
  await expect(modules.locator('.sliding-lens')).toHaveCount(1);
  await expect(modules.getByRole('button', { name: 'Library' })).toBeDisabled();
  await page.keyboard.press('d');
  await expect(modules.getByRole('button', { name: 'Develop' })).toHaveAttribute('aria-current', 'page');
  await expect(async () => lensOver(modules, 'Develop')).toPass();

  // Color / B&W: V moves the indicator, after its 180 ms slide.
  const treatment = page.getByRole('radiogroup', { name: 'Treatment' });
  await expect(async () => lensOver(treatment, 'Color', 'radio')).toPass();
  await page.keyboard.press('v');
  await expect(treatment.getByRole('radio', { name: /B&W/ })).toHaveAttribute('aria-checked', 'true');
  await expect(async () => lensOver(treatment, /B&W/, 'radio')).toPass();

  // The quick actions are flat buttons, with no lens (APP-03).
  await expect(page.getByRole('toolbar', { name: 'Quick actions' }).locator('.lens')).toHaveCount(0);

  // The app menu: one highlight, which follows the keyboard.
  await page.getByRole('button', { name: /App menu/ }).click();
  const menu = page.getByRole('menu', { name: 'App menu' });
  await expect(menu.locator('.sliding-lens')).toHaveCount(1);
  await expect(async () => lensOver(menu, /Open folder/, 'menuitem')).toPass();
  await page.keyboard.press('ArrowDown');
  await expect(async () => lensOver(menu, /Export/, 'menuitem')).toPass();
  await page.keyboard.press('Escape');
  await expect(menu).toBeHidden();
});

/** The share of a screenshot's pixels that are near white: the fake draws a white grid only at 1:1. */
async function whiteShare(page: Page): Promise<number> {
  const png = await page.getByRole('img', { name: 'Photo' }).screenshot();
  return page.evaluate(async (data) => {
    const img = new Image();
    img.src = `data:image/png;base64,${data}`;
    await img.decode();
    const canvas = document.createElement('canvas');
    [canvas.width, canvas.height] = [img.width, img.height];
    const ctx = canvas.getContext('2d');
    if (!ctx) throw new Error('no 2d context');
    ctx.drawImage(img, 0, 0);
    const px = ctx.getImageData(0, 0, img.width, img.height).data;
    let white = 0;
    for (let i = 0; i < px.length; i += 4) if ((px[i] ?? 0) > 235 && (px[i + 1] ?? 0) > 235 && (px[i + 2] ?? 0) > 235) white++;
    return white / (px.length / 4);
  }, png.toString('base64'));
}

test('Zoom: Z, the toolbar, a click and Space go to 1:1 and back; a drag pans; T hides the toolbar (DEV-01 viewport)', async ({
  page,
}) => {
  await develop(page);
  const photo = page.getByRole('img', { name: 'Photo' });
  const toolbar = page.getByRole('toolbar', { name: 'Photo toolbar' });
  const readout = toolbar.locator('.readout');
  await expect(readout).toHaveText(/^Fit \d+%$/);
  await expect.poll(() => whiteShare(page)).toBeLessThan(0.001);

  // Z: 1:1, where the fake's one-pixel grid shows; Z again: Fit.
  await page.keyboard.press('z');
  await expect(readout).toHaveText('1:1');
  await expect(toolbar.getByRole('radio', { name: '1:1' })).toHaveAttribute('aria-checked', 'true');
  await expect.poll(() => whiteShare(page)).toBeGreaterThan(0.01);
  await page.keyboard.press('z');
  await expect(readout).toHaveText(/^Fit/);
  await expect.poll(() => whiteShare(page)).toBeLessThan(0.001);

  // The toolbar's 1:1 and Fit.
  await toolbar.getByRole('radio', { name: '1:1' }).click();
  await expect(readout).toHaveText('1:1');
  await toolbar.getByRole('radio', { name: 'Fit' }).click();
  await expect(readout).toHaveText(/^Fit/);

  // A click zooms to 1:1 there; a drag pans; a click goes back to Fit.
  const box = await photo.boundingBox();
  if (!box) throw new Error('no photo');
  await page.mouse.click(box.x + box.width * 0.3, box.y + box.height * 0.4);
  await expect(readout).toHaveText('1:1');
  await expect.poll(() => whiteShare(page)).toBeGreaterThan(0.01);
  const still = await photo.screenshot();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  for (let i = 1; i <= 5; i++) await page.mouse.move(box.x + box.width / 2 - 4 * i, box.y + box.height / 2 - 3 * i);
  await page.mouse.up();
  await expect(readout).toHaveText('1:1');
  await expect.poll(async () => (await photo.screenshot()).equals(still)).toBe(false);
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
  await expect(readout).toHaveText(/^Fit/);

  // Space held: 1:1 until it goes up.
  await page.mouse.move(box.x + box.width * 0.6, box.y + box.height * 0.5);
  await page.keyboard.down('Space');
  await expect(readout).toHaveText('1:1');
  await page.keyboard.up('Space');
  await expect(readout).toHaveText(/^Fit/);

  // T hides the toolbar and shows it again.
  await page.keyboard.press('t');
  await expect(toolbar).toBeHidden();
  await page.keyboard.press('t');
  await expect(toolbar).toBeVisible();

  // The zoom stays across photos.
  await page.keyboard.press('z');
  await page.keyboard.press('ArrowRight');
  await expect(readout).toHaveText('1:1');
  await expect.poll(() => whiteShare(page)).toBeGreaterThan(0.01);
});

test('Crop and rotate: R, a corner drag, the aspect, turns, flips, the angle, O and Enter (DEV-12)', async ({ page }) => {
  await develop(page);
  await page.keyboard.press('r');
  const panel = page.getByRole('region', { name: 'Crop and rotate' });
  await expect(panel).toBeVisible();
  await expect(page.getByRole('region', { name: 'Basic' })).toBeHidden();
  const size = panel.getByLabel('Crop size');
  // The fake's first photo is 6048 × 4032, uncropped.
  await expect(size).toContainText('W 6048');
  await expect(size).toContainText('H 4032');
  const frame = page.getByRole('application', { name: 'Crop frame' });
  await expect(frame.locator('.handle')).toHaveCount(8);

  // A corner drag, free: one "Crop" step, a smaller crop.
  await panel.getByRole('button', { name: 'Lock the aspect' }).click();
  await expect(panel.getByRole('combobox', { name: 'Aspect' })).toHaveValue('Free');
  const corner = frame.locator('[data-grip="se"]');
  const box = await corner.boundingBox();
  if (!box) throw new Error('no handle');
  await page.mouse.move(box.x + 4, box.y + 4);
  await page.mouse.down();
  for (let i = 1; i <= 6; i++) await page.mouse.move(box.x + 4 - 20 * i, box.y + 4 - 10 * i);
  await page.mouse.up();
  await expect(undo(page)).toHaveAttribute('title', 'Undo Crop  Ctrl Z');
  await expect(size).not.toContainText('W 6048');
  await expect(size).not.toContainText('H 4032');

  // 1:1 fits a square about the center and locks.
  await panel.getByRole('combobox', { name: 'Aspect' }).selectOption('1:1');
  await expect(panel.getByRole('combobox', { name: 'Aspect' })).toHaveValue('1:1');
  await expect(panel.getByRole('button', { name: 'Lock the aspect' })).toHaveAttribute('aria-pressed', 'true');
  const square = (await size.textContent()) ?? '';
  const [w, h] = [/W (\d+)/.exec(square)?.[1], /H (\d+)/.exec(square)?.[1]];
  expect(w).toBe(h);

  // Ctrl Z, then a turn: the size swaps sides.
  await page.keyboard.press('Control+z');
  await page.keyboard.press('Control+z');
  await expect(size).toContainText('W 6048');
  await page.keyboard.press('Control+]');
  await expect(size).toContainText('W 4032');
  await expect(size).toContainText('H 6048');
  await expect(undo(page)).toHaveAttribute('title', 'Undo Rotate  Ctrl Z');
  await panel.getByRole('button', { name: 'Flip H' }).click();
  await expect(undo(page)).toHaveAttribute('title', 'Undo Rotate  Ctrl Z');

  // The angle slider: the engine shrinks the crop to stay inside.
  const angle = panel.getByRole('slider', { name: 'Angle' });
  await angle.focus();
  for (let i = 0; i < 10; i++) await page.keyboard.press('Shift+ArrowRight');
  await expect(angle).toHaveAttribute('aria-valuetext', '+10.0°');
  await expect(size).not.toContainText('W 4032');

  // O cycles the overlay; Enter closes, and the photo shows cropped.
  await page.locator('body').click({ position: { x: 5, y: 5 } });
  await expect(panel).toContainText('Overlay: Thirds');
  await page.keyboard.press('o');
  await expect(panel).toContainText('Overlay: Golden');
  await page.keyboard.press('Enter');
  await expect(panel).toBeHidden();
  await expect(page.getByRole('region', { name: 'Basic' })).toBeVisible();
  await expect(frame).toBeHidden();
  // Reset in the panel brings the whole photo back.
  await page.keyboard.press('r');
  await panel.getByRole('button', { name: 'Reset' }).click();
  await expect(size).toContainText('W 6048');
  await page.keyboard.press('Escape');
  await expect(panel).toBeHidden();
});
