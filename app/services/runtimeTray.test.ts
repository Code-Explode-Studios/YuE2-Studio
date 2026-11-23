import { afterEach, describe, expect, it, vi } from 'vitest';
import { observeTrayPlayerActions, publishTrayPlayback } from './runtimeTray';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('runtime system-tray music controls', () => {
  it('does nothing in the browser without Tauri', async () => {
    const action = vi.fn();
    await expect(publishTrayPlayback({ title: 'Test song', playing: true, playable: true })).resolves.toBeUndefined();
    expect(observeTrayPlayerActions(action)).toBeTypeOf('function');
    expect(action).not.toHaveBeenCalled();
  });

  it('publishes track changes to the existing desktop player tray', async () => {
    const invoke = vi.fn(async () => undefined);
    vi.stubGlobal('__TAURI__', { core: { invoke } });
    await publishTrayPlayback({ title: 'Rainy Days', playing: true, playable: true });
    expect(invoke).toHaveBeenCalledOnce();
    expect(invoke).toHaveBeenCalledWith('set_tray_player_state', {
      title: 'Rainy Days', playing: true, playable: true,
    });
  });

  it('accepts only actual previous/play-pause/next commands', async () => {
    let receive: ((event: { payload: unknown }) => void) | undefined;
    const unlisten = vi.fn();
    const listen = vi.fn(async (_event: string, callback: (event: { payload: unknown }) => void) => {
      receive = callback;
      return unlisten;
    });
    vi.stubGlobal('__TAURI__', { event: { listen } });
    const action = vi.fn();
    const stop = observeTrayPlayerActions(action);
    await Promise.resolve();
    expect(listen).toHaveBeenCalledWith('studio://tray-player-control', expect.any(Function));

    receive?.({ payload: { action: 'previous' } });
    receive?.({ payload: { action: 'toggle' } });
    receive?.({ payload: { action: 'next' } });
    receive?.({ payload: { action: 'quit' } });
    receive?.({ payload: null });
    expect(action.mock.calls.map(args => args[0])).toEqual(['previous', 'toggle', 'next']);
    stop();
    expect(unlisten).toHaveBeenCalledOnce();
    receive?.({ payload: { action: 'next' } });
    expect(action).toHaveBeenCalledTimes(3);
  });

  it('does not leak a late listener when the player unmounts', async () => {
    let finish: ((unlisten: () => void) => void) | undefined;
    const unlisten = vi.fn();
    const listen = vi.fn(() => new Promise<() => void>(resolve => { finish = resolve; }));
    vi.stubGlobal('__TAURI__', { event: { listen } });
    const action = vi.fn();
    observeTrayPlayerActions(action)();
    finish?.(unlisten);
    await Promise.resolve();
    expect(unlisten).toHaveBeenCalledOnce();
  });
});
