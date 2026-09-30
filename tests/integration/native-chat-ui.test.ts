// @vitest-environment jsdom
import { act, createElement } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DeliveryMode, Snapshot, SubscriptionStatus } from '../../apps/desktop/src/types';

const fixture = vi.hoisted(() => {
  const original = '架空のこもれび商店へ、予定変更のお知らせを書いてください。';
  const base = (): Snapshot => ({
    caseId: 'synthetic-case', revision: 1, phase: 'DRAFT', original,
    purpose: 'polish', destination: 'ChatGPT', mode: 'local', transformed: '',
    findings: [], residual: ['日程変更のお知らせであること。'], lost: ['正式な組織名。'],
    blockedReason: null, approvalId: null, jobId: null, restored: '', unknownTokens: [],
    testCopy: null, dictionaryText: '架空のこもれび商店', policyVersion: 'synthetic-policy', modelConfig: 'synthetic-model',
  });
  const disconnected = (): SubscriptionStatus => ({ state: 'disconnected', reason: null, model: null, models: [], deliveryMode: 'api_key' });
  const connected = (): SubscriptionStatus => ({ state: 'connected', reason: null, model: 'synthetic-plan-model', models: [{ slug: 'synthetic-plan-model', displayName: 'Synthetic plan model' }, { slug: 'synthetic-plan-model-2', displayName: 'Synthetic plan model 2' }], deliveryMode: 'chatgpt_plan' });
  return { original, base, disconnected, connected, current: base(), subscription: disconnected(), native: true, bridge: vi.fn(), startNewCase: vi.fn(), modelStatus: vi.fn(), reloadModel: vi.fn(), prepare: vi.fn(), approve: vi.fn(), sendNative: vi.fn(), runDemo: vi.fn(), setApiKey: vi.fn(), apiStatus: vi.fn(), apiConfigured: true, subscriptionStatus: vi.fn(), subscriptionSelect: vi.fn(), subscriptionSignIn: vi.fn(), subscriptionCancel: vi.fn() };
});

vi.mock('../../packages/cushion-sdk/src/index', () => ({ createCushionClient: () => ({
  prepare: fixture.prepare, approve: fixture.approve, sendNative: fixture.sendNative, runDemo: fixture.runDemo,
}) }));
vi.mock('../../apps/desktop/src/bridge', () => ({
  isNative: () => fixture.native, SAMPLE: fixture.original, bridge: fixture.bridge, startNewCase: fixture.startNewCase,
  modelStatus: fixture.modelStatus, reloadModel: fixture.reloadModel,
  apiStatus: fixture.apiStatus,
  setApiKey: fixture.setApiKey, sendNativeApi: vi.fn(),
  subscriptionStatus: fixture.subscriptionStatus, subscriptionSelect: fixture.subscriptionSelect,
  subscriptionSignIn: fixture.subscriptionSignIn, subscriptionCancel: fixture.subscriptionCancel,
}));

import EmbeddedChat from '../../apps/desktop/src/EmbeddedChat';

let container: HTMLDivElement;
let root: Root;

