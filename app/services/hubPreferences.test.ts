import { afterEach, describe, expect, it, vi } from 'vitest';
import { HUB_PROMOTIONS_KEY, setHubPromotions, showHubPromotions } from './hubPreferences';

afterEach(() => {
  localStorage.removeItem(HUB_PROMOTIONS_KEY);
  vi.restoreAllMocks();
});

describe('Studio Hub promotional notice preference', () => {
  it('defaults to visible so existing users keep the previous behaviour', () => {
    expect(showHubPromotions()).toBe(true);
  });
  it('persists opting out and announces the change immediately', () => {
    const changed = vi.fn();
    window.addEventListener('studio:hub-promotions-changed', changed);
    setHubPromotions(false);
    expect(showHubPromotions()).toBe(false);
    expect(localStorage.getItem(HUB_PROMOTIONS_KEY)).toBe('0');
    expect(changed).toHaveBeenCalledTimes(1);
    setHubPromotions(true);
    expect(showHubPromotions()).toBe(true);
    window.removeEventListener('studio:hub-promotions-changed', changed);
  });
});
