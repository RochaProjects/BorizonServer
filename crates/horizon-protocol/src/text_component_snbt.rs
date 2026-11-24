//! Parser for SNBT-form text components used by vanilla command arguments.
//!
//! This module preserves component-shaped values as JSON without resolving
//! translations, styles, keybinds, or any other presentation behavior.
use serde_json::{Map, Value};

/// Parse one SNBT value as the JSON shape used for a text component.
///
/// Returns the parsed value and the number of input bytes consumed, allowing a
/// caller to continue parsing a surrounding command argument. Leading
/// whitespace is included in the consumed count; trailing whitespace is not.
pub fn parse(input: &str) -> Result<(Value, usize), String> {
    let mut parser = Parser {
        input: input.as_bytes(),
        offset: 0,
        depth: 0,
    };
    let value = parser.value()?;
    Ok((value, parser.offset))
}

/// Match vanilla's NBT nesting limit and reject pathological recursion.
const MAX_DEPTH: usize = 512;

struct Parser<'a> {
    input: &'a [u8],
    offset: usize,
    depth: usize,
}

impl Parser<'_> {
    fn whitespace(&mut self) {
        while self.offset < self.input.len() && self.input[self.offset].is_ascii_whitespace() {
            self.offset += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.offset).copied()
    }

    fn expect(&mut self, expected: u8) -> Result<(), String> {
        self.whitespace();
        if self.peek() == Some(expected) {
            self.offset += 1;
            Ok(())
        } else {
            Err(format!("expected '{}'", expected as char))
        }
    }

    fn value(&mut self) -> Result<Value, String> {
        self.whitespace();
        match self.peek() {
            Some(b'{') => {
                if self.depth >= MAX_DEPTH {
                    return Err(format!("tag nesting too deep at {}", self.offset));
                }
                self.depth += 1;
                let result = self.object();
                self.depth -= 1;
                result
            }
            Some(b'[') => {
                if self.depth >= MAX_DEPTH {
                    return Err(format!("tag nesting too deep at {}", self.offset));
                }
                self.depth += 1;
                let result = self.array();
                self.depth -= 1;
                result
            }
            Some(b'"' | b'\'') => Ok(Value::String(self.string()?)),
            Some(_) => {
                let word = self.unquoted();
                if word.is_empty() {
                    return Err("expected a value".into());
                }
                Ok(match word.as_str() {
                    "true" => Value::Bool(true),
                    "false" => Value::Bool(false),
                    _ => {
                        let digits = word
                            .trim_end_matches(['b', 'B', 's', 'S', 'l', 'L', 'f', 'F', 'd', 'D']);
                        // Alphabetic tokens other than exponent markers are
                        // strings, even when Rust would parse NaN/Infinity.
                        if digits
                            .bytes()
                            .any(|byte| byte.is_ascii_alphabetic() && byte != b'e' && byte != b'E')
                        {
                            Value::String(word)
                        } else {
                            match (digits.parse::<i64>(), digits.parse::<f64>()) {
                                (Ok(number), _) if digits.len() + 1 >= word.len() => {
                                    Value::from(number)
                                }
                                (_, Ok(number)) if digits.len() + 1 >= word.len() => {
                                    Value::from(number)
                                }
                                _ => Value::String(word),
                            }
                        }
                    }
                })
            }
            None => Err("expected a value".into()),
        }
    }

    fn object(&mut self) -> Result<Value, String> {
        self.offset += 1;
        let mut values = Map::new();
        self.whitespace();
        if self.peek() == Some(b'}') {
            self.offset += 1;
            return Ok(Value::Object(values));
        }
        loop {
            self.whitespace();
            let key = self.string()?;
            self.expect(b':')?;
            values.insert(key, self.value()?);
            self.whitespace();
            match self.peek() {
                Some(b',') => {
                    self.offset += 1;
                    self.whitespace();
                    if self.peek() == Some(b'}') {
                        self.offset += 1;
                        return Ok(Value::Object(values));
                    }
                }
                Some(b'}') => {
                    self.offset += 1;
                    return Ok(Value::Object(values));
                }
                _ => return Err("expected ',' or '}'".into()),
            }
        }
    }

    fn array(&mut self) -> Result<Value, String> {
        self.offset += 1;
        let mut values = Vec::new();
        self.whitespace();
        // Typed NBT array prefixes (B;), (I;), and (L;) do not change the
        // JSON array shape needed to represent text-component values.
        if matches!(
            self.input.get(self.offset..self.offset + 2),
            Some([b'B' | b'I' | b'L', b';'])
        ) {
            self.offset += 2;
        }
        self.whitespace();
        if self.peek() == Some(b']') {
            self.offset += 1;
            return Ok(Value::Array(values));
        }
        loop {
            values.push(self.value()?);
            self.whitespace();
            match self.peek() {
                Some(b',') => {
                    self.offset += 1;
                    self.whitespace();
                    if self.peek() == Some(b']') {
                        self.offset += 1;
                        return Ok(Value::Array(values));
                    }
                }
                Some(b']') => {
                    self.offset += 1;
                    return Ok(Value::Array(values));
                }
                _ => return Err("expected ',' or ']'".into()),
            }
        }
    }

    fn unquoted(&mut self) -> String {
        let start = self.offset;
        while let Some(byte) = self.peek() {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'+') {
                self.offset += 1;
            } else {
                break;
            }
        }
        String::from_utf8_lossy(&self.input[start..self.offset]).into_owned()
    }

    fn string(&mut self) -> Result<String, String> {
        self.whitespace();
        let Some(quote @ (b'"' | b'\'')) = self.peek() else {
            return Ok(self.unquoted());
        };
        self.offset += 1;
        let mut output = Vec::new();
        while let Some(byte) = self.peek() {
            self.offset += 1;
            match byte {
                b'\\' => {
                    let Some(escaped) = self.peek() else { break };
                    self.offset += 1;
                    output.push(match escaped {
                        b'n' => b'\n',
                        b't' => b'\t',
                        other => other,
                    });
                }
                byte if byte == quote => {
                    return Ok(String::from_utf8_lossy(&output).into_owned());
                }
                byte => output.push(byte),
            }
        }
        Err("unterminated string".into())
    }
}

