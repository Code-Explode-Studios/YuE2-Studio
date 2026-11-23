import React, { useEffect, useState } from 'react';
import { useI18n } from '../context/I18nContext';

type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;

export const RuntimeSettings: React.FC = () => {
  const { t } = useI18n();
  const invoke = (window as unknown as { __TAURI__?: { core?: { invoke?: Invoke } } }).__TAURI__?.core?.invoke;
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [working, setWorking] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    if (invoke) {
      void invoke('runtime_player')
        .then(value => { if (!cancelled) setEnabled(value === true); })
        .catch(reason => { if (!cancelled) setError(String(reason)); });
    }
    return () => { cancelled = true; };
  }, [invoke]);

  const change = async (next: boolean) => {
    if (!invoke) return;
    setError(null);
    setWorking(true);
    try {
      const saved = await invoke('set_runtime_player', { enabled: next });
      setEnabled(saved === true);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setWorking(false);
    }
  };

  return (
    <section className="max-w-xl space-y-3 text-sm">
      <label className="flex items-center gap-3 font-semibold text-zinc-900 dark:text-white">
        <input
          type="checkbox"
          checked={enabled === true}
          disabled={!invoke || enabled === null || working}
          onChange={event => void change(event.target.checked)}
          className="h-4 w-4 accent-pink-500"
        />
        {t('shellRuntimeEnabled')}
      </label>
      <p className="text-xs leading-5 text-zinc-500 dark:text-zinc-400">{t('shellRuntimeHint')}</p>
      {!invoke && <p className="text-xs text-zinc-500">{t('shellRuntimeDesktopOnly')}</p>}
      {error && <p role="alert" className="text-xs text-rose-600 dark:text-rose-300">{error}</p>}
    </section>
  );
};
