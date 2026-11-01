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