beforeEach(async () => {
  fixture.current = fixture.base(); fixture.apiConfigured = true; fixture.native = true; fixture.subscription = fixture.disconnected();
  fixture.bridge.mockReset(); fixture.startNewCase.mockReset(); fixture.prepare.mockReset(); fixture.approve.mockReset();
  fixture.sendNative.mockReset(); fixture.runDemo.mockReset(); fixture.setApiKey.mockReset();
  fixture.modelStatus.mockReset(); fixture.reloadModel.mockReset();
  fixture.apiStatus.mockReset(); fixture.apiStatus.mockImplementation(async () => ({ configuredOpenAI: fixture.apiConfigured, configuredClaude: fixture.apiConfigured }));
  fixture.subscriptionStatus.mockReset(); fixture.subscriptionSelect.mockReset(); fixture.subscriptionSignIn.mockReset(); fixture.subscriptionCancel.mockReset();
  fixture.subscriptionStatus.mockImplementation(async () => fixture.subscription);
  fixture.subscriptionSelect.mockImplementation(async (mode: DeliveryMode, model?: string) => {
    fixture.subscription = { ...fixture.subscription, deliveryMode: mode, ...(model === undefined ? {} : { model }) };
    return fixture.subscription;
  });
  fixture.subscriptionSignIn.mockImplementation(async () => { fixture.subscription = fixture.connected(); return fixture.subscription; });
  fixture.subscriptionCancel.mockImplementation(async () => { fixture.subscription = { ...fixture.disconnected(), deliveryMode: fixture.subscription.deliveryMode }; return fixture.subscription; });
  fixture.modelStatus.mockResolvedValue({ state: 'ready', modelName: 'Synthetic local model', modelConfig: 'synthetic-model', reason: null, contextTokens: 8192, sourceVerified: true, networkIsolated: true });
  fixture.reloadModel.mockResolvedValue(undefined);
  fixture.bridge.mockImplementation(async (request: { op: string; patch?: Partial<Snapshot> }) => {
    if (request.op === 'update') fixture.current = { ...fixture.current, ...request.patch, phase: 'DRAFT', revision: fixture.current.revision + 1, approvalId: null };
    if (request.op === 'cancel') fixture.current = { ...fixture.current, phase: 'DRAFT', approvalId: null };
    return fixture.current;
  });
  fixture.prepare.mockImplementation(async () => {
    fixture.current = { ...fixture.current, phase: 'REVIEW', transformed: '[[CUSHION_00000000000000000000000000000001]]へ日程変更を伝えてください。' };
    return fixture.current;
  });
  fixture.approve.mockResolvedValue('synthetic-approval');
  fixture.setApiKey.mockResolvedValue(undefined);
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { callback(0); return 1; });
  HTMLDialogElement.prototype.showModal = function () { this.setAttribute('open', ''); };
  HTMLDialogElement.prototype.close = function () { this.removeAttribute('open'); };
  container = document.createElement('div'); document.body.append(container);
  root = createRoot(container);
  await act(async () => { root.render(createElement(EmbeddedChat)); });
});

afterEach(async () => {
  await act(async () => root.unmount()); container.remove(); vi.useRealTimers(); vi.unstubAllGlobals();
});

function button(label: string): HTMLButtonElement {
  const found = Array.from(container.querySelectorAll('button')).find(element => element.textContent?.replace(/[→＋]/g, '').trim() === label);
  if (!found) throw new Error(`Missing synthetic UI button: ${label}`);
  return found;
}
async function click(label: string) { await act(async () => button(label).click()); }
async function enterText(element: HTMLInputElement | HTMLTextAreaElement, value: string) {
  const prototype = element instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  await act(async () => {
    Object.getOwnPropertyDescriptor(prototype, 'value')!.set!.call(element, value);
    element.dispatchEvent(new Event('input', { bubbles: true }));
  });
}
async function selectValue(label: string, value: string) {
  const element = container.querySelector(`select[aria-label="${label}"]`) as HTMLSelectElement;
  expect(element).not.toBeNull();
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value')!.set!.call(element, value);
    element.dispatchEvent(new Event('change', { bubbles: true }));
  });
}
async function reviewAndConfirm() {
  await click('機密箇所を確認・変換');
  const confirmation = container.querySelector('.inline-confirmation input') as HTMLInputElement;
  expect(confirmation).not.toBeNull();
  await act(async () => confirmation.click());
}

