//! python built-ins as the debug command queue applies them to json values
//! (int(), float(), truthiness, bytes.fromhex, iteration).

use serde_json::Value;

/// python int(): ints, floats truncated, bools, and integer strings.
pub fn py_int(value: &Value) -> Result<i64, String> {
    match value {
        Value::Bool(b) => Ok(*b as i64),
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().filter(|f| f.is_finite()).map(|f| f.trunc() as i64)).ok_or_else(|| format!("int() of {n}")),
        Value::String(text) => text.trim().replace('_', "").parse().map_err(|_| format!("invalid literal for int(): {text:?}")),
        other => Err(format!("int() argument must be a string or a number, not {other}")),
    }
}

/// python float(): numbers, bools and numeric strings.
pub fn py_float(value: &Value) -> Result<f64, String> {
    match value {
        Value::Bool(b) => Ok(*b as u8 as f64),
        Value::Number(n) => n.as_f64().ok_or_else(|| format!("float() of {n}")),
        Value::String(text) => {
            let t = text.trim().to_ascii_lowercase();
            match t.as_str() {
                "inf" | "+inf" | "infinity" | "+infinity" => Ok(f64::INFINITY),
                "-inf" | "-infinity" => Ok(f64::NEG_INFINITY),
                "nan" | "+nan" | "-nan" => Ok(f64::NAN),
                _ => t.replace('_', "").parse().map_err(|_| format!("could not convert string to float: {text:?}")),
            }
        }
        other => Err(format!("float() argument must be a string or a real number, not {other}")),
    }
}

/// python truthiness of a json value.
pub fn py_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// python bytes.fromhex: ascii whitespace is skipped, pairs of hex digits.
pub fn py_fromhex(text: &str) -> Result<Vec<u8>, String> {
    let digits: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if !digits.len().is_multiple_of(2) || !digits.iter().all(u8::is_ascii_hexdigit) {
        return Err("non-hexadecimal number found in fromhex() arg".into());
    }
    Ok(digits.chunks(2).map(|pair| u8::from_str_radix(std::str::from_utf8(pair).expect("ascii"), 16).expect("hex")).collect())
}

/// python `for v in value`: list items, a string's characters, an object's keys.
pub fn py_iter(value: &Value) -> Result<Vec<Value>, String> {
    match value {
        Value::Array(items) => Ok(items.clone()),
        Value::String(text) => Ok(text.chars().map(|c| Value::String(c.to_string())).collect()),
        Value::Object(map) => Ok(map.keys().map(|k| Value::String(k.clone())).collect()),
        other => Err(format!("TypeError: '{}' object is not iterable", py_type(other))),
    }
}

fn py_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(n) if n.is_f64() => "float",
        Value::Number(_) => "int",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

/// os.unlink: a read-only file is refused on windows (std removes it anyway).
pub fn py_unlink(path: &std::path::Path) -> Result<(), String> {
    if std::fs::metadata(path).map_err(|e| e.to_string())?.permissions().readonly() {
        return Err(format!("PermissionError: [WinError 5] Access is denied: {}", path.display()));
    }
    std::fs::remove_file(path).map_err(|e| e.to_string())
}
