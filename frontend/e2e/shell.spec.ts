import { expect, test } from '@playwright/test';

test('app shell renders against the fake engine', async ({ page }) => {
  await page.goto('/?engine=fake');
  await expect(page.getByRole('navigation', { name: 'Modules' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Develop' })).toHaveAttribute('aria-current', 'page');
  await expect(page.getByRole('toolbar', { name: 'Quick actions' })).toBeVisible();
  await expect(page.getByText('Fake engine', { exact: true })).toBeVisible();
  // The demo folder's first photo replaces the placeholder.
  await expect(page.getByText('No photo open')).toBeHidden();
});

test('the autobench runs in the app, through Develop, and saves its report (M1 item 15)', async ({ page }) => {
  await page.goto('/?engine=fake&autobench');
  // Every case opens its synthetic RAW and drags through the session; then the color check.
  await expect(page.getByRole('status').filter({ hasText: 'Autobench done' })).toContainText('5 of 5 cases', {
    timeout: 60_000,
  });
  await expect(page.getByRole('img', { name: 'Photo' })).toBeVisible();
});

test('gallery shows every shared component', async ({ page }) => {
  await page.goto('/#/gallery');
  for (const code of ['C1', 'C2', 'C3', 'C4', 'C5', 'C6', 'C7', 'C8', 'C9', 'C10', 'C11', 'C12']) {
    await expect(page.getByRole('heading', { name: new RegExp(`^${code} ·`) })).toBeVisible();
  }
});
