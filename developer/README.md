# 自分のアプリへ組み込む

このキットには、動くブラウザ体験版に加えて、ローカルLLMとChatGPT契約を接続するための設計・SDK・Mac版の参考ソースを収録しています。独自部分はMITです。自分の製品へ取り入れて改良できます。

## ダウンロードしてできること

| やりたいこと | このキットでの進め方 |
| --- | --- |
| 置き換え・比較・承認・復元を試したい | ルートREADMEの手順でブラウザ体験版を起動 |
| 自分のチャット画面へ組み込みたい | [SDK](../packages/cushion-sdk/README.md)と[組み込み例](../examples/embedded-chat.ts)を利用 |
| ローカルLLMを入れて実検査したい | [モデルとworkerの作り方](LOCAL_LLM.md)に沿ってnativeホストを実装 |
| ChatGPTの契約で承認後の往復を自動化したい | [公式接続の実装手順](CHATGPT_SUBSCRIPTION.md)と認証・受信の参考ソースを利用 |
| 自分の製品として配布したい | [実装と受入の順序](IMPLEMENTATION_ROADMAP.md)に沿って別PC・品質・配布を検証 |

**ダウンロード直後に動くのは、固定架空データの体験版と通信しない組み込み例です。** モデルの重み、署名済みMacアプリ、利用者の認証情報、nativeビルド一式は含みません。モデルを取得するだけでブラウザ版が本物の検査へ変わるわけではありません。

## 最初の10分

ルートで `npm ci --ignore-scripts` と `npm run verify:wasm` を実行した後、次の順で確認できます。

1. `npm run dev` でサンプルを比較・承認・復元する。
2. 別のターミナルで `npm run example` を実行する。実Rust WASMを使う6つの例が動く。外部通信はしない。
3. [全体の流れ](ARCHITECTURE.md)で、PC内と外部の境界を読む。
4. ローカルLLM → native承認付き送信 → 公式ログインの順に、自分のホストへ実装する。

認証だけ先に通しても、原文検査や安全な送信が完成したことにはなりません。公開キットの検査・承認・復元の境界を維持して接続します。

## 参考ソースの読み順

`reference/native/` はMac版の実装を読むためのソースです。ブラウザのビルド対象ではなく、そのフォルダだけでnativeアプリをビルドできません。

| 順番 | ソースと注目する処理 |
| --- | --- |
| 1 | [Core local.rs](../reference/native/crates/core/src/local.rs): `begin_local_scan`、`finish_local_scan`、`export_approved_with` |
| 2 | [runtime.rs](../reference/native/crates/inference/src/runtime.rs)と[worker](../reference/native/native/worker/main.cpp): 子プロセス・モデル確認・JSON検証 |
| 3 | [main.rs](../reference/native/apps/desktop/src-tauri/src/main.rs): ローカルIPCとworker完了をCoreへ結ぶ |
| 4 | [subscription_host.rs](../reference/native/apps/desktop/src-tauri/src/subscription_host.rs): 接続設定を承認へ結び、変換文だけを送る |
| 5 | [subscription_auth.rs](../reference/native/apps/desktop/src-tauri/src/subscription_auth.rs): 本人の認証・Keychain・モデル一覧 |
| 6 | [subscription_transport.rs](../reference/native/apps/desktop/src-tauri/src/subscription_transport.rs)と[subscription_stream.rs](../reference/native/apps/desktop/src-tauri/src/subscription_stream.rs): 完了した回答だけを受理 |

原本からの差分は、runtimeのテスト内にある架空の絶対パス1か所を汎用のテストパスへ置き換えた点だけです。認証情報やモデルの重みはありません。[Cargo.toml](../reference/native/apps/desktop/src-tauri/Cargo.toml)は依存関係の参考で、公開キットのビルド入口ではありません。参照されるVault、native設定、ビルドスクリプト、runtime lockなどは別途実装が必要です。

ブラウザ依存の権利表示は同梱していますが、native参考ソースから将来ビルドする依存物やモデルの配布条件は別途確認してください。参考コードの収録は、任意文書の業務品質・別PCでの接続・外部サービスの利用資格を保証しません。
