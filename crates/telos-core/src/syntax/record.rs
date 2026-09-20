//! Native `.tel` records for plans and durable receipts.
//!
//! `plan "PLN-..." { key value ... }` uses quoted strings, numbers, booleans,
//! null, nested blocks and bracketed lists. Keys are ASCII identifiers. This
//! grammar intentionally rejects duplicate fields and trailing documents.

use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};

use crate::error::{ErrorCode, TelosError};

pub fn emit(kind: &str, id: &str, value: &impl Serialize) -> Result<String, TelosError> {
    let value = serde_json::to_value(value).map_err(|e| invalid(e.to_string()))?;
    let mut out = format!("{kind} {} ", serde_json::to_string(id).expect("string"));
    emit_value(&mut out, &value, 0);
    out.push('\n');
    Ok(out)
}

fn emit_value(out: &mut String, value: &Value, depth: usize) {
    match value {
        Value::Object(fields) => {
            out.push_str("{\n");
            for (key, value) in fields {
                out.push_str(&"  ".repeat(depth + 1));
                if key
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                {
                    out.push_str(key);
                } else {
                    out.push_str(&serde_json::to_string(key).expect("string"));
                }
                out.push(' ');
                emit_value(out, value, depth + 1);
                out.push('\n');
            }
            out.push_str(&"  ".repeat(depth));
            out.push('}');
        }
        Value::Array(values) if !values.is_empty() => {
            out.push_str("[\n");
            for value in values {
                out.push_str(&"  ".repeat(depth + 1));
                emit_value(out, value, depth + 1);
                out.push('\n');
            }
            out.push_str(&"  ".repeat(depth));
            out.push(']');
        }
        _ => out.push_str(&serde_json::to_string(value).expect("JSON value")),
    }
}

pub fn parse<T: DeserializeOwned>(kind: &str, source: &str) -> Result<(String, T), TelosError> {
    let mut p = Parser { source, pos: 0 };
    if p.word()? != kind {
        return Err(invalid(format!("expected `{kind}` record")));
    }
    let Value::String(id) = p.value(0)? else {
        return Err(invalid("expected a quoted record identity"));
    };
    let value = p.value(0)?;
    p.space();
    if p.pos != source.len() {
        return Err(invalid("unexpected trailing record content"));
    }
    let record = serde_json::from_value(value).map_err(|e| invalid(e.to_string()))?;
    Ok((id, record))
}

struct Parser<'a> {
    source: &'a str,
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.source.as_bytes().get(self.pos).copied()
    }

    fn space(&mut self) {
        loop {
            while self
                .peek()
                .is_some_and(|b| b.is_ascii_whitespace() || b == b',')
            {
                self.pos += 1;
            }
            if self.source[self.pos..].starts_with("//") {
                while self.peek().is_some_and(|b| b != b'\n') {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    fn word(&mut self) -> Result<String, TelosError> {
        self.space();
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            self.pos += 1;
        }
        if start == self.pos {
            return Err(invalid(format!("expected a field at byte {}", self.pos)));
        }
        Ok(self.source[start..self.pos].to_owned())
    }

    fn value(&mut self, depth: usize) -> Result<Value, TelosError> {
        if depth > 64 {
            return Err(invalid("record nesting exceeds 64 levels"));
        }
        self.space();
        match self.peek() {
            Some(b'{') => {
                self.pos += 1;
                let mut map = Map::new();
                loop {
                    self.space();
                    if self.peek() == Some(b'}') {
                        self.pos += 1;
                        break;
                    }
                    let key = if self.peek() == Some(b'"') {
                        let Value::String(key) = self.value(depth + 1)? else {
                            unreachable!()
                        };
                        key
                    } else {
                        self.word()?
                    };
                    let value = self.value(depth + 1)?;
                    if map.insert(key.clone(), value).is_some() {
                        return Err(invalid(format!("duplicate field `{key}`")));
                    }
                }
                Ok(Value::Object(map))
            }
            Some(b'[') => {
                self.pos += 1;
                let mut values = Vec::new();
                loop {
                    self.space();
                    if self.peek() == Some(b']') {
                        self.pos += 1;
                        break;
                    }
                    values.push(self.value(depth + 1)?);
                }
                Ok(Value::Array(values))
            }
            Some(b'"') => {
                let start = self.pos;
                self.pos += 1;
                let mut escaped = false;
                while let Some(b) = self.peek() {
                    self.pos += 1;
                    if b == b'"' && !escaped {
                        return serde_json::from_str(&self.source[start..self.pos])
                            .map_err(|e| invalid(e.to_string()));
                    }
                    escaped = b == b'\\' && !escaped;
                }
                Err(invalid("unterminated string"))
            }
            Some(_) => {
                let start = self.pos;
                while self
                    .peek()
                    .is_some_and(|b| !b.is_ascii_whitespace() && !b",]}".contains(&b))
                {
                    self.pos += 1;
                }
                if start == self.pos {
                    return Err(invalid(format!("expected a value at byte {}", self.pos)));
                }
                let value: Value = serde_json::from_str(&self.source[start..self.pos])
                    .map_err(|e| invalid(e.to_string()))?;
                if value.is_object() || value.is_array() || value.is_string() {
                    return Err(invalid("invalid scalar"));
                }
                Ok(value)
            }
            None => Err(invalid("unexpected end of record")),
        }
    }
}

fn invalid(message: impl Into<String>) -> TelosError {
    TelosError::new(ErrorCode::TelosParseError, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn nested_records_round_trip_without_losing_text() {
        let value = json!({"format": 1, "goal": "A \"quoted\" request\nwith Unicode: λ", "tasks": [{"id": "TSK-001", "done": false, "notes": null}]});
        let text = emit("plan", "PLN-test", &value).unwrap();
        let (id, decoded): (_, Value) = parse("plan", &text).unwrap();
        assert_eq!(id, "PLN-test");
        assert_eq!(decoded, value);
        assert_eq!(emit("plan", &id, &decoded).unwrap(), text);
    }

    #[test]
    fn malformed_or_ambiguous_records_are_rejected() {
        for source in [
            "plan \"x\" { a 1 a 2 }",
            "plan \"x\" {",
            "plan \"x\" { a [ }",
            "plan \"x\" {} garbage",
        ] {
            assert!(parse::<Value>("plan", source).is_err(), "{source}");
        }
    }
}
