import React, { useEffect, useRef, useState } from 'react';
import { Download, Layers, Loader2, Play, Square, Wand2 } from 'lucide-react';
import { useI18n } from '../context/I18nContext';
import type { TranslationKey } from '../i18n/translations';
import { apiUrl } from '../services/apiBase';
import { mapNativeLibrarySong } from '../services/nativeLibrary';
import { saveFile } from '../services/saveFile';
import { SELF_READ } from '../services/studioQueries';
import { coversOf } from '../services/toolsSong';
import type { NativeLibrarySong } from '../types';
import { StemPlayer } from './StemPlayer';
import { WorkProgress } from './WorkProgress';

/**
 * Re-back on the tools page: the chosen track's own vocals over a new backing track, its instrumental cover tuned,
 * time-warped to the original's tempo and stretched. The cover is made first with Create - Cover; the result is a new
 * library track beside the original.
 */

export interface RebackReport {
  tuning_cents: number;
  tuning_measured: boolean;
  original_seconds: number;
  cover_seconds: number;
  slowest: number;
  fastest: number;
  map: [number, number][];
}

export interface RebackRun {
  song_id: string;
  cover_id: string;
  title: string;
  stage: 'stems' | 'tuning' | 'aligning' | 'stretching' | 'mixing' | 'saving' | 'done' | 'stopped';
  started_ms: number;
  done: boolean;
  error: string | null;
  report: RebackReport | null;
  library_song: string | null;
}

export interface RebackStatus {
  run: RebackRun | null;
  separator_ready: boolean;
}

const STAGES: RebackRun['stage'][] = ['stems', 'tuning', 'aligning', 'stretching', 'mixing', 'saving'];

