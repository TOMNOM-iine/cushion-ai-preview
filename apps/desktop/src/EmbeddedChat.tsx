import { useEffect, useMemo, useRef, useState } from 'react';
import { createCushionClient } from '../../../packages/cushion-sdk/src/index';
import { apiStatus, bridge, isNative, modelStatus, reloadModel, SAMPLE, sendNativeApi, setApiKey, startNewCase, subscriptionCancel, subscriptionSelect, subscriptionSignIn, subscriptionStatus } from './bridge';
import supportFixtures from '../../../tests/fixtures/workflows/support-record.json';
import type { ApiProvider, ApiStatus, DeliveryMode, Destination, ModelStatus, Purpose, Snapshot, SubscriptionStatus } from './types';

type Stage = 'draft' | 'checking' | 'review' | 'sending' | 'complete' | 'blocked';
interface RoundTripResult { snapshot: Snapshot; outboundText: string; providerAnswer?: string; restored: string; kind: 'demo' | 'native'; }

function RoomMark({ small = false }: { small?: boolean }) {
  return (
    <svg className={small ? 'embed-leaf small' : 'embed-leaf'} viewBox="0 0 40 40" fill="none" aria-hidden="true">
      <path d="M30 8C16 6 7 12 10 23c2 8 15 10 21-1 2-4 1-10-1-14Z" stroke="currentColor" strokeWidth="1.3" />
      <path d="m9 33 17-20M17 24l-1-9M21 20l8 1" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
    </svg>
  );
}

function LocalMark() {
  return (
    <svg width="14" height="15" viewBox="0 0 15 16" fill="none" aria-hidden="true">
      <rect x="2" y="7" width="11" height="8" rx="1" stroke="currentColor" />
      <path d="M4.5 7V4a3 3 0 0 1 6 0v3" stroke="currentColor" />
    </svg>
  );
}

function errorText(error: unknown) {
  return error instanceof Error ? error.message : typeof error === 'string' ? error : '処理が完了しませんでした。';
}

