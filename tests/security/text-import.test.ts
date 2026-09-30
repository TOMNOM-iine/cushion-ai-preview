import { describe, expect, it } from 'vitest';
import { decodeTextFile, MAX_TEXT_FILE_BYTES } from '../../apps/desktop/src/textImport';

const encode = (text: string) => new TextEncoder().encode(text);
describe('local UTF-8 TXT import boundary', () => {
  it('preserves Unicode spelling, punctuation, newlines, and spacing', () => {
    const original = '  架空の髙森さん\r\nＡ社の📮は、まだ送らない。\n';
    expect(decodeTextFile('synthetic.TXT', encode(original))).toBe(original);
  });
  it('accepts a UTF-8 BOM without making it part of the source', () => {
    expect(decodeTextFile('synthetic.txt', new Uint8Array([0xef, 0xbb, 0xbf, ...encode('架空の手紙')]))).toBe('架空の手紙');
  });
  it('rejects invalid UTF-8 instead of inserting replacement characters', () => {
    expect(() => decodeTextFile('synthetic.txt', new Uint8Array([0xff, 0xfe, 0x30]))).toThrow('UTF-8として');
  });
  it('rejects oversized bytes before decoding', () => {
    expect(() => decodeTextFile('synthetic.txt', new Uint8Array(MAX_TEXT_FILE_BYTES + 1))).toThrow('64 KiB');
  });
  it('accepts exactly the byte cap and does not silently truncate', () => {
    expect(decodeTextFile('synthetic.txt', encode('a'.repeat(MAX_TEXT_FILE_BYTES))).length).toBe(MAX_TEXT_FILE_BYTES);
  });
  it('rejects disguised files and null bytes', () => {
    expect(() => decodeTextFile('synthetic.pdf', encode('text'))).toThrow('TXT');
    expect(() => decodeTextFile('synthetic.txt.pdf', encode('text'))).toThrow('TXT');
    expect(() => decodeTextFile('synthetic.txt', encode('text\0binary'))).toThrow('テキスト以外');
  });
  it('rejects empty files without including their name or text in errors', () => {
    expect(() => decodeTextFile('SYNTHETIC_CANARY.txt', encode(' \n'))).toThrow('原文は変更していません');
    try { decodeTextFile('SYNTHETIC_CANARY.pdf', encode('SYNTHETIC_CONTENT')); } catch (error) {
      expect(String(error)).not.toContain('SYNTHETIC_CANARY');
      expect(String(error)).not.toContain('SYNTHETIC_CONTENT');
    }
  });
});
