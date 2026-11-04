import type { Locator } from '@playwright/test';

// Location, retention and usage share the Rust Settings worker. A native action
// starts only after the rendered read controls acknowledge completion; never
// replay the action or hide an error returned by that action.
export async function waitForStorageReads(card: Locator, expect: typeof import('@playwright/test').expect, timeout: number): Promise<void> {
  await expect(card.getByRole('button', { name: 'Refresh retention', exact: true })).toBeEnabled({ timeout });
  await expect(card.getByRole('button', { name: 'Refresh usage', exact: true })).toBeEnabled({ timeout });
}
