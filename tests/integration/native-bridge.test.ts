import { beforeEach, describe, expect, it, vi } from 'vitest';

const tauri = vi.hoisted(() => ({ native: true, invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => tauri.native,
  invoke: (...args: unknown[]) => tauri.invoke(...args),
}));

beforeEach(() => { tauri.native = true; tauri.invoke.mockReset(); vi.resetModules(); });

describe('narrow native UI bridge', () => {
  it('passes only an approval identifier to api_send', async () => {
    const { sendNativeApi } = await import('../../apps/desktop/src/bridge');
    tauri.invoke.mockResolvedValue({ phase: 'RESTORED_LOCAL' });
    await sendNativeApi('synthetic-approval');
    expect(tauri.invoke).toHaveBeenCalledExactlyOnceWith('api_send', { approvalId: 'synthetic-approval' });
    expect(Object.keys(tauri.invoke.mock.calls[0][1])).toEqual(['approvalId']);
  });

  it('does not place long API requests ahead of Core edits or cancellation', async () => {
    const { bridge, sendNativeApi } = await import('../../apps/desktop/src/bridge');
    let finish!: (value: unknown) => void;
    const response = new Promise(resolve => { finish = resolve; });
    tauri.invoke.mockImplementation((command: string) => command === 'api_send' ? response : Promise.resolve({ phase: 'DRAFT' }));
    const apiRequest = sendNativeApi('synthetic-approval');
    const result = await Promise.race([
      bridge({ op: 'cancel' }).then(() => 'canceled'),
      new Promise(resolve => setTimeout(() => resolve('blocked-by-api'), 100)),
    ]);
    expect(result).toBe('canceled');
    expect(tauri.invoke).toHaveBeenCalledWith('dispatch', { request: { op: 'cancel' } });
    finish({ phase: 'BLOCKED' });
    await apiRequest;
  });

  it('keeps a pending browser login outside the Core queue and permits cancellation', async () => {
    const { bridge, subscriptionSignIn, subscriptionCancel } = await import('../../apps/desktop/src/bridge');
    let finish!: (value: unknown) => void;
    const login = new Promise(resolve => { finish = resolve; });
    tauri.invoke.mockImplementation((command: string) => command === 'subscription_sign_in' ? login : Promise.resolve({ state: 'disconnected' }));
    const pending = subscriptionSignIn();
    await bridge({ op: 'cancel' });
    await subscriptionCancel();
    expect(tauri.invoke.mock.calls).toEqual([
      ['subscription_sign_in'],
      ['dispatch', { request: { op: 'cancel' } }],
      ['subscription_cancel'],
    ]);
    finish({ state: 'disconnected' });
    await pending;
  });

  it('passes only a delivery mode and optional model to subscription selection', async () => {
    const { subscriptionStatus, subscriptionSelect } = await import('../../apps/desktop/src/bridge');
    tauri.invoke.mockResolvedValue({ state: 'disconnected' });
    await subscriptionStatus();
    await subscriptionSelect('api_key');
    await subscriptionSelect('chatgpt_plan', 'synthetic-public-model');
    expect(tauri.invoke.mock.calls).toEqual([
      ['subscription_status'],
      ['subscription_select', { mode: 'api_key' }],
      ['subscription_select', { mode: 'chatgpt_plan', model: 'synthetic-public-model' }],
    ]);
  });

  it('does not pass session data, filenames, or keys to vault commands', async () => {
    const { saveCase, loadCase, deleteCase } = await import('../../apps/desktop/src/bridge');
    tauri.invoke.mockResolvedValue({});
    await saveCase();
    await loadCase('synthetic-entry');
    await deleteCase('synthetic-entry');
    expect(tauri.invoke.mock.calls).toEqual([
      ['vault_save', undefined],
      ['vault_load', { id: 'synthetic-entry' }],
      ['vault_delete', { id: 'synthetic-entry' }],
    ]);
  });

  it('routes a user-provided key only to the local key-registration command', async () => {
    const { setApiKey } = await import('../../apps/desktop/src/bridge');
    tauri.invoke.mockResolvedValue(undefined);
    await setApiKey('OpenAI', 'SYNTHETIC_NOT_A_REAL_KEY');
    expect(tauri.invoke).toHaveBeenCalledExactlyOnceWith('api_set_key', { provider: 'OpenAI', key: 'SYNTHETIC_NOT_A_REAL_KEY' });
  });

  it('keeps browser API and vault capabilities unavailable', async () => {
    tauri.native = false;
    const { apiStatus, getVaultStatus, sendNativeApi, setApiKey, saveCase, subscriptionStatus, subscriptionSignIn, subscriptionCancel, subscriptionSelect } = await import('../../apps/desktop/src/bridge');
    expect(await apiStatus()).toEqual({ configuredOpenAI: false, configuredClaude: false });
    expect((await getVaultStatus()).available).toBe(false);
    await expect(sendNativeApi('synthetic-approval')).rejects.toThrow('ブラウザ');
    await expect(setApiKey('Claude', 'SYNTHETIC_NOT_A_REAL_KEY')).rejects.toThrow('デスクトップ');
    await expect(saveCase()).rejects.toThrow('デスクトップ');
    expect((await subscriptionStatus()).state).toBe('disconnected');
    await expect(subscriptionSignIn()).rejects.toThrow('デスクトップ');
    await expect(subscriptionCancel()).rejects.toThrow('デスクトップ');
    await expect(subscriptionSelect('chatgpt_plan')).rejects.toThrow('デスクトップ');
    expect(tauri.invoke).not.toHaveBeenCalled();
  });
});
