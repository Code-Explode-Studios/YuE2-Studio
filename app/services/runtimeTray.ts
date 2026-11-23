/** Bridge from the hidden Windows system-tray media controls to the *existing*
 * player in App. Nothing is registered in a browser or in a separate webview.
 */
export type TrayPlayerAction = 'previous' | 'toggle' | 'next';
export type TrayPlayback = { title: string | null; playing: boolean; playable: boolean };
type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;
type Unlisten = () => void;
type Listen = (event: string, callback: (event: { payload: unknown }) => void) => Promise<Unlisten>;

function shell(): { invoke?: Invoke; listen?: Listen } {
  const bridge = (window as unknown as {
    __TAURI__?: { core?: { invoke?: Invoke }; event?: { listen?: Listen } }
  }).__TAURI__;
  return { invoke: bridge?.core?.invoke, listen: bridge?.event?.listen };
}

/** Called when the main player changes tracks or pauses; not on every audio tick. */
export async function publishTrayPlayback(state: TrayPlayback): Promise<void> {
  const invoke = shell().invoke;
  if (!invoke) return;
  try {
    await invoke('set_tray_player_state', state);
  } catch (error) {
    console.warn('[runtime tray] cannot update playback state:', error);
  }
}

/** Subscribe once, keeping the current player callbacks in App's ref. If a
 * view unmounts while Tauri's async registration is pending, unsubscribe
 * immediately after registration finishes rather than leaking a listener.
 */
export function observeTrayPlayerActions(onAction: (action: TrayPlayerAction) => void): Unlisten {
  const listen = shell().listen;
  if (!listen) return () => {};
  let active = true;
  let stop: Unlisten | null = null;
  void listen('studio://tray-player-control', event => {
    if (!active) return;
    const action = (event.payload as { action?: unknown } | null)?.action;
    if (action === 'previous' || action === 'toggle' || action === 'next') onAction(action);
  }).then(unlisten => {
    if (active) stop = unlisten;
    else unlisten();
  }).catch(error => {
    if (active) console.warn('[runtime tray] cannot listen for playback controls:', error);
  });
  return () => { active = false; stop?.(); };
}
