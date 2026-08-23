//! RFC 8785 canonicalization for the value subset this kernel emits.
//!
//! A plan digest, a state digest and — later — a release signature all bind
//! bytes, not a structure, so two implementations must agree on exactly one
//! byte sequence for the same value. RFC 8785 defines that sequence.
//!
//! The hard half of RFC 8785 is number serialization: it mandates the
//! ECMAScript `Number::toString` algorithm for doubles, which is easy to get
//! subtly wrong. This kernel avoids that half by construction — it emits
//! integers, strings, booleans, arrays, objects and null, and nothing else.
//! [`to_canonical_bytes`] rejects a floating-point value rather than guessing a
//! spelling for it, so the omission stays a checked property instead of an
//! assumption.

use serde_json::Value;

use crate::error::{Error, ReasonCode, Result};

/// Serialize a value into its RFC 8785 canonical bytes.
///
/// # Errors
///
/// Returns [`ReasonCode::IntegrityMismatch`] if the value contains a
/// floating-point number, which this subset does not define a spelling for.
pub fn to_canonical_bytes(value: &Value) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    write_value(value, &mut out)?;
    Ok(out)
}

fn write_value(value: &Value, out: &mut Vec<u8>) -> Result<()> {
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Number(number) => {
            let Some(rendered) = render_number(number) else {
                return Err(Error::new(
                    ReasonCode::IntegrityMismatch,
                    "canonical JSON in this kernel is integer-only; a floating-point \
                     value has no defined spelling here",
                ));
            };
            out.extend_from_slice(rendered.as_bytes());
        }
        Value::String(text) => write_string(text, out),
        Value::Array(items) => {
            out.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                write_value(item, out)?;
            }
            out.push(b']');
        }
        Value::Object(map) => {
            // RFC 8785 orders members by the UTF-16 code units of their names.
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|left, right| left.encode_utf16().cmp(right.encode_utf16()));
            out.push(b'{');
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                write_string(key, out);
                out.push(b':');
                match map.get(*key) {
                    Some(member) => write_value(member, out)?,
                    None => {
                        return Err(Error::new(
                            ReasonCode::IntegrityMismatch,
                            "object member disappeared between key listing and lookup",
                        ));
                    }
                }
            }
            out.push(b'}');
        }
    }
    Ok(())
}

fn render_number(number: &serde_json::Number) -> Option<String> {
    if let Some(value) = number.as_u64() {
        return Some(value.to_string());
    }
    if let Some(value) = number.as_i64() {
        return Some(value.to_string());
    }
    None
}

fn write_string(text: &str, out: &mut Vec<u8>) {
    out.push(b'"');
    for character in text.chars() {
        match character {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\u{08}' => out.extend_from_slice(b"\\b"),
            '\u{0c}' => out.extend_from_slice(b"\\f"),
            '\n' => out.extend_from_slice(b"\\n"),
            '\r' => out.extend_from_slice(b"\\r"),
            '\t' => out.extend_from_slice(b"\\t"),
            control if (control as u32) < 0x20 => {
                let escaped = format!("\\u{:04x}", control as u32);
                out.extend_from_slice(escaped.as_bytes());
            }
            other => {
                let mut buffer = [0_u8; 4];
                out.extend_from_slice(other.encode_utf8(&mut buffer).as_bytes());
            }
        }
    }
    out.push(b'"');
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use serde_json::json;

    use super::*;

    #[test]
    fn members_are_ordered_and_whitespace_is_absent() {
        let value = json!({ "b": 1, "a": [true, null], "A": "x" });
        let bytes = to_canonical_bytes(&value).unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "{\"A\":\"x\",\"a\":[true,null],\"b\":1}"
        );
    }

    #[test]
    fn short_escapes_are_preferred_and_other_controls_use_the_hex_form() {
        let mut text = String::from("a\nb\tc\"d\\e");
        text.push('\u{01}');
        let value = json!({ "k": text });
        let bytes = to_canonical_bytes(&value).unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "{\"k\":\"a\\nb\\tc\\\"d\\\\e\\u0001\"}"
        );
    }

    #[test]
    fn a_float_is_refused_rather_than_spelled() {
        let value = json!({ "k": 1.5 });
        let error = to_canonical_bytes(&value).unwrap_err();
        assert_eq!(error.reason(), ReasonCode::IntegrityMismatch);
    }

    #[test]
    fn negative_integers_survive_the_integer_only_path() {
        let value = json!({ "k": -42 });
        let bytes = to_canonical_bytes(&value).unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap(), "{\"k\":-42}");
    }
}