describe('native chat UI response adoption and key handling', () => {
  it('never reads API-key status when the native host selects the ChatGPT plan at startup', async () => {
    fixture.subscription = fixture.connected();
    fixture.apiStatus.mockClear();
    fixture.apiStatus.mockImplementation(() => new Promise(() => {}));
    await act(async () => root.render(createElement(EmbeddedChat, { key: 'plan-startup' })));
    expect(container.textContent).toContain('ローカルモデル準備済み');
    expect(container.textContent).toContain('ChatGPTの契約：接続済み · Synthetic plan model');
    expect(container.textContent).not.toContain('APIキー未設定');
    await click('接続設定');
    expect(container.querySelector('input[type="password"]')).toBeNull();
    await reviewAndConfirm();
    expect(button('確認して変換文をChatGPTに送る').disabled).toBe(false);
    expect(fixture.apiStatus).not.toHaveBeenCalled();
    expect(fixture.sendNative).not.toHaveBeenCalled();
  });

  it('shows the ready local model while connection lookup is pending, without claiming API mode', async () => {
    let finish!: (status: SubscriptionStatus) => void;
    fixture.apiStatus.mockClear();
    fixture.subscriptionStatus.mockReturnValue(new Promise(resolve => { finish = resolve; }));
    await act(async () => root.render(createElement(EmbeddedChat, { key: 'pending-plan-startup' })));
    expect(container.textContent).toContain('ローカルモデル準備済み');
    expect(container.textContent).toContain('接続方式を確認中・送信不可');
    expect(container.textContent).not.toContain('ローカル検査・API連携');
    expect(container.textContent).not.toContain('APIキー未設定');
    expect(fixture.apiStatus).not.toHaveBeenCalled();
    await act(async () => finish(fixture.connected()));
    expect(container.textContent).toContain('ChatGPTの契約：接続済み · Synthetic plan model');
    expect(fixture.apiStatus).not.toHaveBeenCalled();
  });

  it('ignores a delayed API-mode refresh after an explicit newer plan selection', async () => {
    let finish!: (status: SubscriptionStatus) => void;
    fixture.apiStatus.mockClear();
    fixture.subscriptionStatus.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
    await click('接続設定');
    await selectValue('OpenAIの接続方式', 'chatgpt_plan');
    await act(async () => finish(fixture.disconnected()));
    expect(container.textContent).toContain('ChatGPTの契約：未接続・送信不可');
    expect(container.querySelector('input[type="password"]')).toBeNull();
    expect(fixture.apiStatus).not.toHaveBeenCalled();
    expect(fixture.sendNative).not.toHaveBeenCalled();
  });

  it('invalidates a review on plan selection and never reuses an API key for a disconnected plan', async () => {
    await reviewAndConfirm();
    await click('接続設定');
    await selectValue('OpenAIの接続方式', 'chatgpt_plan');
    expect(fixture.subscriptionSelect).toHaveBeenCalledExactlyOnceWith('chatgpt_plan', undefined);
    expect(fixture.bridge).toHaveBeenCalledWith({ op: 'cancel' });
    expect(container.querySelector('.inline-confirmation')).toBeNull();
    expect((container.querySelector('#chat-composer') as HTMLTextAreaElement).value).toBe(fixture.original);
    expect(container.querySelector('input[type="password"]')).toBeNull();
    await reviewAndConfirm();
    expect(button('確認して変換文をChatGPTに送る').disabled).toBe(true);
    await click('確認して変換文をChatGPTに送る');
    expect(fixture.apiConfigured).toBe(true);
    expect(fixture.approve).not.toHaveBeenCalled();
    expect(fixture.sendNative).not.toHaveBeenCalled();
    expect(fixture.subscriptionSignIn).not.toHaveBeenCalled();
  });

  it('requires a model from the connected account catalog and invalidates model changes', async () => {
    fixture.subscription = { ...fixture.connected(), model: 'not-in-the-synthetic-catalog' };
    await click('接続設定');
    await reviewAndConfirm();
    expect(button('確認して変換文をChatGPTに送る').disabled).toBe(true);
    await selectValue('利用するChatGPTモデル', 'synthetic-plan-model-2');
    expect(fixture.subscriptionSelect).toHaveBeenCalledExactlyOnceWith('chatgpt_plan', 'synthetic-plan-model-2');
    expect(container.querySelector('.inline-confirmation')).toBeNull();
    expect((container.querySelector('#chat-composer') as HTMLTextAreaElement).value).toBe(fixture.original);
    await reviewAndConfirm();
    expect(container.querySelector('.inline-review')?.textContent).toContain('Synthetic plan model 2');
    expect(button('確認して変換文をChatGPTに送る').disabled).toBe(false);
    expect(fixture.sendNative).not.toHaveBeenCalled();
  });

  it('sends only an approval ID through the native host for a connected plan and labels the result', async () => {
    fixture.apiConfigured = false;
    fixture.subscription = fixture.connected();
    await click('接続設定');
    await reviewAndConfirm();
    fixture.sendNative.mockResolvedValue({ ...fixture.current, phase: 'RESTORED_LOCAL', restored: '架空の復元済み回答です。' });
    await click('確認して変換文をChatGPTに送る');
    expect(fixture.sendNative).toHaveBeenCalledExactlyOnceWith('synthetic-approval');
    expect(container.querySelector('.restored-reply')?.textContent).toContain('ChatGPT契約 · 公式連携');
    expect(fixture.setApiKey).not.toHaveBeenCalled();
  });

  it('allows login cancellation and edits, and ignores a canceled login completion', async () => {
    let finish!: (status: SubscriptionStatus) => void;
    fixture.subscriptionSignIn.mockReturnValue(new Promise(resolve => { finish = resolve; }));
    await reviewAndConfirm();
    await click('接続設定');
    await selectValue('OpenAIの接続方式', 'chatgpt_plan');
    await click('Continue with ChatGPT');
    expect(container.querySelector('.inline-confirmation')).toBeNull();
    expect(button('機密箇所を確認・変換').disabled).toBe(true);
    await click('接続を中止する');
    await enterText(container.querySelector('#chat-composer') as HTMLTextAreaElement, '接続中止後の新しい架空文です。');
    await act(async () => finish(fixture.connected()));
    expect(fixture.subscriptionCancel).toHaveBeenCalledExactlyOnceWith();
    expect((container.querySelector('#chat-composer') as HTMLTextAreaElement).value).toBe('接続中止後の新しい架空文です。');
    expect(container.textContent).toContain('未接続・送信不可');
    expect(container.textContent).not.toContain('ChatGPTとの公式接続を確認しました');
    expect(fixture.sendNative).not.toHaveBeenCalled();
  });

  it('discards a pending plan response after changing the delivery mode', async () => {
    let finish!: (snapshot: Snapshot) => void;
    fixture.subscription = fixture.connected();
    fixture.sendNative.mockReturnValue(new Promise(resolve => { finish = resolve; }));
    await click('接続設定');
    await reviewAndConfirm();
    const old = { ...fixture.current, phase: 'RESTORED_LOCAL' as const, restored: 'OLD_PLAN_REPLY_MUST_NOT_APPEAR' };
    await click('確認して変換文をChatGPTに送る');
    await selectValue('OpenAIの接続方式', 'api_key');
    await act(async () => finish(old));
    expect(container.textContent).not.toContain('OLD_PLAN_REPLY_MUST_NOT_APPEAR');
    expect(container.querySelector('.inline-confirmation')).toBeNull();
    expect((container.querySelector('#chat-composer') as HTMLTextAreaElement).value).toBe(fixture.original);
    expect(fixture.sendNative).toHaveBeenCalledExactlyOnceWith('synthetic-approval');
  });

  it('polls a pending connection until a validated catalog is available', async () => {
    vi.useFakeTimers();
    let finish!: (status: SubscriptionStatus) => void;
    fixture.subscriptionSignIn.mockImplementation(() => {
      fixture.subscription = { ...fixture.disconnected(), state: 'connecting', deliveryMode: 'chatgpt_plan' };
      return new Promise(resolve => { finish = resolve; });
    });
    await click('接続設定');
    await selectValue('OpenAIの接続方式', 'chatgpt_plan');
    await click('Continue with ChatGPT');
    fixture.subscription = fixture.connected();
    const callsBeforePoll = fixture.subscriptionStatus.mock.calls.length;
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    expect(fixture.subscriptionStatus.mock.calls.length).toBeGreaterThan(callsBeforePoll);
    expect(container.textContent).toContain('接続済み · Synthetic plan model');
    expect(button('機密箇所を確認・変換').disabled).toBe(false);
    await act(async () => finish(fixture.connected()));
    expect(fixture.approve).not.toHaveBeenCalled();
    expect(fixture.sendNative).not.toHaveBeenCalled();
  });

  it('does not expose authentication or poll native services in the browser demo', async () => {
    fixture.native = false;
    fixture.subscriptionStatus.mockClear(); fixture.subscriptionSignIn.mockClear(); fixture.subscriptionSelect.mockClear(); fixture.subscriptionCancel.mockClear(); fixture.modelStatus.mockClear();
    await act(async () => root.render(createElement(EmbeddedChat, { key: 'browser-demo' })));
    expect(container.querySelector('.api-settings-dialog')).toBeNull();
    expect(container.textContent).not.toContain('Continue with ChatGPT');
    expect(fixture.subscriptionStatus).not.toHaveBeenCalled();
    expect(fixture.subscriptionSignIn).not.toHaveBeenCalled();
    expect(fixture.subscriptionSelect).not.toHaveBeenCalled();
    expect(fixture.subscriptionCancel).not.toHaveBeenCalled();
    expect(fixture.modelStatus).not.toHaveBeenCalled();
  });

  it('prepares the support workflow in local Core and keeps API send unavailable without a key', async () => {
    fixture.apiConfigured = false;
    await act(async () => root.render(createElement(EmbeddedChat, { supportRecord: true, key: 'support' })));
    await click('架空の支援記録を入れる');
    const source=(container.querySelector('#chat-composer') as HTMLTextAreaElement).value;
    expect(source).toContain('最初は参加しなかった');
    expect(fixture.current.purpose).toBe('support_record');
    expect(container.textContent).toContain('診断・服薬・詳しい家庭事情を含む記録は停止');
    await reviewAndConfirm();
    expect(fixture.prepare).toHaveBeenCalledWith({ original: source, purpose:'support_record', destination:'ChatGPT' });
    expect(button('確認して変換文をAPIに送る').disabled).toBe(true);
    expect(fixture.sendNative).not.toHaveBeenCalled();
  });
  it('reloads a stopped model once, invalidates confirmation, preserves the draft, and follows readiness', async () => {
    vi.useFakeTimers();
    await reviewAndConfirm();
    let state = 'error';
    fixture.modelStatus.mockImplementation(async () => ({ state, modelName: 'Synthetic local model', modelConfig: 'synthetic-model', reason: state === 'error' ? '再読込して再検査してください。' : null, contextTokens: 8192, sourceVerified: true, networkIsolated: true }));
    let finish!: () => void;
    fixture.reloadModel.mockImplementation(() => new Promise<void>(resolve => { finish = resolve; }));
    await click('接続設定');
    const reload = button('モデルを再準備する');
    await act(async () => { reload.click(); reload.click(); });
    expect(fixture.reloadModel).toHaveBeenCalledExactlyOnceWith();
    expect(fixture.bridge).toHaveBeenCalledWith({ op: 'cancel' });
    expect(fixture.current.approvalId).toBeNull();
    expect(container.querySelector('.inline-confirmation')).toBeNull();
    expect(reload.disabled).toBe(true);
    expect(button('機密箇所を確認・変換').disabled).toBe(true);
    expect((container.querySelector('#chat-composer') as HTMLTextAreaElement).value).toBe(fixture.original);
    state = 'verifying';
    await act(async () => finish());
    expect(button('モデルを再準備する').disabled).toBe(true);
    state = 'ready';
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    expect(container.textContent).toContain('ローカルモデル準備済み');
    expect(button('モデルを再準備する').disabled).toBe(false);
    expect(fixture.sendNative).not.toHaveBeenCalled();
  });

  it('keeps a newer edit when a pending model reload finishes', async () => {
    let finish!: () => void;
    fixture.reloadModel.mockReturnValue(new Promise<void>(resolve => { finish = resolve; }));
    await click('接続設定');
    await click('モデルを再準備する');
    await act(async () => (container.querySelector('[aria-label="接続設定を閉じる"]') as HTMLButtonElement).click());
    await enterText(container.querySelector('#chat-composer') as HTMLTextAreaElement, '再準備中の新しい架空の原文です。');
    await act(async () => finish());
    expect((container.querySelector('#chat-composer') as HTMLTextAreaElement).value).toBe('再準備中の新しい架空の原文です。');
    expect(container.querySelector('.inline-confirmation')).toBeNull();
    expect(fixture.sendNative).not.toHaveBeenCalled();
  });

  it('keeps the original and confirmation invalidated when model reload fails', async () => {
    await reviewAndConfirm();
    fixture.reloadModel.mockRejectedValue(new Error('SYNTHETIC_HOST_DETAIL_MUST_NOT_APPEAR'));
    await click('接続設定');
    await click('モデルを再準備する');
    expect((container.querySelector('#chat-composer') as HTMLTextAreaElement).value).toBe(fixture.original);
    expect(container.querySelector('.inline-confirmation')).toBeNull();
    expect(container.textContent).toContain('モデルを再準備できませんでした。原文は残っています。');
    expect(container.textContent).not.toContain('SYNTHETIC_HOST_DETAIL_MUST_NOT_APPEAR');
    expect(button('モデルを再準備する').disabled).toBe(false);
    expect(fixture.sendNative).not.toHaveBeenCalled();
  });

  it('preserves an edit and stops immediately when Core resolves its update as BLOCKED', async () => {
    fixture.bridge.mockResolvedValue({ ...fixture.base(), phase: 'BLOCKED', blockedReason: '入力上限のため処理を止めています。' });
    const visible = '架空の拒否された入力も、ここで修正できます。';
    await enterText(container.querySelector('#chat-composer') as HTMLTextAreaElement, visible);
    expect((container.querySelector('#chat-composer') as HTMLTextAreaElement).value).toBe(visible);
    expect(container.textContent).toContain('入力上限のため処理を止めています');
    expect(container.querySelector('.embedded-room')?.classList.contains('embed-stopped')).toBe(true);
    expect(document.activeElement).toBe(container.querySelector('.embed-error h2'));
    expect(fixture.prepare).not.toHaveBeenCalled();
    expect(fixture.approve).not.toHaveBeenCalled();
    expect(fixture.sendNative).not.toHaveBeenCalled();
  });

  it('keeps the visible source when prepare returns BLOCKED with an older Core source', async () => {
    const visible = '架空の新しい入力は画面に残します。';
    await enterText(container.querySelector('#chat-composer') as HTMLTextAreaElement, visible);
    fixture.prepare.mockResolvedValue({ ...fixture.base(), original: '以前の架空文', phase: 'BLOCKED', blockedReason: '入力上限のため処理を止めています。' });
    await click('機密箇所を確認・変換');
    expect((container.querySelector('#chat-composer') as HTMLTextAreaElement).value).toBe(visible);
    expect(container.textContent).toContain('入力上限のため処理を止めています');
    expect(fixture.approve).not.toHaveBeenCalled();
    expect(fixture.sendNative).not.toHaveBeenCalled();
  });

  it('starts a blank local case without carrying over the reviewed text or confirmation', async () => {
    await reviewAndConfirm();
    fixture.startNewCase.mockResolvedValue({ ...fixture.base(), caseId: 'synthetic-new-case', original: '', mode: 'local', transformed: '', findings: [], approvalId: null, jobId: null });
    await click('この相談を閉じて、空の入力欄に戻る');
    expect(fixture.startNewCase).toHaveBeenCalledExactlyOnceWith();
    expect((container.querySelector('#chat-composer') as HTMLTextAreaElement).value).toBe('');
    expect(container.querySelector('.inline-confirmation')).toBeNull();
    expect(container.querySelector('.outbound-text')).toBeNull();
    expect(container.textContent).toContain('前の承認・回答・登録トークンは引き継ぎません');
    expect(fixture.sendNative).not.toHaveBeenCalled();
  });

  it('keeps explicit confirmation before any native send', async () => {
    await click('機密箇所を確認・変換');
    expect(button('確認して変換文をAPIに送る').disabled).toBe(true);
    await click('確認して変換文をAPIに送る');
    expect(fixture.approve).not.toHaveBeenCalled();
    expect(fixture.sendNative).not.toHaveBeenCalled();
  });

  it('discards an API response that arrives after the user edited the source', async () => {
    let finish!: (value: Snapshot) => void;
    fixture.sendNative.mockReturnValue(new Promise(resolve => { finish = resolve; }));
    await reviewAndConfirm();
    const old = { ...fixture.current, phase: 'RESTORED_LOCAL' as const, restored: 'OLD_SYNTHETIC_REPLY_MUST_NOT_APPEAR' };
    await click('確認して変換文をAPIに送る');
    expect(fixture.sendNative).toHaveBeenCalledExactlyOnceWith('synthetic-approval');
    await enterText(container.querySelector('#chat-composer') as HTMLTextAreaElement, '架空の文章を編集中です。');
    await act(async () => { finish(old); });
    expect(container.textContent).not.toContain('OLD_SYNTHETIC_REPLY_MUST_NOT_APPEAR');
    expect((container.querySelector('#chat-composer') as HTMLTextAreaElement).value).toBe('架空の文章を編集中です。');
    expect(button('機密箇所を確認・変換').disabled).toBe(false);
  });

  it('discards a late API response after stopping receipt and does not claim unsent', async () => {
    let finish!: (value: Snapshot) => void;
    fixture.sendNative.mockReturnValue(new Promise(resolve => { finish = resolve; }));
    await reviewAndConfirm();
    const old = { ...fixture.current, phase: 'RESTORED_LOCAL' as const, restored: 'CANCELED_SYNTHETIC_REPLY_MUST_NOT_APPEAR' };
    await click('確認して変換文をAPIに送る');
    expect(container.textContent).toContain('送信後のデータは取り消せません');
    await click('回答の受け取りを中止');
    await act(async () => { finish(old); });
    expect(container.textContent).not.toContain('CANCELED_SYNTHETIC_REPLY_MUST_NOT_APPEAR');
    expect(container.textContent).not.toContain('まだAIには送信していません');
    expect((container.querySelector('#chat-composer') as HTMLTextAreaElement).value).toBe(fixture.original);
    expect(fixture.bridge).toHaveBeenCalledWith({ op: 'cancel' });
  });

  it('clears a password input immediately while Keychain registration is pending', async () => {
    let finish!: () => void;
    fixture.setApiKey.mockReturnValue(new Promise<void>(resolve => { finish = resolve; }));
    await click('接続設定');
    const input = container.querySelector('input[type="password"]') as HTMLInputElement;
    await enterText(input, 'SYNTHETIC_API_KEY_NOT_REAL');
    await click('Macキーチェーンへ保存');
    expect(input.value).toBe('');
    expect(container.textContent).not.toContain('SYNTHETIC_API_KEY_NOT_REAL');
    expect(fixture.setApiKey).toHaveBeenCalledExactlyOnceWith('OpenAI', 'SYNTHETIC_API_KEY_NOT_REAL');
    await act(async () => { finish(); });
    expect(input.value).toBe('');
    expect(fixture.bridge).toHaveBeenCalledWith({ op: 'cancel' });
  });

  it('does not let delayed key registration replace a newer rejected visible draft', async () => {
    let finish!: () => void;
    fixture.setApiKey.mockReturnValue(new Promise<void>(resolve => { finish = resolve; }));
    await click('接続設定');
    await enterText(container.querySelector('input[type="password"]') as HTMLInputElement, 'SYNTHETIC_API_KEY_NOT_REAL');
    await click('Macキーチェーンへ保存');
    await act(async () => (container.querySelector('[aria-label="接続設定を閉じる"]') as HTMLButtonElement).click());
    fixture.bridge.mockImplementation(async (request: { op: string }) => {
      if (request.op === 'update') throw new Error('架空の入力上限エラー');
      return fixture.current;
    });
    await enterText(container.querySelector('#chat-composer') as HTMLTextAreaElement, 'SYNTHETIC_NEW_DRAFT_MUST_REMAIN');
    await act(async () => { finish(); });
    expect((container.querySelector('#chat-composer') as HTMLTextAreaElement).value).toBe('SYNTHETIC_NEW_DRAFT_MUST_REMAIN');
    expect(container.textContent).toContain('架空の入力上限エラー');
    expect(fixture.bridge.mock.calls.some(([request]) => request.op === 'cancel')).toBe(false);
    expect(container.textContent).not.toContain('SYNTHETIC_API_KEY_NOT_REAL');
  });
});
