//! JSON with JavaScript's object semantics, for data whose key order is part
//! of the evidence.
//!
//! `serde_json::Value` (without the `preserve_order` feature, which this
//! workspace deliberately does not enable) sorts object keys. A mined corpus
//! record, a regenerated refuter prompt and the finding JSON embedded in it all
//! depend on the key order `JSON.parse` produced, so those values are held as
//! [`JsValue`], which keeps JavaScript's property order: integer-index keys
//! first in ascending numeric order, then every other key in insertion order,
//! and a duplicate key keeps its first position with its last value.
//!
//! [`JsValue::stringify`] and [`JsValue::stringify_pretty`] are
//! `JSON.stringify(v)` and `JSON.stringify(v, null, 2)`, including number
//! formatting via [`fmt_number`](super::jsnum::fmt_number). Its `Serialize`
//! impl keeps the order too, so a `JsValue` can sit inside a typed report.

use std::cmp::Ordering;
use std::fmt;

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};

use super::jsnum::{fmt_number, json_number, serialize_js_number};

/// A JSON value with JavaScript property order.
#[derive(Debug, Clone, PartialEq)]
pub enum JsValue {
    /// `null`.
    Null,
    /// A boolean.
    Bool(bool),
    /// A number (every JSON number is a double in JavaScript).
    Number(f64),
    /// A string.
    String(String),
    /// An array.
    Array(Vec<JsValue>),
    /// An object, in JavaScript property order.
    Object(JsObject),
}

/// An object's properties in JavaScript order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct JsObject(Vec<(String, JsValue)>);

/// The array-index value of `key` (a canonical decimal below 2^32 - 1).
fn array_index(key: &str) -> Option<u32> {
    if key.is_empty() || key.len() > 10 || !key.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if key.len() > 1 && key.starts_with('0') {
        return None;
    }
    key.parse::<u64>()
        .ok()
        .filter(|&n| n < u64::from(u32::MAX))
        .and_then(|n| u32::try_from(n).ok())
}

impl JsObject {
    /// An empty object.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets `key`, following JavaScript property-order rules.
    pub fn insert(&mut self, key: impl Into<String>, value: JsValue) {
        let key = key.into();
        if let Some(slot) = self.0.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = value;
            return;
        }
        match array_index(&key) {
            Some(idx) => {
                let at = self
                    .0
                    .iter()
                    .position(|(k, _)| array_index(k).is_none_or(|other| other > idx))
                    .unwrap_or(self.0.len());
                self.0.insert(at, (key, value));
            }
            None => self.0.push((key, value)),
        }
    }

    /// The value at `key`.
    pub fn get(&self, key: &str) -> Option<&JsValue> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Whether `key` is present (`key in obj`).
    pub fn contains_key(&self, key: &str) -> bool {
        self.0.iter().any(|(k, _)| k == key)
    }

    /// Removes `key`, returning its value.
    pub fn remove(&mut self, key: &str) -> Option<JsValue> {
        let at = self.0.iter().position(|(k, _)| k == key)?;
        Some(self.0.remove(at).1)
    }

    /// The properties, in order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &JsValue)> {
        self.0.iter().map(|(k, v)| (k, v))
    }

    /// The keys, in order.
    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.0.iter().map(|(k, _)| k)
    }

    /// The number of properties.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are no properties.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl FromIterator<(String, JsValue)> for JsObject {
    fn from_iter<I: IntoIterator<Item = (String, JsValue)>>(iter: I) -> Self {
        let mut obj = Self::new();
        for (k, v) in iter {
            obj.insert(k, v);
        }
        obj
    }
}

