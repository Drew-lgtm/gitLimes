//! Just enough JSON to emit records, without a serialisation dependency.
//!
//! Output is newline-delimited JSON: one self-contained object per line, never
//! a wrapping array. That is deliberate - an array would have to be closed at
//! the end, which means either buffering the whole result or emitting something
//! that is invalid until the process exits. NDJSON stays streamable, survives
//! `head`, and `jq -s` collects it into an array when a consumer wants one.
//!
//! # Schema stability
//!
//! There is deliberately **no version field** in the output. A version stamp on
//! every line would cost bytes on every commit and, more to the point, would
//! not prevent the failure it appears to guard against: the real risk is a
//! field being renamed by accident, and a consumer reading `v` does not stop
//! that. `tests/cli.rs` pins the exact key set of every command instead, so a
//! rename fails CI rather than someone's script. The tool's own version, from
//! `gitlimes --version`, identifies the format.
//!
//! The contract is **additive**: new fields may appear at any time, so
//! consumers must ignore keys they do not recognise. Existing fields are not
//! renamed, retyped or removed without a version bump that says so in
//! CHANGELOG.md. Some keys are conditional and absent rather than null when
//! they do not apply - `head` on a commit, `track` on a branch, `added` and
//! `removed` without `--lines`.

/// Appends `s` to `out` as a quoted JSON string.
///
/// Escapes what RFC 8259 requires: the quote, the backslash, and every control
/// character below 0x20. Anything else is passed through as UTF-8, so commit
/// subjects keep their accents instead of turning into escape soup.
pub fn quote(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Builds one JSON object, tracking commas so a missing one cannot slip through.
pub struct Obj {
    buf: String,
    empty: bool,
}

impl Default for Obj {
    fn default() -> Obj {
        // Not derived: a derived Default would leave the buffer empty and
        // `empty` false, so the first key would emit a leading comma and no
        // opening brace - malformed JSON from a perfectly ordinary call.
        Obj::new()
    }
}

impl Obj {
    pub fn new() -> Obj {
        Obj {
            buf: String::from("{"),
            empty: true,
        }
    }

    fn key(&mut self, key: &str) {
        if !self.empty {
            self.buf.push(',');
        }
        self.empty = false;
        quote(&mut self.buf, key);
        self.buf.push(':');
    }

    pub fn str(&mut self, key: &str, value: &str) -> &mut Obj {
        self.key(key);
        let mut v = String::new();
        quote(&mut v, value);
        self.buf.push_str(&v);
        self
    }

    /// Omits the key entirely when the value is empty, so consumers can tell
    /// "no refs" from "a ref whose name is the empty string".
    pub fn str_opt(&mut self, key: &str, value: &str) -> &mut Obj {
        if value.is_empty() {
            return self;
        }
        self.str(key, value)
    }

    pub fn num(&mut self, key: &str, value: i64) -> &mut Obj {
        self.key(key);
        self.buf.push_str(&value.to_string());
        self
    }

    pub fn bool(&mut self, key: &str, value: bool) -> &mut Obj {
        self.key(key);
        self.buf.push_str(if value { "true" } else { "false" });
        self
    }

    pub fn strs<'a>(&mut self, key: &str, values: impl Iterator<Item = &'a str>) -> &mut Obj {
        self.key(key);
        self.buf.push('[');
        for (i, v) in values.enumerate() {
            if i > 0 {
                self.buf.push(',');
            }
            let mut q = String::new();
            quote(&mut q, v);
            self.buf.push_str(&q);
        }
        self.buf.push(']');
        self
    }

    pub fn nums(&mut self, key: &str, values: impl Iterator<Item = i64>) -> &mut Obj {
        self.key(key);
        self.buf.push('[');
        for (i, v) in values.enumerate() {
            if i > 0 {
                self.buf.push(',');
            }
            self.buf.push_str(&v.to_string());
        }
        self.buf.push(']');
        self
    }

    /// Nests an already-built object under `key`.
    pub fn obj(&mut self, key: &str, value: Obj) -> &mut Obj {
        self.key(key);
        self.buf.push_str(&value.finish());
        self
    }

    /// Inserts pre-built JSON verbatim, for shapes the typed methods do not
    /// cover (an array of arrays, say). The caller owns its validity.
    pub fn raw(&mut self, key: &str, json: &str) -> &mut Obj {
        self.key(key);
        self.buf.push_str(json);
        self
    }

    pub fn finish(self) -> String {
        let mut s = self.buf;
        s.push('}');
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(s: &str) -> String {
        let mut out = String::new();
        quote(&mut out, s);
        out
    }

    #[test]
    fn quoting_escapes_what_json_requires() {
        assert_eq!(q("plain"), r#""plain""#);
        assert_eq!(q(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(q(r"back\slash"), r#""back\\slash""#);
        assert_eq!(q("two\nlines"), r#""two\nlines""#);
        assert_eq!(q("tab\there"), r#""tab\there""#);
    }

    #[test]
    fn control_characters_become_unicode_escapes() {
        // The separators this tool uses internally must never reach the
        // output as raw control bytes, which would be invalid JSON.
        assert_eq!(q("\u{1f}"), "\"\\u001f\"");
        assert_eq!(q("\u{1e}"), "\"\\u001e\"");
        assert_eq!(q("\u{00}"), "\"\\u0000\"");
    }

    #[test]
    fn non_ascii_is_passed_through_not_escaped() {
        assert_eq!(q("Přílíš žluťoučký kůň"), "\"Přílíš žluťoučký kůň\"");
    }

    #[test]
    fn objects_place_their_own_commas() {
        let mut o = Obj::new();
        o.str("a", "1").num("b", 2).bool("c", true);
        assert_eq!(o.finish(), r#"{"a":"1","b":2,"c":true}"#);
    }

    #[test]
    fn an_empty_object_is_still_valid() {
        assert_eq!(Obj::new().finish(), "{}");
    }

    #[test]
    fn arrays_and_nesting() {
        let mut inner = Obj::new();
        inner.num("col", 0);
        let mut o = Obj::new();
        o.strs("parents", ["aa", "bb"].into_iter())
            .nums("counts", [1i64, 2].into_iter())
            .obj("graph", inner);
        assert_eq!(
            o.finish(),
            r#"{"parents":["aa","bb"],"counts":[1,2],"graph":{"col":0}}"#
        );
    }

    #[test]
    fn empty_arrays_are_emitted_as_empty_not_omitted() {
        let mut o = Obj::new();
        o.strs("refs", std::iter::empty());
        assert_eq!(o.finish(), r#"{"refs":[]}"#);
    }

    #[test]
    fn default_produces_the_same_valid_object_as_new() {
        // A derived Default would open no brace and lead with a comma, so an
        // ordinary `Obj::default()` would emit malformed JSON.
        let mut o = Obj::default();
        o.num("a", 1);
        assert_eq!(o.finish(), r#"{"a":1}"#);
        assert_eq!(Obj::default().finish(), "{}");
    }

    #[test]
    fn str_opt_omits_empty_values() {
        let mut o = Obj::new();
        o.str_opt("a", "").str_opt("b", "x");
        assert_eq!(o.finish(), r#"{"b":"x"}"#);
    }
}
