# 本格実装へ進める順序

モデル取得やログイン成功だけでは、業務文書を送れる状態にはなりません。次の順で、通信しない試験から進めます。試験の文書・氏名・画像は架空データだけを使います。

| 段階 | 実装すること | 進む条件 |
| --- | --- | --- |
| 1. 体験版 | 同梱WASMとSDKで入力・比較・承認・復元を動かす | `npm test`、`npm run example`、`npm run build` が成功 |
| 2. nativeホスト | Tauri IPCとRust Coreを接続。保存はまだ無効でもよい | UIを迂回した未承認送信もCoreが拒否する |
| 3. 実モデル | 検証したモデルとllama.cppのstdio workerを接続 | hash・JSON・全入力処理・job一致・隔離・異常停止を確認 |
| 4. 通信前試験 | 送信部を端末内の捕捉先に置き換える | 変換文だけが出口へ出る。本文・設定変更後の古い承認が通らない |
| 5. 公式接続 | 本人のログイン・モデル一覧・承認付き要求・完了受信 | 新しい架空相談で送信→実回答→復元を確認。無承認の試験送信はしない |
| 6. 別端末での配布 | モデルパス、installer、署名・公証、認証取消し、更新を実装 | 開発Macのフォルダがなくても導入できる |
| 7. 業務導入判定 | 独立した入力で検出漏れ・意味・停止・操作性を評価 | 定めた品質・運用条件を満たす。サンプル成功率で代用しない |

## まず追加・変更するファイル

| 自分のnative実装に置く部品 | 参考 |
| --- | --- |
| `crates/core/src/local.rs` | [Core参考](../reference/native/crates/core/src/local.rs)。既存の `lib.rs` と組み合わせる |
| `crates/inference/src/runtime.rs`、`model_profiles.rs` | [workerホスト](../reference/native/crates/inference/src/runtime.rs)。開発用の固定root参照を、利用者ごとの管理ディレクトリへ変更 |
| `native/worker/main.cpp`、モデルmanifest、ビルド/導入スクリプト | [ローカルLLM](LOCAL_LLM.md)。バイナリ・ライブラリ・モデルの信頼できるhash lockも作る |
| `apps/desktop/src-tauri/src/main.rs` とTauri設定 | [ホスト参考](../reference/native/apps/desktop/src-tauri/src/main.rs)。Core・worker・IPCを接続。Vaultを省くなら該当commandを公開しない |
| `subscription_auth.rs`、`subscription_host.rs`、`subscription_transport.rs`、`subscription_stream.rs` | [ChatGPT接続](CHATGPT_SUBSCRIPTION.md)。認証・送信・完了確認・復元を一体で接続 |
| 自分のローカルチャット画面 | [TypeScript例](../examples/embedded-chat.ts)。比較と確認を省略せずSDKへ接続 |

公開キットはこのnativeビルド一式を生成しません。参考ソースを読むだけでも構成は追えますが、移植先で依存関係とTauri設定を整え、コンパイル・試験する作業が必要です。未収録のVault等を空の成功実装で代用しないでください。

## 参考コードに残る制限

- nativeの持ち出し口も、現在は登録済み架空fixtureの完全一致と既知機密範囲の被覆を要求します。独立1,000件評価・二名評価・運用受入は未完了です。この条件だけを外して業務版にしません。
- `development_root()` はビルド時のソース位置に依存します。モデル・runtimeの配置と検証を作り直す必要があります。
- ChatGPTの接続中止と、Keychainの削除・提供者側の権限取消しは別です。実際のsign-out、複数アカウント、token更新、権限取消しを別途検証します。
- APIキー方式の鍵を別のアプリコピーから変更した場合まで、承認との結び付けが完全ではありません。鍵の世代・選択アカウントをnativeの承認指紋へ含める改修が必要です。
- Claudeサブスクの接続経路は含みません。`provider.rs` は別契約のAPIキー方式の参考です。
- 削除・一般化・VoiceOver・IME・一般ユーザーの理解・別OS・署名済みインストーラーは別の受入対象です。

## 必ず確かめる境界

未承認、期限切れ、改変した承認ID、別相談、本文やモデル変更、worker停止、JSON異常、途中の回答で送信・復元が止まることを確認します。認証の取り消しと処理の遅い終了が競合しても、古い回答を現在の相談へ入れません。

HTML・Markdownの画像・リンクプレビューは勝手に実行・取得しません。エラーの本文やHTTP応答全体をログへ書かず、固定の原因分類を使います。検査中・停止中も原文を保持し、修正に戻れる状態にします。

ブラウザの合成host試験、実Rust WASM試験、native実機操作、workerのOS通信制限、実際の外部往復は別の証拠として記録します。MITは改変・配布の条件であり、品質認証ではありません。
