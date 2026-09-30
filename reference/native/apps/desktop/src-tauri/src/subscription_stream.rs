//! Per-request in-memory stream ledger. No partial text leaves this module.
use super::Stop;
use serde_json::Value;
use std::collections::BTreeMap;

const INDEX_LIMIT: usize = 1024;
const TEXT_LIMIT: usize = 24_000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Message,
    Reasoning,
}
struct Part {
    text: Option<String>,
    text_done: bool,
    done: bool,
}
struct Item {
    id: String,
    kind: Kind,
    parts: BTreeMap<usize, Part>,
    done: bool,
}
#[derive(Default)]
pub(super) struct Ledger {
    items: BTreeMap<usize, Item>,
    text_chars: usize,
    last_sequence: Option<u64>,
}

fn index(v: &Value) -> Result<usize, Stop> {
    let n = v.as_u64().ok_or(Stop::MalformedOutput)?;
    if n >= INDEX_LIMIT as u64 {
        return Err(Stop::ByteLimit);
    }
    Ok(n as usize)
}
fn item_id(v: &Value) -> Result<&str, Stop> {
    let id = v.as_str().ok_or(Stop::ResponseIdentity)?;
    if id.is_empty()
        || id.len() > 256
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
    {
        return Err(Stop::ResponseIdentity);
    }
    Ok(id)
}
fn kind(item: &Value) -> Result<Kind, Stop> {
    match item["type"].as_str() {
        Some("message") => {
            if item["role"] != "assistant" {
                return Err(Stop::UnexpectedRole);
            }
            Ok(Kind::Message)
        }
        Some("reasoning") => Ok(Kind::Reasoning),
        Some(_) => Err(Stop::UnsupportedItem),
        None => Err(Stop::MalformedOutput),
    }
}
fn part_kind(part: &Value) -> Result<Kind, Stop> {
    match part["type"].as_str() {
        Some("output_text") => Ok(Kind::Message),
        Some("reasoning_text") => Ok(Kind::Reasoning),
        Some("refusal") => Err(Stop::Refusal),
        Some(_) => Err(Stop::UnsupportedPart),
        None => Err(Stop::MalformedOutput),
    }
}
pub(super) fn reasoning_status(source: &Value, completed: bool) -> Result<(), Stop> {
    if let Some(status) = source.get("status") {
        match status.as_str() {
            Some("completed") => {}
            Some("in_progress") if !completed => {}
            _ => return Err(Stop::Partial),
        }
    }
    Ok(())
}

impl Ledger {
    pub(super) fn sequence(&mut self, event: &Value) -> Result<(), Stop> {
        // Some historical fixtures omit this field. If supplied, it cannot
        // repeat or go backwards, regardless of which event carries it.
        if let Some(value) = event.get("sequence_number") {
            let n = value.as_u64().ok_or(Stop::EventOrder)?;
            if self.last_sequence.is_some_and(|last| n <= last) {
                return Err(Stop::EventOrder);
            }
            self.last_sequence = Some(n);
        }
        Ok(())
    }

    pub(super) fn added(&mut self, event: &Value) -> Result<(), Stop> {
        let source = &event["item"];
        let item_kind = kind(source)?;
        let position = index(&event["output_index"])?;
        if position != self.items.len() {
            return Err(Stop::EventOrder);
        }
        let id = item_id(&source["id"])?;
        if self.items.values().any(|item| item.id == id) {
            return Err(Stop::ResponseIdentity);
        }
        if item_kind == Kind::Message {
            if !matches!(source["status"].as_str(), Some("in_progress" | "completed")) {
                return Err(Stop::IncompleteMessage);
            }
            if !source["content"]
                .as_array()
                .ok_or(Stop::MalformedOutput)?
                .is_empty()
            {
                // Content must be introduced by its corresponding part events.
                return Err(Stop::EventOrder);
            }
        } else {
            reasoning_status(source, false)?;
        }
        self.items.insert(
            position,
            Item {
                id: id.into(),
                kind: item_kind,
                parts: BTreeMap::new(),
                done: false,
            },
        );
        Ok(())
    }

