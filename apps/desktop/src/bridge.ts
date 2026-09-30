import { invoke, isTauri } from '@tauri-apps/api/core';
import type { ApiProvider, ApiStatus, CoreRequest, DeliveryMode, ModelStatus, Snapshot, SubscriptionStatus, VaultEntry, VaultStatus } from './types';

export const SAMPLE = 'こもれび商店の小森さんへ、発送日が9月20日から9月23日に変わるお知らせを書いてください。注文番号はKMR-4821です。';
export const isNative = () => isTauri();
type WasmCore = { dispatch(request: string): string };
let engine: Promise<WasmCore> | undefined;
function loadCore(): Promise<WasmCore> {
  return engine ??= (async () => {
    const { default: init, DemoCore } = await import('./wasm/cushion_core.js');
    await init();
    return new DemoCore();
  })();
}
// Every call is serialized, including initialization. A failed command does not
// poison the queue. The Rust Core remains the authority for revisions and export.
let tail: Promise<unknown> = Promise.resolve();
async function dispatchCore(request: CoreRequest): Promise<Snapshot> {
  if (isNative()) return invoke<Snapshot>('dispatch', { request });
  const core = await loadCore();
  return JSON.parse(core.dispatch(JSON.stringify(request))) as Snapshot;
}
export function bridge(request: CoreRequest): Promise<Snapshot> {
  const task = tail.then(() => dispatchCore(request));
  tail = task.catch(() => undefined);
  return task;
}

export function startNewCase(): Promise<Snapshot> {
  // Core resets the case, approval, jobs and token mapping. Establish the host's
  // mode within the same queue turn before exposing this blank case to the UI.
  const mode = isNative() ? 'local' : 'demo';
  const task = tail.then(async () => {
    const fresh = await dispatchCore({ op: 'new_case' });
    return fresh.mode === mode ? fresh : dispatchCore({ op: 'update', patch: { mode } });
  });
  tail = task.catch(() => undefined);
  return task;
}

export function modelStatus(): Promise<ModelStatus> {
  if (!isNative()) return Promise.resolve({
    state: 'unavailable', modelName: 'ローカルモデル', modelConfig: null,
    reason: 'ブラウザは架空データのデモ専用です。実推論はデスクトップアプリで確認してください。',
    contextTokens: null, sourceVerified: false, networkIsolated: false,
  });
  return invoke<ModelStatus>('model_status');
}
export const reloadModel = () => nativeOperation<void>('model_reload');

function nativeOperation<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isNative()) return Promise.reject(new Error('この操作はデスクトップアプリ内だけで利用できます。'));
  const task = tail.then(() => invoke<T>(command, args));
  tail = task.catch(() => undefined);
  return task;
}
export const getVaultStatus = (): Promise<VaultStatus> => isNative()
  ? nativeOperation<VaultStatus>('vault_status')
  : Promise.resolve({ available: false, unlocked: false, reason: 'ブラウザでは保存を利用できません。' });
export const unlockVault = () => nativeOperation<VaultStatus>('vault_unlock');
export const listVaultEntries = () => nativeOperation<VaultEntry[]>('vault_list');
export const saveCase = () => nativeOperation<VaultEntry>('vault_save');
export const loadCase = (id: string) => nativeOperation<Snapshot>('vault_load', { id });
export const lockVault = () => nativeOperation<VaultStatus>('vault_lock');
export const deleteCase = (id: string) => nativeOperation<boolean>('vault_delete', { id });

export function apiStatus(): Promise<ApiStatus> {
  return isNative() ? invoke<ApiStatus>('api_status') : Promise.resolve({ configuredOpenAI: false, configuredClaude: false });
}
export function setApiKey(provider: ApiProvider, key: string): Promise<void> {
  if (!isNative()) return Promise.reject(new Error('APIキーはデスクトップアプリ内だけで登録できます。'));
  return invoke<void>('api_set_key', { provider, key });
}
export function subscriptionStatus(): Promise<SubscriptionStatus> {
  return isNative() ? invoke<SubscriptionStatus>('subscription_status') : Promise.resolve({
    state: 'disconnected', reason: 'ブラウザデモではChatGPTへ接続しません。',
    model: null, models: [], deliveryMode: 'api_key',
  });
}
// Login waits for the system browser. Keep these commands outside the Core
// queue so cancellation and local edits remain available while it is pending.
export function subscriptionSignIn(): Promise<SubscriptionStatus> {
  if (!isNative()) return Promise.reject(new Error('ChatGPTとの接続はデスクトップアプリ内だけで利用できます。'));
  return invoke<SubscriptionStatus>('subscription_sign_in');
}
export function subscriptionCancel(): Promise<SubscriptionStatus> {
  if (!isNative()) return Promise.reject(new Error('ChatGPTとの接続はデスクトップアプリ内だけで利用できます。'));
  return invoke<SubscriptionStatus>('subscription_cancel');
}
export function subscriptionSelect(mode: DeliveryMode, model?: string): Promise<SubscriptionStatus> {
  if (!isNative()) return Promise.reject(new Error('接続方式の選択はデスクトップアプリ内だけで利用できます。'));
  return invoke<SubscriptionStatus>('subscription_select', { mode, ...(model === undefined ? {} : { model }) });
}
export function sendNativeApi(approvalId: string): Promise<Snapshot> {
  if (!isNative()) return Promise.reject(new Error('ブラウザでは外部APIへ送信しません。'));
  // Keep the long provider call off this JavaScript queue. UI epochs reject
  // late results immediately; the host serializes Core changes around its
  // in-flight export transaction. Cancel cannot retract a request already sent.
  return invoke<Snapshot>('api_send', { approvalId });
}
