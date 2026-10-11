//! Preserve host syntax and withdraw only the provider's recorded top-level keys.

use std::collections::BTreeSet;

use jsonc_parser::{
    CollectOptions, ParseOptions, ast,
    cst::{CstInputValue, CstRootNode},
};
use provider_v3::bundle::Contribution;
use setup_core::forward::Content;
use toml_edit::DocumentMut;

use super::{Result, refused};

fn key(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 255 || value.chars().any(char::is_control) {
        return Err(refused());
    }
    Ok(())
}

fn text(bytes: &[u8]) -> Result<&str> {
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(refused());
    }
    std::str::from_utf8(bytes).map_err(|_| refused())
}

fn extension(path: &str) -> Option<&str> {
    std::path::Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
}

fn options(path: &str) -> ParseOptions {
    let comments = extension(path) == Some("jsonc");
    ParseOptions {
        allow_comments: comments,
        allow_trailing_commas: comments,
        allow_loose_object_property_names: false,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
        allow_bare_decimal_point_numbers: false,
        allow_non_finite_numbers: false,
        allow_extended_string_escapes: false,
    }
}

fn input(value: ast::Value<'_>, depth: usize, nodes: &mut usize) -> Result<CstInputValue> {
    *nodes += 1;
    if depth > 128 || *nodes > 20_000 {
        return Err(refused());
    }
    Ok(match value {
        ast::Value::NullKeyword(_) => CstInputValue::Null,
        ast::Value::BooleanLit(value) => CstInputValue::Bool(value.value),
        ast::Value::StringLit(value) => CstInputValue::String(value.value.into_owned()),
        // Do not round identifiers through floating point.
        ast::Value::NumberLit(value) => CstInputValue::Number(value.value.into()),
        ast::Value::Array(value) => CstInputValue::Array(
            value
                .elements
                .into_iter()
                .map(|item| input(item, depth + 1, nodes))
                .collect::<Result<_>>()?,
        ),
        ast::Value::Object(value) => {
            let mut names = BTreeSet::new();
            let mut fields = Vec::new();
            for property in value.properties {
                let name = property.name.into_string();
                if !names.insert(name.clone()) {
                    return Err(refused());
                }
                fields.push((name, input(property.value, depth + 1, nodes)?));
            }
            CstInputValue::Object(fields)
        }
    })
}

fn json(source: &str, path: &str) -> Result<CstInputValue> {
    let parsed = jsonc_parser::parse_to_ast(source, &CollectOptions::default(), &options(path))
        .map_err(|_| refused())?;
    let result = input(parsed.value.ok_or_else(refused)?, 0, &mut 0)?;
    if !matches!(result, CstInputValue::Object(_)) {
        return Err(refused());
    }
    Ok(result)
}

pub(super) fn check_host(path: &str, held: Option<&Content>, claim: &Contribution) -> Result<()> {
    key(&claim.ownership_key)?;
    let expected = held.map_or(serde_json::Value::Null, |content| {
        setup_core::digest::of_bytes(&content.bytes).into()
    });
    let parser = if extension(path) == Some("toml") {
        "toml/1"
    } else if extension(path) == Some("json") || extension(path) == Some("jsonc") {
        "json/1"
    } else {
        return Err(refused());
    };
    if claim.parser_id != parser || claim.host_digest != expected {
        return Err(refused());
    }
    Ok(())
}

/// A missing incoming key withdraws the old contribution. Empty host files stay
/// as valid documents; no comment or unrelated field authorizes file deletion.
pub(super) fn merge(
    path: &str,
    existing: Option<&Content>,
    incoming: Option<(&[u8], &str)>,
    old_keys: &[String],
) -> Result<Vec<u8>> {
    if old_keys.len() > 2000 {
        return Err(refused());
    }
    for name in old_keys {
        key(name)?;
    }
    let bytes = existing.map_or(&[][..], |content| content.bytes.as_slice());
    let result = if extension(path) == Some("toml") {
        let mut host: DocumentMut = text(bytes)?.parse().map_err(|_| refused())?;
        for name in old_keys {
            host.remove(name);
        }
        if let Some((bytes, name)) = incoming {
            key(name)?;
            let document: DocumentMut = text(bytes)?.parse().map_err(|_| refused())?;
            let value = document
                .get(name)
                .filter(|item| item.is_table_like())
                .ok_or_else(refused)?;
            host.insert(name, value.clone());
        }
        host.to_string()
    } else if extension(path) == Some("json") || extension(path) == Some("jsonc") {
        let source = if existing.is_some() {
            text(bytes)?
        } else {
            "{}"
        };
        json(source, path)?;
        let document = CstRootNode::parse(source, &options(path)).map_err(|_| refused())?;
        let object = document.object_value().ok_or_else(refused)?;
        for name in old_keys {
            if let Some(property) = object.get(name) {
                property.remove();
            }
        }
        if let Some((bytes, name)) = incoming {
            key(name)?;
            let CstInputValue::Object(properties) = json(text(bytes)?, path)? else {
                return Err(refused());
            };
            let value = properties
                .into_iter()
                .find_map(|(key, value)| (key == name).then_some(value))
                .filter(|value| matches!(value, CstInputValue::Object(_)))
                .ok_or_else(refused)?;
            match object.get(name) {
                Some(property) => property.set_value(value),
                None => {
                    object.append(name, value);
                }
            }
        }
        document.to_string()
    } else {
        return Err(refused());
    };
    text(result.as_bytes())?;
    Ok(result.into_bytes())
}
