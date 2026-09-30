# ブラウザ体験版の第三者ライセンスと帰属表示

確認日: 2026-09-30。対象は架空データ専用のブラウザ・WASM体験版です。インストール済みの依存物と固定された `package-lock.json` / `Cargo.lock` に対応するライセンス原文を、この文書と同じ配布物の `licenses/` に収録しています。各ファイルは確認したローカル実体から内容を変更せずコピーしました。同じ本文でも、部品ごとの帰属を残すため別ファイルで保持しています。

この文書は第三者部品の通知です。この体験版の独自部分はMIT Licenseですが、下記の第三者部品にはそれぞれの条件が適用されます。[配布ライセンスの範囲](LICENSE_STATUS.md)も参照してください。

## 対象と選択

- JavaScript実行依存4件と、`cushion-core` の `wasm32-unknown-unknown` / `wasm` featureに解決される第三者34 package-versionを掲載します。
- Rustの一覧は通常依存だけでなくbuild依存・proc-macroを含む保守的な範囲です。全項目が最終WASMバイナリ内に残ることを示す一覧ではありません。
- 複数ライセンスから選べる部品は、下表ではMIT条件を選択しています。`unicode-ident` に付随するUnicode-3.0は追加条件として保持します。元の宣言も表に残しています。
- Cargoの依存一覧に現れないRust標準ライブラリの帰属・例外・依存通知も別途収録します。
- この体験版の対象に、ローカル推論モデルの重み、llama.cpp、Mac専用の保存・認証実装は含めません。これらを同梱する配布物は別の確認が必要です。

## JavaScript実行依存

