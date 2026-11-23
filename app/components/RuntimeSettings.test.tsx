import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { RuntimeSettings } from './RuntimeSettings';

vi.mock('../context/I18nContext', () => ({
  useI18n: () => ({ t: (key: string) => key }),
}));

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
let root: Root | null = null;
let host: HTMLDivElement | null = null;

afterEach(() => {
  act(() => root?.unmount());
  host?.remove();
  root = null;
  host = null;
  vi.unstubAllGlobals();
});

describe('Runtime Settings', () => {
  it('starts disabled and saves the background player switch', async () => {
    const invoke = vi.fn(async (command: string, args?: { enabled: boolean }) => {
      if (command === 'runtime_player') return false;
      if (command === 'set_runtime_player') return args?.enabled ?? false;
      throw new Error('unexpected command');
    });
    vi.stubGlobal('__TAURI__', { core: { invoke } });
    host = document.createElement('div');
    document.body.append(host);
    root = createRoot(host);
    await act(async () => { root?.render(<RuntimeSettings />); });

    const checkbox = host.querySelector<HTMLInputElement>('input[type="checkbox"]');
    expect(checkbox?.checked).toBe(false);
    expect(checkbox?.disabled).toBe(false);
    await act(async () => { checkbox?.click(); });
    expect(invoke).toHaveBeenCalledWith('set_runtime_player', { enabled: true });
    expect(checkbox?.checked).toBe(true);
    await act(async () => { checkbox?.click(); });
    expect(invoke).toHaveBeenCalledWith('set_runtime_player', { enabled: false });
    expect(checkbox?.checked).toBe(false);
  });

  it('does not offer a nonfunctional toggle in a browser', async () => {
    host = document.createElement('div');
    document.body.append(host);
    root = createRoot(host);
    await act(async () => { root?.render(<RuntimeSettings />); });
    expect(host.querySelector<HTMLInputElement>('input')?.disabled).toBe(true);
    expect(host.textContent).toContain('shellRuntimeDesktopOnly');
  });
});
