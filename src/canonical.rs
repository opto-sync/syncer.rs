//! Canonical JSON serialization, byte-compatible with the C core's yyjson
//! writer so every engine in the ecosystem emits identical merge output.
//!
//! yyjson's finite-double format, restated over the shortest round-trip
//! digits `d[0].d[1..] × 10^n`:
//! - `n` in `[-6, 20]`: fixed notation with at least one fractional digit
//!   (`1230.0`, `0.000001`), zero-prefixed below one (`0.05526`).
//! - otherwise: scientific `d.ddde<n>` — no `+` on positive exponents, no
//!   zero padding, and no `.0` for a single-digit significand (`2e34`).
//! - zero is `0.0` (sign-preserving: `-0.0`).
//!
//! Everything else (integers, strings, structure) already matches
//! serde_json's compact output byte-for-byte.

use std::io;

use serde::Serialize;
use serde_json::ser::{Formatter, Serializer};
use serde_json::Value;

/// Serializes a value to compact JSON with yyjson-compatible doubles.
pub(crate) fn to_canonical_string(value: &Value) -> serde_json::Result<String> {
    let mut out = Vec::with_capacity(128);
    let mut serializer = Serializer::with_formatter(&mut out, YyjsonNumberFormatter);
    value.serialize(&mut serializer)?;
    Ok(String::from_utf8(out).expect("serde_json writes UTF-8"))
}

struct YyjsonNumberFormatter;

impl Formatter for YyjsonNumberFormatter {
    fn write_f64<W>(&mut self, writer: &mut W, value: f64) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        writer.write_all(format_f64_yyjson(value).as_bytes())
    }

    fn write_f32<W>(&mut self, writer: &mut W, value: f32) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        self.write_f64(writer, value as f64)
    }
}

fn format_f64_yyjson(value: f64) -> String {
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0.0".to_owned()
        } else {
            "0.0".to_owned()
        };
    }

    // `{:e}` renders the shortest round-trip significand and the decimal
    // exponent of the leading digit, e.g. "-4.5526e1".
    let rendered = format!("{value:e}");
    let (mantissa, exponent) = rendered
        .split_once('e')
        .expect("LowerExp always contains an exponent");
    let n: i32 = exponent.parse().expect("LowerExp exponent is an integer");
    let (sign, mantissa) = match mantissa.strip_prefix('-') {
        Some(unsigned) => ("-", unsigned),
        None => ("", mantissa),
    };
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();

    let dot_offset = n + 1; // digits before the decimal point in fixed form
    if (-5..=21).contains(&dot_offset) {
        if dot_offset <= 0 {
            let zeros = "0".repeat(dot_offset.unsigned_abs() as usize);
            format!("{sign}0.{zeros}{digits}")
        } else if dot_offset as usize >= digits.len() {
            let zeros = "0".repeat(dot_offset as usize - digits.len());
            format!("{sign}{digits}{zeros}.0")
        } else {
            let (integral, fractional) = digits.split_at(dot_offset as usize);
            format!("{sign}{integral}.{fractional}")
        }
    } else {
        let (head, tail) = digits.split_at(1);
        if tail.is_empty() {
            format!("{sign}{head}e{n}")
        } else {
            format!("{sign}{head}.{tail}e{n}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::format_f64_yyjson;
    use crate::{merge_json, MergeOptions};

    #[test]
    fn merge_output_is_yyjson_canonical() {
        // parse must be correctly rounded (float_roundtrip) and the writer
        // must match yyjson, or cross-engine byte parity breaks.
        let cases: &[(&str, &str)] = &[
            ("9e29", "9e29"),
            ("2e34", "2e34"),
            ("6e34", "6e34"),
            ("3e25", "3e25"),
            ("1e11", "100000000000.0"),
            ("7e19", "70000000000000000000.0"),
            ("123.0", "123.0"),
            ("-0.0", "-0.0"),
        ];
        for (input, expected) in cases {
            let merged = merge_json("null", input, &MergeOptions::default()).unwrap();
            assert_eq!(&merged, expected, "input {input}");
        }
    }

    #[test]
    fn doubles_match_the_yyjson_writer() {
        let cases: &[(f64, &str)] = &[
            (0.0, "0.0"),
            (-0.0, "-0.0"),
            (1.0, "1.0"),
            (123.0, "123.0"),
            (45.526, "45.526"),
            (0.0001, "0.0001"),
            (0.000001, "0.000001"),
            (1e-7, "1e-7"),
            (1e11, "100000000000.0"),
            (7e12, "7000000000000.0"),
            (1e20, "100000000000000000000.0"),
            (1e21, "1e21"),
            (2e34, "2e34"),
            (9e37, "9e37"),
            (1.234e56, "1.234e56"),
            (5e-324, "5e-324"),
            (-2.5e-10, "-2.5e-10"),
            (f64::MAX, "1.7976931348623157e308"),
        ];
        for (value, expected) in cases {
            assert_eq!(format_f64_yyjson(*value), *expected, "value {value:e}");
        }
    }
}
