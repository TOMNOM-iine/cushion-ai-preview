//! Receives approved text only; this module has no Core, original, dictionary or history access.
use security_framework::passwords::{
    generic_password, set_generic_password_options, PasswordOptions,
};
use serde_json::{json, Value};
use std::{io::Read, time::Duration};
use zeroize::Zeroizing;
const SERVICE: &str = "local.cushion.api.v1";
fn account(provider: &str) -> Result<&'static str, String> {
    match provider {
        "OpenAI" => Ok("OpenAI"),
        "Claude" => Ok("Claude"),
        _ => Err("このAPI接続先には対応していません。".into()),
    }
}
pub fn configured(provider: &str) -> bool {
    key(provider).is_ok()
}
fn options(provider: &str) -> Result<PasswordOptions, String> {
    let mut options = PasswordOptions::new_generic_password(SERVICE, account(provider)?);
    options.set_access_synchronized(Some(false));
    Ok(options)
}
pub fn key(provider: &str) -> Result<Zeroizing<Vec<u8>>, String> {
    generic_password(options(provider)?)
        .map(Zeroizing::new)
        .map_err(|_| "APIキーをこのMacのキーチェーンへ登録してください。".into())
}
pub fn set_key(provider: &str, key: &str) -> Result<(), String> {
    if key.len() < 16 || key.len() > 512 || !key.bytes().all(|b| b.is_ascii_graphic()) {
        return Err("APIキーの形式を確認してください。".into());
    }
    set_generic_password_options(key.as_bytes(), options(provider)?)
        .map_err(|_| "APIキーをキーチェーンに保存できませんでした。".into())
}
pub fn wire(provider: &str, text: &str) -> Result<(&'static str, Value), String> {
    // Any change to these models/body options MUST update runtime::PIPELINE_CONTRACT.
    // That fingerprint participates in Core approval, including the local policy.
    if text.is_empty() || text.len() > 65536 {
        return Err("変換文がAPI送信の上限を超えています。".into());
    }
    match provider {
        "OpenAI" => Ok((
            "https://api.openai.com/v1/responses",
            json!({"model":"gpt-4.1-mini","input":text,"store":false,"max_output_tokens":2048}),
        )),
        "Claude" => Ok((
            "https://api.anthropic.com/v1/messages",
            json!({"model":"claude-sonnet-4-6","max_tokens":2048,"messages":[{"role":"user","content":text}]}),
        )),
        _ => Err("このAPI接続先には対応していません。".into()),
    }
}
const DELIVERY: &str =
    "APIの回答を確認できませんでした。送信先に届いた可能性があります。自動再送はしません。";
pub fn exchange(provider: &str, text: &str, key: &[u8]) -> Result<String, String> {
    let (url, body) = wire(provider, text)?;
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|_| DELIVERY)?;
    let secret = std::str::from_utf8(key).map_err(|_| DELIVERY)?;
    let request = client.post(url).json(&body);
    let request = if provider == "OpenAI" {
        request.bearer_auth(secret)
    } else {
        request
            .header("x-api-key", secret)
            .header("anthropic-version", "2023-06-01")
    };
    let response = request.send().map_err(|_| DELIVERY)?;
    if !response.status().is_success() {
        return Err(DELIVERY.into());
    }
    let mut bytes = Vec::new();
    response
        .take(131073)
        .read_to_end(&mut bytes)
        .map_err(|_| DELIVERY)?;
    if bytes.len() > 131072 {
        return Err(DELIVERY.into());
    }
    let v: Value = serde_json::from_slice(&bytes).map_err(|_| DELIVERY)?;
    read_text(provider, &v)
}
pub fn read_text(provider: &str, v: &Value) -> Result<String, String> {
    let mut text = String::new();
    if provider == "OpenAI" {
        if v["status"] != "completed" {
            return Err(DELIVERY.into());
        }
        for item in v["output"].as_array().ok_or(DELIVERY)? {
            if item["type"] != "message"
                || item["role"] != "assistant"
                || item["status"] != "completed"
            {
                return Err(DELIVERY.into());
            }
            for c in item["content"].as_array().ok_or(DELIVERY)? {
                if c["type"] != "output_text" {
                    return Err(DELIVERY.into());
                }
                text.push_str(c["text"].as_str().ok_or(DELIVERY)?);
            }
        }
    } else if provider == "Claude" {
        if v["type"] != "message" || v["role"] != "assistant" || v["stop_reason"] != "end_turn" {
            return Err(DELIVERY.into());
        }
        for c in v["content"].as_array().ok_or(DELIVERY)? {
            if c["type"] != "text" {
                return Err(DELIVERY.into());
            }
            text.push_str(c["text"].as_str().ok_or(DELIVERY)?);
        }
    } else {
        return Err(DELIVERY.into());
    }
    if text.is_empty() || text.chars().count() > 24000 {
        return Err(DELIVERY.into());
    }
    Ok(text)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn request_only_contains_approved_text() {
        let (u, b) = wire("OpenAI", "[[CUSHION_TEST]]").unwrap();
        assert_eq!(u, "https://api.openai.com/v1/responses");
        assert_eq!(b.as_object().unwrap().len(), 4);
        assert_eq!(b["store"], false);
        assert_eq!(b["input"], "[[CUSHION_TEST]]");
        assert!(wire("https://evil.test", "x").is_err());
    }
    #[test]
    fn refuses_tools_refusals_and_partial() {
        for v in [
            json!({"status":"incomplete"}),
            json!({"status":"completed","output":[{"type":"function_call"}]}),
            json!({"status":"completed","output":[{"type":"message","role":"assistant","status":"completed","content":[{"type":"refusal","refusal":"x"}]}]}),
        ] {
            assert!(read_text("OpenAI", &v).is_err());
        }
    }
    #[test]
    fn literal_html_is_only_text() {
        let v = json!({"type":"message","role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"<img src=https://invalid.test>"}]});
        assert_eq!(
            read_text("Claude", &v).unwrap(),
            "<img src=https://invalid.test>"
        );
    }
    #[test]
    fn claude_partial_and_tools_refused() {
        assert!(read_text(
            "Claude",
            &json!({"type":"message","role":"assistant","stop_reason":"max_tokens","content":[]})
        )
        .is_err());
        assert!(read_text("Claude",&json!({"type":"message","role":"assistant","stop_reason":"end_turn","content":[{"type":"tool_use"}]})).is_err());
    }
}