    fn active_item(&mut self, event: &Value) -> Result<&mut Item, Stop> {
        let position = index(&event["output_index"])?;
        let id = item_id(&event["item_id"])?;
        let item = self.items.get_mut(&position).ok_or(Stop::EventOrder)?;
        if item.id != id {
            return Err(Stop::ResponseIdentity);
        }
        if item.done {
            return Err(Stop::EventOrder);
        }
        Ok(item)
    }

    pub(super) fn part_added(&mut self, event: &Value) -> Result<(), Stop> {
        let part_type = part_kind(&event["part"])?;
        let position = index(&event["content_index"])?;
        let text = if part_type == Kind::Message {
            Some(
                event["part"]["text"]
                    .as_str()
                    .ok_or(Stop::MalformedOutput)?
                    .to_owned(),
            )
        } else {
            None
        };
        let chars = text.as_ref().map_or(0, |text| text.chars().count());
        if self.text_chars + chars > TEXT_LIMIT {
            return Err(Stop::TextLimit);
        }
        let item = self.active_item(event)?;
        if item.kind != part_type {
            return Err(Stop::UnsupportedPart);
        }
        if position != item.parts.len() {
            return Err(Stop::EventOrder);
        }
        item.parts.insert(
            position,
            Part {
                text,
                text_done: false,
                done: false,
            },
        );
        self.text_chars += chars;
        Ok(())
    }

    pub(super) fn delta(&mut self, event: &Value) -> Result<(), Stop> {
        let delta = event["delta"].as_str().ok_or(Stop::MalformedOutput)?;
        let chars = delta.chars().count();
        if self.text_chars + chars > TEXT_LIMIT {
            return Err(Stop::TextLimit);
        }
        let position = index(&event["content_index"])?;
        let item = self.active_item(event)?;
        if item.kind != Kind::Message {
            return Err(Stop::UnsupportedPart);
        }
        let part = item.parts.get_mut(&position).ok_or(Stop::EventOrder)?;
        if part.text_done || part.done {
            return Err(Stop::EventOrder);
        }
        part.text
            .as_mut()
            .ok_or(Stop::UnsupportedPart)?
            .push_str(delta);
        self.text_chars += chars;
        Ok(())
    }

    pub(super) fn text_done(&mut self, event: &Value) -> Result<(), Stop> {
        let text = event["text"].as_str().ok_or(Stop::MalformedOutput)?;
        let position = index(&event["content_index"])?;
        let item = self.active_item(event)?;
        if item.kind != Kind::Message {
            return Err(Stop::UnsupportedPart);
        }
        let part = item.parts.get_mut(&position).ok_or(Stop::EventOrder)?;
        if part.text_done || part.done {
            return Err(Stop::EventOrder);
        }
        if part.text.as_deref() != Some(text) {
            return Err(Stop::MalformedOutput);
        }
        part.text_done = true;
        Ok(())
    }

    pub(super) fn part_done(&mut self, event: &Value) -> Result<(), Stop> {
        let part_type = part_kind(&event["part"])?;
        let position = index(&event["content_index"])?;
        let item = self.active_item(event)?;
        if item.kind != part_type {
            return Err(Stop::UnsupportedPart);
        }
        let part = item.parts.get_mut(&position).ok_or(Stop::EventOrder)?;
        if part.done {
            return Err(Stop::EventOrder);
        }
        if part_type == Kind::Message {
            if !part.text_done {
                return Err(Stop::EventOrder);
            }
            let text = event["part"]["text"]
                .as_str()
                .ok_or(Stop::MalformedOutput)?;
            if part.text.as_deref() != Some(text) {
                return Err(Stop::MalformedOutput);
            }
        }
        part.done = true;
        Ok(())
    }

    pub(super) fn annotation(&mut self, event: &Value) -> Result<(), Stop> {
        let position = index(&event["content_index"])?;
        let item = self.active_item(event)?;
        if item.kind != Kind::Message {
            return Err(Stop::UnsupportedPart);
        }
        item.parts.get(&position).ok_or(Stop::EventOrder)?;
        Ok(())
    }

    pub(super) fn reasoning(&mut self, event: &Value) -> Result<(), Stop> {
        let item = self.active_item(event)?;
        if item.kind != Kind::Reasoning {
            return Err(Stop::UnsupportedPart);
        }
        // Reasoning content remains unretained. The item identity is still bound.
        Ok(())
    }

