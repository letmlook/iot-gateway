import { test, expect } from '@playwright/test';

test.describe('登录', () => {
  test('正确凭据登录成功', async ({ page }) => {
    await page.goto('/login');
    await page.waitForLoadState('networkidle');
    // 原生 input.login-input
    await page.locator('input.login-input').first().fill('admin');
    await page.locator('input.login-input').nth(1).fill('admin123');
    await page.getByRole('button', { name: '登录' }).click();
    await page.waitForURL('**/dashboard', { timeout: 8000 });
    await expect(page).toHaveURL(/\/dashboard/);
  });

  test('错误密码登录失败', async ({ page }) => {
    await page.goto('/login');
    await page.waitForLoadState('networkidle');
    await page.locator('input.login-input').first().fill('admin');
    await page.locator('input.login-input').nth(1).fill('wrongpass');
    await page.getByRole('button', { name: '登录' }).click();
    await page.waitForTimeout(2000);
    // 停留在登录页（不跳转 dashboard）
    await expect(page).toHaveURL(/\/login/);
  });

  test('未登录访问 dashboard 跳转登录', async ({ page }) => {
    await page.goto('/dashboard');
    await page.waitForURL('**/login**', { timeout: 5000 });
    await expect(page).toHaveURL(/\/login/);
  });
});