export const RebackTool: React.FC<{
  songId: string;
  songTitle: string;
  songs: NativeLibrarySong[];
  /** The chosen track has its vocal stem already, so nothing needs splitting. */
  hasVocals: boolean;
  card: string;
  /** A run ended: the library has a new track. */
  onDone: () => void;
}> = ({ songId, songTitle, songs, hasVocals, card, onDone }) => {
  const { t } = useI18n();
  const [status, setStatus] = useState<RebackStatus | null>(null);
  const [coverId, setCoverId] = useState('');
  const [alignOn, setAlignOn] = useState<'stems' | 'mix'>('stems');
  const [vocalGain, setVocalGain] = useState(0);
  const [backingGain, setBackingGain] = useState(0);
  const [tune, setTune] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const finished = useRef(onDone);
  finished.current = onDone;

  const covers = coversOf(songs, songId);
  useEffect(() => {
    // the newest cover of the chosen track, until one is picked
    setCoverId(current => (covers.some(song => song.id === current) ? current : covers[0]?.id ?? ''));
  }, [songId, covers.map(song => song.id).join(',')]);

  useEffect(() => {
    let alive = true;
    let wasRunning = false;
    const read = async () => {
      const next: RebackStatus = await fetch('/v1/reback', { headers: SELF_READ }).then(response => response.json());
      if (!alive) return;
      setStatus(next);
      const running = Boolean(next.run && !next.run.done);
      if (wasRunning && !running) finished.current();
      wasRunning = running;
    };
    void read().catch(() => undefined);
    const timer = window.setInterval(() => void read().catch(() => undefined), 1000);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, []);

  const post = async (path: string, body: object) => {
    setBusy(true);
    setError(null);
    try {
      const response = await fetch(path, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
      if (!response.ok) {
        const problem = await response.json().catch(() => null);
        throw new Error(problem?.error || `HTTP ${response.status}`);
      }
    } catch (problem) {
      setError(problem instanceof Error ? problem.message : String(problem));
    } finally {
      setBusy(false);
    }
  };

  const start = () =>
    post('/v1/reback/start', { song_id: songId, cover_id: coverId, align_on: alignOn, vocal_gain_db: vocalGain, backing_gain_db: backingGain, tune });

  const makeCover = () => {
    const chosen = songs.find(song => song.id === songId);
    if (chosen) window.dispatchEvent(new CustomEvent('yue:transcribe-song', { detail: { song: mapNativeLibrarySong(chosen), melodyOnly: false } }));
  };

  const run = status?.run ?? null;
  const running = Boolean(run && !run.done);
  const needsSplit = !hasVocals && status !== null && !status.separator_ready;
  const canStart = Boolean(songId && coverId) && !running && !busy && !needsSplit;
  const resultUrl = run?.library_song ? apiUrl(`/v1/library/media/${encodeURIComponent(run.library_song)}`) : null;
  const step = run ? STAGES.indexOf(run.stage) : -1;
  const report = run?.report ?? null;
  const gain = (label: TranslationKey, value: number, change: (value: number) => void) => (
    <label className="flex items-center gap-2 text-xs text-zinc-600 dark:text-zinc-300">
      <span className="w-24 shrink-0">{t(label)}</span>
      <input type="range" min={-24} max={12} step={0.5} value={value} onChange={event => change(Number(event.target.value))} className="flex-1 accent-pink-500" />
      <span className="w-14 text-right tabular-nums">{value > 0 ? '+' : ''}{value} dB</span>
    </label>
  );

  return (
    <section className={card}>
      <div className="flex items-center gap-2 text-sm font-semibold text-zinc-900 dark:text-white">
        <Layers size={17} className="text-pink-500" /> {t('rebackTitle')}
      </div>
      <p className="mt-2 text-sm text-zinc-600 dark:text-zinc-300">{t('rebackHint')}</p>
      <p className="mt-2 truncate text-xs text-zinc-500">{songTitle || t('restoreNoSong')}</p>

      <label className="mt-3 block text-xs font-semibold text-zinc-600 dark:text-zinc-300" htmlFor="reback-cover">{t('rebackCover')}</label>
      {covers.length > 0 ? (
        <select
          id="reback-cover"
          value={coverId}
          onChange={event => setCoverId(event.target.value)}
          className="mt-1 w-full rounded-lg border border-zinc-200 bg-white px-3 py-2 text-sm text-zinc-900 dark:border-white/10 dark:bg-black/30 dark:text-white"
        >
          {covers.map(song => <option key={song.id} value={song.id}>{song.title}</option>)}
        </select>
      ) : (
        <p className="mt-1 text-xs leading-5 text-zinc-500 dark:text-zinc-400">{t('rebackNoCover')}</p>
      )}
      <button
        type="button"
        onClick={makeCover}
        disabled={!songId}
        className="mt-2 inline-flex items-center gap-2 rounded-lg border border-pink-500/30 px-3 py-1.5 text-xs font-bold text-pink-600 hover:bg-pink-500/10 disabled:opacity-50 dark:text-pink-300"
      >
        <Wand2 size={13} /> {t('rebackMakeCover')}
      </button>

      <div className="mt-3 space-y-2">
        <div role="radiogroup" aria-label={t('rebackAlignOn')} className="inline-flex rounded-lg border border-zinc-200 p-0.5 text-xs font-semibold dark:border-white/10">
          {(['stems', 'mix'] as const).map(way => (
            <button
              key={way}
              type="button"
              role="radio"
              aria-checked={alignOn === way}
              onClick={() => setAlignOn(way)}
              className={`rounded-md px-3 py-1.5 ${alignOn === way ? 'bg-pink-600 text-white' : 'text-zinc-600 hover:text-pink-600 dark:text-zinc-300'}`}
            >
              {t(way === 'stems' ? 'rebackAlignStems' : 'rebackAlignMix')}
            </button>
          ))}
        </div>
        {gain('rebackVocalGain', vocalGain, setVocalGain)}
        {gain('rebackBackingGain', backingGain, setBackingGain)}
        <label className="flex items-center gap-2 text-xs text-zinc-600 dark:text-zinc-300">
          <input type="checkbox" checked={tune} onChange={event => setTune(event.target.checked)} className="accent-pink-500" />
          {t('rebackTune')}
        </label>
      </div>

      {needsSplit && <p className="mt-3 text-xs leading-5 text-amber-700 dark:text-amber-300">{t('rebackNeedsStems')}</p>}

      <div className="mt-3 flex flex-wrap gap-2">
        <button
          type="button"
          onClick={() => void start()}
          disabled={!canStart}
          className="inline-flex items-center gap-2 rounded-lg bg-linear-to-r from-orange-500 to-pink-600 px-4 py-2 text-sm font-bold text-white disabled:cursor-not-allowed disabled:opacity-50"
        >
          {running ? <Loader2 size={14} className="animate-spin" /> : <Play size={14} />}
          {t('rebackStart')}
        </button>
        {running && (
          <button type="button" onClick={() => void post('/v1/reback/cancel', {})} className="inline-flex items-center gap-2 rounded-lg border border-zinc-300 px-3 py-2 text-xs font-semibold text-zinc-700 hover:border-pink-400 dark:border-white/15 dark:text-zinc-200">
            <Square size={12} /> {t('restoreStop')}
          </button>
        )}
      </div>

      {running && run && (
        <WorkProgress
          label={`${t('rebackRunning').replace('{title}', run.title)} · ${t(`rebackStage_${run.stage}` as TranslationKey)}`}
          percent={step >= 0 ? Math.round((step / STAGES.length) * 100) : null}
          elapsedMs={Math.max(0, Date.now() - run.started_ms)}
        />
      )}

      {!running && run?.stage === 'done' && resultUrl && (
        <div className="mt-3 space-y-2">
          <div className="flex items-center gap-2">
            <div className="min-w-0 flex-1">
              <StemPlayer src={resultUrl} label={t('rebackedLabel').replace('{title}', run.title)} />
            </div>
            <button
              type="button"
              onClick={() => void saveFile(`${run.title} - re-backed.wav`, { url: resultUrl })}
              className="shrink-0 text-zinc-400 hover:text-pink-500"
              title={t('download')}
            >
              <Download size={15} />
            </button>
          </div>
          {report && (
            <p className="text-[11px] leading-4 text-zinc-500">
              {t('rebackReport')
                .replace('{cents}', `${report.tuning_cents > 0 ? '+' : ''}${report.tuning_cents.toFixed(0)}`)
                .replace('{slowest}', `${Math.round(report.slowest * 100)}%`)
                .replace('{fastest}', `${Math.round(report.fastest * 100)}%`)}
            </p>
          )}
        </div>
      )}
      {!running && run?.stage === 'stopped' && !error && <p className="mt-3 text-xs text-zinc-500">{t('rebackStopped')}</p>}
      {(run?.error || error) && (
        <p role="alert" className="mt-3 select-text rounded-lg bg-rose-500/10 px-3 py-2 text-xs text-rose-700 wrap-anywhere dark:text-rose-300">{run?.error || error}</p>
      )}
    </section>
  );
};
