use regex::Regex;
use serde_json::{Map, Value};

pub(super) fn parse_tool_call_args(name: &str, raw: &str) -> Option<Value> {
    serde_json::from_str::<Value>(raw)
        .ok()
        .or_else(|| (name == "restricted_exec").then(|| repair_restricted_exec_args(raw))?)
}

fn repair_restricted_exec_args(raw: &str) -> Option<Value> {
    let regex = Regex::new(r#""(command[1-8])"\s*:"#).ok()?;
    let matches = regex
        .captures_iter(raw)
        .filter_map(|captures| {
            let key = captures.get(1)?.as_str().to_string();
            let full_match = captures.get(0)?;
            Some((key, full_match.start(), full_match.end()))
        })
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return None;
    }

    let mut repaired = Map::new();
    for (idx, (key, _, value_start)) in matches.iter().enumerate() {
        let value_end = matches
            .get(idx + 1)
            .map(|(_, key_start, _)| *key_start)
            .unwrap_or(raw.len());
        if let Some(value) = repair_json_object_fragment(&raw[*value_start..value_end]) {
            repaired.insert(key.clone(), value);
        }
    }

    (!repaired.is_empty()).then_some(Value::Object(repaired))
}

fn repair_json_object_fragment(fragment: &str) -> Option<Value> {
    let mut candidate = fragment
        .trim()
        .trim_end_matches(|ch: char| ch.is_whitespace() || ch == ',')
        .to_string();
    if candidate.is_empty() {
        return None;
    }

    for _ in 0..=4 {
        if let Ok(value) = serde_json::from_str::<Value>(&candidate) {
            return value.as_object().is_some().then_some(value);
        }
        if let Some(completed) = complete_json_fragment(&candidate)
            && let Ok(value) = serde_json::from_str::<Value>(&completed)
        {
            return value.as_object().is_some().then_some(value);
        }
        if !remove_last_non_ws_char(&mut candidate, '}') {
            break;
        }
    }

    None
}

fn complete_json_fragment(fragment: &str) -> Option<String> {
    let mut stack = Vec::new();
    let mut in_string = false;
    let mut escaped = false;

    for ch in fragment.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => stack.push('}'),
            '[' => stack.push(']'),
            '}' | ']' if stack.pop() != Some(ch) => return None,
            '}' | ']' => {}
            _ => {}
        }
    }
    if in_string {
        return None;
    }

    let mut completed = fragment.to_string();
    for ch in stack.iter().rev() {
        completed.push(*ch);
    }
    Some(completed)
}

fn remove_last_non_ws_char(value: &mut String, expected: char) -> bool {
    let Some((idx, ch)) = value
        .char_indices()
        .rev()
        .find(|(_, ch)| !ch.is_whitespace())
    else {
        return false;
    };
    if ch != expected {
        return false;
    }
    value.truncate(idx);
    true
}