    pub(super) fn item_done(&mut self, event: &Value) -> Result<(), Stop> {
        let source = &event["item"];
        let item_kind = kind(source)?;
        let position = index(&event["output_index"])?;
        let id = item_id(&source["id"])?;
        let item = self.items.get_mut(&position).ok_or(Stop::EventOrder)?;
        if item.id != id || item.kind != item_kind {
            return Err(Stop::ResponseIdentity);
        }
        if item.done || item.parts.values().any(|part| !part.done) {
            return Err(Stop::EventOrder);
        }
        if item.kind == Kind::Message {
            match_message(item, source)?;
        } else {
            reasoning_status(source, true)?;
        }
        item.done = true;
        Ok(())
    }

    pub(super) fn finish(
        &self,
        response: &Value,
        terminal: Result<String, Stop>,
    ) -> Result<String, Stop> {
        // completed_text already checks response status, errors and every final
        // output type. Only absence of text may be satisfied by the stream path.
        if let Err(stop) = terminal.as_ref() {
            if *stop != Stop::EmptyAnswer {
                return Err(*stop);
            }
        }
        if self.items.is_empty() {
            return terminal;
        }
        if self.items.values().any(|item| !item.done) {
            return Err(Stop::EventOrder);
        }
        let output = response["output"].as_array().ok_or(Stop::MalformedOutput)?;
        let has_final_message = output.iter().any(|item| item["type"] == "message");
        if has_final_message {
            if output.len() != self.items.len() {
                return Err(Stop::ResponseIdentity);
            }
            for (position, source) in output.iter().enumerate() {
                self.match_final_item(position, source)?;
            }
        } else {
            // A message-free lifecycle snapshot is a completion gate. Any
            // reasoning items it does include must still identify tracked items.
            let mut seen = std::collections::BTreeSet::new();
            for (position, source) in output.iter().enumerate() {
                let id = item_id(&source["id"])?;
                if !seen.insert(id) {
                    return Err(Stop::ResponseIdentity);
                }
                self.match_final_item(position, source)?;
            }
        }
        let mut text = String::new();
        for item in self
            .items
            .values()
            .filter(|item| item.kind == Kind::Message)
        {
            for part in item.parts.values() {
                text.push_str(part.text.as_deref().ok_or(Stop::MalformedOutput)?);
            }
        }
        if text.is_empty() {
            return Err(Stop::EmptyAnswer);
        }
        if text.chars().count() > TEXT_LIMIT {
            return Err(Stop::TextLimit);
        }
        if has_final_message && terminal.as_ref().ok() != Some(&text) {
            return Err(Stop::MalformedOutput);
        }
        Ok(text)
    }

    fn match_final_item(&self, position: usize, source: &Value) -> Result<(), Stop> {
        let item = self.items.get(&position).ok_or(Stop::ResponseIdentity)?;
        if item.id != item_id(&source["id"])? || item.kind != kind(source)? {
            return Err(Stop::ResponseIdentity);
        }
        if item.kind == Kind::Message {
            match_message(item, source)?;
        } else {
            reasoning_status(source, true)?;
        }
        Ok(())
    }
}

fn match_message(item: &Item, source: &Value) -> Result<(), Stop> {
    if source["role"] != "assistant" {
        return Err(Stop::UnexpectedRole);
    }
    if source["status"] != "completed" {
        return Err(Stop::IncompleteMessage);
    }
    let content = source["content"].as_array().ok_or(Stop::MalformedOutput)?;
    if content.len() != item.parts.len() {
        return Err(Stop::MalformedOutput);
    }
    for (position, source_part) in content.iter().enumerate() {
        if part_kind(source_part)? != Kind::Message {
            return Err(Stop::UnsupportedPart);
        }
        let part = item.parts.get(&position).ok_or(Stop::EventOrder)?;
        if !part.text_done || !part.done {
            return Err(Stop::EventOrder);
        }
        let text = source_part["text"].as_str().ok_or(Stop::MalformedOutput)?;
        if part.text.as_deref() != Some(text) {
            return Err(Stop::MalformedOutput);
        }
    }
    Ok(())
}