impl JsValue {
    /// Parses JSON text (`JSON.parse`).
    ///
    /// # Errors
    ///
    /// The parser's error when `text` is not JSON.
    pub fn parse(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// `{}`.
    pub fn empty_object() -> Self {
        Self::Object(JsObject::new())
    }

    /// The object, when this is one.
    pub fn as_object(&self) -> Option<&JsObject> {
        match self {
            Self::Object(o) => Some(o),
            _ => None,
        }
    }

    /// The string, when this is one.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    /// The number, when this is one.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// The boolean, when this is one.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The array, when this is one.
    pub fn as_array(&self) -> Option<&Vec<JsValue>> {
        match self {
            Self::Array(a) => Some(a),
            _ => None,
        }
    }

    /// Whether this is a plain (non-array) object.
    pub fn is_object(&self) -> bool {
        matches!(self, Self::Object(_))
    }

    /// Object member `key` (`None` for a missing key or a non-object).
    pub fn get(&self, key: &str) -> Option<&JsValue> {
        self.as_object().and_then(|o| o.get(key))
    }

    /// JavaScript truthiness.
    pub fn truthy(&self) -> bool {
        match self {
            Self::Null => false,
            Self::Bool(b) => *b,
            Self::Number(n) => *n != 0.0 && !n.is_nan(),
            Self::String(s) => !s.is_empty(),
            Self::Array(_) | Self::Object(_) => true,
        }
    }

    /// `String(v)`.
    pub fn js_string(&self) -> String {
        match self {
            Self::Null => "null".to_owned(),
            Self::Bool(b) => b.to_string(),
            Self::Number(n) => fmt_number(*n),
            Self::String(s) => s.clone(),
            Self::Array(a) => a
                .iter()
                .map(|v| match v {
                    Self::Null => String::new(),
                    other => other.js_string(),
                })
                .collect::<Vec<_>>()
                .join(","),
            Self::Object(_) => "[object Object]".to_owned(),
        }
    }

    /// `JSON.stringify(v)`.
    pub fn stringify(&self) -> String {
        let mut out = String::new();
        write_value(&mut out, self, None, 0);
        out
    }

    /// `JSON.stringify(v, null, 2)`.
    pub fn stringify_pretty(&self) -> String {
        let mut out = String::new();
        write_value(&mut out, self, Some(2), 0);
        out
    }

    /// Converts to a `serde_json::Value` (key order is lost; numbers that are
    /// integral become JSON integers).
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Null => serde_json::Value::Null,
            Self::Bool(b) => serde_json::Value::Bool(*b),
            Self::Number(n) => json_number(*n),
            Self::String(s) => serde_json::Value::String(s.clone()),
            Self::Array(a) => serde_json::Value::Array(a.iter().map(Self::to_json).collect()),
            Self::Object(o) => {
                serde_json::Value::Object(o.iter().map(|(k, v)| (k.clone(), v.to_json())).collect())
            }
        }
    }

    /// Converts from a `serde_json::Value` (sorted-key order).
    pub fn from_json(value: &serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => Self::Null,
            serde_json::Value::Bool(b) => Self::Bool(*b),
            serde_json::Value::Number(n) => Self::Number(n.as_f64().unwrap_or(f64::NAN)),
            serde_json::Value::String(s) => Self::String(s.clone()),
            serde_json::Value::Array(a) => Self::Array(a.iter().map(Self::from_json).collect()),
            serde_json::Value::Object(o) => Self::Object(
                o.iter()
                    .map(|(k, v)| (k.clone(), Self::from_json(v)))
                    .collect(),
            ),
        }
    }
}

/// `JSON.stringify` of a string.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    write_string(&mut out, s);
    out
}

fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn newline(out: &mut String, indent: Option<usize>, depth: usize) {
    if let Some(step) = indent {
        out.push('\n');
        out.push_str(&" ".repeat(step * depth));
    }
}

fn write_value(out: &mut String, v: &JsValue, indent: Option<usize>, depth: usize) {
    match v {
        JsValue::Null => out.push_str("null"),
        JsValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        JsValue::Number(n) => {
            if n.is_finite() {
                out.push_str(&fmt_number(*n));
            } else {
                out.push_str("null");
            }
        }
        JsValue::String(s) => write_string(out, s),
        JsValue::Array(a) => {
            if a.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, item) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, indent, depth + 1);
                write_value(out, item, indent, depth + 1);
            }
            newline(out, indent, depth);
            out.push(']');
        }
        JsValue::Object(o) => {
            if o.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (i, (k, item)) in o.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, indent, depth + 1);
                write_string(out, k);
                out.push(':');
                if indent.is_some() {
                    out.push(' ');
                }
                write_value(out, item, indent, depth + 1);
            }
            newline(out, indent, depth);
            out.push('}');
        }
    }
}

impl Serialize for JsValue {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Null => s.serialize_unit(),
            Self::Bool(b) => s.serialize_bool(*b),
            Self::Number(n) => serialize_js_number(n, s),
            Self::String(v) => s.serialize_str(v),
            Self::Array(a) => {
                let mut seq = s.serialize_seq(Some(a.len()))?;
                for item in a {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Self::Object(o) => o.serialize(s),
        }
    }
}

