//! Strict reading of request parameters.
//!
//! Both transports used to pull numbers out of JSON with
//! `value.get(key).and_then(|v| v.as_u64()).unwrap_or(default)`. `as_u64`
//! returns `None` for anything that is not a JSON *integer*, so `3.0`, `"3"`
//! and `true` all silently became the default. JSON has one number type and
//! plenty of clients emit `3.0` for an integer — it is even schema-valid
//! against the MCP tools' own `"type": "integer"` declarations — so
//! `max_pages: 3.0` quietly ran the 500-page default and `delay_ms: 1000.0`
//! quietly dropped the caller's rate limit to 250 ms. Silently discarding a
//! politeness setting aimed at someone else's production server is the
//! dangerous half of that.
//!
//! So: accept anything that unambiguously *is* the integer the caller meant
//! (including integral floats), and reject everything else loudly instead of
//! substituting a default the caller did not ask for.

use serde_json::Value;

/// Read an optional non-negative integer.
///
/// `Ok(None)` means "not supplied" — only a missing key or an explicit `null`.
/// Anything present but not interpretable as a non-negative integer is an
/// error naming the parameter and what arrived.
pub fn opt_u64(args: &Value, key: &str) -> Result<Option<u64>, String> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    match value {
        Value::Null => Ok(None),
        Value::Number(n) => {
            if let Some(v) = n.as_u64() {
                return Ok(Some(v));
            }
            if let Some(v) = n.as_i64() {
                return Err(negative(key, v));
            }
            match n.as_f64() {
                Some(f) if !f.is_finite() => Err(format!(
                    "Parameter '{key}' must be a finite number, got {value}."
                )),
                Some(f) if f < 0.0 => Err(negative(key, f)),
                Some(f) if f.fract() != 0.0 => Err(format!(
                    "Parameter '{key}' must be a whole number, got {f}."
                )),
                Some(f) if f > u64::MAX as f64 => {
                    Err(format!("Parameter '{key}' is out of range: {f}."))
                }
                // Integral floats are the common wire form of an integer.
                Some(f) => Ok(Some(f as u64)),
                None => Err(format!(
                    "Parameter '{key}' is not a usable number: {value}."
                )),
            }
        }
        other => Err(format!(
            "Parameter '{key}' must be a number, got {}: {other}.",
            type_name(other)
        )),
    }
}

/// As [`opt_u64`], falling back to `default` when the parameter is absent.
pub fn u64_or(args: &Value, key: &str, default: u64) -> Result<u64, String> {
    Ok(opt_u64(args, key)?.unwrap_or(default))
}

/// Read an optional boolean, rejecting other types rather than defaulting.
pub fn opt_bool(args: &Value, key: &str) -> Result<Option<bool>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(other) => Err(format!(
            "Parameter '{key}' must be a boolean, got {}: {other}.",
            type_name(other)
        )),
    }
}

/// As [`opt_bool`], falling back to `default` when the parameter is absent.
pub fn bool_or(args: &Value, key: &str, default: bool) -> Result<bool, String> {
    Ok(opt_bool(args, key)?.unwrap_or(default))
}

/// Read an optional string, rejecting other types rather than defaulting.
pub fn opt_str<'a>(args: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str())),
        Some(other) => Err(format!(
            "Parameter '{key}' must be a string, got {}: {other}.",
            type_name(other)
        )),
    }
}

/// As [`opt_str`], falling back to `default` when the parameter is absent.
pub fn str_or<'a>(args: &'a Value, key: &str, default: &'a str) -> Result<&'a str, String> {
    Ok(opt_str(args, key)?.unwrap_or(default))
}

/// Constrain a string parameter to a known set, naming the alternatives.
pub fn one_of<'a>(key: &str, value: &'a str, allowed: &[&str]) -> Result<&'a str, String> {
    if allowed.contains(&value) {
        Ok(value)
    } else {
        Err(format!(
            "Parameter '{key}' must be one of {}, got '{value}'.",
            allowed.join(", ")
        ))
    }
}

fn negative(key: &str, value: impl std::fmt::Display) -> String {
    format!("Parameter '{key}' must not be negative, got {value}.")
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The bug: `3.0` is a JSON number that `as_u64` rejects, so the caller's
    /// value was replaced by the default without a word.
    #[test]
    fn an_integral_float_is_the_integer_it_looks_like() {
        let args = json!({ "max_pages": 3.0, "delay_ms": 1000.0 });
        assert_eq!(u64_or(&args, "max_pages", 500).unwrap(), 3);
        assert_eq!(u64_or(&args, "delay_ms", 250).unwrap(), 1000);
    }

    #[test]
    fn a_plain_integer_still_works() {
        let args = json!({ "max_pages": 3 });
        assert_eq!(u64_or(&args, "max_pages", 500).unwrap(), 3);
    }

    #[test]
    fn an_absent_parameter_takes_the_default() {
        let args = json!({});
        assert_eq!(u64_or(&args, "max_pages", 500).unwrap(), 500);
        assert_eq!(
            opt_u64(&json!({ "max_pages": null }), "max_pages").unwrap(),
            None
        );
    }

    /// The dangerous half: a value that cannot be honored must be refused, not
    /// swapped for a default that crawls harder than the caller asked for.
    #[test]
    fn unusable_values_are_refused_rather_than_defaulted() {
        for bad in [
            json!(2.5),
            json!(-1),
            json!(-2.5),
            json!("3"),
            json!(true),
            json!([3]),
            json!({}),
        ] {
            let args = json!({ "max_pages": bad });
            let err = u64_or(&args, "max_pages", 500)
                .expect_err(&format!("{bad} must be refused, not silently defaulted"));
            assert!(
                err.contains("max_pages"),
                "error should name the parameter: {err}"
            );
        }
    }

    /// A number too large to be a `u64` is refused rather than wrapped or
    /// defaulted. (`1e400` is not testable here: serde_json rejects a literal
    /// that overflows `f64` at parse time, so it never reaches this code.)
    #[test]
    fn out_of_range_numbers_are_refused() {
        let args: Value = serde_json::from_str(r#"{ "delay_ms": 1e30 }"#).unwrap();
        let err = u64_or(&args, "delay_ms", 250).unwrap_err();
        assert!(err.contains("out of range"), "{err}");
    }

    #[test]
    fn huge_but_valid_integers_are_accepted_for_the_caller_to_clamp() {
        let args = json!({ "max_pages": 1_000_000 });
        assert_eq!(u64_or(&args, "max_pages", 500).unwrap(), 1_000_000);
    }

    #[test]
    fn booleans_and_strings_are_read_strictly() {
        let args = json!({ "pagespeed": "yes", "backend": 5 });
        assert!(bool_or(&args, "pagespeed", false).is_err());
        assert!(str_or(&args, "backend", "local").is_err());
        assert!(!bool_or(&json!({}), "pagespeed", false).unwrap());
        assert_eq!(str_or(&json!({}), "backend", "local").unwrap(), "local");
    }

    #[test]
    fn one_of_names_the_alternatives() {
        assert!(one_of("mode", "static", &["static", "rendered"]).is_ok());
        let err = one_of("mode", "turbo", &["static", "rendered"]).unwrap_err();
        assert!(err.contains("static") && err.contains("rendered") && err.contains("turbo"));
    }
}
