//! Public ChatGPT plan-use transport. Receives only the Core-approved text.
//! No original, dictionary, restored history, tools or arbitrary destination.
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read};
use std::time::Duration;

#[path = "subscription_stream.rs"]
mod stream;

pub const CONTRACT: &str = "siwc-plan-v7|public-responses|single-user-array|store:false|stream:true|accept:text/event-stream|strict-sse-with-shape-diagnostics|sse:single-leading-bom-cr-lf-crlf|answer:stream-ledger-v1|output-diagnostics:refusal-item-part-role-message-empty-malformed|http:bounded-known-error-codes|no-content:204-205-stop|non-sse-empty:stop|completed-only|reasoning-and-annotations:discard|no-tools-history|wire-cap:1048576|answer-cap:24000";
#[cfg(test)]
const DELIVERY: &str = "ChatGPTの回答の完了を確認できませんでした。変換文が届いた可能性があります。自動再送はしません。接続状態と利用上限を確認してから、再検査してください。";
const WIRE_LIMIT: u64 = 1_048_576;

/// Fixed diagnostic classes only. No response content, headers or credentials.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    Network,
    HttpDenied,
    HttpLimit,
    UsageUnavailable,
    HttpOther,
    HttpNoContent,
    EmptyBody,
    ContentType,
    ContentTypeJson,
    ContentTypeHtml,
    ContentTypeMissing,
    ContentTypeOther,
    Truncated,
    Json,
    EventOrder,
    UnknownEvent,
    UnexpectedOutput,
    Refusal,
    UnsupportedItem,
    UnsupportedPart,
    UnexpectedRole,
    IncompleteMessage,
    EmptyAnswer,
    MalformedOutput,
    ResponseIdentity,
    ByteLimit,
    TextLimit,
    Partial,
}
impl Stop {
    pub fn code(self) -> &'static str {
        match self {
            Self::Network => "cushion-plan:network",
            Self::HttpDenied => "cushion-plan:http-denied",
            Self::HttpLimit => "cushion-plan:http-limit",
            Self::UsageUnavailable => "cushion-plan:usage-unavailable",
            Self::HttpOther => "cushion-plan:http-other",
            Self::HttpNoContent => "cushion-plan:http-no-content",
            Self::EmptyBody => "cushion-plan:empty-body",
            Self::ContentType => "cushion-plan:content-type",
            Self::ContentTypeJson => "cushion-plan:content-type-json",
            Self::ContentTypeHtml => "cushion-plan:content-type-html",
            Self::ContentTypeMissing => "cushion-plan:content-type-missing",
            Self::ContentTypeOther => "cushion-plan:content-type-other",
            Self::Truncated => "cushion-plan:truncated",
            Self::Json => "cushion-plan:json",
            Self::EventOrder => "cushion-plan:event-order",
            Self::UnknownEvent => "cushion-plan:unknown-event",
            Self::UnexpectedOutput => "cushion-plan:unexpected-output",
            Self::Refusal => "cushion-plan:refusal",
            Self::UnsupportedItem => "cushion-plan:unsupported-item",
            Self::UnsupportedPart => "cushion-plan:unsupported-part",
            Self::UnexpectedRole => "cushion-plan:unexpected-role",
            Self::IncompleteMessage => "cushion-plan:incomplete-message",
            Self::EmptyAnswer => "cushion-plan:empty-answer",
            Self::MalformedOutput => "cushion-plan:malformed-output",
            Self::ResponseIdentity => "cushion-plan:response-identity",
            Self::ByteLimit => "cushion-plan:byte-limit",
            Self::TextLimit => "cushion-plan:text-limit",
            Self::Partial => "cushion-plan:partial",
        }
    }
    pub fn from_code(code: &str) -> Option<Self> {
        [
            Self::Network,
            Self::HttpDenied,
            Self::HttpLimit,
            Self::UsageUnavailable,
            Self::HttpOther,
            Self::HttpNoContent,
            Self::EmptyBody,
            Self::ContentType,
            Self::ContentTypeJson,
            Self::ContentTypeHtml,
            Self::ContentTypeMissing,
            Self::ContentTypeOther,
            Self::Truncated,
            Self::Json,
            Self::EventOrder,
            Self::UnknownEvent,
            Self::UnexpectedOutput,
            Self::Refusal,
            Self::UnsupportedItem,
            Self::UnsupportedPart,
            Self::UnexpectedRole,
            Self::IncompleteMessage,
            Self::EmptyAnswer,
            Self::MalformedOutput,
            Self::ResponseIdentity,
            Self::ByteLimit,
            Self::TextLimit,
            Self::Partial,
        ]
        .into_iter()
        .find(|stop| stop.code() == code)
    }
}

pub fn wire(model: &str, text: &str) -> Result<Value, String> {
    if model.is_empty()
        || model.len() > 128
        || !model.bytes().all(|b| b.is_ascii_graphic())
        || text.is_empty()
        || text.len() > 65_536
    {
        return Err("モデルまたは変換文の送信条件を確認できません。再検査してください。".into());
    }
    Ok(json!({"model":model,"input":[{"role":"user","content":text}],"store":false,"stream":true}))
}

pub fn exchange_classified(model: &str, text: &str, access_token: &str) -> Result<String, Stop> {
    let body = wire(model, text).map_err(|_| Stop::UnexpectedOutput)?;
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|_| Stop::Network)?;
    let response = client
        .post("https://api.openai.com/v1/responses")
        .bearer_auth(access_token)
        .header(reqwest::header::ACCEPT, "text/event-stream")
        .json(&body)
        .send()
        .map_err(|_| Stop::Network)?;
    let status = response.status().as_u16();
    let response_type = ResponseType::from_header(
        response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
    );
    read_http_response_classified(status, response, response_type)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResponseType {
    Stream,
    Json,
    Html,
    Missing,
    Other,
}
impl ResponseType {
    fn from_header(value: Option<&str>) -> Self {
        let Some(value) = value else {
            return Self::Missing;
        };
        let mime = value.split(';').next().unwrap_or("").trim();
        if mime.is_empty() {
            Self::Missing
        } else if mime.eq_ignore_ascii_case("text/event-stream") {
            Self::Stream
        } else if mime.eq_ignore_ascii_case("application/json")
            || mime.eq_ignore_ascii_case("application/problem+json")
        {
            Self::Json
        } else if mime.eq_ignore_ascii_case("text/html")
            || mime.eq_ignore_ascii_case("application/xhtml+xml")
        {
            Self::Html
        } else {
            Self::Other
        }
    }

    fn stop(self) -> Stop {
        match self {
            Self::Stream => Stop::ContentType,
            Self::Json => Stop::ContentTypeJson,
            Self::Html => Stop::ContentTypeHtml,
            Self::Missing => Stop::ContentTypeMissing,
            Self::Other => Stop::ContentTypeOther,
        }
    }
}

fn read_http_response_classified(
    status: u16,
    reader: impl Read,
    response_type: ResponseType,
) -> Result<String, Stop> {
    match status {
        // Definitive status classes keep their fixed meaning without body I/O.
        401 | 403 => Err(Stop::HttpDenied),
        429 => Err(Stop::HttpLimit),
        204 | 205 => Err(Stop::HttpNoContent),
        200..=299 => read_response_classified(reader, response_type),
        _ => {
            // Error bodies never enter the answer parser, including a body that
            // resembles a completed stream. Only known machine codes classify.
            let bytes = read_bounded(reader)?;
            Err(known_json_error(&bytes).unwrap_or(Stop::HttpOther))
        }
    }
}

