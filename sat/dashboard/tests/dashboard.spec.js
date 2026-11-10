import { test, expect } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const screenshotDir = fileURLToPath(
  new URL('../../target/dashboard-preview/', import.meta.url),
);

test('pie drilldown, filters, pagination, full arguments and responsive layout', async ({
  page,
}) => {
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('dialog', () => {
    throw new Error('Stored arguments must never execute as HTML');
  });
  await page.goto('/');
  await expect(
    page.getByRole('heading', { name: 'Tool activity.' }),
  ).toBeVisible();
  await expect(
    page.getByRole('button', { name: /python: 50 executions/ }),
  ).toBeVisible();
  mkdirSync(screenshotDir, { recursive: true });
  await page.screenshot({
    path: `${screenshotDir}/overview.png`,
    fullPage: true,
  });
  await page.getByRole('button', { name: /python: 50 executions/ }).click();
  await expect(
    page.getByRole('heading', { name: /python executions/ }),
  ).toBeVisible();
  await expect(
    page.getByRole('combobox', { name: 'Filter by tool' }),
  ).toHaveValue('python');
  await expect(page.locator('tbody tr')).toHaveCount(25);
  await page.getByRole('button', { name: 'Next page' }).click();
  await expect(page.getByText('Page 2', { exact: true })).toBeVisible();
  await page
    .getByRole('combobox', { name: 'Filter by status' })
    .selectOption('failed');
  await expect(page.getByText('Page 1', { exact: true })).toBeVisible();
  await expect(page.locator('tbody .status-badge.failed')).toHaveCount(8);
  await page
    .getByRole('textbox', { name: 'Search tools or arguments' })
    .fill('--read');
  await page
    .getByRole('button', { name: /^View execution / })
    .first()
    .click();
  const dialog = page.getByRole('dialog');
  await expect(
    dialog.getByRole('heading', { name: 'All provided arguments 4' }),
  ).toBeVisible();
  await expect(dialog.locator('.argument-list')).toContainText(
    '<img src=x onerror=alert(1)>',
  );
  await expect(dialog.locator('.argument-list img')).toHaveCount(0);
  await page.screenshot({
    path: `${screenshotDir}/execution.png`,
    fullPage: true,
  });
  await page.keyboard.press('Escape');
  await expect(dialog).not.toBeVisible();
  await page
    .getByRole('textbox', { name: 'Search tools or arguments' })
    .fill('no-match-example');
  await expect(
    page.getByRole('heading', { name: 'No matching executions' }),
  ).toBeVisible();
  await page
    .getByRole('button', { name: 'History settings', exact: true })
    .click();
  await expect(page.getByRole('dialog')).toContainText('sat dashboard reset');
  await page.keyboard.press('Escape');
  await page.getByRole('textbox', { name: 'Search tools or arguments' }).fill('');
  await page.getByRole('combobox', { name: 'Filter by tool' }).selectOption('');
  await page.getByRole('combobox', { name: 'Filter by status' }).selectOption('');
  await expect(page.locator('tbody tr')).toHaveCount(25);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({
    path: `${screenshotDir}/mobile.png`,
    fullPage: true,
  });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBeTruthy();
  expect(errors).toEqual([]);
});


test('empty history, disabled recording, and request recovery', async ({ page }) => {
  test.skip(!process.env.SAT_DASHBOARD_HOME, 'Needs the integration suite temporary registry');
  const sat = fileURLToPath(new URL(`../../target/release/sat${process.platform === 'win32' ? '.exe' : ''}`, import.meta.url));
  execFileSync(sat, ['dashboard', 'disable']);
  execFileSync(sat, ['dashboard', 'reset']);
  await page.goto('/');
  await expect(page.getByText('Recording disabled', { exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Your next command starts the story' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'A fresh start' })).toBeVisible();
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: `${screenshotDir}/empty.png`, fullPage: true });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBeTruthy();
  await page.route('**/api/overview', route => route.fulfill({ status: 500, body: '{}' }));
  await page.getByRole('button', { name: 'Refresh dashboard' }).click();
  await expect(page.getByRole('alert')).toContainText('Dashboard request failed');
  await page.unroute('**/api/overview');
  await page.getByRole('button', { name: /Retry/ }).click();
  await expect(page.getByRole('alert')).not.toBeVisible();
});
