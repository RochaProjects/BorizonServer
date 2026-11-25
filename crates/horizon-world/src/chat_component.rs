//! Presentation-neutral chat-component JSON and NBT codecs.
//!
//! The functions here preserve component content as JSON and do not resolve
//! translations, styles for display, or any client presentation state. The
//! canonicalization rules follow vanilla `ComponentSerialization`.
use crate::nbt::Tag;
use serde_json::{Map, Value};

/// A text component in its JSON form.
pub type Component = Value;

/// Convert a component JSON value to its vanilla canonical JSON form.
///
/// A bare literal without style or siblings becomes a string; a component list
/// becomes its first element with the remaining elements appended as siblings;
/// the `type` discriminator is omitted, as in vanilla's codec.
pub fn canonical_component(component: &Component) -> Component {
    let (mut node, extra) = decompose(component);
    finish(&mut node, extra)
}

/// Canonicalize a JSON style object as vanilla's `Style.Serializer.MAP_CODEC`
/// re-encodes it. The `text` content field is not part of a style.
pub fn canonical_style(style: &Value) -> Value {
    let (mut node, _) = decompose(style);
    node.remove("text");
    Value::Object(node)
}

fn decompose(component: &Component) -> (Map<String, Value>, Vec<Component>) {
    match component {
        Value::Array(list) if !list.is_empty() => {
            let (mut node, mut extra) = decompose(&list[0]);
            extra.extend(list[1..].iter().map(canonical_component));
            node.remove("extra");
            (node, extra)
        }
        Value::Object(object) => {
            let mut node = Map::new();
            let mut extra = Vec::new();
            for (key, value) in object {
                match key.as_str() {
                    "type" => {}
                    "extra" => {
                        if let Value::Array(list) = value {
                            extra.extend(list.iter().map(canonical_component));
                        }
                    }
                    "bold" | "italic" | "underlined" | "strikethrough" | "obfuscated" => {
                        let flag = match value {
                            Value::Bool(flag) => *flag,
                            Value::Number(number) => {
                                number.as_f64().is_some_and(|value| value as i8 != 0)
                            }
                            _ => continue,
                        };
                        node.insert(key.clone(), Value::Bool(flag));
                    }
                    "color" => {
                        let color = value.as_str().map(|color| {
                            if color.starts_with('#') {
                                hex_color(color)
                                    .map(|rgb| format!("#{rgb:06X}"))
                                    .unwrap_or_else(|| color.to_owned())
                            } else {
                                color.to_owned()
                            }
                        });
                        if let Some(color) = color {
                            node.insert(key.clone(), Value::String(color));
                        }
                    }
                    "with" => {
                        if let Value::Array(args) = value {
                            let args = args
                                .iter()
                                .map(|arg| match arg {
                                    Value::Object(_) | Value::Array(_) => canonical_component(arg),
                                    other => other.clone(),
                                })
                                .collect();
                            node.insert(key.clone(), Value::Array(args));
                        }
                    }
                    "separator" => {
                        node.insert(key.clone(), canonical_component(value));
                    }
                    "hover_event" => {
                        let mut hover = value.clone();
                        if let Some(hover_text) = hover.get("value").cloned()
                            && hover.get("action").and_then(Value::as_str) == Some("show_text")
                        {
                            hover["value"] = canonical_component(&hover_text);
                        }
                        node.insert(key.clone(), hover);
                    }
                    _ => {
                        // Keep unknown component fields losslessly for forward compatibility.
                        node.insert(key.clone(), value.clone());
                    }
                }
            }
            (node, extra)
        }
        Value::String(text) => {
            let mut node = Map::new();
            node.insert("text".into(), Value::String(text.clone()));
            (node, Vec::new())
        }
        other => {
            let mut node = Map::new();
            node.insert("text".into(), Value::String(other.to_string()));
            (node, Vec::new())
        }
    }
}

fn finish(node: &mut Map<String, Value>, extra: Vec<Component>) -> Component {
    if extra.is_empty()
        && node.len() == 1
        && let Some(Value::String(text)) = node.get("text")
    {
        return Value::String(text.clone());
    }
    if !extra.is_empty() {
        node.insert("extra".into(), Value::Array(extra));
    }
    Value::Object(node.clone())
}

/// Parse vanilla's `TextColor.parseColor` form, including signed hexadecimal
/// values accepted by `Integer.parseInt`, retaining only 24-bit RGB values.
fn hex_color(color: &str) -> Option<u32> {
    let value = i64::from_str_radix(color.strip_prefix('#')?, 16).ok()?;
    (0..=0xFF_FFFF).contains(&value).then_some(value as u32)
}

/// Convert arbitrary JSON to the NBT value form used by component codecs.
pub fn json_to_tag(value: &Value) -> Tag {
    match value {
        Value::Null => Tag::String("".into()),
        Value::Bool(value) => Tag::Byte(i8::from(*value)),
        Value::Number(number) => match number.as_i64() {
            Some(value) => i32::try_from(value).map_or(Tag::Long(value), Tag::Int),
            None => Tag::Double(number.as_f64().unwrap_or(0.0).to_bits()),
        },
        Value::String(value) => Tag::String(value.as_str().into()),
        Value::Array(values) => Tag::List(values.iter().map(json_to_tag).collect()),
        Value::Object(values) => Tag::compound(
            values
                .iter()
                .map(|(key, value)| (key.as_str(), json_to_tag(value))),
        ),
    }
}

