import { expect, test } from '@playwright/test';

test('app shell renders against the fake engine', async ({ page }) => {
  await page.goto('/?engine=fake');
  await expect(page.getByRole('navigation', { name: 'Modules' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Develop' })).toHaveAttribute('aria-current', 'page');
  await expect(page.getByRole('toolbar', { name: 'Quick actions' })).toBeVisible();
  await expect(page.getByText('Fake engine', { exact: true })).toBeVisible();
  // The first test-pattern frame replaces the placeholder.
  await expect(page.getByText('No photo open')).toBeHidden();
});

test('exposure slider drives the latency probe through the spike pipeline', async ({ page }) => {
  await page.goto('/?engine=fake');
  const slider = page.getByRole('slider', { name: 'Exposure' });
  await slider.focus();
  for (let i = 0; i < 5; i++) await page.keyboard.press('ArrowRight');
  await expect(slider).toHaveAttribute('aria-valuenow', '0.05');
  await expect(page.getByText(/Input → pixels p95/)).toBeVisible();
  await expect(page.getByText('No photo open')).toBeHidden();
});

test('test pattern source drives the phase slider', async ({ page }) => {
  await page.goto('/?engine=fake');
  await page.getByRole('combobox').first().selectOption('Test pattern');
  const slider = page.getByRole('slider', { name: 'Phase' });
  await slider.focus();
  for (let i = 0; i < 5; i++) await page.keyboard.press('ArrowRight');
  await expect(slider).toHaveAttribute('aria-valuenow', '5');
});

test('gallery shows every shared component', async ({ page }) => {
  await page.goto('/#/gallery');
  for (const code of ['C1', 'C2', 'C3', 'C4', 'C5', 'C6', 'C7', 'C8', 'C9', 'C10', 'C11', 'C12']) {
    await expect(page.getByRole('heading', { name: new RegExp(`^${code} ·`) })).toBeVisible();
  }
});
