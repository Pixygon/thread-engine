//! Models as **programs**: evaluate a Weft package export into a [`Model`].
//!
//! This is the difference between a model you *wrote* and a model you can
//! *ask for*. A Weft modeling program takes parameters (how tall, how many
//! steps, how worn), loops (`Iota`/`Map`/`Fold` — a spiral staircase is a
//! fold, not forty pasted blocks), and returns the flat carving sequence
//! plus its PBR materials. Because it is Weft it is also **verified before
//! it runs** (total, fuel-bounded, effect-free), **content-addressed** (the
//! model *is* its hash — cache it forever, share it by name), and
//! **deterministic** (the same program yields the same vertices on every
//! machine, which is what makes a hash mean anything).
//!
//! Modeling runs at author time or load time — never in a frame (weft-v0.1
//! §1.3, the nervous-system rule).

use infinite_manifest::model::Model;
use weft::{Ty, Value};

/// Fuel for one model evaluation. Generous — modeling is a load-time act,
/// not a frame-time one — but still bounded: an infinite model is a bug, and
/// the verifier's totality guarantee means this is a budget, not a rescue.
pub const MODEL_FUEL: u64 = 40_000_000;

/// Evaluate `export`, and take whatever it gives: a whole [`Model`], or bare
/// geometry (a list of carving steps) — which is dressed in `material` (a
/// second export of the same package) so that *any* part previews and
/// exports PBR-complete. Asking for a column should not require also
/// knowing how to assemble a model.
pub fn eval_model_or_part(
    package_json: &str,
    export: &str,
    args: &[serde_json::Value],
    material: Option<&str>,
) -> Result<Model, String> {
    let value = eval_export(package_json, export, args)?;
    if value.is_array() {
        let mat_name = material.unwrap_or("plaster");
        let mat = eval_export(package_json, mat_name, &[])
            .map_err(|e| format!("material '{mat_name}': {e}"))?;
        let model = serde_json::json!({
            "name": export,
            "nodes": value,
            "materials": [mat],
        });
        let model: Model = serde_json::from_value(model)
            .map_err(|e| format!("'{export}' returned neither a model nor carving steps: {e}"))?;
        model.validate()?;
        return Ok(model);
    }
    let model: Model = serde_json::from_value(value)
        .map_err(|e| format!("'{export}' did not return a model: {e}"))?;
    model.validate()?;
    Ok(model)
}

/// Evaluate `export` in a Weft package (JSON) with JSON arguments, and
/// decode the result as a model.
pub fn eval_model(
    package_json: &str,
    export: &str,
    args: &[serde_json::Value],
) -> Result<Model, String> {
    let value = eval_export(package_json, export, args)?;
    let model: Model = serde_json::from_value(value)
        .map_err(|e| format!("'{export}' did not return a model: {e}"))?;
    model.validate()?;
    Ok(model)
}

/// Evaluate a part export straight to a [`Shape`] — the seam for *generators*
/// (world builders that want the library's geometry inside a manifest they
/// are writing, rather than a file on disk).
pub fn eval_part_shape(
    package_json: &str,
    export: &str,
    args: &[serde_json::Value],
) -> Result<infinite_manifest::shape::Shape, String> {
    let model = eval_model_or_part(package_json, export, args, None)?;
    let mut parts = model.resolve()?;
    if parts.is_empty() {
        return Err(format!("'{export}' resolved to no geometry"));
    }
    Ok(parts.remove(0).shape)
}

/// The built-in library, serialized once — generators call this rather than
/// carrying a package file around.
pub fn standard_library() -> String {
    serde_json::to_string(&weft::model_lib::package()).unwrap_or_default()
}