fn read_bounded(reader: impl Read) -> Result<Vec<u8>, Stop> {
    let mut bytes = Vec::new();
    reader
        .take(WIRE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Stop::Network)?;
    if bytes.len() as u64 > WIRE_LIMIT {
        return Err(Stop::ByteLimit);
    }
    Ok(bytes)
}

fn read_response_classified(
    reader: impl Read,
    response_type: ResponseType,
) -> Result<String, Stop> {
    if response_type == ResponseType::Stream {
        // A completed stream returns immediately, without waiting for HTTP EOF.
        return read_stream_classified(reader);
    }
    // Mislabelled responses stay bounded and in memory. Their declared MIME can
    // classify a failure, but only the same strict SSE parser can return text.
    let bytes = read_bounded(reader)?;
    if bytes.is_empty() {
        return Err(Stop::EmptyBody);
    }
    if has_sse_framing(&bytes) {
        // The header cannot hide the reason why an event stream stopped.
        // Framing alone never permits output: all semantic checks still apply.
        return read_stream_classified(bytes.as_slice());
    }
    // Non-SSE bodies never become answers. Shape and closed machine codes are
    // diagnostics only; no message, header, unknown code or body leaves here.
    Err(known_json_error(&bytes).unwrap_or_else(|| body_shape_stop(&bytes, response_type)))
}

fn without_leading_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes)
}

fn has_sse_framing(bytes: &[u8]) -> bool {
    let first_line = without_leading_bom(bytes)
        .split(|byte| matches!(byte, b'\r' | b'\n'))
        .find(|line| !line.is_empty());
    first_line.is_some_and(|line| {
        [b"data:".as_slice(), b"event:", b"id:", b"retry:", b":"]
            .iter()
            .any(|prefix| line.starts_with(prefix))
    })
}

fn body_shape_stop(bytes: &[u8], response_type: ResponseType) -> Stop {
    let body = without_leading_bom(bytes).trim_ascii_start();
    if matches!(body.first(), Some(b'{' | b'[')) || serde_json::from_slice::<Value>(body).is_ok() {
        return Stop::ContentTypeJson;
    }
    if [b"<!doctype html".as_slice(), b"<html", b"<head", b"<body"]
        .iter()
        .any(|prefix| {
            body.get(..prefix.len())
                .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
        })
    {
        return Stop::ContentTypeHtml;
    }
    response_type.stop()
}

fn known_json_error(bytes: &[u8]) -> Option<Stop> {
    serde_json::from_slice::<Value>(without_leading_bom(bytes))
        .ok()
        .and_then(|value| known_error(value["error"]["code"].as_str()))
}

fn known_error(code: Option<&str>) -> Option<Stop> {
    match code {
        Some("subscription_sharing_usage_limit_exceeded") => Some(Stop::HttpLimit),
        Some("subscription_sharing_usage_unavailable") => Some(Stop::UsageUnavailable),
        _ => None,
    }
}

/// Bound total bytes even for an unterminated line; deltas never leave this module.
#[cfg(test)]
fn read_stream(reader: impl Read) -> Result<String, String> {
    read_stream_classified(reader).map_err(|_| DELIVERY.into())
}
pub fn read_stream_classified(reader: impl Read) -> Result<String, Stop> {
    let mut reader = BufReader::new(reader.take(WIRE_LIMIT + 1));
    let mut line = Vec::new();
    let mut data = String::new();
    let mut event_name = String::new();
    let mut bytes = 0u64;
    let mut response_id: Option<String> = None;
    let mut skip_lf = false;
    let mut first_line = true;
    let mut ledger = stream::Ledger::default();
    loop {
        if !read_sse_line(&mut reader, &mut line, &mut bytes, &mut skip_lf)? {
            return Err(Stop::Truncated);
        }
        let decoded = std::str::from_utf8(&line).map_err(|_| Stop::Json)?;
        let line = if first_line {
            decoded.strip_prefix('\u{feff}').unwrap_or(decoded)
        } else {
            decoded
        };
        first_line = false;
        if line.is_empty() {
            if data.is_empty() {
                event_name.clear();
                continue;
            }
            let v: Value =
                serde_json::from_str(data.trim_end_matches('\n')).map_err(|_| Stop::Json)?;
            let kind = v["type"].as_str().ok_or(Stop::Json)?;
            if !event_name.is_empty() && event_name != kind {
                return Err(Stop::EventOrder);
            }
            ledger.sequence(&v)?;
            // Admission failures may precede response.created. Recognize only
            // closed machine codes on failure events; no output is accepted.
            if matches!(kind, "error" | "response.failed") {
                if let Some(stop) = known_error(
                    v["response"]["error"]["code"]
                        .as_str()
                        .or(v["code"].as_str()),
                ) {
                    return Err(stop);
                }
            }
            if kind != "response.created" && response_id.is_none() {
                return Err(Stop::EventOrder);
            }
            match kind {
                "response.created" => {
                    if response_id.is_some() {
                        return Err(Stop::EventOrder);
                    }
                    response_id = Some(valid_id(&v["response"])?);
                }
                "response.in_progress" | "response.queued" => {
                    check_id(&v["response"], &response_id)?
                }
                "response.completed" => {
                    check_id(&v["response"], &response_id)?;
                    return ledger.finish(&v["response"], completed_text(&v["response"]));
                }
                "response.output_item.added" => ledger.added(&v)?,
                "response.output_item.done" => ledger.item_done(&v)?,
                "response.content_part.added" => ledger.part_added(&v)?,
                "response.content_part.done" => ledger.part_done(&v)?,
                "response.output_text.delta" => ledger.delta(&v)?,
                "response.output_text.done" => ledger.text_done(&v)?,
                "response.output_text.annotation.added" => ledger.annotation(&v)?,
                "response.reasoning_summary_part.added"
                | "response.reasoning_summary_part.done"
                | "response.reasoning_summary_text.delta"
                | "response.reasoning_summary_text.done"
                | "response.reasoning_text.delta"
                | "response.reasoning_text.done" => ledger.reasoning(&v)?,
                // Failure, refusal, tools, partial completion and unknown contracts stop.
                "response.failed" | "error" => return Err(Stop::Partial),
                "response.incomplete" => return Err(Stop::Partial),
                "response.refusal.delta" | "response.refusal.done" => return Err(Stop::Refusal),
                _ => return Err(Stop::UnknownEvent),
            }
            data.clear();
            event_name.clear();
        } else if let Some(value) = line.strip_prefix("data:") {
            data.push_str(value.strip_prefix(' ').unwrap_or(value));
            data.push('\n');
        } else if let Some(value) = line.strip_prefix("event:") {
            if !event_name.is_empty() {
                return Err(Stop::EventOrder);
            }
            event_name = value.trim().into();
        } else if line.starts_with(':') || line.starts_with("id:") || line.starts_with("retry:") {
            // SSE transport metadata only; never fetch or execute anything.
        } else {
            return Err(Stop::EventOrder);
        }
    }
}

