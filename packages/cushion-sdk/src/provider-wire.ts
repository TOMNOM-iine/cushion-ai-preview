/** Pure wire formatting only; this module never authorizes or sends a request. */
export type ProviderFormat = 'openai-responses' | 'openai-chat-completions' | 'claude-messages';
export type PreparedText = Readonly<{ destination: 'ChatGPT' | 'Claude'; text: string }>;

export function formatProviderRequest(
  format: ProviderFormat,
  prepared: PreparedText,
  model: string,
) {
  if (!prepared || Object.keys(prepared).sort().join(',') !== 'destination,text' ||
      typeof prepared.text !== 'string' || !prepared.text.trim() ||
      new TextEncoder().encode(prepared.text).byteLength > 128 * 1024 ||
      !/^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/.test(model)) {
    throw new Error('変換文とモデルの形式を確認できません。送信していません。');
  }
  if (format === 'claude-messages' && prepared.destination === 'Claude') {
    return {
      method: 'POST' as const, endpoint: 'https://api.anthropic.com/v1/messages',
      body: { model, max_tokens: 1024, stream: false,
        messages: [{ role: 'user' as const, content: prepared.text }] },
    };
  }
  if (prepared.destination !== 'ChatGPT') throw new Error('承認時の送信先と一致しません。');
  if (format === 'openai-responses') {
    return {
      method: 'POST' as const, endpoint: 'https://api.openai.com/v1/responses',
      body: { model, input: prepared.text, store: false, stream: false },
    };
  }
  if (format === 'openai-chat-completions') {
    return {
      method: 'POST' as const, endpoint: 'https://api.openai.com/v1/chat/completions',
      body: { model, messages: [{ role: 'user' as const, content: prepared.text }],
        store: false, stream: false },
    };
  }
  throw new Error('対応していないAPI形式です。');
}

/** Plain text extraction; never executes a tool, Markdown, HTML, or a URL. */
export function readProviderText(format: ProviderFormat, value: unknown): string {
  const bad = () => new Error('回答の完了と形式を確認できません。復元していません。');
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw bad();
  const data = value as Record<string, unknown>;
  let text: unknown;
  if (format === 'openai-chat-completions') {
    if (!Array.isArray(data.choices) || data.choices.length !== 1) throw bad();
    const choice = data.choices[0];
    if (choice?.finish_reason !== 'stop' || choice?.message?.refusal || choice?.message?.tool_calls) throw bad();
    text = choice?.message?.content;
  } else {
    const blocks: unknown[] = [];
    if (format === 'openai-responses') {
      if (data.status !== 'completed' || !Array.isArray(data.output)) throw bad();
      for (const item of data.output) {
        if (item?.type === 'reasoning') continue;
        if (item?.type !== 'message' || item?.status !== 'completed' || !Array.isArray(item.content)) throw bad();
        blocks.push(...item.content);
      }
    } else if (format === 'claude-messages') {
      if (data.stop_reason !== 'end_turn' || !Array.isArray(data.content)) throw bad();
      blocks.push(...data.content);
    } else throw bad();
    text = blocks.map((block) => {
      if (!block || typeof block !== 'object') throw bad();
      const b = block as Record<string, unknown>;
      if (b.type !== (format === 'openai-responses' ? 'output_text' : 'text') || typeof b.text !== 'string') throw bad();
      return b.text;
    }).join('\n');
  }
  if (typeof text !== 'string' || !text.trim() || [...text].length > 24_000) throw bad();
  return text;
}
