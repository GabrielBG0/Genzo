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
