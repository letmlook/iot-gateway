import { test, expect } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.goto('/login');
  await page.waitForLoadState('networkidle');
  await page.locator('input.login-input').first().fill('admin');
  await page.locator('input.login-input').nth(1).fill('admin123');
  await page.getByRole('button', { name: '登录' }).click();
  await page.waitForURL('**/dashboard', { timeout: 8000 });
});

test.describe('登录', () => {
  test('正确凭据登录跳转 dashboard', async ({ page }) => {
    await expect(page).toHaveURL(/\/dashboard/);
  });
});

test.describe('南向设备', () => {
  test('列表页可访问', async ({ page }) => {
    await page.goto('/south');
    await page.waitForLoadState('networkidle');
    await page.waitForTimeout(500);
    await expect(page.getByRole('heading', { name: '南向设备' })).toBeVisible();
    await expect(page).not.toHaveURL(/\/login/);
  });

  test('新建页可访问', async ({ page }) => {
    await page.goto('/south/new');
    await page.waitForLoadState('networkidle');
    await page.waitForTimeout(500);
    await expect(page).not.toHaveURL(/\/login/);
  });
});

test.describe('北向应用', () => {
  test('列表页可访问', async ({ page }) => {
    await page.goto('/north');
    await page.waitForLoadState('networkidle');
    await page.waitForTimeout(500);
    await expect(page).not.toHaveURL(/\/login/);
  });

  test('新建页可访问', async ({ page }) => {
    await page.goto('/north/new');
    await page.waitForLoadState('networkidle');
    await page.waitForTimeout(500);
    await expect(page).not.toHaveURL(/\/login/);
  });
});

test.describe('数据流编排', () => {
  test('Flow 列表页可访问', async ({ page }) => {
    await page.goto('/flows');
    await page.waitForLoadState('networkidle');
    await page.waitForTimeout(500);
    await expect(page).not.toHaveURL(/\/login/);
  });

  test('Flow 创建页可访问', async ({ page }) => {
    await page.goto('/flows/new');
    await page.waitForLoadState('networkidle');
    await page.waitForTimeout(500);
    await expect(page).not.toHaveURL(/\/login/);
  });
});

test.describe('导航', () => {
  test('侧边栏完整可见', async ({ page }) => {
    await page.goto('/dashboard');
    await page.waitForLoadState('networkidle');
    await expect(page.locator('.el-aside, aside').first()).toBeVisible({ timeout: 3000 });
  });

  test('主要页面均可访问不重定向登录', async ({ page }) => {
    const routes = [
      '/dashboard', '/south', '/south/new',
      '/north', '/north/new',
      '/flows', '/flows/new',
      '/plugins', '/monitor',
      '/settings/license', '/settings/info',
    ];
    for (const path of routes) {
      await page.goto(path);
      await page.waitForLoadState('networkidle');
      await page.waitForTimeout(300);
      await expect(page).not.toHaveURL(/\/login/, `页面 ${path} 跳转到了登录页`);
    }
  });
});
