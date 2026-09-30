# クッションAI 組み込みSDK

自分のチャット画面で、ローカル検査 → 比較 → 人の承認 → 変換文の送信 → ローカル復元をつなぐTypeScript部品です。SDK自身は通信・保存・ログ出力を行いません。承認の最終判定はRust Coreが行います。

## 動く例

公開キットのルートで `npm run example`。実Rust WASMを使い、架空文章の検査、承認、固定のデモ回答、復元と停止条件を実行します。LLMや外部AIへの通信はありません。

- [組み込みコード](../../examples/embedded-chat.ts)
- [実行する試験](../../tests/examples/embedded-chat.test.ts)
- [開発手順の入口](../../developer/README.md)

```ts
const client = createCushionClient({ dispatch: trustedLocalCore });
const review = await client.prepare({ original, purpose: 'polish', destination: 'ChatGPT' });
// reviewをローカル画面に表示し、人が原文と変換文を比較する。
// 次の2行は「確認して進む」操作のハンドラーでだけ呼ぶ。
const approvalId = await client.approve(review.revision);
const result = await client.runDemo(approvalId); // 固定の架空デモ回答
```

## 本物の送信につなぐとき

`connectNativeChat(invoke)` はTauriのローカルIPCを接続する例です。対応するRustホストの実装が別途必要です。Coreへの入力はPC内のIPCに限り、Webサーバーや外部APIを `dispatch` に差し込まないでください。

```ts
import { invoke } from '@tauri-apps/api/core';
import { connectNativeChat } from '../../examples/embedded-chat';

// 利用者が新しい相談を始めるときに一度だけ初期化する。
// nativeホストが未実装・モデル未準備なら、後の検査は停止する。
await invoke('dispatch', { request: { op: 'new_case' } });
await invoke('dispatch', { request: { op: 'update', patch: { mode: 'local' } } });
const client = connectNativeChat(invoke);
const review = await client.prepare({ original, purpose: 'polish', destination: 'ChatGPT' });
// REVIEWなら原文と変換文を表示。BLOCKEDなら理由を表示して修正へ戻る。
// 同じclientを保ち、利用者の確認操作からだけ次の関数を呼ぶ。
async function onUserConfirmation() {
  const approvalId = await client.approve(review.revision);
  const localResult = await client.sendNative(approvalId);
  // localResult.restoredをローカル画面へ文字として表示する。
}
```

これは接続箇所を示す抜粋です。`connectNativeChat` 自体は相談を初期化しないため、上記のlocal設定と同じclientでのprepareが必要です。公開ブラウザ版へ貼るだけではnative送信は動きません。`isNative: true` は権限証明ではなく、ホスト側で有効な承認・検査結果・案件・版・モデル・送信先を再検査します。

`api_send` には承認IDだけを渡します。原文・`Snapshot`・辞書・復元文を送信本文に使いません。Rustの `export_approved_with` が返す変換文だけを固定の送信先へ渡し、同じ相談と版の回答だけを復元します。

`sendApproved(id, transport)` は意図的に無効です。`provider-wire.ts` はAPIキー方式の要求形式を試験する純粋関数であり、承認・送信機能でも、サブスク用の送信契約でもありません。サブスクのstream受信はnative参考コードを使って検討してください。

会話履歴、RAG、添付、自由なsystem指示、toolsは現在の入力契約にありません。復元した回答を次の外部会話へそのまま追加せず、続きも新しい検査・確認・承認を通します。
