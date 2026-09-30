// @vitest-environment jsdom
import { act, createElement } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { initSync, DemoCore } from '../../apps/desktop/src/wasm/cushion_core.js';
import type { ModelStatus, Snapshot } from '../../apps/desktop/src/types';

const fixture = vi.hoisted(() => {
  const base = (): Snapshot => ({
    caseId: 'synthetic-case', revision: 1, phase: 'DRAFT', original: '架空の手紙です。', purpose: 'polish', destination: 'ChatGPT', mode: 'local',
    transformed: '', findings: [], residual: ['お知らせであること。'], lost: [], blockedReason: null, approvalId: null, jobId: null,
    restored: '', unknownTokens: [], testCopy: null, dictionaryText: '架空の辞書', policyVersion: 'test-policy', modelConfig: 'test-model',
  });
  return { base, current: base(), bridge: vi.fn(), startNewCase: vi.fn(), modelStatus: vi.fn(), save: vi.fn(), load: vi.fn(), list: vi.fn(), events: [] as string[] };
});
vi.mock('../../apps/desktop/src/bridge', () => ({
  bridge: fixture.bridge, startNewCase: fixture.startNewCase, isNative: () => true,
  modelStatus: fixture.modelStatus,
  reloadModel: vi.fn(), getVaultStatus: async () => ({ available: true, unlocked: true, reason: null }),
  unlockVault: vi.fn(), lockVault: vi.fn(), deleteCase: vi.fn(), saveCase: fixture.save, loadCase: fixture.load, listVaultEntries: fixture.list,
}));
import App from '../../apps/desktop/src/App';

let container: HTMLDivElement;
let root: Root;
const entry = { id: 'synthetic-saved-case', createdMs: 1790550000000, bytes: 256 };
beforeAll(() => initSync({ module: readFileSync('apps/desktop/src/wasm/cushion_core_bg.wasm') }));

beforeEach(async () => {
  fixture.current = fixture.base(); fixture.events.length = 0;
  fixture.bridge.mockReset(); fixture.startNewCase.mockReset(); fixture.modelStatus.mockReset(); fixture.save.mockReset(); fixture.load.mockReset(); fixture.list.mockReset();
  fixture.modelStatus.mockResolvedValue({ state: 'ready', modelName: 'Synthetic model', modelConfig: 'test-model', reason: null, sourceVerified: true, networkIsolated: true, contextTokens: 8192 });
  fixture.bridge.mockImplementation(async (request: { op: string; patch?: Partial<Snapshot> }) => {
    if (request.op === 'update') fixture.current = { ...fixture.current, ...request.patch, phase: 'DRAFT', revision: fixture.current.revision + 1 };
    return fixture.current;
  });
  fixture.list.mockResolvedValue([entry]);
  fixture.save.mockImplementation(async () => { fixture.events.push(`save:${fixture.current.original}`); return entry; });
  fixture.load.mockImplementation(async () => {
    fixture.events.push('load');
    fixture.current = { ...fixture.base(), caseId: 'synthetic-loaded-case', revision: 20, original: '架空の保存済み文です。' };
    return fixture.current;
  });
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal('matchMedia', () => ({ matches: false }));
  HTMLDialogElement.prototype.showModal = function () { this.setAttribute('open', ''); };
  HTMLDialogElement.prototype.close = function () { this.removeAttribute('open'); };
  container = document.createElement('div'); document.body.append(container); root = createRoot(container);
  await act(async () => { root.render(createElement(App)); });
});

afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.useRealTimers(); vi.unstubAllGlobals(); });
function button(label: string) {
  const result = Array.from(container.querySelectorAll('button')).find(element => element.textContent?.replace(/→/g, '').trim() === label);
  if (!result) throw new Error(`Missing synthetic UI button: ${label}`);
  return result;
}
async function click(label: string) { await act(async () => button(label).click()); }
async function enterOriginal(value: string) {
  const input = container.querySelector('#original-input') as HTMLTextAreaElement;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
}
function pauseFirstEdit() {
  let resume!: () => void;
  let updates = 0;
  fixture.bridge.mockImplementation(async (request: { op: string; patch?: Partial<Snapshot> }) => {
    if (request.op === 'update') {
      if (++updates === 1) await new Promise<void>(resolve => { resume = resolve; });
      fixture.current = { ...fixture.current, ...request.patch, phase: 'DRAFT', revision: fixture.current.revision + 1 };
      fixture.events.push(`update:${fixture.current.original}`);
    }
    return fixture.current;
  });
  return () => resume();
}

describe('native App scheduling boundaries', () => {
  for (const action of ['edit', 'cancel']) it(`keeps following a busy model after ${action} returns the document to DRAFT`, async () => {
    vi.useFakeTimers();
    let state: ModelStatus['state'] = 'busy';
    fixture.modelStatus.mockImplementation(async () => ({ state, modelName: 'Synthetic model', modelConfig: 'test-model', reason: null, sourceVerified: true, networkIsolated: true, contextTokens: 8192 }));
    fixture.bridge.mockImplementation(async (request: { op: string; patch?: Partial<Snapshot> }) => {
      if (request.op === 'update') fixture.current = { ...fixture.current, ...request.patch, phase: 'DRAFT', jobId: null };
      if (request.op === 'cancel') fixture.current = { ...fixture.current, phase: 'DRAFT', jobId: null };
      if (request.op === 'begin_scan') fixture.current = { ...fixture.current, phase: 'SCANNING', jobId: 'synthetic-local-job' };
      return fixture.current;
    });
    await click('機密箇所を確認・変換');
    if (action === 'edit') await enterOriginal('編集中の架空の手紙を残します。');
    else await click('キャンセル');
    expect(fixture.current.phase).toBe('DRAFT');
    await click('設定');
    expect(button('モデルを再準備する').disabled).toBe(true);
    state = 'ready';
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    expect(container.textContent).toContain('モデルの準備ができています');
    expect(button('モデルを再準備する').disabled).toBe(false);
    const statusCalls = fixture.modelStatus.mock.calls.length;
    await act(async () => { await vi.advanceTimersByTimeAsync(3000); });
    expect(fixture.modelStatus.mock.calls.length).toBe(statusCalls);
    expect((container.querySelector('#original-input') as HTMLTextAreaElement).value).toBe(action === 'edit' ? '編集中の架空の手紙を残します。' : fixture.base().original);
  });

  it('preserves actual Core size-rejected drafts, never scans/saves older text, and permits shortening back', async () => {
    const core = new DemoCore();
    const source = fixture.base().original;
    core.dispatch(JSON.stringify({ op: 'update', patch: { original: source, mode: 'local' } }));
    let release!: () => void;
    let hold = true;
    fixture.bridge.mockImplementation(async (request: { op: string; patch?: Partial<Snapshot> }) => {
      if (request.op === 'update' && hold) {
        hold = false;
        await new Promise<void>(resolve => { release = resolve; });
      }
      fixture.current = JSON.parse(core.dispatch(JSON.stringify(request)));
      return fixture.current;
    });
    await act(async () => { root.unmount(); root = createRoot(container); root.render(createElement(App)); });
    const oversized = 'X'.repeat(65537);
    await enterOriginal(oversized);
    await click('設定');
    await click('この相談を暗号化して保存');
    await act(async () => release());
    expect((container.querySelector('#original-input') as HTMLTextAreaElement).value.length).toBe(65537);
    expect(fixture.current.phase).toBe('BLOCKED');
    expect(fixture.current.original).toBe(source);
    expect(container.querySelector('.letter-room')?.classList.contains('motion-off')).toBe(true);
    expect(fixture.save).not.toHaveBeenCalled();
    await act(async () => (container.querySelector('[aria-label="設定を閉じる"]') as HTMLButtonElement).click());
    await click('機密箇所を確認・変換');
    expect(fixture.bridge.mock.calls.some(([request]) => request.op === 'begin_scan')).toBe(false);
    expect((container.querySelector('#original-input') as HTMLTextAreaElement).value.length).toBe(65537);
    await enterOriginal(source);
    expect(fixture.current.phase).toBe('DRAFT');
    expect((container.querySelector('#original-input') as HTMLTextAreaElement).value).toBe(source);
    expect(container.querySelector('#letter-main [role="alert"]')).toBeNull();
    expect(button('この相談を暗号化して保存').disabled).toBe(false);
  });

  it('keeps an actual Core-rejected dictionary across later source edits without silently restoring the older dictionary', async () => {
    const core = new DemoCore();
    core.dispatch(JSON.stringify({ op: 'update', patch: { original: fixture.base().original, mode: 'local' } }));
    fixture.bridge.mockImplementation(async (request: { op: string }) => {
      fixture.current = JSON.parse(core.dispatch(JSON.stringify(request)));
      return fixture.current;
    });
    await act(async () => { root.unmount(); root = createRoot(container); root.render(createElement(App)); });
    await click('設定');
    const dictionary = container.querySelector('.technical-inner textarea') as HTMLTextAreaElement;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(dictionary, 'D'.repeat(8193));
      dictionary.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await enterOriginal('新しい架空の原文です。');
    expect(dictionary.value.length).toBe(8193);
    expect((container.querySelector('#original-input') as HTMLTextAreaElement).value).toBe('新しい架空の原文です。');
    expect(button('この相談を暗号化して保存').disabled).toBe(true);
    await click('機密箇所を確認・変換');
    expect(fixture.bridge.mock.calls.some(([request]) => request.op === 'begin_scan')).toBe(false);
  });

  it('builds a synthetic answer from exported text without guessing candidate order or honorifics', async () => {
    const transformed = '発送日は[[CUSHION_00000000000000000000000000000001]]。宛先は[[CUSHION_00000000000000000000000000000002]]。';
    fixture.current = { ...fixture.base(), phase: 'EXPORTED', transformed, approvalId: 'synthetic-approved', findings: [
      { start: 0, end: 3, original: '9月23日', replacement: '[[CUSHION_00000000000000000000000000000001]]', action: '置換', reason: '日付' },
      { start: 4, end: 8, original: '小森さん', replacement: '[[CUSHION_00000000000000000000000000000002]]', action: '置換', reason: '敬称を含む氏名' },
    ] };
    await act(async () => { root.unmount(); root = createRoot(container); root.render(createElement(App)); });
    await click('AIの回答を貼り付け');
    await click('架空の回答を入れる');
    const answer = (container.querySelector('#answer-input') as HTMLTextAreaElement).value;
    expect(answer).toContain(transformed);
    expect(answer).not.toContain(']]さん');
    expect(answer).not.toContain('注文番号');
    expect(answer).toContain('外部AIの回答ではありません');
  });

  it('closes a case through the host-mode lifecycle and shows a clean local draft', async () => {
    fixture.startNewCase.mockResolvedValue({ ...fixture.base(), caseId: 'synthetic-new-case', original: '', mode: 'local', transformed: '', findings: [], approvalId: null, jobId: null });
    await click('設定');
    await click('この相談を閉じて、空の便箋に戻る');
    expect(fixture.startNewCase).toHaveBeenCalledExactlyOnceWith();
    expect((container.querySelector('#original-input') as HTMLTextAreaElement).value).toBe('');
    expect(container.textContent).toContain('実推論の試作・架空サンプルの持ち出しのみ');
    expect(container.querySelector('.confirmation')).toBeNull();
  });

  it('saves only after all edits already queued by the user have reached Core', async () => {
    const resume = pauseFirstEdit();
    await enterOriginal('架空の第一編集');
    await enterOriginal('架空の第二編集');
    await click('設定');
    await click('この相談を暗号化して保存');
    expect(fixture.save).not.toHaveBeenCalled();
    await act(async () => resume());
    expect(fixture.events).toEqual(['update:架空の第一編集', 'update:架空の第二編集', 'update:架空の第二編集', 'save:架空の第二編集']);
  });

  it('refuses to save older Core text when a pending visible edit is rejected after Save was clicked', async () => {
    let rejectPending!: () => void;
    let first = true;
    fixture.bridge.mockImplementation(async (request: { op: string; patch?: Partial<Snapshot> }) => {
      if (request.op === 'update' && request.patch?.original === '架空の保存できない最新文') {
        if (first) {
          first = false;
          await new Promise<void>((_resolve, reject) => { rejectPending = () => reject(new Error('架空の入力上限エラー')); });
        }
        throw new Error('架空の入力上限エラー');
      }
      return fixture.current;
    });
    await enterOriginal('架空の保存できない最新文');
    await click('設定');
    await click('この相談を暗号化して保存');
    await act(async () => rejectPending());
    expect(fixture.save).not.toHaveBeenCalled();
    expect((container.querySelector('#original-input') as HTMLTextAreaElement).value).toBe('架空の保存できない最新文');
    expect(container.textContent).toContain('架空の入力上限エラー');
  });

  it('loads after prior edits and does not let an older queued edit overwrite the loaded case', async () => {
    const resume = pauseFirstEdit();
    await enterOriginal('架空の第一編集');
    await enterOriginal('架空の第二編集');
    await click('設定');
    const select = Array.from(container.querySelectorAll('select')).find(element => Array.from(element.options).some(option => option.value === entry.id))!;
    await act(async () => { select.value = entry.id; select.dispatchEvent(new Event('change', { bubbles: true })); });
    await click('選択した保存を開く');
    expect(fixture.load).not.toHaveBeenCalled();
    await act(async () => resume());
    expect(fixture.events).toEqual(['update:架空の第一編集', 'update:架空の第二編集', 'load']);
    expect((container.querySelector('#original-input') as HTMLTextAreaElement).value).toBe('架空の保存済み文です。');
    expect(container.textContent).toContain('承認は引き継がず');
  });

  it('polls a native worker result and never sends demo finish_scan for local inference', async () => {
    vi.useFakeTimers();
    let scanning = false;
    fixture.bridge.mockImplementation(async (request: { op: string }) => {
      if (request.op === 'begin_scan') { scanning = true; fixture.current = { ...fixture.current, revision: 2, phase: 'SCANNING', jobId: 'synthetic-local-job' }; }
      else if (request.op === 'snapshot' && scanning) fixture.current = { ...fixture.current, phase: 'REVIEW', jobId: null, transformed: '架空の変換済み文章' };
      return fixture.current;
    });
    await click('機密箇所を確認・変換');
    expect(container.textContent).toContain('ローカルモデルで機密候補を調べています');
    await act(async () => { await vi.advanceTimersByTimeAsync(500); });
    expect(container.textContent).toContain('外へ渡す文');
    expect(fixture.bridge.mock.calls.some(([request]) => request.op === 'finish_scan')).toBe(false);
  });

  it('keeps a rejected visible draft and never starts scanning the Core older source', async () => {
    fixture.bridge.mockImplementation(async (request: { op: string; patch?: Partial<Snapshot> }) => {
      if (request.op === 'update' && request.patch?.original === '架空の上限超過文') throw new Error('架空の入力上限エラー');
      return fixture.current;
    });
    await enterOriginal('架空の上限超過文');
    await click('機密箇所を確認・変換');
    expect(fixture.bridge.mock.calls.some(([request]) => request.op === 'begin_scan')).toBe(false);
    expect((container.querySelector('#original-input') as HTMLTextAreaElement).value).toBe('架空の上限超過文');
    await click('設定');
    expect(button('この相談を暗号化して保存').disabled).toBe(true);
  });
});