/// Convert NBT to JSON, retaining unknown component fields and values.
pub fn tag_to_json(tag: &Tag) -> Value {
    match tag {
        Tag::End => Value::Null,
        Tag::Byte(value) => Value::Bool(*value != 0),
        Tag::Short(value) => Value::from(*value),
        Tag::Int(value) => Value::from(*value),
        Tag::Long(value) => Value::from(*value),
        Tag::Float(value) => Value::from(f64::from(f32::from_bits(*value))),
        Tag::Double(value) => Value::from(f64::from_bits(*value)),
        Tag::String(value) => Value::String(value.to_string_lossy()),
        Tag::List(values) => Value::Array(values.iter().map(tag_to_json).collect()),
        Tag::Compound(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.to_string_lossy(), tag_to_json(value)))
                .collect(),
        ),
        Tag::ByteArray(values) => {
            Value::Array(values.iter().map(|value| Value::from(*value)).collect())
        }
        Tag::IntArray(values) => {
            Value::Array(values.iter().map(|value| Value::from(*value)).collect())
        }
        Tag::LongArray(values) => {
            Value::Array(values.iter().map(|value| Value::from(*value)).collect())
        }
    }
}

/// Encode a component as vanilla's `ComponentSerialization.CODEC` writes it to NBT.
pub fn component_tag(component: &Component) -> Tag {
    let mut tag = json_to_tag(&canonical_component(component));
    normalize_hover_entity_uuids(&mut tag);
    tag
}

/// Normalize `HoverEvent.ShowEntity` UUIDs to the vanilla four-int NBT form.
pub fn normalize_hover_entity_uuids(tag: &mut Tag) {
    match tag {
        Tag::Compound(compound) => {
            let show_entity = matches!(
                compound.get(&"action".into()),
                Some(Tag::String(action)) if action.to_string_lossy() == "show_entity"
            );
            for (key, value) in compound.iter_mut() {
                if show_entity && key.to_string_lossy() == "uuid" {
                    let ints = match value {
                        Tag::List(list) => list
                            .iter()
                            .map(|tag| match tag {
                                Tag::Int(value) => Some(*value),
                                _ => None,
                            })
                            .collect::<Option<Vec<i32>>>(),
                        Tag::String(text) => {
                            horizon_core::uuid::uuid_from_text(&text.to_string_lossy())
                                .map(Vec::from)
                        }
                        _ => None,
                    };
                    if let Some(ints) = ints.filter(|ints| ints.len() == 4) {
                        *value = Tag::IntArray(ints);
                    }
                } else {
                    normalize_hover_entity_uuids(value);
                }
            }
        }
        Tag::List(list) => list.iter_mut().for_each(normalize_hover_entity_uuids),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalizes_component_shape_style_and_hex_color() {
        let input = serde_json::json!({
            "type": "text",
            "text": "hello",
            "bold": 1,
            "color": "#+FF",
            "ignored_future_field": {"opaque": [1, 2]},
            "extra": [{"text": "tail"}]
        });
        assert_eq!(
            canonical_component(&input),
            serde_json::json!({
                "text": "hello",
                "bold": true,
                "color": "#0000FF",
                "ignored_future_field": {"opaque": [1, 2]},
                "extra": ["tail"]
            })
        );
    }

    #[test]
    fn canonicalizes_lists_and_nested_translation_arguments() {
        let input = serde_json::json!([
            {"text": "first", "extra": [{"text": "sibling"}]},
            {"translate": "example.key", "with": [{"text": "arg"}]}
        ]);
        assert_eq!(
            canonical_component(&input),
            serde_json::json!({
                "text": "first",
                "extra": ["sibling", {"translate": "example.key", "with": ["arg"]}]
            })
        );
    }

    #[test]
    fn style_canonicalization_excludes_component_text_and_preserves_extensions() {
        let style = serde_json::json!({
            "text": "not style data",
            "bold": 1,
            "future_style": {"opaque": true}
        });
        assert_eq!(
            canonical_style(&style),
            serde_json::json!({"bold": true, "future_style": {"opaque": true}})
        );
    }

    #[test]
    fn component_nbt_codec_preserves_unknown_fields_and_uuid_shape() {
        let component = serde_json::json!({
            "text": "future",
            "extension": {"value": [true, 7]},
            "hover_event": {
                "action": "show_entity",
                "id": "minecraft:pig",
                "uuid": "00112233-4455-6677-8899-aabbccddeeff"
            }
        });
        let tag = component_tag(&component);
        let hover = tag.get("hover_event").unwrap();
        assert_eq!(
            hover.get("uuid"),
            Some(&Tag::IntArray(vec![
                0x0011_2233,
                0x4455_6677,
                0x8899_aabb_u32 as i32,
                0xccdd_eeff_u32 as i32
            ]))
        );
        let json = tag_to_json(&tag);
        assert_eq!(json["extension"], component["extension"]);
        assert_eq!(json["text"], "future");
    }
}
