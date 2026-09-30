# ローカルLLMを追加する開発ガイド

この公開版は、固定の架空データを使うブラウザ／WASMデモです。本物のローカルLLMやネイティブホストを実行するビルド対象、モデル重み、モデル取得用installerは含みません。`reference/native/` に開発元のnativeソースを設計参考として同梱しますが、完全なnativeプロジェクトではありません。自分のホストへ実装を移し、依存関係・権限・配布設定を揃えて検証する必要があります。

この文書はMac試作の契約を基に、その実装順序と判断の所在を説明します。以下のnativeファイル名は移植先の論理パスで、参考ファイルへのリンクは `reference/native/` を指します。

**ローカルモデルが動くこと、機密候補を正しく検出すること、承認した情報だけを外へ出すことは別々に検証します。** 開発・動作確認には架空文書だけを使います。現在のデモのfixture制限を外すだけでは実用版になりません。

## 1. 処理を四つに分ける

```text
端末内のUI
  → Rust Core：原文・目的・辞書・版・jobを管理
  → native host → パイプ → 専用worker＋ローカルモデル
  ← 候補表記・分類・完了／不確定フラグのJSON
  → Rust Core：候補検証、全出現箇所の照合、置換、残存検査
  → UI：原文と変換文の全文比較、利用者による承認
  → Rust Core：最新の検査・承認・宛先・持ち出し条件を再確認
  → nativeの固定された出口：承認済み変換文だけ
```

| 部品 | 担当 | 担当させないこと |
| --- | --- | --- |
| ローカルモデル | 原文中の候補表記と分類、高機密・不確定・完了フラグ | 送信許可、トークン生成、位置計算、自由な書き換え、復元 |
| Rust Core | 範囲照合、決定的な置換、残存検査、承認、持ち出し、完全一致復元 | ネットワーク・OSコピー等の直接実行 |
| native host | 検証済みworkerの起動、限定IPC、取消し、Coreの判断後の副作用 | UIから受け取った `approved: true` の信用、任意URLへの送信 |
| UI | 入力、比較、限界と停止理由の表示、明示的な確認 | 検査完了やモデル準備済みの自己申告による権限付与 |

LLMへの「外へ送らない」という指示だけでは通信制限になりません。workerはネットワーク、shell、ブラウザ、任意ファイル検索の機能を持たず、OS側でも通信・書き込み・子プロセス生成を制限します。原文は専用パイプでだけ渡し、HTTPサービス、コマンド引数、環境変数、一時ファイル、ログに置きません。

## 2. 固定モデルと取得物を決める

開発元の既定は **Qwen3.5-4B Q8_0** です。9B Q8_0は品質基準の比較候補、4B／9BのQ4_K_Mは軽量比較候補です。4Bの既定採用は少数の架空文書による速度比較に基づき、未知の業務文書での品質合格を意味しません。

以下は2026-09-28に固定し、開発元のmanifest／catalogに記録した取得物です。現時点の最新版の推奨や、公開デモへの同梱を示す表ではありません。

| 項目 | 既定4B Q8_0 | 比較9B Q8_0 |
| --- | --- | --- |
| 原モデル | `Qwen/Qwen3.5-4B` | `Qwen/Qwen3.5-9B` |
| 原モデルrevision | `851bf6e806efd8d0a36b00ddf55e13ccb7b8cd0a` | `c202236235762e1c871ad0ccb60c8ee5ba337b9a` |
| GGUF作成元 | `bartowski/Qwen_Qwen3.5-4B-GGUF` | `bartowski/Qwen_Qwen3.5-9B-GGUF` |
| GGUF revision | `4168f45a16a1290d65a4ec0fa312ae917a4c15d6` | `182be2fd6c7bc44887d88a91cb03ff009cc9f549` |
| ファイル | `Qwen_Qwen3.5-4B-Q8_0.gguf` | `Qwen_Qwen3.5-9B-Q8_0.gguf` |
| バイト数 | `4622131168` | `9804541984` |
| SHA-256 | `5c74c0ede371924357dff0cb6ba145bd67208b9b2389ded681adfff3f7608db7` | `b58fe056b5435070240de259f3f981aa38fee96825bbd78c088d5fd90e46f2b5` |

