//! Public recorded ownership without exposing the component's private ledger format.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use serde::{Deserialize, Deserializer, de};
use serde_json::{Value, json};

use super::{Harness, StateReading, Target};

/// Unavailable or inconsistent metadata is unknown, never an empty ownership claim.
pub(super) fn observe(harness: &Harness, target: &Target, state: &StateReading) -> Value {
    recorded(harness, target, state).unwrap_or(Value::Null)
}

fn recorded(harness: &Harness, target: &Target, state: &StateReading) -> Option<Value> {
    let (_, content) = setup_core::forward::observe(
        target,
        &format!("{}/written-fields.json", harness.control_directory),
    )
    .ok()?;
    let fields = match content {
        None => BTreeMap::new(),
        Some(content) => serde_json::from_slice::<Fields>(&content.bytes).ok()?.0,
    };
    let current = match state {
        StateReading::Absent if fields.is_empty() => return Some(json!([])),
        StateReading::Current(current) => current,
        _ => return None,
    };
    if current.provider_id != harness.provider_id
        || current.harness_id != harness.harness_id
        || current.canonical_target != target.root().to_str()?
        || current.written_paths.len() > 8192
    {
        return None;
    }
    let paths: BTreeSet<_> = current.written_paths.iter().collect();
    if paths.len() != current.written_paths.len() || fields.keys().any(|path| !paths.contains(path))
    {
        return None;
    }
    let mut rows = Vec::new();
    for path in paths {
        if !relative(path) {
            return None;
        }
        let row = if let Some(keys) = fields.get(path) {
            let unique: BTreeSet<_> = keys.iter().collect();
            if keys.is_empty()
                || keys.len() > 2000
                || unique.len() != keys.len()
                || keys.iter().any(|key| {
                    key.is_empty() || key.len() > 255 || key.chars().any(char::is_control)
                })
            {
                return None;
            }
            let parser = match Path::new(path).extension().and_then(|ext| ext.to_str()) {
                Some("json" | "jsonc") => "json/1",
                Some("toml") => "toml/1",
                _ => return None,
            };
            json!({"path":path,"ownership":"contribution","parser_id":parser,"ownership_keys":unique})
        } else {
            json!({"path":path,"ownership":"whole","parser_id":null,"ownership_keys":[]})
        };
        rows.push(row);
    }
    Some(rows.into())
}

fn relative(path: &str) -> bool {
    path.len() <= 4096
        && !path
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '\\' | ':'))
        && path
            .split('/')
            .all(|part| !matches!(part, "" | "." | "..") && part.len() <= 255)
}

struct Fields(BTreeMap<String, Vec<String>>);

impl<'de> Deserialize<'de> for Fields {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> de::Visitor<'de> for Visitor {
            type Value = Fields;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("unique recorded paths and their top-level keys")
            }

            fn visit_map<M: de::MapAccess<'de>>(self, mut map: M) -> Result<Fields, M::Error> {
                let mut fields = BTreeMap::new();
                while let Some((path, keys)) = map.next_entry::<String, Vec<String>>()? {
                    if fields.len() >= 8192 || fields.insert(path, keys).is_some() {
                        return Err(de::Error::custom(
                            "ambiguous recorded configuration ownership",
                        ));
                    }
                }
                Ok(Fields(fields))
            }
        }
        deserializer.deserialize_map(Visitor)
    }
}
