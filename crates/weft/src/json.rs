//! **The JSON seam** — plain JSON in, plain JSON out, guided by types.
//!
//! A host (the Node bridge, a shell) speaks JSON; Weft speaks typed values.
//! Inputs are decoded *against the definition's parameter types*, so a JSON
//! number becomes an `Int` or a `Fix` depending on what the parameter asks
//! for, and anything that does not fit is a clear error rather than a guess.
//! Outputs are plain JSON: `Fix` as a decimal number (millionths ÷ 10⁶,
//! exact in f64 at these magnitudes), records as objects, actions as
//! `{"$effect": "<kind>", "fields": {...}}`.

use std::collections::BTreeMap;

use serde_json::{json, Map, Value as J};

use crate::text::ty_text;
use crate::{Def, EffectKind, Ty, Value, FIX_SCALE};

fn effect_tag(k: EffectKind) -> String {
    serde_json::to_value(k)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| format!("{k:?}"))
}

/// Decode one JSON value as a Weft value of type `ty`.
pub fn value_from_json(ty: &Ty, v: &J, path: &str) -> Result<Value, String> {
    match ty {
        Ty::Int => match v {
            J::Number(n) => n
                .as_i64()
                .map(Value::Int)
                .ok_or_else(|| format!("{path}: expected an integer, got {n}")),
            other => Err(format!("{path}: expected Int, got {}", kind(other))),
        },
        Ty::Fix => match v {
            J::Number(n) => {
                let f = n
                    .as_f64()
                    .ok_or_else(|| format!("{path}: bad number {n}"))?;
                if !f.is_finite() {
                    return Err(format!("{path}: Fix must be finite"));
                }
                let raw = (f * FIX_SCALE as f64).round();
                if raw.abs() > i64::MAX as f64 / 2.0 {
                    return Err(format!("{path}: {f} is out of Fix range"));
                }
                Ok(Value::Fix(raw as i64))
            }
            other => Err(format!(
                "{path}: expected Fix (a number), got {}",
                kind(other)
            )),
        },
        Ty::Bool => match v {
            J::Bool(b) => Ok(Value::Bool(*b)),
            other => Err(format!("{path}: expected Bool, got {}", kind(other))),
        },
        Ty::Text => match v {
            J::String(s) => Ok(Value::Text(s.clone())),
            other => Err(format!("{path}: expected Text, got {}", kind(other))),
        },
        Ty::Action => Err(format!("{path}: an Action cannot be given as input")),
        Ty::List(elem) => match v {
            J::Array(items) => items
                .iter()
                .enumerate()
                .map(|(i, it)| value_from_json(elem, it, &format!("{path}[{i}]")))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::List),
            other => Err(format!(
                "{path}: expected List {}, got {}",
                ty_text(elem),
                kind(other)
            )),
        },
        Ty::Record(fields) => match v {
            J::Object(obj) => {
                let mut out = BTreeMap::new();
                for (k, t) in fields {
                    let fv = obj
                        .get(k)
                        .ok_or_else(|| format!("{path}: missing field '{k}'"))?;
                    out.insert(k.clone(), value_from_json(t, fv, &format!("{path}.{k}"))?);
                }
                if let Some(extra) = obj.keys().find(|k| !fields.contains_key(*k)) {
                    return Err(format!(
                        "{path}: unexpected field '{extra}' (record type is {})",
                        ty_text(ty)
                    ));
                }
                Ok(Value::Rec(out))
            }
            other => Err(format!(
                "{path}: expected a record {}, got {}",
                ty_text(ty),
                kind(other)
            )),
        },
    }
}

fn kind(v: &J) -> &'static str {
    match v {
        J::Null => "null",
        J::Bool(_) => "a boolean",
        J::Number(_) => "a number",
        J::String(_) => "a string",
        J::Array(_) => "an array",
        J::Object(_) => "an object",
    }
}

/// Decode the argument list for `def` from a JSON input. The input is an
/// array with one element per parameter; when the definition takes exactly
/// one parameter that is not itself a list, a bare (non-array) value is
/// accepted as that single argument.
pub fn args_from_json(def: &Def, input: &J) -> Result<Vec<Value>, String> {
    let items: Vec<&J> = match input {
        J::Array(xs) => xs.iter().collect(),
        other if def.params.len() == 1 && !matches!(def.params[0], Ty::List(_)) => vec![other],
        _ => {
            return Err(format!(
                "input must be a JSON array of {} argument(s)",
                def.params.len()
            ))
        }
    };
    if items.len() != def.params.len() {
        return Err(format!(
            "expected {} argument(s), got {}",
            def.params.len(),
            items.len()
        ));
    }
    def.params
        .iter()
        .zip(items)
        .enumerate()
        .map(|(i, (t, v))| value_from_json(t, v, &format!("arg{i}")))
        .collect()
}

/// Encode a Weft value as plain JSON.
pub fn value_to_json(v: &Value) -> J {
    match v {
        Value::Int(i) => json!(i),
        Value::Fix(raw) => json!(*raw as f64 / FIX_SCALE as f64),
        Value::Bool(b) => json!(b),
        Value::Text(s) => json!(s),
        Value::List(xs) => J::Array(xs.iter().map(value_to_json).collect()),
        Value::Rec(fs) => {
            let mut m = Map::new();
            for (k, x) in fs {
                m.insert(k.clone(), value_to_json(x));
            }
            J::Object(m)
        }
        Value::Action { kind, fields } => {
            let mut m = Map::new();
            for (k, x) in fields {
                m.insert(k.clone(), value_to_json(x));
            }
            json!({ "$effect": effect_tag(*kind), "fields": m })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_decodes_by_type_and_encodes_plainly() {
        let ty = Ty::Record(
            [
                ("n".to_string(), Ty::Int),
                ("f".to_string(), Ty::Fix),
                ("t".to_string(), Ty::Text),
                ("xs".to_string(), Ty::List(Box::new(Ty::Bool))),
            ]
            .into(),
        );
        let v = value_from_json(
            &ty,
            &json!({"n": 3, "f": 1.5, "t": "hi", "xs": [true]}),
            "in",
        )
        .unwrap();
        assert_eq!(
            value_to_json(&v),
            json!({"n": 3, "f": 1.5, "t": "hi", "xs": [true]})
        );
        assert!(
            value_from_json(&ty, &json!({"n": 3.5, "f": 1, "t": "", "xs": []}), "in")
                .unwrap_err()
                .contains("expected an integer")
        );
        assert!(value_from_json(
            &ty,
            &json!({"n": 3, "f": 1, "t": "", "xs": [], "zz": 1}),
            "in"
        )
        .unwrap_err()
        .contains("unexpected field 'zz'"));
        assert!(
            value_from_json(&ty, &json!({"n": 3, "f": 1, "xs": []}), "in")
                .unwrap_err()
                .contains("missing field 't'")
        );
    }
}
