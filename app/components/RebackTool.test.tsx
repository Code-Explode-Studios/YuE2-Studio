import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('../context/I18nContext', () => ({ useI18n: () => ({ t: (key: string) => key, language: 'en' }) }));

const { RebackTool } = await import('./RebackTool');

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
let root: Root;
let host: HTMLDivElement;
const calls: { path: string; body: unknown }[] = [];

afterEach(() => {
  act(() => root?.unmount());
  host?.remove();
  calls.length = 0;
  vi.unstubAllGlobals();
});

const song = (id: string, title: string, derived?: { from: string; tool: string }, created_at = '2026-01-01') =>
  ({ id, title, created_at, metadata: derived ? { derived } : {} }) as never;
const songs = [
  song('orig', 'Bei Mir Bist Du Schön'),
  song('c1', 'Cover one', { from: 'orig', tool: 'cover' }, '2026-01-02'),
  song('c2', 'Cover two', { from: 'orig', tool: 'cover' }, '2026-01-03'),
  song('other', 'Another cover', { from: 'elsewhere', tool: 'cover' }),
];

function serve(status: unknown, answers: Record<string, { status: number; body: unknown }> = {}) {
  vi.stubGlobal('fetch', vi.fn(async (path: string, init?: RequestInit) => {
    calls.push({ path, body: init?.body ? JSON.parse(String(init.body)) : undefined });
    const answer = path === '/v1/reback' ? { status: 200, body: status } : answers[path] ?? { status: 200, body: { started: true } };
    return new Response(JSON.stringify(answer.body), { status: answer.status, headers: { 'Content-Type': 'application/json' } });
  }));
}

async function open(props: Partial<React.ComponentProps<typeof RebackTool>> = {}) {
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
  await act(async () => root.render(<RebackTool songId="orig" songTitle="Bei Mir Bist Du Schön" songs={songs} hasVocals card="" onDone={() => undefined} {...props} />));
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });
}

const button = (label: string) => [...document.querySelectorAll('button')].find(entry => entry.textContent?.includes(label));
const click = async (element: Element | undefined) => {
  expect(element).toBeDefined();
  await act(async () => { (element as HTMLElement).click(); });
};

describe('RebackTool', () => {
  it('offers only the covers of the chosen track, newest first, and starts with the one chosen', async () => {
    serve({ run: null, separator_ready: true });
    await open();
    const options = [...document.querySelectorAll('option')].map(option => option.textContent);
    expect(options).toEqual(['Cover two', 'Cover one']);
    await click(button('rebackStart'));
    const start = calls.find(call => call.path === '/v1/reback/start');
    expect(start?.body).toMatchObject({ song_id: 'orig', cover_id: 'c2', align_on: 'stems', vocal_gain_db: 0, backing_gain_db: 0, tune: true });
  });

  it('sends the track to Create - Cover when it has no cover yet', async () => {
    serve({ run: null, separator_ready: true });
    const sent: unknown[] = [];
    const listen = (event: Event) => sent.push((event as CustomEvent).detail);
    window.addEventListener('yue:transcribe-song', listen);
    await open({ songs: [songs[0]] });
    expect(host.textContent).toContain('rebackNoCover');
    expect(button('rebackStart')!.disabled).toBe(true);
    await click(button('rebackMakeCover'));
    window.removeEventListener('yue:transcribe-song', listen);
    expect(sent).toHaveLength(1);
    expect((sent[0] as { song: { id: string } }).song.id).toBe('orig');
  });

  it('asks for stems or HT-Demucs when the track has no vocal stem and nothing can split it', async () => {
    serve({ run: null, separator_ready: false });
    await open({ hasVocals: false });
    expect(host.textContent).toContain('rebackNeedsStems');
    expect(button('rebackStart')!.disabled).toBe(true);
  });

  it('shows the stage at work, and the result with what the run found', async () => {
    const run = { song_id: 'orig', cover_id: 'c2', title: 'Bei Mir Bist Du Schön', started_ms: Date.now(), error: null, library_song: null, report: null };
    serve({ run: { ...run, stage: 'aligning', done: false }, separator_ready: true });
    await open();
    expect(host.textContent).toContain('rebackStage_aligning');
    await click(button('restoreStop'));
    expect(calls.some(call => call.path === '/v1/reback/cancel')).toBe(true);

    act(() => root.unmount());
    host.remove();
    const report = { tuning_cents: 32.4, tuning_measured: true, original_seconds: 180, cover_seconds: 175, slowest: 0.91, fastest: 1.12, map: [] };
    serve({ run: { ...run, stage: 'done', done: true, library_song: 'new', report }, separator_ready: true });
    await open();
    expect(host.textContent).toContain('rebackReport');
  });
});
