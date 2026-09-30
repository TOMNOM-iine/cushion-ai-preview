import { describe, expect, it } from 'vitest';
import { formatProviderRequest, readProviderText } from '../../packages/cushion-sdk/src/provider-wire';

describe('provider wire contract (no network)', () => {
  it('formats only the separate prepared text for OpenAI Responses', () => {
    const request = formatProviderRequest('openai-responses', { destination:'ChatGPT', text:'[[CUSHION_TEST]] の案内' }, 'test-model');
    expect(request.body).toEqual({model:'test-model', input:'[[CUSHION_TEST]] の案内',store:false,stream:false});
    expect(request.endpoint).toBe('https://api.openai.com/v1/responses');
  });
  it('preserves existing Chat Completions format without history passthrough', () => {
    expect(formatProviderRequest('openai-chat-completions',{destination:'ChatGPT',text:'架空の変換文'},'test-model').body)
      .toEqual({model:'test-model',messages:[{role:'user',content:'架空の変換文'}],store:false,stream:false});
  });
  it('formats Claude Messages separately', () => {
    expect(formatProviderRequest('claude-messages',{destination:'Claude',text:'架空の変換文'},'test-model').body)
      .toEqual({model:'test-model',messages:[{role:'user',content:'架空の変換文'}],max_tokens:1024,stream:false});
  });
  it('rejects whole Core snapshots or extra hidden fields', () => {
    expect(() => formatProviderRequest('openai-responses', {destination:'ChatGPT',text:'変換文',original:'架空の原文'} as never,'test-model')).toThrow();
  });
  it('rejects destination mismatch and invalid models', () => {
    expect(() => formatProviderRequest('claude-messages',{destination:'ChatGPT',text:'変換文'},'test-model')).toThrow();
    expect(() => formatProviderRequest('openai-responses',{destination:'ChatGPT',text:'変換文'},'改行\n')).toThrow();
  });
  it('extracts completed Responses text including later output items', () => {
    expect(readProviderText('openai-responses',{status:'completed',output:[{type:'reasoning'},{type:'message',status:'completed',content:[{type:'output_text',text:'架空回答'}]}]})).toBe('架空回答');
  });
  it('extracts completed Claude and Chat Completions literal text', () => {
    const text = '<img src="https://example.invalid/canary">';
    expect(readProviderText('claude-messages',{stop_reason:'end_turn',content:[{type:'text',text}]})).toBe(text);
    expect(readProviderText('openai-chat-completions',{choices:[{finish_reason:'stop',message:{content:text}}]})).toBe(text);
  });
  it('rejects incomplete, refusal, and tool outputs', () => {
    expect(() => readProviderText('openai-responses',{status:'incomplete',output:[]})).toThrow();
    expect(() => readProviderText('claude-messages',{stop_reason:'tool_use',content:[]})).toThrow();
    expect(() => readProviderText('openai-chat-completions',{choices:[{finish_reason:'stop',message:{content:'',refusal:'拒否'}}]})).toThrow();
  });
});