impl Serialize for JsObject {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(self.0.len()))?;
        for (k, v) in &self.0 {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

struct JsValueVisitor;

impl<'de> Visitor<'de> for JsValueVisitor {
    type Value = JsValue;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<JsValue, E> {
        Ok(JsValue::Bool(v))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<JsValue, E> {
        #[allow(clippy::cast_precision_loss)]
        Ok(JsValue::Number(v as f64))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<JsValue, E> {
        #[allow(clippy::cast_precision_loss)]
        Ok(JsValue::Number(v as f64))
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<JsValue, E> {
        Ok(JsValue::Number(v))
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<JsValue, E> {
        Ok(JsValue::String(v.to_owned()))
    }

    fn visit_string<E: de::Error>(self, v: String) -> Result<JsValue, E> {
        Ok(JsValue::String(v))
    }

    fn visit_unit<E: de::Error>(self) -> Result<JsValue, E> {
        Ok(JsValue::Null)
    }

    fn visit_none<E: de::Error>(self) -> Result<JsValue, E> {
        Ok(JsValue::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<JsValue, A::Error> {
        let mut out = Vec::new();
        while let Some(item) = seq.next_element()? {
            out.push(item);
        }
        Ok(JsValue::Array(out))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<JsValue, A::Error> {
        let mut obj = JsObject::new();
        while let Some((k, v)) = map.next_entry::<String, JsValue>()? {
            obj.insert(k, v);
        }
        Ok(JsValue::Object(obj))
    }
}

impl<'de> Deserialize<'de> for JsValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(JsValueVisitor)
    }
}

/// JavaScript string comparison (`a < b`): UTF-16 code-unit order.
pub fn js_str_cmp(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// JavaScript `String.prototype.length` (UTF-16 code units).
pub fn js_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// Structural equality under JavaScript `===` for primitives: two numbers
/// compare numerically, two strings by content; objects and arrays are never
/// `===` to a different parsed value.
pub fn strict_equals(a: &JsValue, b: &JsValue) -> bool {
    match (a, b) {
        (JsValue::Null, JsValue::Null) => true,
        (JsValue::Bool(x), JsValue::Bool(y)) => x == y,
        (JsValue::Number(x), JsValue::Number(y)) => x == y,
        (JsValue::String(x), JsValue::String(y)) => x == y,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_keeps_js_property_order() {
        let v = JsValue::parse(r#"{"b":1,"a":2,"10":3,"2":4,"b":5}"#).unwrap_or(JsValue::Null);
        let keys: Vec<&String> = v
            .as_object()
            .map(|o| o.keys().collect())
            .unwrap_or_default();
        assert_eq!(keys, ["2", "10", "b", "a"]);
        assert_eq!(v.get("b"), Some(&JsValue::Number(5.0)));
    }

    #[test]
    fn stringify_matches_json_stringify() {
        let v = JsValue::parse(
            r#"{"z":[1,2.5,{"q":"a\"b\n"}],"a":{},"e":[],"n":null,"t":true,"f":90.0}"#,
        )
        .unwrap_or(JsValue::Null);
        assert_eq!(
            v.stringify(),
            r#"{"z":[1,2.5,{"q":"a\"b\n"}],"a":{},"e":[],"n":null,"t":true,"f":90}"#
        );
        assert_eq!(
            v.stringify_pretty(),
            "{\n  \"z\": [\n    1,\n    2.5,\n    {\n      \"q\": \"a\\\"b\\n\"\n    }\n  ],\n  \"a\": {},\n  \"e\": [],\n  \"n\": null,\n  \"t\": true,\n  \"f\": 90\n}"
        );
        let serde = serde_json::to_string(&v).unwrap_or_default();
        assert_eq!(serde, v.stringify());
        assert_eq!(quote("\u{1}"), "\"\\u0001\"");
    }

    #[test]
    fn js_string_and_order_helpers() {
        assert_eq!(JsValue::Number(3.0).js_string(), "3");
        assert_eq!(JsValue::Null.js_string(), "null");
        assert_eq!(js_str_cmp("a", "b"), Ordering::Less);
        // U+FF5E sorts after U+1F600 in UTF-8 order but before it in UTF-16.
        assert_eq!(js_str_cmp("\u{ff5e}", "\u{1f600}"), Ordering::Greater);
        assert_eq!(js_len("\u{1f600}"), 2);
    }
}

/// A string-keyed map that serializes in insertion order (a JavaScript object
/// built by assignment, for example a tally keyed by first appearance).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct JsMap<V>(pub Vec<(String, V)>);

impl<V> JsMap<V> {
    /// An empty map.
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// The value at `key`.
    pub fn get(&self, key: &str) -> Option<&V> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// The value at `key`, inserting `init()` first if absent.
    pub fn entry(&mut self, key: &str, init: impl FnOnce() -> V) -> &mut V {
        let at = match self.0.iter().position(|(k, _)| k == key) {
            Some(at) => at,
            None => {
                self.0.push((key.to_owned(), init()));
                self.0.len() - 1
            }
        };
        &mut self.0[at].1
    }

    /// The entries, in order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &V)> {
        self.0.iter().map(|(k, v)| (k, v))
    }

    /// Whether the map is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<V: Serialize> Serialize for JsMap<V> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(self.0.len()))?;
        for (k, v) in &self.0 {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

/// Builds a [`JsValue::Object`] from `(key, value)` pairs, in order.
pub fn obj<I, K>(pairs: I) -> JsValue
where
    I: IntoIterator<Item = (K, JsValue)>,
    K: Into<String>,
{
    let mut o = JsObject::new();
    for (k, v) in pairs {
        o.insert(k, v);
    }
    JsValue::Object(o)
}

impl From<&str> for JsValue {
    fn from(s: &str) -> Self {
        Self::String(s.to_owned())
    }
}

impl From<String> for JsValue {
    fn from(s: String) -> Self {
        Self::String(s)
    }
}

impl From<bool> for JsValue {
    fn from(b: bool) -> Self {
        Self::Bool(b)
    }
}

impl From<f64> for JsValue {
    fn from(n: f64) -> Self {
        Self::Number(n)
    }
}

impl From<usize> for JsValue {
    fn from(n: usize) -> Self {
        #[allow(clippy::cast_precision_loss)]
        Self::Number(n as f64)
    }
}

impl<T: Into<JsValue>> From<Option<T>> for JsValue {
    fn from(v: Option<T>) -> Self {
        v.map_or(Self::Null, Into::into)
    }
}