| 部品 | 版 | upstream | 元の宣言 | この一覧で選択する条件 | 収録原文 |
| --- | --- | --- | --- | --- | --- |
| `react` | 19.3.0 | [upstream](https://github.com/react/react.git) | MIT | MIT | [LICENSE](licenses/npm-react-19.3.0-LICENSE.txt) |
| `react-dom` | 19.3.0 | [upstream](https://github.com/react/react.git) | MIT | MIT | [LICENSE](licenses/npm-react-dom-19.3.0-LICENSE.txt) |
| `scheduler` | 0.28.0 | [upstream](https://github.com/react/react.git) | MIT | MIT | [LICENSE](licenses/npm-scheduler-0.28.0-LICENSE.txt) |
| `@tauri-apps/api` | 2.12.0 | [upstream](https://github.com/tauri-apps/tauri.git) | Apache-2.0 OR MIT | MIT | [LICENSE-MIT](licenses/npm-tauri-apps-api-2.12.0-LICENSE-MIT.txt) |

## Rust / WASM関連依存

| 部品 | 版 | upstream | 元の宣言 | この一覧で選択する条件 | 収録原文 |
| --- | --- | --- | --- | --- | --- |
| `block-buffer` | 0.10.4 | [upstream](https://github.com/RustCrypto/utils) | MIT OR Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-block-buffer-0.10.4-LICENSE-MIT.txt) |
| `bumpalo` | 3.20.3 | [upstream](https://github.com/fitzgen/bumpalo) | MIT OR Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-bumpalo-3.20.3-LICENSE-MIT.txt) |
| `cfg-if` | 1.0.5 | [upstream](https://github.com/rust-lang/cfg-if) | MIT OR Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-cfg-if-1.0.5-LICENSE-MIT.txt) |
| `crypto-common` | 0.1.7 | [upstream](https://github.com/RustCrypto/traits) | MIT OR Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-crypto-common-0.1.7-LICENSE-MIT.txt) |
| `digest` | 0.10.7 | [upstream](https://github.com/RustCrypto/traits) | MIT OR Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-digest-0.10.7-LICENSE-MIT.txt) |
| `generic-array` | 0.14.7 | [upstream](https://github.com/fizyk20/generic-array.git) | MIT | MIT | [LICENSE](licenses/rust-generic-array-0.14.7-LICENSE-MIT.txt) |
| `getrandom` | 0.2.17 | [upstream](https://github.com/rust-random/getrandom) | MIT OR Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-getrandom-0.2.17-LICENSE-MIT.txt) |
| `itoa` | 1.0.18 | [upstream](https://github.com/dtolnay/itoa) | MIT OR Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-itoa-1.0.18-LICENSE-MIT.txt) |
| `js-sys` | 0.3.81 | [upstream](https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/js-sys) | MIT OR Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-js-sys-0.3.81-LICENSE-MIT.txt) |
| `log` | 0.4.34 | [upstream](https://github.com/rust-lang/log) | MIT OR Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-log-0.4.34-LICENSE-MIT.txt) |
| `memchr` | 2.8.3 | [upstream](https://github.com/BurntSushi/memchr) | Unlicense OR MIT | MIT | [LICENSE-MIT](licenses/rust-memchr-2.8.3-LICENSE-MIT.txt) |
| `once_cell` | 1.21.4 | [upstream](https://github.com/matklad/once_cell) | MIT OR Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-once_cell-1.21.4-LICENSE-MIT.txt) |
| `proc-macro2` | 1.0.107 | [upstream](https://github.com/dtolnay/proc-macro2) | MIT OR Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-proc-macro2-1.0.107-LICENSE-MIT.txt) |
| `quote` | 1.0.47 | [upstream](https://github.com/dtolnay/quote) | MIT OR Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-quote-1.0.47-LICENSE-MIT.txt) |
| `rustversion` | 1.0.23 | [upstream](https://github.com/dtolnay/rustversion) | MIT OR Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-rustversion-1.0.23-LICENSE-MIT.txt) |
| `serde` | 1.0.229 | [upstream](https://github.com/serde-rs/serde) | MIT OR Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-serde-1.0.229-LICENSE-MIT.txt) |
| `serde_core` | 1.0.229 | [upstream](https://github.com/serde-rs/serde) | MIT OR Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-serde_core-1.0.229-LICENSE-MIT.txt) |
| `serde_derive` | 1.0.229 | [upstream](https://github.com/serde-rs/serde) | MIT OR Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-serde_derive-1.0.229-LICENSE-MIT.txt) |
| `serde_json` | 1.0.151 | [upstream](https://github.com/serde-rs/json) | MIT OR Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-serde_json-1.0.151-LICENSE-MIT.txt) |
| `sha2` | 0.10.9 | [upstream](https://github.com/RustCrypto/hashes) | MIT OR Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-sha2-0.10.9-LICENSE-MIT.txt) |
| `syn` | 2.0.119 | [upstream](https://github.com/dtolnay/syn) | MIT OR Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-syn-2.0.119-LICENSE-MIT.txt) |
| `syn` | 3.0.6 | [upstream](https://github.com/dtolnay/syn) | MIT OR Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-syn-3.0.6-LICENSE-MIT.txt) |
| `tinyvec` | 1.13.3 | [upstream](https://github.com/Lokathor/tinyvec) | Zlib OR Apache-2.0 OR MIT | MIT | [LICENSE-MIT.md](licenses/rust-tinyvec-1.13.3-LICENSE-MIT.txt) |
| `typenum` | 1.20.1 | [upstream](https://github.com/paholg/typenum) | MIT OR Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-typenum-1.20.1-LICENSE-MIT.txt) |
| `unicode-ident` | 1.0.26 | [upstream](https://github.com/dtolnay/unicode-ident) | (MIT OR Apache-2.0) AND Unicode-3.0 | MIT AND Unicode-3.0 | [LICENSE-MIT](licenses/rust-unicode-ident-1.0.26-LICENSE-MIT.txt) / [LICENSE-UNICODE](licenses/rust-unicode-ident-1.0.26-LICENSE-UNICODE.txt) |
| `unicode-normalization` | 0.1.25 | [upstream](https://github.com/unicode-rs/unicode-normalization) | MIT OR Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-unicode-normalization-0.1.25-LICENSE-MIT.txt) / [COPYRIGHT](licenses/rust-unicode-normalization-0.1.25-COPYRIGHT.txt) |
| `unicode-segmentation` | 1.13.3 | [upstream](https://github.com/unicode-rs/unicode-segmentation) | MIT OR Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-unicode-segmentation-1.13.3-LICENSE-MIT.txt) / [COPYRIGHT](licenses/rust-unicode-segmentation-1.13.3-COPYRIGHT.txt) |
| `version_check` | 0.9.5 | [upstream](https://github.com/SergioBenitez/version_check) | MIT/Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-version_check-0.9.5-LICENSE-MIT.txt) |
| `wasm-bindgen` | 0.2.104 | [upstream](https://github.com/wasm-bindgen/wasm-bindgen) | MIT OR Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-wasm-bindgen-0.2.104-LICENSE-MIT.txt) |
| `wasm-bindgen-backend` | 0.2.104 | [upstream](https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/backend) | MIT OR Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-wasm-bindgen-backend-0.2.104-LICENSE-MIT.txt) |
| `wasm-bindgen-macro` | 0.2.104 | [upstream](https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/macro) | MIT OR Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-wasm-bindgen-macro-0.2.104-LICENSE-MIT.txt) |
| `wasm-bindgen-macro-support` | 0.2.104 | [upstream](https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/macro-support) | MIT OR Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-wasm-bindgen-macro-support-0.2.104-LICENSE-MIT.txt) |
| `wasm-bindgen-shared` | 0.2.104 | [upstream](https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/shared) | MIT OR Apache-2.0 | MIT | [LICENSE-MIT](licenses/rust-wasm-bindgen-shared-0.2.104-LICENSE-MIT.txt) |
| `zmij` | 1.0.23 | [upstream](https://github.com/dtolnay/zmij) | MIT | MIT | [LICENSE-MIT](licenses/rust-zmij-1.0.23-LICENSE-MIT.txt) |

## Rust標準ライブラリ

| 部品 | 確認したtoolchain | upstream | ライセンスと例外 | 収録原文 |
| --- | --- | --- | --- | --- |
| Rust Standard Library | `1.98.1` / `48a229cea` / 2026-09-01 | [rust-lang/rust](https://github.com/rust-lang/rust) | 原則Apache-2.0 OR MIT。Unicode-3.0などの個別例外と依存物の条件は収録原文を参照 | [COPYRIGHT-library.html](licenses/rust-standard-library-1.98.1-COPYRIGHT-library.html) |

標準ライブラリの通知はtoolchain同梱の全文を保持しているため、他target向けの項目も含みます。一般のRustライセンスだけで個別例外を置き換えていません。本文には第三者の著作権表示・許諾文・免責条項を収録しています。

## 検証範囲と維持

収録した42ファイルはコピー元とのバイト一致を確認しました。[SHA-256一覧](licenses/SHA256SUMS.txt)は配布物内の照合用です。上流リンクは確認したpackage metadataに記載された出所であり、将来も同じ内容を提供する保証ではありません。依存版またはRust toolchainを変えた場合は、一覧・原文・配布物を更新して再確認してください。

Rust advisory auditは未実施です。この通知の作成は脆弱性監査、最終成果物のリンク解析、第三者権利の包括的な審査、業務利用品質の認証ではありません。公開前には、実際に配る成果物と通知の対応、プロジェクト自身の配布条件を確認してください。