原モデルと第三者による量子化を区別します。出所を調べる入口は [Qwenの4Bモデル](https://huggingface.co/Qwen/Qwen3.5-4B)、[Qwenの9Bモデル](https://huggingface.co/Qwen/Qwen3.5-9B)、[4Bの量子化配布元](https://huggingface.co/bartowski/Qwen_Qwen3.5-4B-GGUF)、[9Bの量子化配布元](https://huggingface.co/bartowski/Qwen_Qwen3.5-9B-GGUF) です。採用時は上表の固定revisionのモデルカード・ライセンス本文・取得ファイルを照合し、取得時点と利用条件を記録してください。開発元の保存記録ではQwenモデルはApache-2.0、llama.cppはMITです。アプリ独自部分のライセンスだけでモデル重みや第三者ライブラリの条件を置き換えないでください。

開発元は [llama.cpp](https://github.com/ggml-org/llama.cpp) の `b11146`、commit `7fe450e19305b828c199d602c23a8337aaa1f03b` を固定しています。ソースarchiveのSHA-256は `a6861d549427f814dc591c439e08206f67ffaba0248344d421589abf18199e67` です。モデル・runtime・chat templateを別々に管理し、モデル名だけで互換性を決めません。

取得処理を追加するときは次の契約にします。

1. モデル取得はアプリの相談処理と切り離し、利用者の明示操作で行う。取得先へ原文や端末内文書を渡さない。
2. 配布元、固定revision、ファイル名、サイズ、SHA-256、ライセンス、template版をアプリ側の許可リストに固定する。UIから任意URLや任意実行ファイルを受け付けない。
3. 一時取得物のサイズとhashを検査し、一致後だけ利用対象へ移す。不一致・取得中断は準備失敗にする。
4. worker実行ファイル、読み込む共有ライブラリ、Metal資産も列挙して検査する。開発元のMetal構成は21ファイルのversion-2 lockを使う。
5. hashの期待値自体を信頼できる配布経路で渡す。同じ場所の実行ファイルとlockを共に書き換えられる場合、hash一致だけでは配布元の真正性を証明できない。

この公開版に存在しないinstallerの実行は必要ありません。nativeを追加する際に、別途セットアップ処理を実装・検証します。開発元のMac試作はApple Silicon、選択済みXcode、Python 3.12以上、CMake、Metal Toolchainを用います。別OSへの移植は、プロセス隔離とGPU設定を含む別の検証です。

## 3. 専用workerを作る

開発元の `native/worker/main.cpp` は、1行JSONの標準入力・標準出力で動く常駐workerです。HTTPサーバーやRPCサーバーは使いません。標準出力は契約フレームだけ、エラーは固定コードだけにし、モデルやライブラリの内容ログも無効にします。相談ごとにモデルの会話キャッシュを消去します。

現在の設定は、総コンテキスト8,192トークン、出力予約1,024トークン、同時1件、greedy生成、処理期限240秒です。原文は最大4,000 Unicodeコードポイントで、文字数に収まっても、実際のtokenizerで指示・原文・出力予約の合計が上限を超えたら停止します。長文を黙って切り捨てません。分割を追加する場合は全chunkの完了と境界の対応を別に設計します。

原文を挟むchat templateは固定し、原文部分を `parse_special=false` でtokenizeします。本文中のChatML風文字列を新しいsystem命令として解釈させません。GBNF等の文法制約を使い、候補抽出だけを返させます。

架空入力のパイプ契約例です。原文は実行引数やログへ出さず、native hostがメモリから書き込みます。

```json
{"document":"架空の小森さんへ連絡してください。","document_id":"synthetic-job-001"}
```

モデルが返す内側のJSON例です。

```json
{"complete":true,"findings":[{"surface":"小森","category":"PERSON"}],"context_flags":[],"unresolved":[]}
```

workerはその結果に、要求に束縛したjob IDと実際の検査ビューの文字数を追加します。以下はCoreの候補検査へ渡す `result` 部分の例です。worker外側のフレームはさらに `ok`、`inputTokens`、`outputTokens`、`elapsedMs` を持ちます。

```json
{
  "schema_version": "surface-set-v1",
  "document_id": "synthetic-job-001",
  "complete": true,
  "covered_chars": 17,
  "findings": [{"surface": "小森", "category": "PERSON"}],
  "context_flags": [],
  "unresolved": []
}
```

`document_id` と `covered_chars` はモデルに計算させません。`complete`、`context_flags`、`unresolved` はモデルの返答をそのまま検査し、hostが都合よく成功へ変更しません。`covered_chars` の一致は、送った検査ビューの範囲が契約どおりであることの確認です。**モデルがすべての秘密を認識した証明にはなりません。**

## 4. JSON・Unicode・全入力の対応をCoreで検証する

公開版にもある `crates/inference/src/lib.rs` と `surface_set.rs` に、候補の検査ロジックがあります。native側へ接続してもこの判定を省きません。

- 不正JSON、余分なフィールド、重複キー、版違い、job ID違いは拒否する。検査前に汎用JSONオブジェクトへ変換して重複キーを消さない。
- `complete=false`、全範囲不一致、不確定候補、高機密フラグは停止する。
- 候補は128種類以内、各表記は256コードポイント以内。原文に存在しない候補、空表記、重複・重なりを拒否する。
- 候補の出現数と位置はRustが計算し、同じ表記の全出現箇所を保護する。モデルのoffsetや出現数を信用しない。
- 内部範囲はUnicodeコードポイントで扱い、JavaScriptのUTF-16位置やUTF-8 byte offsetと混同しない。

現行の検査ビューは `U+200B`、`U+200C`、`U+200D`、`U+2060`、`U+FEFF` だけを除き、元の位置との対応を保持します。workerが返した表記を検査ビューへ完全一致で照合してから、原文の正確な範囲へ戻します。原文は書き換えません。Coreの辞書照合は別に正規化を使いますが、モデル候補を曖昧に補正したり、似た人名へ推測一致させたりはしません。

Coreは検証済み候補、秘密辞書、数値・日付・連絡先等の決定的な規則を合わせ、置換後には固定の依頼文も含む全文を残存検査します。診断・服薬・詳細な家庭事情・認証情報などを、単なる名前の置換で外部利用可能と扱いません。

## 5. 非同期の結果を現在の相談へ束縛する

モデル準備完了は、native hostがモデルhash、runtime、OS隔離、readyフレームを検査して決めます。UIに準備済みフラグを入力させても、実検査権限は得られない構成にします。

`begin_local_scan` は、相談ID、原文のrevision、job ID、目的、辞書・ポリシー・モデル構成・送信構成に結び付いた状態をCoreに作ります。処理中の編集、取消し、新しい相談、モデル再準備で旧jobを失効させます。終了時の `finish_local_scan` は、現在の状態とjobを再照合し、古い回答を採用しません。

以下はホスト実装の順序を示すRust風の擬似コードです。`verified_worker_state` 等の補助関数は実装対象であり、この公開版で実行できるAPIではありません。エラー処理・型・OS処理を省略しています。

```rust
// 準備状態と指紋はnativeだけが作る。UIから受け取らない。
let verified = verified_worker_state()?;
let (job, generation) = {
    let mut core = state.core.lock()?;
    let job = core.begin_local_scan(&verified.fingerprint, verified.ready)?;
    (job, state.worker_generation())
};

// 推論中はCoreの編集・取消しを可能にする。
let response = worker.infer(&job.original, &job.document_id, &cancel);

let mut core = state.core.lock()?;
if cancel.is_cancelled() || generation != state.worker_generation() {
    return discard_late_result();
}
match response {
    Ok(result) => core.finish_local_scan(&job.job_id, &result.result.to_string()),
    Err(_) => core.fail_local_scan(&job.job_id, fixed_failure()),
}
```

この例のJSON再直列化は、信頼する固定workerが返すフレームに対する現行ホストの構造です。第三者のモデルサービスや任意JSONを接続する場合は、重複キー等を消す前の元byte列を厳密検査できる契約にしてください。擬似コードをそのまま任意HTTP応答の検査に転用しないでください。

起動時は重みだけでなく、worker・ライブラリ・template・推論設定・検査ポリシーをまとめた指紋を作ります。これらを変更したら既存の検査と承認を失効させます。Metal失敗時に、同じ承認のまま別モデル・CPU・クラウドへ自動切替しません。

## 6. トークン生成・承認・持ち出し・復元

置換トークンはCoreが暗号学的乱数で作ります。氏名のhashや利用者番号から生成せず、同一相談内では一貫し、別相談では別トークンにします。原文とトークンの対応表は端末内だけに保持し、モデルや外部AIへ一覧として送信しません。

利用者が変換文の全文、外へ残る情報、失われる意味を確認してから承認します。承認は相談ID、revision、内容・目的・辞書・ポリシー・モデル・送信構成の指紋と有効期限を含みます。UIのcheckboxだけを承認記録にしません。

実際のコピーや外部送信は、Coreの `export_approved_with` の再検査後にだけ行います。native hostは同じCoreロックを保持し、callbackへ承認済み変換文だけを渡します。callbackへ原文、辞書、対応表、復元文を添付しません。公開SDKの任意transport口を有効にするだけで代替しないでください。

開発元の試作は、ここでさらに「凍結した架空原文と完全一致する」「既知の禁止範囲を実際の検出結果がすべて覆う」「禁止表記が残らない」を検査します。fixtureの正解範囲は最後の停止判定にだけ使い、LLM候補へ補充して検出成績を良く見せません。

持ち出し処理の成功後にだけ、その相談の持ち出し済みトークン集合を登録します。回答の復元はその集合と完全一致するトークンだけに限定し、未知・改変・別相談のトークンは推測復元しません。復元文を次の外部履歴へ自動追加せず、続きの相談は新しい原文として再検査します。回答のHTML、URL、画像、コードは文字として扱います。

## 7. 追加するnativeモジュール

次は移植時に必要となる責務の一覧です。表中のnative実装、設定、installerは公開デモのビルド対象に含まれません。保存や外部AI接続はローカル検査とは別機能なので、最初の検査実装に不要な権限を加えません。

| 配置するモジュールの例 | 必要な実装 |
| --- | --- |
| [`native/worker/main.cpp`](../reference/native/native/worker/main.cpp) | 固定llama.cppによる候補抽出、限定stdio、上限、ready応答、親終了時の停止 |
| [`crates/inference/src/runtime.rs`](../reference/native/crates/inference/src/runtime.rs) | artifact検証、OS隔離、worker起動・終了、パイプサイズ制限、期限・取消し、設定指紋 |
| [`crates/inference/src/model_profiles.rs`](../reference/native/crates/inference/src/model_profiles.rs) | compile-timeのモデル許可リスト、既定と比較対象、任意パス／URL拒否 |
| `models/manifest.json`、[`models/comparison-catalog.json`](../reference/native/models/comparison-catalog.json) | 固定出所、revision、サイズ、hash、template・推論設定。manifest全体は同梱せず、必要値は本書に記載。実行時の認証情報は含めない |
| モデル・runtimeのライセンス資料 | 実際に採用する固定revisionの条件と権利表示。重みやバイナリの同梱可否を別に確認 |
| [`crates/core/src/local.rs`](../reference/native/crates/core/src/local.rs) | native専用の検査開始・終了、job照合、決定的置換、架空fixture制限、承認付き持ち出し |
| [`apps/desktop/src-tauri/src/main.rs`](../reference/native/apps/desktop/src-tauri/src/main.rs) 相当のホスト | 限定したTauriコマンド、Core／workerの所有、非同期処理、世代管理、Coreと副作用の同一transaction |
| Tauriのmanifest・権限・CSP | ローカル画面だけのIPC、任意URL・汎用shell権限なし、ローカル素材、native依存のOS分離 |
| セットアップ／ビルド処理 | 固定取得先とhash照合、明示取得、runtime lock生成、ライブラリ配置、失敗時の停止 |
| native境界テスト | 未準備・旧job・取消し・改変構成・fixture外入力で実際のcopy／送信callbackが呼ばれない検証 |

公開版の [`crates/core/src/lib.rs`](../crates/core/src/lib.rs)、[`crates/inference/src/lib.rs`](../crates/inference/src/lib.rs)、[`crates/inference/src/surface_set.rs`](../crates/inference/src/surface_set.rs)、架空fixture、UI／SDKは契約を読む入口です。nativeファイルを追加するだけでは足りず、Cargoの対象・依存関係、WASMとの条件分岐、ホストIPC、配布方法を揃える必要があります。

参考ホストの `main.rs` は認証や保存庫等の別モジュールにも依存するため、単体で実行できません。最初はruntimeの準備、Coreの検査開始・終了、取消し、承認付きコピーに必要な部分を自分のホストへ移し、それぞれの境界を試験してください。`reference/native/` にあるAPI／サブスク処理を、ローカル検査を動かすためだけに有効にする必要はありません。

開発元のruntimeは開発checkoutを参照する仕組みです。その絶対位置を埋め込んだアプリを一般配布する設計は引き継がず、アプリの管理するローカル資産置き場、固定モデルID、信頼できるmanifest、署名済み更新の対応を設計してください。任意ユーザーパスを許可して動くようにする変更では代替しません。

## 8. 実装と受入の順序

1. **デモを固定する。** 既存のWASM試験を通し、変更前の架空入力・置換・承認失効・復元結果を記録する。
2. **workerの契約を先に作る。** 架空JSONで正常応答、壊れた応答、巨大応答、切断、親終了を確認する。合成応答の成功をLLM品質の成功に数えない。
3. **OS隔離と固定runtimeを検証する。** 同じ実行条件のworkerで通信禁止・書込禁止・起動失敗を確認する。macOS試作の通信probeはloopback接続の拒否確認であり、全packet監査の代わりではない。
4. **実モデルを1種類だけ接続する。** まず固定4B Q8_0で、入力token数・出力予約・JSON完了・範囲対応・取消しを確認する。標準エラーへ内容を出して調査しない。
5. **Coreへつなぐ。** 旧job、編集、モデル再準備、辞書変更、別相談、準備不能で検査・承認・持ち出しが止まることを確認する。
6. **nativeの出口を追加する。** まず副作用を観測できるテストsink、次に架空fixtureだけのOSコピーで検証する。外部AI接続は別契約として、承認済み文字列だけを受け取る固定transportを用意する。
7. **同じ評価集合で比較する。** 9Bや量子化違いへ変える際は、新しい指紋で再検査し、漏れ・過剰停止・意味保持・速度・メモリを別々に測る。
8. **業務利用の判定を別に行う。** 開発用fixtureと独立した評価、文書単位の機密残存、人手による意味確認、対象端末での通信・操作・配布検証を揃える。開発元仕様の独立1,000件・二名評価や運用受入は未完了であり、現在の試作制限を解除する根拠にはできない。

最低限の停止試験は、未登録モデル・hash不一致、worker死亡、壊れたJSON、未処理範囲、token上限、期限切れ、取消し、古いjob、Unicodeのずれ、同名人物の同定不能、候補の捏造・欠落、承認後の本文／宛先／構成変更、未知・別相談トークンです。通常経路の成功だけでなく、「停止時に出口が一度も呼ばれない」ことを確認します。

このガイドを実装しても未知の秘密の見落としをゼロと保証することはできません。実worker試験、Core境界試験、native画面操作、通信監査、独立品質評価、利用者受入を分けて記録し、未確認の部分を準備済みや業務利用可能と表示しないでください。