/// SSE uses CR, LF or CRLF as line endings and ignores one initial BOM.
/// Count the raw wire bytes, including delimiters/BOM, before decoding a line.
fn read_sse_line(
    reader: &mut impl BufRead,
    line: &mut Vec<u8>,
    bytes: &mut u64,
    skip_lf: &mut bool,
) -> Result<bool, Stop> {
    line.clear();
    loop {
        let buffer = reader.fill_buf().map_err(|_| Stop::Network)?;
        let Some(&byte) = buffer.first() else {
            return Ok(!line.is_empty());
        };
        reader.consume(1);
        *bytes += 1;
        if *bytes > WIRE_LIMIT {
            return Err(Stop::ByteLimit);
        }
        if *skip_lf {
            *skip_lf = false;
            if byte == b'\n' {
                continue;
            }
        }
        match byte {
            b'\r' => {
                *skip_lf = true;
                return Ok(true);
            }
            b'\n' => return Ok(true),
            _ => line.push(byte),
        }
    }
}
/// Reasoning metadata stays inert. Only completed assistant output_text becomes an answer.
fn completed_text(response: &Value) -> Result<String, Stop> {
    if response["status"] != "completed"
        || !response["error"].is_null()
        || !response["incomplete_details"].is_null()
    {
        return Err(Stop::Partial);
    }
    let mut text = String::new();
    for item in response["output"].as_array().ok_or(Stop::MalformedOutput)? {
        match item["type"].as_str() {
            Some("reasoning") => {
                stream::reasoning_status(item, true)?;
                continue;
            }
            Some("message") => {}
            Some(_) => return Err(Stop::UnsupportedItem),
            None => return Err(Stop::MalformedOutput),
        }
        if item["role"] != "assistant" {
            return Err(Stop::UnexpectedRole);
        }
        if item["status"] != "completed" {
            return Err(Stop::IncompleteMessage);
        }
        for part in item["content"].as_array().ok_or(Stop::MalformedOutput)? {
            match part["type"].as_str() {
                Some("refusal") => return Err(Stop::Refusal),
                Some("output_text") => {}
                Some(_) => return Err(Stop::UnsupportedPart),
                None => return Err(Stop::MalformedOutput),
            }
            text.push_str(part["text"].as_str().ok_or(Stop::MalformedOutput)?);
        }
    }
    if text.is_empty() {
        return Err(Stop::EmptyAnswer);
    }
    if text.chars().count() > 24_000 {
        return Err(Stop::TextLimit);
    }
    Ok(text)
}
fn valid_id(response: &Value) -> Result<String, Stop> {
    let id = response["id"].as_str().ok_or(Stop::ResponseIdentity)?;
    if !id.starts_with("resp_")
        || id.len() > 256
        || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(Stop::ResponseIdentity);
    }
    Ok(id.into())
}
fn check_id(response: &Value, expected: &Option<String>) -> Result<(), Stop> {
    if expected.as_deref() != Some(valid_id(response)?.as_str()) {
        return Err(Stop::ResponseIdentity);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(v: Value) -> String {
        format!("event: {}\ndata: {}\n\n", v["type"].as_str().unwrap(), v)
    }
    fn created() -> String {
        event(json!({"type":"response.created","response":{"id":"resp_synthetic"}}))
    }
    fn completed(text: &str) -> String {
        event(
            json!({"type":"response.completed","response":{"id":"resp_synthetic","status":"completed","output":[message(text)]}}),
        )
    }
    fn message(text: &str) -> Value {
        json!({"id":"msg_synthetic","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":text}]})
    }
    fn message_events(text: &str, position: usize) -> Vec<Value> {
        vec![
            json!({"type":"response.output_item.added","output_index":position,"item":{"id":"msg_synthetic","type":"message","role":"assistant","status":"in_progress","content":[]}}),
            json!({"type":"response.content_part.added","output_index":position,"item_id":"msg_synthetic","content_index":0,"part":{"type":"output_text","text":""}}),
            json!({"type":"response.output_text.delta","output_index":position,"item_id":"msg_synthetic","content_index":0,"delta":text}),
            json!({"type":"response.output_text.done","output_index":position,"item_id":"msg_synthetic","content_index":0,"text":text}),
            json!({"type":"response.content_part.done","output_index":position,"item_id":"msg_synthetic","content_index":0,"part":{"type":"output_text","text":text}}),
            json!({"type":"response.output_item.done","output_index":position,"item":message(text)}),
        ]
    }
    fn terminal(output: Value) -> String {
        event(
            json!({"type":"response.completed","response":{"id":"resp_synthetic","status":"completed","output":output}}),
        )
    }
    fn events_wire(events: &[Value]) -> String {
        events.iter().map(|value| event(value.clone())).collect()
    }
    fn classified(bytes: &[u8], mime: Option<&str>) -> Result<String, Stop> {
        read_http_response_classified(200, bytes, ResponseType::from_header(mime))
    }
    #[test]
    fn request_is_only_approved_text_and_public_plan_options() {
        let v = wire("fixture-model", "[[CUSHION_SYNTHETIC]]").unwrap();
        assert_eq!(v.as_object().unwrap().len(), 4);
        assert_eq!(
            v["input"],
            json!([{"role":"user","content":"[[CUSHION_SYNTHETIC]]"}])
        );
        assert_eq!(v["store"], false);
        assert_eq!(v["stream"], true);
        for field in [
            "max_output_tokens",
            "previous_response_id",
            "tools",
            "metadata",
        ] {
            assert!(v.get(field).is_none());
        }
    }
    #[test]
    fn full_completion_returns_plain_text_not_deltas() {
        let wire = created()
            + &events_wire(&message_events(
                "<img src=https://invalid.test>[[CUSHION_SYNTHETIC]]",
                0,
            ))
            + &completed("<img src=https://invalid.test>[[CUSHION_SYNTHETIC]]");
        assert_eq!(
            read_stream(wire.as_bytes()).unwrap(),
            "<img src=https://invalid.test>[[CUSHION_SYNTHETIC]]"
        );
        assert!(read_stream(
            (created() + &event(json!({"type":"response.output_text.delta","delta":"partial"})))
                .as_bytes()
        )
        .is_err());
    }
    #[test]
    fn failure_incomplete_refusal_tools_and_mismatched_id_stop() {
        for kind in [
            "response.failed",
            "response.incomplete",
            "error",
            "response.refusal.delta",
            "unknown",
        ] {
            assert!(read_stream(
                (created() + &event(json!({"type":kind})) + &completed("never")).as_bytes()
            )
            .is_err());
        }
        assert!(read_stream(
            (created()
                + &event(
                    json!({"type":"response.output_item.added","item":{"type":"function_call"}})
                ))
                .as_bytes()
        )
        .is_err());
        assert!(read_stream(
            (created() + &completed("x").replace("resp_synthetic", "resp_other")).as_bytes()
        )
        .is_err());
        assert!(read_stream(completed("missing created").as_bytes()).is_err());
    }
    #[test]
    fn invalid_json_event_confusion_and_wire_overflow_stop_without_echo() {
        for wire in [
            "data: {invalid}\n\n".to_string(),
            "event: response.completed\ndata: {\"type\":\"response.created\"}\n\n".to_string(),
            "data: ".to_string() + &"x".repeat(WIRE_LIMIT as usize),
        ] {
            let error = read_stream(wire.as_bytes()).unwrap_err();
            assert_eq!(error, DELIVERY);
        }
    }
    #[test]
    fn reasoning_is_discarded_and_answer_cap_uses_characters() {
        let response = json!({"id":"resp_synthetic","status":"completed","output":[
            {"type":"reasoning","summary":[{"type":"summary_text","text":"private reasoning never displayed"}]},
            {"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"あ".repeat(24000)}]}
        ]});
        let wire = created() + &event(json!({"type":"response.completed","response":response}));
        assert_eq!(read_stream(wire.as_bytes()).unwrap(), "あ".repeat(24000));
        assert!(read_stream((created() + &completed(&"あ".repeat(24001))).as_bytes()).is_err());
        assert!(read_stream(
            (event(json!({"type":"response.output_text.delta","delta":"early"}))
                + &created()
                + &completed("answer"))
                .as_bytes()
        )
        .is_err());
    }
    #[test]
    fn official_reasoning_parts_and_annotations_are_inert() {
        let wire = created()
            + &event(json!({"type":"response.queued","response":{"id":"resp_synthetic"}}))
            + &event(
                json!({"type":"response.output_item.added","output_index":0,"item":{"id":"rs_synthetic","type":"reasoning"}}),
            )
            + &event(
                json!({"type":"response.content_part.added","output_index":0,"item_id":"rs_synthetic","content_index":0,"part":{"type":"reasoning_text","text":"never exposed"}}),
            )
            + &event(
                json!({"type":"response.reasoning_text.delta","output_index":0,"item_id":"rs_synthetic","content_index":0,"delta":"never exposed"}),
            )
            + &event(
                json!({"type":"response.content_part.done","output_index":0,"item_id":"rs_synthetic","content_index":0,"part":{"type":"reasoning_text","text":"never exposed"}}),
            )
            + &event(
                json!({"type":"response.output_item.done","output_index":0,"item":{"id":"rs_synthetic","type":"reasoning"}}),
            )
            + &events_wire(&message_events("final only", 1))
            + &terminal(json!([]));
        assert_eq!(
            read_stream_classified(wire.as_bytes()).unwrap(),
            "final only"
        );
        assert_eq!(
            read_stream_classified(
                (created()
                    + &event(
                        json!({"type":"response.content_part.added","part":{"type":"refusal"}})
                    ))
                    .as_bytes()
            ),
            Err(Stop::Refusal)
        );
    }
    #[test]
    fn streamed_output_diagnostics_are_closed_and_do_not_accept_unsupported_items() {
        for (value, expected) in [
            (
                json!({"type":"response.refusal.delta","delta":"private refusal never echoed"}),
                Stop::Refusal,
            ),
            (
                json!({"type":"response.refusal.done","refusal":"private refusal never echoed"}),
                Stop::Refusal,
            ),
            (
                json!({"type":"response.content_part.added","part":{"type":"refusal","refusal":"never echoed"}}),
                Stop::Refusal,
            ),
            (
                json!({"type":"response.content_part.done","part":{"type":"unknown-part-never-echoed","text":"never echoed"}}),
                Stop::UnsupportedPart,
            ),
            (
                json!({"type":"response.content_part.done","part":{"type":42}}),
                Stop::MalformedOutput,
            ),
            (
                json!({"type":"response.output_item.added","item":{"type":"function_call","name":"never executed"}}),
                Stop::UnsupportedItem,
            ),
            (
                json!({"type":"response.output_item.done","item":{"type":"unknown-item-never-echoed"}}),
                Stop::UnsupportedItem,
            ),
            (
                json!({"type":"response.output_item.added","item":{"type":"message","role":"user","text":"never echoed"}}),
                Stop::UnexpectedRole,
            ),
            (
                json!({"type":"response.output_item.done","item":{"type":"message"}}),
                Stop::UnexpectedRole,
            ),
            (
                json!({"type":"response.output_item.done","item":null}),
                Stop::MalformedOutput,
            ),
        ] {
            let wire = created() + &event(value) + &completed("never accepted");
            for mime in [None, Some("text/event-stream"), Some("application/json")] {
                assert_eq!(classified(wire.as_bytes(), mime), Err(expected));
            }
            assert_eq!(Stop::from_code(expected.code()), Some(expected));
            assert!(!expected.code().contains("never echoed"));
        }
        let wire =
            created() + &events_wire(&message_events("final only", 0)) + &completed("final only");
        assert_eq!(classified(wire.as_bytes(), None), Ok("final only".into()));
    }
    #[test]
    fn completed_output_diagnostics_keep_the_same_acceptance_boundary() {
        for (output, expected) in [
            (
                json!([{"type":"message","role":"assistant","status":"completed","content":[{"type":"refusal","refusal":"never echoed"}]}]),
                Stop::Refusal,
            ),
            (
                json!([{"type":"function_call","name":"never executed"}]),
                Stop::UnsupportedItem,
            ),
            (
                json!([{"type":"message","role":"user","status":"completed","content":[]}]),
                Stop::UnexpectedRole,
            ),
            (
                json!([{"type":"message","status":"completed","content":[]}]),
                Stop::UnexpectedRole,
            ),
            (
                json!([{"type":"message","role":"assistant","content":[]}]),
                Stop::IncompleteMessage,
            ),
            (
                json!([{"type":"message","role":"assistant","status":"incomplete","content":[]}]),
                Stop::IncompleteMessage,
            ),
            (
                json!([{"type":"message","role":"assistant","status":"in_progress","content":[]}]),
                Stop::IncompleteMessage,
            ),
            (
                json!([{"type":"message","role":"assistant","status":"completed","content":[]}]),
                Stop::EmptyAnswer,
            ),
            (
                json!([{"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":""}]}]),
                Stop::EmptyAnswer,
            ),
            (
                json!([{"type":"reasoning","summary":[]}]),
                Stop::EmptyAnswer,
            ),
            (json!([]), Stop::EmptyAnswer),
            (
                json!([{"type":"message","role":"assistant","status":"completed","content":[{"type":"reasoning_text","text":"never exposed"}]}]),
                Stop::UnsupportedPart,
            ),
            (
                json!([{"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":false}]}]),
                Stop::MalformedOutput,
            ),
            (
                json!([{"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text"}]}]),
                Stop::MalformedOutput,
            ),
            (
                json!([{"type":"message","role":"assistant","status":"completed","content":null}]),
                Stop::MalformedOutput,
            ),
            (
                json!([{"type":"message","role":"assistant","status":"completed","content":[null]}]),
                Stop::MalformedOutput,
            ),
            (json!([{"type":null}]), Stop::MalformedOutput),
            (Value::Null, Stop::MalformedOutput),
        ] {
            let wire = created()
                + &event(
                    json!({"type":"response.completed","response":{"id":"resp_synthetic","status":"completed","output":output}}),
                );
            for mime in [None, Some("text/event-stream"), Some("text/html")] {
                assert_eq!(classified(wire.as_bytes(), mime), Err(expected));
            }
            assert_eq!(Stop::from_code(expected.code()), Some(expected));
        }
    }
    #[test]
    fn output_diagnostics_cannot_bypass_response_order_identity_or_completion() {
        let new_codes = [
            Stop::Refusal,
            Stop::UnsupportedItem,
            Stop::UnsupportedPart,
            Stop::UnexpectedRole,
            Stop::IncompleteMessage,
            Stop::EmptyAnswer,
            Stop::MalformedOutput,
        ];
        for code in new_codes {
            assert_eq!(Stop::from_code(code.code()), Some(code));
        }
        assert_eq!(
            Stop::from_code("cushion-plan:unsupported-item:private-value"),
            None
        );
        let empty_completed = json!({"type":"response.completed","response":{"id":"resp_synthetic","status":"completed","output":[]}});
        assert_eq!(
            classified(event(empty_completed.clone()).as_bytes(), None),
            Err(Stop::EventOrder)
        );
        let wrong_id = event(empty_completed.clone()).replace("resp_synthetic", "resp_foreign");
        assert_eq!(
            classified((created() + &wrong_id).as_bytes(), None),
            Err(Stop::ResponseIdentity)
        );
        let incomplete =
            event(empty_completed).replace("\"status\":\"completed\"", "\"status\":\"incomplete\"");
        assert_eq!(
            classified((created() + &incomplete).as_bytes(), None),
            Err(Stop::Partial)
        );
        assert_eq!(
            classified(
                (created() + &events_wire(&message_events("never returned", 0)[..3])).as_bytes(),
                None
            ),
            Err(Stop::Truncated)
        );
    }
    #[test]
    fn ledger_returns_only_completed_verified_text_when_final_output_is_empty() {
        let text = "架空の回答 <img src=https://invalid.test>";
        let mut events = message_events(text, 0);
        events.insert(5, json!({"type":"response.output_text.annotation.added","output_index":0,"content_index":0,"item_id":"msg_synthetic","annotation":{"url":"https://invalid.test"}}));
        let wire = created() + &events_wire(&events) + &terminal(json!([]));
        for mime in [None, Some("text/event-stream"), Some("application/json")] {
            assert_eq!(classified(wire.as_bytes(), mime), Ok(text.into()));
        }
        assert_eq!(
            classified((created() + &terminal(json!([]))).as_bytes(), None),
            Err(Stop::EmptyAnswer)
        );
    }
    #[test]
    fn ledger_final_messages_must_match_including_empty_content() {
        let stream = created() + &events_wire(&message_events("verified text", 0));
        assert_eq!(
            classified(
                (stream.clone() + &completed("verified text")).as_bytes(),
                None
            ),
            Ok("verified text".into())
        );
        for output in [
            json!([message("different text")]),
            json!([message("")]),
            json!([{"id":"msg_synthetic","type":"message","role":"assistant","status":"completed","content":[]}]),
            json!([{"id":"msg_foreign","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"verified text"}]}]),
            json!([message("verified text"), message("verified text")]),
            json!([{"id":"rs_unknown","type":"reasoning"}]),
        ] {
            assert!(classified((stream.clone() + &terminal(output)).as_bytes(), None).is_err());
        }
    }
    #[test]
    fn ledger_requires_every_message_lifecycle_step() {
        let events = message_events("complete", 0);
        for missing in 0..events.len() {
            let mut incomplete = events.clone();
            incomplete.remove(missing);
            for end in [terminal(json!([])), completed("complete")] {
                assert!(classified(
                    (created() + &events_wire(&incomplete) + &end).as_bytes(),
                    None
                )
                .is_err());
            }
        }
        for count in 1..events.len() {
            assert!(classified(
                (created() + &events_wire(&events[..count]) + &completed("complete")).as_bytes(),
                None
            )
            .is_err());
        }
    }
    #[test]
    fn ledger_rejects_duplicates_late_updates_and_identity_index_changes() {
        let events = message_events("complete", 0);
        for duplicate in 0..events.len() {
            let mut altered = events.clone();
            altered.insert(duplicate + 1, events[duplicate].clone());
            // Repeated delta changes the assembled text and must mismatch done.
            assert!(classified(
                (created() + &events_wire(&altered) + &terminal(json!([]))).as_bytes(),
                None
            )
            .is_err());
        }
        for late in [
            events[2].clone(),
            events[3].clone(),
            events[4].clone(),
            events[5].clone(),
        ] {
            assert!(classified(
                (created() + &events_wire(&events) + &event(late) + &terminal(json!([])))
                    .as_bytes(),
                None
            )
            .is_err());
        }
        for (which, field, value) in [
            (0, "output_index", json!(1)),
            (0, "output_index", json!(-1)),
            (2, "output_index", json!(1)),
            (2, "content_index", json!(1)),
            (2, "item_id", json!("msg_foreign")),
            (2, "content_index", json!(0.5)),
        ] {
            let mut altered = events.clone();
            altered[which][field] = value;
            assert!(classified(
                (created() + &events_wire(&altered) + &terminal(json!([]))).as_bytes(),
                None
            )
            .is_err());
        }
    }
    #[test]
    fn ledger_done_texts_and_message_status_cannot_conflict() {
        let events = message_events("complete", 0);
        for which in [3, 4, 5] {
            let mut altered = events.clone();
            match which {
                3 => altered[which]["text"] = json!("different"),
                4 => altered[which]["part"]["text"] = json!("different"),
                _ => altered[which]["item"]["content"][0]["text"] = json!("different"),
            }
            assert_eq!(
                classified(
                    (created() + &events_wire(&altered) + &terminal(json!([]))).as_bytes(),
                    None
                ),
                Err(Stop::MalformedOutput)
            );
        }
        for status in [Value::Null, json!("in_progress"), json!("incomplete")] {
            let mut altered = events.clone();
            altered[5]["item"]["status"] = status;
            assert_eq!(
                classified(
                    (created() + &events_wire(&altered) + &terminal(json!([]))).as_bytes(),
                    None
                ),
                Err(Stop::IncompleteMessage)
            );
        }
    }
    #[test]
    fn ledger_bounds_accumulated_unicode_text_before_completion() {
        let text = "あ".repeat(24_000);
        assert_eq!(
            classified(
                (created() + &events_wire(&message_events(&text, 0)) + &terminal(json!([])))
                    .as_bytes(),
                None
            ),
            Ok(text)
        );
        assert_eq!(
            classified(
                (created()
                    + &events_wire(&message_events(&"あ".repeat(24_001), 0))
                    + &terminal(json!([])))
                    .as_bytes(),
                None
            ),
            Err(Stop::TextLimit)
        );
    }
    #[test]
    fn ledger_reasoning_is_inert_but_must_finish_without_status_conflicts() {
        let added = json!({"type":"response.output_item.added","output_index":0,"item":{"id":"rs_synthetic","type":"reasoning"}});
        let done = json!({"type":"response.output_item.done","output_index":0,"item":{"id":"rs_synthetic","type":"reasoning"}});
        let output = json!([{"id":"rs_synthetic","type":"reasoning"}]);
        let message_stream = events_wire(&message_events("complete", 1));
        assert_eq!(
            classified(
                (created()
                    + &event(added.clone())
                    + &event(done.clone())
                    + &message_stream
                    + &terminal(output.clone()))
                    .as_bytes(),
                None
            ),
            Ok("complete".into())
        );
        assert!(classified(
            (created() + &event(added.clone()) + &message_stream + &terminal(output.clone()))
                .as_bytes(),
            None
        )
        .is_err());
        for status in [
            json!("in_progress"),
            json!("incomplete"),
            Value::Null,
            json!("unknown"),
        ] {
            let mut wrong_done = done.clone();
            wrong_done["item"]["status"] = status.clone();
            assert_eq!(
                classified(
                    (created()
                        + &event(added.clone())
                        + &event(wrong_done)
                        + &message_stream
                        + &terminal(output.clone()))
                        .as_bytes(),
                    None
                ),
                Err(Stop::Partial)
            );
            let mut wrong_final = output.clone();
            wrong_final[0]["status"] = status;
            assert_eq!(
                classified(
                    (created()
                        + &event(added.clone())
                        + &event(done.clone())
                        + &message_stream
                        + &terminal(wrong_final))
                        .as_bytes(),
                    None
                ),
                Err(Stop::Partial)
            );
        }
    }
    #[test]
    fn final_only_reasoning_allows_omitted_status_but_rejects_explicit_noncompletion() {
        for status in [
            None,
            Some(json!("completed")),
            Some(json!("in_progress")),
            Some(json!("incomplete")),
            Some(Value::Null),
            Some(json!("unknown")),
        ] {
            let mut reasoning = json!({"id":"rs_synthetic","type":"reasoning","summary":[{"type":"summary_text","text":"never exposed"}]});
            if let Some(value) = status.clone() {
                reasoning["status"] = value;
            }
            let result = classified(
                (created() + &terminal(json!([reasoning, message("final only")]))).as_bytes(),
                None,
            );
            if status.is_none() || status == Some(json!("completed")) {
                assert_eq!(result, Ok("final only".into()));
            } else {
                assert_eq!(result, Err(Stop::Partial));
            }
        }
    }
    #[test]
    fn ledger_sequence_numbers_reject_duplicates_and_backwards_events() {
        let mut events = message_events("complete", 0);
        for (i, value) in events.iter_mut().enumerate() {
            value["sequence_number"] = json!(10 + i);
        }
        assert_eq!(
            classified(
                (created() + &events_wire(&events) + &terminal(json!([]))).as_bytes(),
                None
            ),
            Ok("complete".into())
        );
        for number in [json!(10), json!(0), json!(-1), json!(1.5), Value::Null] {
            let mut altered = events.clone();
            altered[2]["sequence_number"] = number;
            assert_eq!(
                classified(
                    (created() + &events_wire(&altered) + &terminal(json!([]))).as_bytes(),
                    None
                ),
                Err(Stop::EventOrder)
            );
        }
        events[2].as_object_mut().unwrap().remove("sequence_number");
        assert_eq!(
            classified(
                (created() + &events_wire(&events) + &terminal(json!([]))).as_bytes(),
                None
            ),
            Ok("complete".into())
        );
    }
    #[test]
    fn ledger_requires_same_terminal_response_and_rejects_any_failure() {
        let stream = created() + &events_wire(&message_events("complete", 0));
        assert_eq!(classified(stream.as_bytes(), None), Err(Stop::Truncated));
        assert_eq!(
            classified(
                (stream.clone() + &terminal(json!([])).replace("resp_synthetic", "resp_foreign"))
                    .as_bytes(),
                None
            ),
            Err(Stop::ResponseIdentity)
        );
        for bad in [
            json!({"type":"response.failed"}),
            json!({"type":"response.incomplete"}),
            json!({"type":"response.refusal.done"}),
            json!({"type":"unknown"}),
            json!({"type":"response.output_item.added","output_index":1,"item":{"type":"function_call"}}),
        ] {
            assert!(classified(
                (stream.clone() + &event(bad) + &terminal(json!([]))).as_bytes(),
                None
            )
            .is_err());
        }
    }
    #[test]
    fn diagnostic_codes_are_closed_and_never_echo_server_data() {
        assert_eq!(Stop::from_code(Stop::Json.code()), Some(Stop::Json));
        assert_eq!(Stop::from_code("raw server text <img>"), None);
        assert_eq!(
            read_stream_classified(b"data: {broken}\n\n".as_slice()),
            Err(Stop::Json)
        );
        assert_eq!(
            read_stream_classified(
                (created() + &event(json!({"type":"unrecognized-event","secret":"never echoed"})))
                    .as_bytes()
            ),
            Err(Stop::UnknownEvent)
        );
        for (code, stop) in [
            ("subscription_sharing_usage_limit_exceeded", Stop::HttpLimit),
            (
                "subscription_sharing_usage_unavailable",
                Stop::UsageUnavailable,
            ),
        ] {
            assert_eq!(
                read_stream_classified(
                    (created()
                        + &event(json!({"type":"response.failed","response":{"error":{"code":code,"message":"never echoed"}}})))
                        .as_bytes()
                ),
                Err(stop)
            );
            assert_eq!(Stop::from_code(stop.code()), Some(stop));
        }
    }
    #[test]
    fn mislabelled_sse_preserves_known_failure_codes_after_created() {
        for (code, expected) in [
            ("subscription_sharing_usage_limit_exceeded", Stop::HttpLimit),
            (
                "subscription_sharing_usage_unavailable",
                Stop::UsageUnavailable,
            ),
        ] {
            for failure in [
                json!({"type":"response.failed","response":{"error":{"code":code,"message":"never echoed"}}}),
                json!({"type":"error","code":code,"message":"never echoed"}),
            ] {
                let wire = created() + &event(failure) + &completed("never accepted");
                for mime in [
                    None,
                    Some(""),
                    Some("application/json"),
                    Some("text/html"),
                    Some("application/octet-stream"),
                ] {
                    assert_eq!(classified(wire.as_bytes(), mime), Err(expected));
                }
            }
        }
    }
    #[test]
    fn known_admission_failures_before_created_stop_with_fixed_codes() {
        for (code, expected) in [
            ("subscription_sharing_usage_limit_exceeded", Stop::HttpLimit),
            (
                "subscription_sharing_usage_unavailable",
                Stop::UsageUnavailable,
            ),
        ] {
            for failure in [
                json!({"type":"error","code":code,"message":"never echoed"}),
                json!({"type":"response.failed","response":{"error":{"code":code,"message":"never echoed"}}}),
            ] {
                let wire = event(failure) + &created() + &completed("never accepted");
                for mime in [None, Some("text/event-stream"), Some("application/json")] {
                    assert_eq!(classified(wire.as_bytes(), mime), Err(expected));
                }
            }
        }
    }
    #[test]
    fn pre_created_unknown_failures_and_output_keep_event_order_checks() {
        let known = "subscription_sharing_usage_limit_exceeded";
        for early in [
            json!({"type":"error","code":"unknown-code","message":"never echoed"}),
            json!({"type":"response.failed","response":{"error":{"code":"unknown-code"}}}),
            json!({"type":"unknown","code":known}),
            json!({"type":"response.output_text.delta","code":known,"delta":"never accepted"}),
            json!({"type":"response.completed","code":known,"response":{"id":"resp_synthetic","status":"completed","output":[]}}),
        ] {
            let wire = event(early) + &created() + &completed("never accepted");
            assert_eq!(
                read_stream_classified(wire.as_bytes()),
                Err(Stop::EventOrder)
            );
            assert_eq!(classified(wire.as_bytes(), None), Err(Stop::EventOrder));
        }
        let mismatched = format!(
            "event: response.completed\ndata: {}\n\n{}{}",
            json!({"type":"error","code":known}),
            created(),
            completed("never accepted"),
        );
        assert_eq!(
            read_stream_classified(mismatched.as_bytes()),
            Err(Stop::EventOrder)
        );
        assert_eq!(
            classified(mismatched.as_bytes(), None),
            Err(Stop::EventOrder)
        );
    }
    #[test]
    fn normalized_sse_returns_at_completion_without_waiting_for_eof() {
        struct FailAfterData(std::io::Cursor<Vec<u8>>);
        impl Read for FailAfterData {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                if self.0.position() == self.0.get_ref().len() as u64 {
                    return Err(std::io::Error::other("synthetic read failure never echoed"));
                }
                self.0.read(output)
            }
        }
        let wire = created() + &completed("completed answer");
        assert_eq!(
            read_response_classified(
                FailAfterData(std::io::Cursor::new(wire.into_bytes())),
                ResponseType::from_header(Some("  TEXT/EVENT-STREAM ; charset=utf-8")),
            )
            .unwrap(),
            "completed answer"
        );
        for response_type in [ResponseType::Stream, ResponseType::Missing] {
            assert_eq!(
                read_response_classified(
                    FailAfterData(std::io::Cursor::new(created().into_bytes())),
                    response_type,
                ),
                Err(Stop::Network)
            );
        }
    }
    #[test]
    fn missing_or_mislabelled_mime_accepts_only_strict_completed_sse() {
        let wire = created() + &completed("<img src=https://invalid.test>literal text");
        for mime in [
            None,
            Some("application/json"),
            Some("text/html"),
            Some("application/octet-stream"),
        ] {
            assert_eq!(
                classified(wire.as_bytes(), mime).unwrap(),
                "<img src=https://invalid.test>literal text"
            );
        }
        for (wire, expected) in [
            (completed("missing created"), Stop::EventOrder),
            (created(), Stop::Truncated),
            (
                created() + &completed("foreign id").replace("resp_synthetic", "resp_other"),
                Stop::ResponseIdentity,
            ),
            (
                created() + &event(json!({"type":"response.refusal.delta","delta":"refusal"})),
                Stop::Refusal,
            ),
            (
                created()
                    + &event(
                        json!({"type":"response.output_item.added","item":{"type":"function_call"}}),
                    ),
                Stop::UnsupportedItem,
            ),
            (
                created() + &event(json!({"type":"unknown"})) + &completed("never"),
                Stop::UnknownEvent,
            ),
        ] {
            for mime in [None, Some("application/json"), Some("text/html")] {
                assert_eq!(classified(wire.as_bytes(), mime), Err(expected));
            }
        }
    }
    #[test]
    fn plain_json_completed_answers_and_bad_bodies_stop_with_fixed_shape_classes() {
        let plain = serde_json::to_vec(&json!({"id":"resp_synthetic","status":"completed","output":[
            {"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"never returned"}]}
        ]})).unwrap();
        for (mime, stop) in [
            (
                Some("APPLICATION/JSON; charset=utf-8"),
                Stop::ContentTypeJson,
            ),
            (Some("application/problem+json"), Stop::ContentTypeJson),
            (
                Some("TEXT/HTML; private=never-echoed"),
                Stop::ContentTypeHtml,
            ),
            (None, Stop::ContentTypeMissing),
            (Some(""), Stop::ContentTypeMissing),
            (
                Some("unrecognized/private-server-type"),
                Stop::ContentTypeOther,
            ),
        ] {
            for (bytes, expected) in [
                (plain.as_slice(), Stop::ContentTypeJson),
                (b"{bad JSON}".as_slice(), Stop::ContentTypeJson),
                (b"data: {bad JSON}\n\n".as_slice(), Stop::Json),
                (
                    b"<html>never echoed</html>".as_slice(),
                    Stop::ContentTypeHtml,
                ),
                (b"unrecognized shape".as_slice(), stop),
            ] {
                assert_eq!(classified(bytes, mime), Err(expected));
            }
            assert_eq!(Stop::from_code(stop.code()), Some(stop));
        }
        assert_eq!(
            classified(&plain, Some("text/event-stream")),
            Err(Stop::EventOrder)
        );
    }
    #[test]
    fn non_sse_json_classifies_only_recognized_machine_error_codes() {
        for (code, expected) in [
            ("subscription_sharing_usage_limit_exceeded", Stop::HttpLimit),
            (
                "subscription_sharing_usage_unavailable",
                Stop::UsageUnavailable,
            ),
            ("unknown-server-code-never-echoed", Stop::ContentTypeJson),
        ] {
            let bytes = serde_json::to_vec(
                &json!({"error":{"code":code,"message":"server message never echoed"}}),
            )
            .unwrap();
            assert_eq!(classified(&bytes, Some("application/json")), Err(expected));
        }
        let embedded =
            serde_json::to_vec(&json!({"text":"subscription_sharing_usage_limit_exceeded"}))
                .unwrap();
        assert_eq!(
            classified(&embedded, Some("application/json")),
            Err(Stop::ContentTypeJson)
        );
    }
    #[test]
    fn non_sse_buffer_and_completed_answer_caps_remain_authoritative() {
        let exact = vec![b'x'; WIRE_LIMIT as usize];
        assert_eq!(classified(&exact, None), Err(Stop::ContentTypeMissing));
        let overflow = vec![b'x'; WIRE_LIMIT as usize + 1];
        assert_eq!(
            classified(&overflow, Some("application/json")),
            Err(Stop::ByteLimit)
        );
        assert_eq!(
            classified(
                (created() + &completed(&"あ".repeat(24_001))).as_bytes(),
                None
            ),
            Err(Stop::TextLimit)
        );
        let known_error_prefix = serde_json::to_vec(
            &json!({"error":{"code":"subscription_sharing_usage_limit_exceeded"}}),
        )
        .unwrap();
        let mut oversized_error = known_error_prefix;
        oversized_error.resize(WIRE_LIMIT as usize + 1, b' ');
        assert_eq!(
            classified(&oversized_error, Some("application/json")),
            Err(Stop::ByteLimit)
        );
    }
    #[test]
    fn no_content_statuses_and_definitive_errors_never_read_a_body() {
        struct Unread;
        impl Read for Unread {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                panic!("this HTTP status must not read a body or start the SSE parser")
            }
        }
        for (status, expected) in [
            (204, Stop::HttpNoContent),
            (205, Stop::HttpNoContent),
            (401, Stop::HttpDenied),
            (403, Stop::HttpDenied),
            (429, Stop::HttpLimit),
        ] {
            for response_type in [
                ResponseType::Stream,
                ResponseType::Json,
                ResponseType::Missing,
            ] {
                assert_eq!(
                    read_http_response_classified(status, Unread, response_type),
                    Err(expected)
                );
            }
        }
        assert_eq!(
            Stop::from_code(Stop::HttpNoContent.code()),
            Some(Stop::HttpNoContent)
        );
    }
    #[test]
    fn successful_non_sse_empty_bodies_are_distinct_from_nonempty_malformed_bodies() {
        for status in [200, 201, 202, 299] {
            for (response_type, expected) in [
                (ResponseType::Json, Stop::ContentTypeJson),
                (ResponseType::Html, Stop::ContentTypeHtml),
                (ResponseType::Missing, Stop::ContentTypeMissing),
                (ResponseType::Other, Stop::ContentTypeOther),
            ] {
                assert_eq!(
                    read_http_response_classified(status, b"".as_slice(), response_type),
                    Err(Stop::EmptyBody)
                );
                for bytes in [b" \r\n\t".as_slice(), b"unexpected body"] {
                    assert_eq!(
                        read_http_response_classified(status, bytes, response_type),
                        Err(expected)
                    );
                }
                assert_eq!(
                    read_http_response_classified(status, b"{bad JSON}".as_slice(), response_type),
                    Err(Stop::ContentTypeJson)
                );
            }
        }
        assert_eq!(
            read_http_response_classified(200, b"".as_slice(), ResponseType::Stream),
            Err(Stop::Truncated)
        );
        assert_eq!(
            Stop::from_code(Stop::EmptyBody.code()),
            Some(Stop::EmptyBody)
        );
    }
    #[test]
    fn sse_bom_and_all_line_endings_work_with_fragmented_unicode() {
        struct OneByte(std::io::Cursor<Vec<u8>>);
        impl Read for OneByte {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                let count = output.len().min(1);
                self.0.read(&mut output[..count])
            }
        }
        for newline in ["\n", "\r", "\r\n"] {
            for bom in ["", "\u{feff}"] {
                let wire = format!(
                    "{bom}{}",
                    (created() + &completed("架空の回答だけ")).replace('\n', newline)
                );
                assert_eq!(
                    read_stream_classified(OneByte(std::io::Cursor::new(wire.as_bytes().to_vec()))),
                    Ok("架空の回答だけ".into())
                );
                for mime in [None, Some("application/json"), Some("text/event-stream")] {
                    assert_eq!(
                        classified(wire.as_bytes(), mime),
                        Ok("架空の回答だけ".into())
                    );
                }
            }
        }
    }
    #[test]
    fn bom_normalization_does_not_hide_extra_bytes_or_incomplete_events() {
        for newline in ["\n", "\r", "\r\n"] {
            for wire in [
                "\u{feff}\u{feff}".to_string() + &created() + &completed("never"),
                created() + "\u{feff}" + &completed("never"),
                " \u{feff}".to_string() + &created() + &completed("never"),
                (created() + &completed("never"))
                    .trim_end_matches('\n')
                    .to_string(),
            ] {
                for mime in [None, Some("text/event-stream")] {
                    assert!(classified(wire.replace('\n', newline).as_bytes(), mime).is_err());
                }
            }
        }
    }
    #[test]
    fn sse_failure_reason_survives_missing_mime_and_transport_metadata() {
        for prefix in ["", "\u{feff}", "\r\n", ": keepalive\r\nid: synthetic\r\n"] {
            for (wire, expected) in [
                ("data: {invalid}\n\n".into(), Stop::Json),
                (created() + "data: {invalid}\n\n", Stop::Json),
                (
                    created() + &event(json!({"type":"response.failed"})),
                    Stop::Partial,
                ),
                (
                    created() + &event(json!({"type":"unknown","text":"never echoed"})),
                    Stop::UnknownEvent,
                ),
                (
                    "data: {\"type\":\"response.created\"}\n\n".into(),
                    Stop::ResponseIdentity,
                ),
            ] {
                let wire = format!("{prefix}{wire}");
                for mime in [None, Some("text/html"), Some("application/json")] {
                    assert_eq!(classified(wire.as_bytes(), mime), Err(expected));
                }
            }
        }
    }
    #[test]
    fn non_sse_shape_diagnostics_never_promote_json_or_html_to_answers() {
        for bytes in [
            b"\xef\xbb\xbf {\"detail\":\"private text never echoed\"}".as_slice(),
            b"[\"data: fake event text\"]",
            b"{\"status\":\"completed\",\"output\":[\"never accepted\"]}",
        ] {
            assert_eq!(classified(bytes, None), Err(Stop::ContentTypeJson));
        }
        assert_eq!(
            classified(
                b" \r\n<!DOCTYPE HTML><html><img src=https://invalid.test></html>",
                None
            ),
            Err(Stop::ContentTypeHtml)
        );
        assert_eq!(
            classified(
                b"\xef\xbb\xbf{\"error\":{\"code\":\"subscription_sharing_usage_unavailable\"}}",
                None
            ),
            Err(Stop::UsageUnavailable)
        );
        assert_eq!(
            classified(b"opaque text data: ignored", None),
            Err(Stop::ContentTypeMissing)
        );
    }
    #[test]
    fn raw_wire_byte_limit_includes_bom_crlf_and_empty_lines() {
        let ending = (created() + &completed("last answer")).replace('\n', "\r\n");
        let padding = WIRE_LIMIT as usize - ending.len() - 3;
        // Leading blank LF lines plus the BOM consume the original byte budget.
        let wire = format!("\u{feff}{}{ending}", "\n".repeat(padding));
        assert_eq!(wire.len(), WIRE_LIMIT as usize);
        assert_eq!(classified(wire.as_bytes(), None), Ok("last answer".into()));
        let overflow = format!("\u{feff}\n{}{ending}", "\n".repeat(padding));
        assert_eq!(classified(overflow.as_bytes(), None), Err(Stop::ByteLimit));
        assert_eq!(
            read_stream_classified(
                format!("\u{feff}{}", "\r\n".repeat(WIRE_LIMIT as usize / 2)).as_bytes()
            ),
            Err(Stop::ByteLimit)
        );
    }
    #[test]
    fn cr_terminated_completed_stream_does_not_wait_for_http_eof() {
        struct FailAfterData(std::io::Cursor<Vec<u8>>);
        impl Read for FailAfterData {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                if self.0.position() == self.0.get_ref().len() as u64 {
                    return Err(std::io::Error::other("synthetic trailing network error"));
                }
                self.0.read(output)
            }
        }
        let wire = (created() + &completed("only finished answer")).replace('\n', "\r");
        assert_eq!(
            read_stream_classified(FailAfterData(std::io::Cursor::new(wire.into_bytes()))),
            Ok("only finished answer".into())
        );
    }
    #[test]
    fn http_error_bodies_are_bounded_and_only_known_codes_classify_failure() {
        let known = serde_json::to_vec(&json!({"error":{
            "code":"subscription_sharing_usage_unavailable",
            "message":"server detail never echoed"
        }}))
        .unwrap();
        for response_type in [
            ResponseType::Stream,
            ResponseType::Json,
            ResponseType::Missing,
        ] {
            assert_eq!(
                read_http_response_classified(503, known.as_slice(), response_type),
                Err(Stop::UsageUnavailable)
            );
        }
        let unsupported = serde_json::to_vec(&json!({"error":{
            "code":"unknown-server-code-never-echoed",
            "message":"server detail never echoed"
        }}))
        .unwrap();
        let plain_answer = serde_json::to_vec(&json!({"status":"completed","output":[
            {"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"never accepted"}]}
        ]})).unwrap();
        let completed_stream = created() + &completed("never accepted");
        for bytes in [
            b"".as_slice(),
            b"{\"detail\":\"never echoed\"}",
            unsupported.as_slice(),
            plain_answer.as_slice(),
            completed_stream.as_bytes(),
        ] {
            assert_eq!(
                read_http_response_classified(503, bytes, ResponseType::Missing),
                Err(Stop::HttpOther)
            );
        }
        let exact = vec![b'x'; WIRE_LIMIT as usize];
        assert_eq!(
            read_http_response_classified(503, exact.as_slice(), ResponseType::Json),
            Err(Stop::HttpOther)
        );
        let mut oversized = known;
        oversized.resize(WIRE_LIMIT as usize + 1, b' ');
        assert_eq!(
            read_http_response_classified(503, oversized.as_slice(), ResponseType::Json),
            Err(Stop::ByteLimit)
        );
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other(
                    "synthetic server failure never echoed",
                ))
            }
        }
        assert_eq!(
            read_http_response_classified(503, Broken, ResponseType::Json),
            Err(Stop::Network)
        );
    }
}