export default function EmbeddedChat({ supportRecord = false }: { supportRecord?: boolean } = {}) {
  const native = isNative();
  const client = useMemo(() => createCushionClient({
    dispatch: bridge,
    ...(native ? { localOnlyHost: { isNative: true as const, sendApproved: sendNativeApi } } : {}),
  }), [native]);
  const [original, setOriginal] = useState('');
  const [purpose, setPurpose] = useState<Purpose>(supportRecord ? 'support_record' : 'polish');
  const [destination, setDestination] = useState<Destination>('ChatGPT');
  const [stage, setStage] = useState<Stage>('draft');
  const [review, setReview] = useState<Snapshot | null>(null);
  const [result, setResult] = useState<RoundTripResult | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [ready, setReady] = useState(false);
  const [quiet, setQuiet] = useState(false);
  const [companion, setCompanion] = useState(true);
  const [announcement, setAnnouncement] = useState('');
  const [model, setModel] = useState<ModelStatus | null>(null);
  const [api, setApi] = useState<ApiStatus>({ configuredOpenAI: false, configuredClaude: false });
  const [subscription, setSubscription] = useState<SubscriptionStatus>({ state: 'disconnected', reason: null, model: null, models: [], deliveryMode: 'api_key' });
  const [connectionChecked, setConnectionChecked] = useState(false);
  const [apiChecked, setApiChecked] = useState(false);
  const [modelStatusError, setModelStatusError] = useState('');
  const [deliveryStatusError, setDeliveryStatusError] = useState('');
  const [subscriptionSelecting, setSubscriptionSelecting] = useState(false);
  const [connectionOpen, setConnectionOpen] = useState(false);
  const [keyProvider, setKeyProvider] = useState<ApiProvider>('OpenAI');
  const [apiKey, setKey] = useState('');
  const [keyBusy, setKeyBusy] = useState(false);
  const [modelReloading, setModelReloading] = useState(false);
  const [connectionError, setConnectionError] = useState('');
  const [connectionNotice, setConnectionNotice] = useState('');
  const epoch = useRef(0);
  const connectionEpoch = useRef(0);
  const modelRefreshEpoch = useRef(0);
  const deliveryRefreshEpoch = useRef(0);
  const inFlight = useRef(false);
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const reviewHeading = useRef<HTMLHeadingElement>(null);
  const replyHeading = useRef<HTMLHeadingElement>(null);
  const blockedHeading = useRef<HTMLHeadingElement>(null);
  const connectionDialog = useRef<HTMLDialogElement>(null);
  const connectionButton = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (stage === 'blocked') blockedHeading.current?.focus();
  }, [stage, error]);

  useEffect(() => {
    let active = true;
    bridge({ op: 'snapshot' }).then(async snapshot => {
      if (native && snapshot.mode !== 'local') snapshot = await bridge({ op: 'update', patch: { mode: 'local' } });
      if (supportRecord && snapshot.purpose !== 'support_record') snapshot = await bridge({ op: 'update', patch: { purpose: 'support_record' } });
      if (!active) return;
      setOriginal(snapshot.original);
      setPurpose(snapshot.purpose);
      setDestination(snapshot.destination === 'Claude' ? 'Claude' : 'ChatGPT');
      setReady(true);
    }).catch(reason => {
      if (active) {
        setError(errorText(reason));
        setStage('blocked');
      }
    });
    if (native) refreshConnection();
    return () => { active = false; };
  }, []);

  async function refreshConnection() {
    const current = connectionEpoch.current;
    const modelRequest = ++modelRefreshEpoch.current;
    const deliveryRequest = ++deliveryRefreshEpoch.current;
    const modelCheck = modelStatus().then(status => {
      if (modelRequest !== modelRefreshEpoch.current) return;
      setModel(status); setModelStatusError('');
    }).catch(() => {
      if (modelRequest !== modelRefreshEpoch.current) return;
      setModel(null);
      setModelStatusError('ローカルモデルの状態を確認できませんでした。外部送信を止めています。');
    });
    const deliveryCheck = subscriptionStatus().then(async status => {
      if (current !== connectionEpoch.current || deliveryRequest !== deliveryRefreshEpoch.current) return;
      setSubscription(status); setConnectionChecked(true); setConnectionError('');
      await refreshApiForSelection(status, current, deliveryRequest);
    }).catch(() => {
      if (current !== connectionEpoch.current || deliveryRequest !== deliveryRefreshEpoch.current) return;
      setConnectionChecked(false); setApiChecked(false);
      setApi({ configuredOpenAI: false, configuredClaude: false });
      setSubscription(previous => ({ ...previous, state: 'error', model: null, models: [] }));
      setDeliveryStatusError('接続設定の状態を確認できませんでした。外部送信を止めています。');
    });
    // Each branch publishes its own result. A permission prompt for one
    // connection must not hide a ready local worker or block plan-only setup.
    await Promise.all([modelCheck, deliveryCheck]);
  }

  async function refreshApiForSelection(status: SubscriptionStatus, current: number, request: number) {
    setApiChecked(false); setDeliveryStatusError('');
    setApi({ configuredOpenAI: false, configuredClaude: false });
    if (status.deliveryMode !== 'api_key') return;
    try {
      const configured = await apiStatus();
      if (current !== connectionEpoch.current || request !== deliveryRefreshEpoch.current) return;
      setApi(configured); setApiChecked(true);
    } catch {
      if (current !== connectionEpoch.current || request !== deliveryRefreshEpoch.current) return;
      setDeliveryStatusError('APIキーの登録状態を確認できませんでした。外部送信を止めています。');
    }
  }

  useEffect(() => {
    const dialog = connectionDialog.current;
    if (connectionOpen && dialog && !dialog.open) dialog.showModal();
    if (!connectionOpen && dialog?.open) dialog.close();
  }, [connectionOpen]);

  useEffect(() => {
    if (!native || (model?.state !== 'verifying' && model?.state !== 'busy')) return;
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      const status = await modelStatus().catch(() => null);
      if (!active) return;
      setModel(status);
      if (status?.state === 'verifying' || status?.state === 'busy') timer = setTimeout(poll, 1000);
    };
    timer = setTimeout(poll, 1000);
    return () => { active = false; clearTimeout(timer); };
  }, [native, model?.state]);

  useEffect(() => {
    if (!native || subscription.state !== 'connecting') return;
    let active = true;
    const current = connectionEpoch.current;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      const status = await subscriptionStatus().catch(() => null);
      if (!active || current !== connectionEpoch.current) return;
      if (status) { setSubscription(status); setConnectionChecked(true); }
      else {
        setSubscription(previous => ({ ...previous, state: 'error', model: null, models: [] }));
        setConnectionError('ChatGPTとの接続状態を確認できませんでした。送信を止めています。');
      }
      if (status?.state === 'connecting') timer = setTimeout(poll, 1000);
    };
    timer = setTimeout(poll, 1000);
    return () => { active = false; clearTimeout(timer); };
  }, [native, subscription.state]);

  const clearConnectionReview = () => {
    const current = ++epoch.current;
    setConfirmed(false); setReview(null); setResult(null); setError(''); setStage('draft');
    setConnectionError(''); setConnectionNotice('');
    setAnnouncement('接続設定を変更しました。原文を残して、前の確認・承認を取り消しました。再検査してください。');
    return current;
  };

  const selectSubscription = async (mode: DeliveryMode, selectedModel?: string) => {
    if (!native || subscriptionSelecting) return;
    if (selectedModel !== undefined && !subscription.models.some(item => item.slug === selectedModel)) return;
    const operation = ++connectionEpoch.current;
    const current = clearConnectionReview();
    setSubscriptionSelecting(true); setKey('');
    setApiChecked(false); setDeliveryStatusError('');
    // Disable dispatch immediately. Only the host's returned selection can
    // enable it; selecting a disconnected plan never reuses a registered key.
    setSubscription(previous => ({ ...previous, deliveryMode: mode }));
    try {
      await bridge({ op: 'cancel' });
      if (operation !== connectionEpoch.current) return;
      const status = await subscriptionSelect(mode, selectedModel);
      if (operation !== connectionEpoch.current) return;
      setSubscription(status);
      setConnectionChecked(true);
      await refreshApiForSelection(status, operation, ++deliveryRefreshEpoch.current);
      if (operation !== connectionEpoch.current) return;
      if (epoch.current === current) setConnectionNotice('接続方式とモデルを変更しました。原文を再検査してください。');
    } catch {
      if (operation === connectionEpoch.current) {
        setSubscription(previous => ({ ...previous, state: 'error', model: null, models: [] }));
        setConnectionError('接続設定を変更できませんでした。外部送信を止めています。');
      }
    } finally {
      if (operation === connectionEpoch.current) setSubscriptionSelecting(false);
    }
  };

  const signInSubscription = async () => {
    if (!native || subscriptionSelecting || subscription.state === 'connecting') return;
    const operation = ++connectionEpoch.current;
    const current = clearConnectionReview();
    setSubscription(previous => ({ ...previous, state: 'connecting', reason: null, model: null, models: [], deliveryMode: 'chatgpt_plan' }));
    setConnectionChecked(true); setApiChecked(false); setDeliveryStatusError('');
    try {
      await bridge({ op: 'cancel' });
      if (operation !== connectionEpoch.current) return;
      const status = await subscriptionSignIn();
      if (operation !== connectionEpoch.current) return;
      setSubscription(status);
      setConnectionChecked(true);
      if (epoch.current === current && status.state === 'connected') setConnectionNotice('ChatGPTとの公式接続を確認しました。モデルを確認し、原文を再検査してください。');
    } catch {
      if (operation === connectionEpoch.current) {
        setSubscription(previous => ({ ...previous, state: 'error', model: null, models: [] }));
        setConnectionError('ChatGPTとの接続を完了できませんでした。ブラウザと利用条件を確認して、もう一度お試しください。');
      }
    }
  };

  const cancelSubscription = async () => {
    if (!native) return;
    const operation = ++connectionEpoch.current;
    const current = clearConnectionReview();
    setSubscriptionSelecting(true);
    setSubscription(previous => ({ ...previous, state: 'disconnected', reason: null, model: null, models: [] }));
    try {
      const status = await subscriptionCancel();
      if (operation !== connectionEpoch.current) return;
      setSubscription(status);
      setConnectionChecked(true);
      if (epoch.current === current) await bridge({ op: 'cancel' });
      if (epoch.current === current) setConnectionNotice('この画面の接続を止めました。ログイン情報はMacのキーチェーンに残ります。原文はそのまま残っています。');
    } catch {
      if (operation === connectionEpoch.current) setConnectionError('接続の解除状態を確認できませんでした。外部送信を止めています。');
    } finally {
      if (operation === connectionEpoch.current) setSubscriptionSelecting(false);
    }
  };

  const closeConnection = () => {
    setKey(''); setConnectionOpen(false);
    requestAnimationFrame(() => connectionButton.current?.focus());
  };

  const registerKey = async () => {
    if (!native || !apiKey.trim() || keyBusy || modelReloading) return;
    const current = ++epoch.current;
    const secret = apiKey.trim();
    setKey(''); setKeyBusy(true); setConnectionError(''); setConnectionNotice('');
    setConfirmed(false); setReview(null); setResult(null); setStage('draft');
    try {
      await setApiKey(keyProvider, secret);
      // The host invalidates its approval when changing the key. Do not let a
      // delayed completion erase a newer visible draft or cancel a newer review.
      if (epoch.current === current) await bridge({ op: 'cancel' });
      await refreshConnection();
      setConnectionNotice('APIキーをMacのキーチェーンに保存しました。入力欄からキーを消去しました。');
    } catch {
      setConnectionError('APIキーを保存できませんでした。キーチェーンと接続先の設定を確認してください。');
    } finally { setKeyBusy(false); }
  };

  const prepareModel = async () => {
    if (!native || inFlight.current || keyBusy || model?.state === 'busy' || model?.state === 'verifying') return;
    const current = ++epoch.current;
    ++modelRefreshEpoch.current;
    inFlight.current = true;
    setBusy(true); setModelReloading(true); setConfirmed(false);
    setReview(null); setResult(null); setStage('draft'); setError('');
    setConnectionError(''); setConnectionNotice('');
    try {
      await bridge({ op: 'cancel' });
      if (epoch.current !== current) return;
      await reloadModel();
      await refreshConnection();
      if (epoch.current === current) setConnectionNotice('モデルの再準備を開始しました。原文はそのまま残しています。モデルの状態を確認し、準備後に再検査してください。前の承認は引き継ぎません。');
    } catch {
      if (epoch.current === current) setConnectionError('モデルを再準備できませんでした。原文は残っています。処理が終わってから、状態を確認してもう一度お試しください。');
    } finally {
      inFlight.current = false; setBusy(false); setModelReloading(false);
    }
  };

  const invalidate = (patch: Record<string, unknown>) => {
    const current = ++epoch.current;
    setConfirmed(false);
    setReview(null);
    setResult(null);
    setError('');
    setStage('draft');
    setAnnouncement('内容を変更しました。連携する前に、もう一度確認します。');
    bridge({ op: 'update', patch }).then(snapshot => {
      if (epoch.current !== current) return;
      if (snapshot.phase === 'BLOCKED' || Object.entries(patch).some(([key, value]) => snapshot[key as keyof Snapshot] !== value)) {
        // A rejected update may resolve with the previous Core source. Preserve
        // the visible draft and explain that it still needs correction.
        setError(snapshot.blockedReason || '入力や設定の全文を受理できませんでした。原文を短くするなど修正してください。');
        setStage('blocked');
      }
    }).catch(reason => {
      if (epoch.current === current) {
        setError(errorText(reason));
        setStage('blocked');
      }
    });
  };

  const changeOriginal = (value: string) => {
    setOriginal(value);
    invalidate({ original: value });
  };

  const prepare = async () => {
    if (!original.trim() || inFlight.current || !ready || (native && (subscriptionSelecting || subscription.state === 'connecting'))) return;
    const current = ++epoch.current;
    inFlight.current = true;
    setBusy(true);
    setConfirmed(false);
    setResult(null);
    setError('');
    setStage('checking');
    setAnnouncement('このPC内で辞書と変換候補を確認しています。');
    if (native) refreshConnection();
    try {
      const snapshot = await client.prepare({ original, purpose, destination });
      if (current !== epoch.current) return;
      setReview(snapshot);
      if (native) refreshConnection();
      if (snapshot.phase === 'REVIEW') {
        setStage('review');
        setAnnouncement('変換文を用意しました。外へ残る情報とあわせて確認してください。');
        requestAnimationFrame(() => reviewHeading.current?.focus());
      } else {
        setStage('blocked');
        setError(snapshot.blockedReason || '文章の最後まで確認できませんでした。');
        setAnnouncement('処理を止めています。原文は入力欄に残っています。');
      }
    } catch (reason) {
      if (current === epoch.current) {
        setError(errorText(reason));
        setStage('blocked');
      }
    } finally {
      inFlight.current = false;
      setBusy(false);
    }
  };

  const runDemo = async () => {
    if (!review || !confirmed || inFlight.current) return;
    if (native && (!connectionConfigured || connectionFailure)) {
      setError(usingPlan ? 'ChatGPTとの接続と利用モデルを確認できません。接続設定を確認してください。' : '接続先のAPIキーが未設定、または設定の状態を確認できません。接続設定を確認してください。');
      return;
    }
    const current = ++epoch.current;
    inFlight.current = true;
    setBusy(true);
    setError('');
    setStage('sending');
    setAnnouncement(native ? `承認を確認し、変換文だけを${usingPlan ? 'ChatGPTの公式連携' : 'API'}へ渡して応答を待っています。` : '承認を確認し、接続デモの応答をこのPC内で生成しています。');
    try {
      const approvalId = await client.approve(review.revision);
      if (current !== epoch.current) return;
      const received: RoundTripResult = native
        ? await client.sendNative(approvalId).then(snapshot => ({ snapshot, outboundText: snapshot.transformed, restored: snapshot.restored, kind: 'native' as const }))
        : await client.runDemo(approvalId).then(result => ({ ...result, kind: 'demo' as const }));
      if (current !== epoch.current) return;
      setResult(received);
      setConfirmed(false);
      setStage('complete');
      setAnnouncement(native ? `${usingPlan ? 'ChatGPTの公式連携' : 'API'}から受け取った回答の登録トークンを、このPC内で復元しました。` : '接続デモの合成回答を、このPC内で復元しました。外部APIへは送信していません。');
      requestAnimationFrame(() => replyHeading.current?.focus());
    } catch (reason) {
      if (current === epoch.current) {
        setError(errorText(reason));
        setConfirmed(false);
        setStage('blocked');
        setAnnouncement('処理を止めました。原文は入力欄に残っています。');
      }
    } finally {
      inFlight.current = false;
      setBusy(false);
    }
  };

  const returnToEdit = async () => {
    const current = ++epoch.current;
    setConfirmed(false);
    setReview(null);
    setResult(null);
    setError('');
    setStage('draft');
    setAnnouncement('原文を修正できます。前の承認は使いません。');
    try {
      await bridge({ op: 'cancel' });
      if (epoch.current === current) requestAnimationFrame(() => composerRef.current?.focus());
    } catch (reason) {
      if (epoch.current === current) { setError(errorText(reason)); setStage('blocked'); }
    }
  };

  const restart = async () => {
    if (inFlight.current || !ready) return;
    const current = ++epoch.current;
    inFlight.current = true; setBusy(true); setConfirmed(false); setError('');
    try {
      let fresh = await startNewCase();
      if (supportRecord) fresh = await bridge({ op: 'update', patch: { purpose: 'support_record' } });
      if (epoch.current !== current) return;
      setOriginal(fresh.original); setPurpose(fresh.purpose);
      setDestination(fresh.destination === 'Claude' ? 'Claude' : 'ChatGPT');
      setReview(null); setResult(null); setStage('draft');
      setAnnouncement('新しい相談です。前の承認・回答・登録トークンは引き継ぎません。');
      requestAnimationFrame(() => composerRef.current?.focus());
    } catch (reason) {
      if (epoch.current === current) { setError(errorText(reason)); setStage('blocked'); }
    } finally { inFlight.current = false; setBusy(false); }
  };

  const fillSample = () => {
    changeOriginal(supportRecord ? supportFixtures[0].original : SAMPLE);
    setAnnouncement(supportRecord ? '架空の支援記録を入れました。実在の児童・施設の情報は含まれていません。' : '架空のお知らせ文を入れました。実在の情報は含まれていません。');
    requestAnimationFrame(() => composerRef.current?.focus());
  };

  const showReview = review && ['review', 'sending', 'complete'].includes(stage);
  const providerName = destination === 'Claude' ? 'Claude' : 'OpenAI / GPT';
  const apiConfigured = destination === 'Claude' ? api.configuredClaude : api.configuredOpenAI;
  const usingPlan = native && destination === 'ChatGPT' && subscription.deliveryMode === 'chatgpt_plan';
  const subscriptionConfigured = subscription.state === 'connected' && Boolean(subscription.model) && subscription.models.some(item => item.slug === subscription.model);
  const connectionFailure = connectionError || modelStatusError || deliveryStatusError;
  const connectionConfigured = connectionChecked && !subscriptionSelecting && (usingPlan ? subscriptionConfigured : apiChecked && apiConfigured);
  const selectedPlanModel = subscription.models.find(item => item.slug === subscription.model);
  const transportLabel = usingPlan ? 'ChatGPT契約 · 公式連携' : 'API';
  const subscriptionLabel = subscription.state === 'connecting' ? '接続を確認中' : subscriptionConfigured ? `接続済み · ${selectedPlanModel?.displayName}` : subscription.state === 'connected' ? '接続済み・モデル未確認・送信不可' : subscription.state === 'error' ? '接続を確認できません・送信不可' : '未接続・送信不可';

  const openConnection = () => {
    setKey(''); setKeyProvider(destination === 'Claude' ? 'Claude' : 'OpenAI');
    setConnectionNotice(''); setConnectionOpen(true); refreshConnection();
  };

  return (
    <div className={`embedded-room ${supportRecord ? 'support-room' : ''} ${quiet ? 'embed-quiet' : ''} ${stage === 'blocked' || error || connectionFailure || model?.state === 'error' || (usingPlan && subscription.state === 'error') ? 'embed-stopped' : ''}`}>
      <div className="embed-scenery" aria-hidden="true" />
      <a href="#chat-composer" className="embed-skip">入力欄へ移動</a>
      <header className="embed-header">
        <div className="embed-brand"><RoomMark /><div><strong>クッションAI</strong><span>{supportRecord ? '支援の記録に、小さな手紙室を。' : 'いつものチャットに、小さな手紙室を。'}</span></div></div>
        <details className="embed-view-settings">
          <summary>表示設定</summary>
          <div className="embed-view-panel">
            <label><input type="checkbox" checked={quiet} onChange={event => setQuiet(event.target.checked)} />静かな表示</label>
            <label><input type="checkbox" checked={companion} onChange={event => setCompanion(event.target.checked)} />ポッケを表示する</label>
            <p>音はOFF。常時動く演出はありません。</p><button className="embed-secondary" disabled={!ready || busy} onClick={restart}>この相談を閉じて、空の入力欄に戻る</button>
          </div>
        </details>
      </header>

      <main className="embed-main">
        <div className="embed-demo-boundary"><span><i aria-hidden="true" />{native ? !connectionChecked ? 'ローカル検査・接続を確認中' : usingPlan ? 'ローカル検査・ChatGPT公式連携' : 'ローカル検査・API連携' : '組み込みチャット・接続デモ'}</span><strong>{native ? '業務利用未承認' : '外部API未接続'}</strong><strong>実データ不可</strong>{native && <strong>架空サンプルの持ち出しのみ</strong>}</div>
        <section className="chat-room" aria-labelledby="chat-title">
          <header className="chat-room-header">
            <div><span className="chat-eyebrow">A CONVERSATION, GENTLY WRAPPED</span><h1 id="chat-title">{supportRecord ? '今日の場面を、ことばに。' : 'この部屋で、会話のつづきを。'}</h1></div>
            <label className="embed-provider">{native ? '接続先' : '接続先の想定'}<select aria-label={native ? '接続先' : '接続先の想定'} value={destination} disabled={!ready || subscriptionSelecting} onChange={event => {
              const value = event.target.value as Destination;
              setDestination(value);
              invalidate({ destination: value });
              if (native && value === 'Claude' && subscription.deliveryMode === 'chatgpt_plan') void selectSubscription('api_key');
            }}><option value="ChatGPT">OpenAI / GPT</option><option value="Claude">Claude</option></select></label>
          </header>

          {native && <section className="native-connection" aria-label="モデルと接続の状態">
            <div><span className="local-chip"><LocalMark />{model?.state === 'ready' ? 'ローカルモデル準備済み' : model?.state === 'busy' ? 'ローカルモデル処理中' : model?.state === 'verifying' ? 'ローカルモデルを準備中' : !model && !modelStatusError ? 'ローカルモデルの状態を確認中' : 'ローカルモデル未準備'}</span><span>{!connectionChecked ? '接続方式を確認中・送信不可' : usingPlan ? `ChatGPTの契約：${subscriptionLabel}` : `${providerName}：${!apiChecked ? 'APIキーの登録状態を確認中・送信不可' : apiConfigured ? 'APIキー登録済み' : 'APIキー未設定・送信不可'}`}</span></div>
            <button ref={connectionButton} className="embed-secondary" onClick={openConnection}>接続設定</button>
            {(connectionFailure || (model?.state !== 'ready' && model?.state !== 'busy' && model?.reason)) && <p role="status">{connectionFailure || model?.reason}</p>}
          </section>}

          <div className="chat-transcript">
            {!showReview && stage !== 'blocked' && (
              <div className="chat-introduction">
                <div className="intro-letter" aria-hidden="true"><img src="/art/envelope-closed.svg" alt="" /><RoomMark small /></div>
                <h2>{supportRecord ? '見たことを、そのまま大切に。' : 'いつもの会話に、ひと呼吸。'}</h2>
                <p>{supportRecord ? <>活動・観察・関わりの記録を、読みやすい文章へ。<br />事実を足さず、否定や出来事の順序を残します。</> : <>ここで文章を確かめて、承認する。<br />戻った回答は、同じ画面でいつもの名前に。</>}</p>
                <div className="inline-journey" aria-label={native ? '確認と外部連携の流れ' : '接続デモの流れ'}><span>このPCで変換</span><b aria-hidden="true">→</b><span>あなたが承認</span><b aria-hidden="true">→</b><span>このPCで復元</span></div>
                <p className="intro-demo-note">{native ? <>実推論の試作です。内蔵の架空サンプルを検査・承認した変換文だけを{!connectionChecked ? '確認済みの公式接続先' : usingPlan ? 'ChatGPTの公式連携' : 'API'}へ送れます。<br/>業務利用の受入基準は未達です。実在の情報は入れないでください。</> : <>この試作の応答は合成データです。<br className="embed-narrow-break" /> ローカルLLM・外部AIの実推論ではありません。</>}</p>
              </div>
            )}

            {showReview && (
              <>
                <article className="local-user-message" aria-label="あなたの原文">
                  <div className="message-byline"><span>あなたの文章</span><span className="local-chip"><LocalMark />原文・このPC内</span></div>
                  <p className="message-text">{review.original}</p>
                </article>

                <section className={`inline-review ${stage === 'complete' ? 'review-completed' : ''}`} aria-labelledby="inline-review-title">
                  <div className="review-avatar" aria-hidden="true"><RoomMark small /></div>
                  <div className="inline-review-content">
                    <div className="message-byline"><span>クッションAI</span><span className="local-chip">端末内の確認</span></div>
                    <h2 id="inline-review-title" ref={reviewHeading} tabIndex={-1}>{stage === 'complete' ? native ? `確認した変換文だけを、${usingPlan ? 'ChatGPTの公式連携' : 'API'}へ渡しました。` : '確認した文だけで、連携を試しました。' : '外へ渡ることばを、確かめてください。'}</h2>
                    <p className="inline-review-summary">登録された機密候補を、相談専用のトークンに置き換えました。</p>
                    {usingPlan && <p className="inline-review-summary">送信先：ChatGPTの契約・公式連携<br />モデル：{selectedPlanModel?.displayName || '未選択・送信不可'}</p>}

                    <details className="outbound-details" open={stage !== 'complete'}>
                      <summary><span>外へ渡る変換文の全文</span><small>{usingPlan ? 'ChatGPTの契約' : providerName} 向け・{stage === 'complete' ? native ? '応答あり' : 'デモ連携済み' : '未連携'}</small></summary>
                      <pre className="outbound-text" aria-label="外へ渡る変換文">{result?.outboundText || review.transformed}</pre>
                      <div className="inline-changes" aria-label="置き換えた箇所">{review.findings.map((finding, index) => <span key={`${finding.start}-${index}`}><b>{finding.action}</b>{finding.original}</span>)}</div>
                    </details>

                    <div className="inline-meaning">
                      <div><h3><span aria-hidden="true">↗</span> 外へ残る情報</h3><ul>{review.residual.map((item, index) => <li key={index}>{item}</li>)}</ul></div>
                      <div><h3><span aria-hidden="true">↘</span> 失われる意味</h3><ul>{review.lost.map((item, index) => <li key={index}>{item}</li>)}</ul></div>
                    </div>

                    {stage !== 'complete' && <div className="inline-approval">
                      <label className="inline-confirmation"><input type="checkbox" checked={confirmed} disabled={stage === 'sending'} onChange={event => setConfirmed(event.target.checked)} /><span>変換文と、外へ残る情報・失われる意味を確認しました。</span></label>
                      <div className="inline-approval-actions"><button className="embed-secondary" onClick={returnToEdit}>原文の修正に戻る</button><button className="embed-primary" disabled={!confirmed || busy || Boolean(error) || (native && (!connectionConfigured || Boolean(connectionFailure)))} onClick={runDemo}>{stage === 'sending' ? native ? usingPlan ? 'ChatGPTの応答を待っています' : 'APIの応答を待っています' : 'このPC内で連携を試しています' : native ? !connectionChecked ? '接続方式を確認しています' : usingPlan ? '確認して変換文をChatGPTに送る' : '確認して変換文をAPIに送る' : '確認して連携を試す'}<span aria-hidden="true">→</span></button></div>
                      <p>{native ? '内蔵の架空サンプルを検査・承認した変換文だけを送れます。原文・対応表・復元済み回答は外部へ渡しません。' : '接続デモのため、実際のAPI送信とクリップボード操作は行いません。'}</p>{native && !connectionConfigured && <p className="api-missing">{!connectionChecked ? '接続方式の状態を確認中のため、送信できません。原文と変換文はここに残ります。' : usingPlan ? 'ChatGPTとの接続と利用モデルを確認できないため、送信できません。上の「接続設定」で確認してください。' : !apiChecked ? 'APIキーの登録状態を確認中のため、送信できません。ローカルの確認は続けられます。' : 'この接続先のAPIキーが未設定のため、送信できません。上の「接続設定」から登録してください。'}</p>}
                    </div>}
                  </div>
                </section>
              </>
            )}

            {stage === 'complete' && result && <article className="restored-reply" aria-labelledby="reply-title">
              <div className="reply-avatar" aria-hidden="true"><span>返</span></div>
              <div className="reply-content"><div className="message-byline"><span>{usingPlan ? transportLabel : `${providerName}${native ? ' · API' : ' 接続デモ'}`}</span><span className="local-chip"><LocalMark />このPC内で復元</span></div><h2 id="reply-title" ref={replyHeading} tabIndex={-1}>回答が、戻りました。</h2><p className="synthetic-answer-label">{native ? `${usingPlan ? 'ChatGPT公式連携の回答' : 'API回答'}・登録トークンをこのPC内で復元` : '合成回答・外部AIは呼び出していません'}</p>{supportRecord && <p className="support-result-note">記録の下書きです。事実・主語・否定・時系列に変化がないか確認してください。業務記録への保存や転記は行っていません。</p>}<pre className="restored-reply-text" aria-label="このPC内で復元した回答">{result.restored}</pre>
                {result.snapshot.unknownTokens.length > 0 && <div className="embed-error"><strong>復元できないトークンを、そのまま残しています。</strong><p>未知・改変済み・別相談のトークンは推測して戻しません。</p></div>}
                <p className="restored-boundary">具体的な名前が戻っています。この回答を外部AIへ再送しないでください。</p>
                <details className="demo-trace"><summary>{native ? `この往復で${usingPlan ? 'ChatGPTの公式連携' : 'API'}へ渡した変換文を確認` : 'この往復で渡した文と受け取った文を確認'}</summary><p>{native ? '以下は、承認時の変換文です。原文や辞書を送信対象へ追加していません。' : 'APIの送信対象となるのは変換文だけです。このデモではネットワーク送信していません。'}</p><h3>渡した変換文・全文</h3><pre>{result.outboundText}</pre>{result.kind === 'demo' && <><h3>復元前の合成回答・全文</h3><pre>{result.providerAnswer}</pre></>}<p>{native ? '復元は、今回外へ渡した登録トークンだけを完全一致で行いました。回答の内容の正しさはご自身で確認してください。' : '外部APIリクエスト: なし。復元は、今回渡した登録トークンだけを完全一致で行いました。'}</p></details>
                <button className="embed-secondary reply-edit" onClick={returnToEdit}>原文の修正に戻る</button>
              </div>
            </article>}

            {stage === 'blocked' && <section className="embed-error" role="alert"><span className="error-eyebrow">この文章は、ここに残しています。</span><h2 ref={blockedHeading} tabIndex={-1}>連携を止めています。</h2><p>{error || 'この構成では変換文を確認できませんでした。'}</p><p>原文は下の入力欄に残っています。内容を修正するか、架空サンプルで確かめてください。原文を外部へ送る代替処理はありません。</p><button className="embed-secondary" onClick={returnToEdit}>原文を修正する</button></section>}

            {(stage === 'checking' || stage === 'sending') && <div className="embed-processing" role="status"><span className="processing-mark" aria-hidden="true" /><div><strong>{stage === 'checking' ? 'このPC内で辞書と変換候補を確認しています。' : native ? `承認済みの変換文だけを${usingPlan ? 'ChatGPTの公式連携' : 'API'}へ渡して、応答を待っています。` : '承認済みの変換文で接続デモを試しています。'}</strong><span>{native && stage === 'sending' ? '送信後のデータは取り消せません。中止した場合、戻った回答は画面に採用しません。原文は残ります。' : '原文はここに残ります。編集やキャンセルができます。'}</span></div><button className="embed-secondary" onClick={returnToEdit}>{native && stage === 'sending' ? '回答の受け取りを中止' : 'キャンセル'}</button></div>}
          </div>

          {supportRecord && <aside className="support-scope" aria-label="支援記録で扱う情報"><strong>この試作で扱う記録</strong><p>活動・観察した行動・職員の関わり。氏名、施設名、日付、数値は隠します。診断・服薬・詳しい家庭事情を含む記録は停止します。</p><p>まずは下の架空サンプルで。書き換えた文章はローカルで確認できますが、登録済みの架空文と一致しない場合は持ち出せません。</p></aside>}
          <section className={`chat-composer ${showReview ? 'composer-with-review' : ''}`} aria-label="相談の入力">
            <div className="composer-topline"><label htmlFor="chat-composer"><LocalMark />原文・このPC内</label><span>保存しません</span></div>
            <textarea ref={composerRef} id="chat-composer" value={original} disabled={!ready} onChange={event => changeOriginal(event.target.value)} placeholder={supportRecord ? '活動：どんな場面でしたか。\n観察：実際に見た行動は。\n関わり：どんな声かけをしましたか。' : '書きかけの文章を、ここへ。'} spellCheck={false} autoComplete="off" aria-describedby="composer-boundary" />
            <div className="composer-options"><button className="embed-sample" onClick={fillSample} disabled={!ready}><span aria-hidden="true">＋</span>{supportRecord ? '架空の支援記録を入れる' : '架空のお知らせ文を入れる'}</button>{supportRecord ? <span className="support-purpose">目的：支援記録の文章を整える</span> : <label>目的<select aria-label="相談の目的" value={purpose} disabled={!ready} onChange={event => { const value = event.target.value as Purpose; setPurpose(value); invalidate({ purpose: value }); }}><option value="polish">文章を整える</option><option value="summary">要点を整理する</option><option value="perspectives">一般的な観点を出す</option><option value="support_record">支援記録の文章を整える</option></select></label>}</div>
            <div className="composer-bottom"><p id="composer-boundary">実データ不可。まずは架空サンプルで。<br />入力や接続先を変えると、承認は無効になります。</p><button className="embed-primary" onClick={prepare} disabled={!original.trim() || busy || !ready || stage === 'review' || (native && (subscriptionSelecting || subscription.state === 'connecting'))}>{stage === 'checking' ? 'このPC内で確認しています' : '機密箇所を確認・変換'}<span aria-hidden="true">→</span></button></div>
          </section>
        </section>
        <div className="embed-status" role="status" aria-live="polite">{announcement}</div>
        <footer className="embed-footer"><span>画面を移動せず、確認から復元まで。</span><details><summary>{native ? 'この接続の仕組みと境界' : 'このデモの仕組みと境界'}</summary><p>{native ? 'ローカルモデルとRust Coreで候補抽出・辞書照合・仮名化・承認を行います。承認した変換文だけを、ネイティブ側から固定の公式接続先へ送信します。APIキーとChatGPTの接続情報はMacキーチェーンで管理し、この画面には読み戻しません。会話履歴や復元済み回答を送信文へ追加しません。' : '同じRust Coreで辞書照合・仮名化・承認・復元を行い、TypeScript SDKが1往復をつなぎます。応答は固定の合成データです。ブラウザでは外部APIへ接続しません。実推論・外部接続・認証情報の管理はデスクトップ版の機能です。'}</p><p>原文、辞書、対応表、復元済み回答を外部へ渡しません。承認がない・変更された・処理が失敗した場合は停止します。本文と回答は通常の文字として表示し、HTMLや外部画像を実行・取得しません。</p><p>承認済み変換文だけを受け取る境界を維持します。APIキー方式の契約・料金と、ChatGPTの契約による公式連携は別の接続方式です。</p></details></footer>
      </main>
      {companion && <div className="embed-companion" aria-hidden="true"><img src="/art/companion-idle.png" alt="" /></div>}
      {native && <dialog ref={connectionDialog} className="api-settings-dialog" aria-labelledby="api-settings-title" onCancel={event => { event.preventDefault(); closeConnection(); }} onClose={() => { setKey(''); setConnectionOpen(false); }}>
        <div className="api-dialog-heading"><h2 id="api-settings-title">このPCの接続設定</h2><button className="api-close" aria-label="接続設定を閉じる" onClick={closeConnection}>×</button></div>
        {destination === 'ChatGPT' && <label className="api-field">OpenAIの接続方式<select aria-label="OpenAIの接続方式" value={connectionChecked ? subscription.deliveryMode : ''} disabled={!connectionChecked || subscriptionSelecting || keyBusy} onChange={event => void selectSubscription(event.target.value as DeliveryMode)}><option value="" disabled>接続方式を確認中</option><option value="api_key">APIキー</option><option value="chatgpt_plan">ChatGPTの契約</option></select></label>}
        {!connectionChecked ? <p role="status">接続方式を確認しています。登録済みのAPIキーを読み出す前に、この画面の接続先を確認します。</p> : usingPlan ? <section aria-label="ChatGPTの公式接続" style={{ fontSize: '18px', lineHeight: 1.7 }}>
          <p>いつものブラウザでChatGPTへログインし、公式の同意画面でこのアプリとの接続を選びます。パスワードや接続情報を、この入力画面へ貼り付ける必要はありません。</p>
          <p>利用できる契約・モデル・残りの利用枠はChatGPT側の条件に従います。このアプリとアカウントが利用条件に合うかは、接続時に確認します。</p>
          <p role="status">ChatGPTの契約：{subscriptionLabel}{subscription.reason && <><br />{subscription.reason}</>}</p>
          <button onClick={signInSubscription} disabled={subscriptionSelecting || subscription.state === 'connecting'} style={{ minHeight: '44px', padding: '10px 20px', border: '1px solid #000', borderRadius: '4px', background: '#000', color: '#fff', fontSize: '18px', fontWeight: 600, cursor: 'pointer' }}>Continue with ChatGPT</button>
          {(subscription.state === 'connecting' || subscription.state === 'connected') && <p><button className="embed-secondary" onClick={cancelSubscription} disabled={subscriptionSelecting}>{subscription.state === 'connecting' ? '接続を中止する' : 'この画面の接続を止める'}</button></p>}
          {subscription.state === 'connected' && <label className="api-field" style={{ fontSize: '18px' }}>利用するChatGPTモデル<select aria-label="利用するChatGPTモデル" value={selectedPlanModel?.slug || ''} disabled={subscriptionSelecting || subscription.models.length === 0} style={{ fontSize: '18px' }} onChange={event => void selectSubscription('chatgpt_plan', event.target.value)}>
            <option value="" disabled>モデルを選んでください</option>{subscription.models.map(item => <option key={item.slug} value={item.slug}>{item.displayName}</option>)}
          </select></label>}
          <p>現在は内蔵の架空サンプルだけを持ち出せます。接続の成功は、業務利用の認証ではありません。</p>
          <p>接続・モデル変更・接続中止では、前の確認と承認を使いません。原文はこのPC内に残して、もう一度検査します。</p>
        </section> : <>
        <p>APIキーはMacのキーチェーンに保存します。登録済みのキーを画面へ読み戻しません。</p>
        <p className="api-contract-note">APIの利用契約・料金は、ChatGPTやClaudeのサブスク画面とは別です。現在は内蔵の架空サンプルだけを持ち出せます。キーの登録やローカル検査の成功は、業務利用の認証ではありません。</p>
        <label className="api-field">キーの接続先<select value={keyProvider} disabled={keyBusy} onChange={event => { setKey(''); setKeyProvider(event.target.value as ApiProvider); setConnectionNotice(''); }}><option value="OpenAI">OpenAI / GPT</option><option value="Claude">Claude</option></select></label>
        <label className="api-field">APIキー<input type="password" value={apiKey} disabled={keyBusy} onChange={event => setKey(event.target.value)} autoComplete="off" autoCorrect="off" spellCheck={false} placeholder="登録するキーを入力" /></label>
        <p className="api-key-state">{!apiChecked ? 'APIキーの登録状態を確認しています。' : keyProvider === 'OpenAI' ? api.configuredOpenAI ? 'OpenAIのキーは登録済みです。入力すると置き換えます。' : 'OpenAIのキーは未設定です。' : api.configuredClaude ? 'Claudeのキーは登録済みです。入力すると置き換えます。' : 'Claudeのキーは未設定です。'}</p>
        <button className="embed-primary" disabled={!apiKey.trim() || keyBusy || modelReloading} onClick={registerKey}>{keyBusy ? 'キーチェーンに保存しています' : 'Macキーチェーンへ保存'}</button>
        <p className="api-key-state">キーを登録すると、前の確認・承認は無効になります。文章を再検査してください。</p>
        </>}
        {connectionNotice && <p className="api-success" role="status">{connectionNotice}</p>}
        {connectionFailure && <p className="api-dialog-error" role="alert">{connectionFailure}</p>}
        <div className="api-model-summary"><strong>ローカルモデル</strong><p>{model?.modelName || '状態を確認できません'}<br/>{model?.reason || (model?.state === 'ready' ? 'このPC内で実行する準備ができています。' : '状態を確認しています。')}</p><button className="embed-secondary" onClick={refreshConnection}>接続とモデルの状態を確認する</button><button className="embed-secondary" disabled={busy || keyBusy || model?.state === 'busy' || model?.state === 'verifying'} onClick={prepareModel}>{modelReloading ? 'モデルを再準備しています' : 'モデルを再準備する'}</button><p>原文を残して再準備します。前の確認・承認は無効になるため、準備後に再検査してください。</p></div>
      </dialog>}
    </div>
  );
}