/// Evaluate one export to raw JSON — the general seam.
pub fn eval_export(
    package_json: &str,
    export: &str,
    args: &[serde_json::Value],
) -> Result<serde_json::Value, String> {
    let pkg: weft::pack::Package =
        serde_json::from_str(package_json).map_err(|e| format!("not a Weft package: {e}"))?;
    pkg.verify()
        .map_err(|e| format!("package failed verification: {e}"))?;
    let hash = pkg
        .export(export)
        .ok_or_else(|| format!("package '{}' has no export '{export}'", pkg.name))?;
    let entry = pkg
        .defs
        .get(&hash)
        .cloned()
        .ok_or("export verified to exist")?;
    let module = weft::pack::link(&[pkg], vec![entry.clone()], 0)
        .map_err(|e| format!("link failed: {e}"))?;

    if args.len() != entry.params.len() {
        return Err(format!(
            "'{export}' takes {} argument(s), got {}",
            entry.params.len(),
            args.len()
        ));
    }
    let values: Vec<Value> = entry
        .params
        .iter()
        .zip(args)
        .map(|(ty, v)| json_to_value(ty, v))
        .collect::<Result<_, _>>()?;

    let out = weft::eval_call(&module, module.entry, values, MODEL_FUEL)
        .map_err(|e| format!("model program failed: {e:?}"))?;
    Ok(value_to_json(&out.value))
}

/// A JSON argument, typed into Weft's value space by the parameter's type —
/// so an agent writes `[2.4, 12]` and the Fix/Int distinction is handled.
fn json_to_value(ty: &Ty, v: &serde_json::Value) -> Result<Value, String> {
    Ok(match (ty, v) {
        (Ty::Fix, serde_json::Value::Number(n)) => {
            let f = n.as_f64().unwrap_or(0.0);
            Value::Fix((f * weft::FIX_SCALE as f64).round() as i64)
        }
        // An agent writes `12` or `12.0` and means the same thing; accept
        // both, but refuse `12.5` where a count belongs rather than
        // truncating it into a silent surprise.
        (Ty::Int, serde_json::Value::Number(n)) => match n.as_i64() {
            Some(i) => Value::Int(i),
            None => {
                let f = n.as_f64().unwrap_or(f64::NAN);
                if f.fract() == 0.0 && f.is_finite() {
                    Value::Int(f as i64)
                } else {
                    return Err(format!("expected a whole number, got {n}"));
                }
            }
        },
        (Ty::Bool, serde_json::Value::Bool(b)) => Value::Bool(*b),
        (Ty::Text, serde_json::Value::String(s)) => Value::Text(s.clone()),
        (Ty::List(inner), serde_json::Value::Array(xs)) => Value::List(
            xs.iter()
                .map(|x| json_to_value(inner, x))
                .collect::<Result<_, _>>()?,
        ),
        (Ty::Record(fields), serde_json::Value::Object(map)) => {
            let mut out = std::collections::BTreeMap::new();
            for (k, fty) in fields {
                let fv = map
                    .get(k)
                    .ok_or_else(|| format!("argument missing field '{k}'"))?;
                out.insert(k.clone(), json_to_value(fty, fv)?);
            }
            Value::Rec(out)
        }
        (t, got) => return Err(format!("argument type mismatch: wanted {t:?}, got {got}")),
    })
}

/// Weft value → JSON. `Fix` crosses as a plain number (millionths → f64
/// exactly at these magnitudes), which is precisely what the model format's
/// float fields expect.
fn value_to_json(v: &Value) -> serde_json::Value {
    match v {
        Value::Int(i) => serde_json::Value::from(*i),
        Value::Fix(raw) => serde_json::Value::from(*raw as f64 / weft::FIX_SCALE as f64),
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Text(s) => serde_json::Value::String(s.clone()),
        Value::List(xs) => serde_json::Value::Array(xs.iter().map(value_to_json).collect()),
        Value::Rec(fs) => serde_json::Value::Object(
            fs.iter()
                .map(|(k, x)| (k.clone(), value_to_json(x)))
                .collect(),
        ),
        Value::Action { .. } => serde_json::Value::Null,
    }
}
