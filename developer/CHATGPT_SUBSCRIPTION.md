# ChatGPTプラン接続をローカルアプリに実装する

確認日: 2026-09-30。OpenAIの公式Sign in with ChatGPT（SIWC）資料と、クッションAIのMac試作を基にした開発者向けガイドです。

**公開キットでそのまま実行できるのは、架空データ専用のブラウザ・WASMデモです。** ChatGPTへのログイン・実送信・Mac Keychainを使う処理はブラウザ版では動きません。`reference/native/` に収録したMac試作のソースは実装を検討するための参考であり、そのディレクトリだけで完成したnativeアプリをビルドできる構成ではありません。

公開キットの独自コードにはMITを適用します。これによってOpenAI側の利用適格性、第三者依存物の条件、キット外のMacアプリ全体の配布条件まで決まるわけではありません。利用者の認証情報、ローカルモデルの重み、端末設定は含めません。

## 1. 接続できる条件を先に確認する

SIWCでは、本人確認と「本人のChatGPTプランで推論する許可」は別です。本人確認だけ成功しても推論してはいけません。対象の利用者・workspace・プランで使用が認められ、実際に付与されたscopeに `chatgpt.tokens.use.direct` があることを確認します。ChatGPTの過去の会話やアカウント内の文脈を読める権限ではありません。[公式概要](https://developers.openai.com/siwc/token-sharing-open-source)

公式資料は、オープンソースのツールやローカルで動く個人プロジェクト向けの接続を案内しています。有料または遠隔ホスト型アプリとして提供する場合は、公式資料の申請経路で利用条件を確認してください。単にソースをMITで公開したことを、任意の商用Webサービスで使える許可と解釈しません。[公式導入例・提供条件](https://developers.openai.com/cookbook/articles/sign-in-with-chatgpt)

この経路はOAuthの公開クライアントで、利用者がOpenAI APIキーやclient secretを入力する方式ではありません。利用量は利用者のChatGPTプラン・許可されたcredits・設定上限に従います。「無制限」「追加費用が絶対にない」と表示しないでください。OpenAI／ClaudeのAPIキー方式は別の接続・課金経路です。未接続や上限到達時に、本人の選択なしでAPIキー方式へ切り替えてはいけません。[セッション・利用状況](https://developers.openai.com/siwc/token-sharing-open-source/profiles-and-sessions)

**Claudeサブスクとの自動接続は、このキットにもMac試作にも実装していません。** Claude APIキー方式と、Claudeのサブスク画面を自動操作する方式は異なります。Cookieの取り出し、非公開API、ログイン自動化、DOMへの自動入力、回答スクレイピングを代替経路にしません。

## 2. 処理は端末内のnative hostに置く

この製品では、原文・秘密辞書・対応表・復元文を端末内に残します。OAuth tokenの保管、公式HTTP通信、Coreの承認確認もnative hostが担当します。WebViewやブラウザJavaScriptへtokenを返しません。公開Webサーバーへ原文を預けてから変換する構成にはしません。

```text
利用者のローカル画面
  → ローカル検査 → 原文と変換文の比較 → 明示承認
  → approvalIdだけをnative hostへ
  → Rust Coreが承認と現在の版を検証
  → 承認済み変換文だけを公式Responsesへ
  → 完了済み回答をCoreで検証・復元 → 同じローカル画面
```

認証の一時callbackだけは `127.0.0.1` で待ち受けます。受け取るのは認証コードと認証処理の識別情報であり、原文や相談本文を受け取るHTTPサービスではありません。ローカル推論workerへの原文は限定した端末内IPCで渡し、クラウドの検査サービスへ代替送信しません。

## 3. 利用者自身がログインし、接続を許可する

1. 利用者が「Continue with ChatGPT」を押してから認証を始めます。通常のシステムブラウザで、利用者自身がログイン・workspace選択・権限確認を行います。アプリはパスワードを収集しません。
2. インストール単位で安定した不透明な `ext_agent_host_id` を作り、再起動や再ログインでも再利用します。Mac試作は `urn:uuid:` 形式を使います。メールアドレスや利用者IDをhost IDにせず、認証情報とも扱いません。公式資料では公開鍵から導出するJWK thumbprint URIを推奨し、UUID形式も認めています。
3. 試行ごとに新しい乱数の `state`、OIDC `nonce`、PKCE verifierを生成します。PKCE challengeは `BASE64URL(SHA256(verifier))`、方式は `S256`。verifierなどはnative側で当該試行の間だけ保持します。
4. ブラウザを開く前に、`http://127.0.0.1:<一時port>/auth/callback` を待ち受けます。認証開始とコード交換では同一のURIを使います。再ログイン時にportが変わっても、scheme・host・pathは維持します。`localhost` へ置き換えません。
5. 初回は `client_id=dynamic_agent_client` とアプリの実名の `agent_name_hint` を使います。これは初回登録の入口です。返された発行済みclient IDを登録に紐付け、再認証とtoken交換にはそのIDを使います。再認証で異なるclient IDが返った場合は停止します。
6. callbackのHost・path・method・サイズ・期限・重複queryを検証し、保存した `state` と一致する場合だけ処理します。拒否や取消しではコードを交換しません。callback受信後は待受を閉じます。
7. コードをPKCE verifier、発行済みclient ID、同じredirect URI・resourceとともに公式token endpointへ送ります。ID tokenの署名をJWKSで検証し、issuer、audience、有効期限、nonceを確認します。再認証では検証済みsubjectと既存の登録も一致させます。
8. token応答の付与scopeを確認してから接続を有効にします。callbackの表示や、未検証JWTのdecode結果だけを信用しません。遅延して戻った取消し済みログイン結果で、現在の接続を上書きしません。

公式認証先は `https://auth.openai.com/api/accounts/authorize`、コード交換先は `https://auth.openai.com/api/accounts/oauth/token` です。要求scopeは `openid profile email offline_access resource.invoke chatgpt.tokens.use.direct`、resourceは `https://api.openai.com/v1`。コード・state・tokenをチャット、診断出力、スクリーンショット、ログへ残さないでください。[公式登録・認証手順](https://developers.openai.com/siwc/token-sharing-open-source/sign-in)

## 4. Keychainと更新処理を一体で管理する

Mac試作は、access token・refresh token・ID token・発行済みclient ID・検証済みsubject・付与scope・期限を、端末間で同期しない専用のMac Keychain項目に保持します。APIキー保存とは分離します。Keychainが使えない場合は停止し、平文JSONやブラウザstorageへ保存する代替を設けません。

access tokenの期限が近づいたら、同じ登録のrefresh tokenと発行済みclient IDで更新します。回転するrefresh tokenの競合を避けるため、同じセッションの更新を直列化し、新しいcredential一式をまとめて置き換えます。古いaccess tokenと新しいrefresh tokenなどを混ぜません。Mac試作はプロセス内mutexと、本文・認証情報を入れない空のファイルロックを組み合わせています。

認証更新が失敗・不明なら推論送信を止めます。一時的な通信障害を理由にただちに登録を消したり、別の課金経路へ移ったりしません。正式に失効したtokenの処理、再認証、複数登録を分ける方法は、公式手順に合わせて実装・試験します。[公式アカウント・セッション管理](https://developers.openai.com/siwc/token-sharing-open-source/profiles-and-sessions)、[更新失敗時の扱い](https://developers.openai.com/siwc/token-sharing-open-source/errors-and-recovery)

公式手順には再認証時の `id_token_hint` もあります。Mac試作は認証情報をURLへ追加するhintを使用していません。追加する設計では、公式認証endpointに限ることと認証URLの診断出力抑止が必要です。access tokenやrefresh tokenをURLへ入れてはいけません。

## 5. 契約ごとのモデル一覧とCore承認を結ぶ

接続後、native hostから同じOAuth access tokenで `GET https://api.openai.com/v1/models` を要求します。応答の `models` 配列から `visibility: "list"` を選び、表示は `display_name`、推論要求には `slug` を使います。サーバーの並び順を保ち、アカウント変更時には再取得します。公開ページに書かれたモデル名や固定のモデル一覧で、本人の利用可能モデルを推定しません。[公式モデル取得](https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference)

Mac試作は、接続方式、登録の非秘密の指紋、選択モデル、受信処理の契約版をCoreの承認へ紐付けます。本文、辞書、目的、検査モデル、送信先、接続・選択モデルが変わると検査・承認を失効させます。モデルを変えた後に、以前の確認チェックを再利用しません。

UIから実送信を呼ぶ引数は次の範囲です。これはnative版の呼出し形の説明で、公開ブラウザデモから使える送信APIではありません。

```ts
type ApprovedSend = { approvalId: string };
// approvalIdはRust Coreが発行したものを使う。
await invoke<Snapshot>("api_send", { approvalId });
```

UIが `approved: true`、原文、任意の変換文、URL、OAuth tokenを送信引数として渡す設計にしません。native hostが現在の接続・利用可能モデル・token・検査workerの動作を確認し、Coreの `export_approved_with` が承認ID、相談、版、内容、期限を検証して初めて変換文を取り出します。その検証と外向きの副作用を同じCoreの制御下に置きます。

## 6. 公開Responsesに変換文だけを送り、完了を待つ

推論先は **`POST https://api.openai.com/v1/responses`**。native hostがOAuth access tokenをBearer認証として付けます。ChatGPTの `backend-api` やWeb画面のCookieを使用しません。本文の形は次の最小例です。山括弧内は説明用であり、実際の値や送信可能なfixtureではありません。

```json
{
  "model": "<catalogで選択したslug>",
  "input": [{ "role": "user", "content": "<Core承認済み変換文>" }],
  "store": false,
  "stream": true
}
```

`store:false` と `stream:true` は、この公式プラン経路で各HTTP要求に必要です。Mac試作は原文・秘密辞書・対応表・復元文・任意履歴・添付・tools・metadata・前回応答IDを本文へ追加しません。HTTPの `previous_response_id` や、一般のAPI例からコピーした未対応パラメータを加えないでください。`store:false` は、この製品の「端末外に何も出ない」保証ではありません。変換文は利用者の承認後にOpenAIへ送られます。[公式推論形式](https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference)、[プレビュー制限](https://developers.openai.com/siwc/token-sharing-open-source/preview-limitations)

成功条件は、**同一応答の `response.completed` を受信し、完成した本文の検証に通ること**です。HTTP 200、最初の文字、`text.done`、接続済み表示だけでは成功としません。公式資料でも、受信開始後に利用上限などの失敗が届く可能性が説明されています。[公式完了条件](https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference)

Mac試作では、SSEの本文をnativeメモリへ収集し、response ID、item ID、output/contentのindex、deltaと各doneイベントの本文一致、itemの完了を検証します。最終snapshotに本文がある場合は一致を確認し、本文がない場合も、完成を確認できた台帳の本文だけを採用します。途中本文やreasoningをUIへ返しません。これらはクッションAI側で追加した保守的な条件です。

失敗、未完了、拒否、未知event、tool出力、順序や内容の矛盾、切断、上限超過では停止します。現試作の受信上限は1MiB、完成回答は24,000文字です。生レスポンスやサーバーの任意エラー文をログ・UIへ出さず、固定の停止理由を表示します。送達が不明な場合も自動再送しません。取消しは、既に送った要求を未送達に戻す保証ではありません。

## 7. 復元は当該相談のローカルCoreだけで行う

native hostは持ち出しと同じCore transactionで、成功した回答を `RestoreIfCurrent` に渡します。相談IDと本文revisionが一致し、実際に持ち出したトークンだけを完全一致で復元します。未知・改変・別相談のトークンは推測で補修しません。復元済み文をクラウドの会話履歴に戻さず、回答中のHTML・コード・URL・外部画像を実行・取得しません。

追加質問は新しい原文として再検査・比較・承認します。認証済みだから次の文章も承認済み、という扱いにはしません。

## 8. 参考ソースの読み順

以下は `reference/native/` 内の参照用コピーです。元のMac試作の相対階層を保持しています。公開デモの起動・ビルド対象には含まれません。

| 参考ソース | 担当する処理 |
| --- | --- |
| [subscription_auth.rs](../reference/native/apps/desktop/src-tauri/src/subscription_auth.rs) | PKCE、callback、OIDC検証、Keychain、token更新、モデル取得 |
| [subscription_host.rs](../reference/native/apps/desktop/src-tauri/src/subscription_host.rs) | 接続の世代、Coreへの接続指紋、承認IDからの送信、同一相談への復元 |
| [subscription_transport.rs](../reference/native/apps/desktop/src-tauri/src/subscription_transport.rs) | 固定公式endpoint、要求形式、HTTP/SSE検査、停止理由 |
| [subscription_stream.rs](../reference/native/apps/desktop/src-tauri/src/subscription_stream.rs) | item・本文・完了イベントを照合する受信台帳 |
| [main.rs](../reference/native/apps/desktop/src-tauri/src/main.rs) | Tauri command、共有状態、Coreとworkerへの結線 |
| [Core local.rs](../reference/native/crates/core/src/local.rs) | native専用の承認束縛、失効、実推論結果、持ち出し条件 |
| [inference runtime.rs](../reference/native/crates/inference/src/runtime.rs)・[model_profiles.rs](../reference/native/crates/inference/src/model_profiles.rs)・[worker main.cpp](../reference/native/native/worker/main.cpp) | ローカル検査workerと許可したモデル構成の参照 |
| [native Cargo.toml](../reference/native/apps/desktop/src-tauri/Cargo.toml)・[provider.rs](../reference/native/apps/desktop/src-tauri/src/provider.rs) | 依存と既存APIキー分岐の参照。APIキー分岐はプラン接続の代替動作ではない |

`subscription_host.rs` は `main.rs` の共有状態、Core、ローカルworker、既存API分岐に依存します。参考ファイルのコピーだけでは、Tauri設定、他のmodule、Vault、検証済みモデル・worker、署名・配布構成などは揃いません。機能の境界を理解してローカルアプリへ統合するための資料です。

公開キットの[第三者ライセンス一覧](../THIRD_PARTY_NOTICES.md)はブラウザ配布物を対象にしています。参考ソースが要求するnative追加依存のコード・ライブラリを、このキットはコンパイル・同梱していません。参考ソースからnativeアプリを作成・配布する場合は、その解決済み依存、モデル・runtime、対象OSのライセンスと脆弱性を別途監査し、必要な原文・帰属表示を同梱してください。

## 9. 実装済みの境界と、本格提供までの残作業

Mac試作には、上記の公式認証、Keychain、モデル取得、Core承認付き要求、完了streamの検証、ローカル復元が実装されています。ただし、公開ブラウザデモでそれらの実通信を確認したという意味ではありません。Coreのnative持ち出し制限は凍結した架空fixtureに限定され、業務利用品質は未認証です。公開キットの例を動かすために、この制限を外して実在情報を送らないでください。

| 項目 | 現試作の状態と、本格提供時に必要なこと |
| --- | --- |
| 複数アカウント・workspace | 保存する登録は1件。公式手順の複数profile管理と利用者に分かる選択表示を追加し、client ID・subject・tokenを登録間で混ぜない試験が必要 |
| 切断とログアウト | 画面の接続停止は実装。Keychain内tokenの削除・OpenAI側のrevocationは未実装。公式のrevocation endpointで確認し、未確認なら利用者へ伝える正式なsign-outを別途実装 |
| 利用状況 | 上限・接続失敗では停止。ChatGPT SettingsのUsageへ利用者が移動できる導線など、公式UI指針に沿う提供画面を整える |
| 更新と障害 | token更新の実装と合成試験は、実環境での更新成功・権限取消し・複数プロセス競合・取消し競合の受入とは分ける |
| 配布 | 参考sourceだけでは単独nativeビルド不可。別端末への導入、通信監査、署名・公証、native依存通知、モデル品質・操作受入が必要 |

正式なsign-outは、更新可能なセッションの失効とローカルtokenの破棄を扱う機能です。現在の「この画面の接続を止める」を「OpenAIの権限も取り消した」と表示しないでください。[公式logout・revocation](https://developers.openai.com/siwc/token-sharing-open-source/profiles-and-sessions#end-the-renewable-session)

検証は架空データだけで、認証成功、モデル一覧取得、推論完了、正しい相談への復元を分けて記録します。少なくとも、同意拒否、不正state、nonce不一致、署名不正、scope不足、期限切れ、モデル変更後の旧承認、更新競合、上限到達、途中切断、完了なし、別応答ID、未知トークンを確認します。接続済み表示や1件の成功だけで、全アカウント・モデル・端末や実業務への適合を保証しません。

## 公式資料

- [SIWC概要・hostとclientの区別](https://developers.openai.com/siwc/token-sharing-open-source)
- [登録、PKCE、callback、ID token検証](https://developers.openai.com/siwc/token-sharing-open-source/sign-in)
- [アカウント、token更新、revocation](https://developers.openai.com/siwc/token-sharing-open-source/profiles-and-sessions)
- [モデル一覧、Responses、完了条件](https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference)
- [要求形式のプレビュー制限](https://developers.openai.com/siwc/token-sharing-open-source/preview-limitations)
- [エラーと復旧](https://developers.openai.com/siwc/token-sharing-open-source/errors-and-recovery)
- [公式導入例と提供条件](https://developers.openai.com/cookbook/articles/sign-in-with-chatgpt)

これらは確認日時点の資料です。実装・公開時には最新の公式条件とschemaを確認し直してください。
