export const MAX_TEXT_FILE_BYTES = 64 * 1024;

/** Local-only TXT decoding. No content or filenames appear in error messages. */
export function decodeTextFile(name: string, bytes: Uint8Array): string {
  if (!/\.txt$/i.test(name)) throw new Error('UTF-8のTXTファイルだけを読み込めます。PDF・Office・画像には対応していません。');
  if (bytes.byteLength > MAX_TEXT_FILE_BYTES) throw new Error('ファイルが64 KiBを超えています。短いTXTファイルに分けてください。');
  let decoded: string;
  try {
    decoded = new TextDecoder('utf-8', { fatal: true, ignoreBOM: false }).decode(bytes);
  } catch {
    throw new Error('UTF-8として読み込めませんでした。文字コードをUTF-8にして保存し直してください。');
  }
  if (decoded.includes('\0')) throw new Error('テキスト以外のデータが含まれています。通常のUTF-8 TXTファイルを選んでください。');
  if (!decoded.trim()) throw new Error('文章が入っていないファイルです。原文は変更していません。');
  return decoded;
}