#[cfg(test)]
mod tests {
    use super::parse;
    use serde_json::json;

    #[test]
    fn parses_component_objects_and_reports_consumed_bytes() {
        let input = r#" {text:"hello",color:red,extra:[{text:'world'}]} trailing"#;
        let (value, consumed) = parse(input).unwrap();
        assert_eq!(
            value,
            json!({"text":"hello", "color":"red", "extra":[{"text":"world"}]})
        );
        assert_eq!(&input[consumed..], " trailing");
    }

    #[test]
    fn parses_json_like_values_numbers_booleans_and_typed_arrays() {
        assert_eq!(
            parse(r#"["a",{"text":"b"}]"#).unwrap().0,
            json!(["a", {"text":"b"}])
        );
        assert_eq!(parse("[B;1b,0b,-1b,]").unwrap().0, json!([1, 0, -1]));
        assert_eq!(
            parse("{n:1.5,enabled:true,off:false,count:12L}").unwrap().0,
            json!({
                "n": 1.5, "enabled": true, "off": false, "count": 12
            })
        );
        assert_eq!(parse("{text:nan}").unwrap().0["text"], "nan");
        assert_eq!(parse("{text:Infinity}").unwrap().0["text"], "Infinity");
    }

    #[test]
    fn permits_one_trailing_comma_but_rejects_empty_entries() {
        assert_eq!(
            parse("{text:\"a\",color:\"red\",}").unwrap().0,
            parse("{text:\"a\",color:\"red\"}").unwrap().0
        );
        assert_eq!(
            parse("[\"a\",{text:\"b\",},]").unwrap().0,
            json!(["a", {"text":"b"}])
        );
        assert!(parse("{text:\"a\",,}").is_err());
        assert!(parse("[\"a\",,]").is_err());
    }

    #[test]
    fn strings_decode_escapes_and_reject_unterminated_quotes() {
        assert_eq!(
            parse(r#"{text:"a\nb\t\\\"c"}"#).unwrap().0["text"],
            "a\nb\t\\\"c"
        );
        assert!(parse("{text:\"unfinished}").is_err());
    }

    #[test]
    fn nesting_is_bounded_at_the_nbt_limit() {
        let nest = |count: usize| format!("{}0{}", "[".repeat(count), "]".repeat(count));
        assert!(parse(&nest(512)).is_ok());
        assert!(parse(&nest(513)).is_err());
        assert!(parse(&nest(100_000)).is_err());
    }

    #[test]
    fn malformed_and_empty_inputs_return_errors() {
        for input in ["", " ", "{text}", "[1 2]", "{text:\"x\""] {
            assert!(parse(input).is_err(), "accepted malformed input {input:?}");
        }
    }
}
