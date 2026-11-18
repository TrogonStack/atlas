use prost_reflect::{FieldDescriptor, Kind, MessageDescriptor};
use serde_json::{Map, Value};
use trogon_atlas_core::transcode::TranscodePool;

pub(crate) fn validate(pool: &TranscodePool, descriptor: &MessageDescriptor, json: &Value) -> bool {
    Validator {
        pool,
        remaining: 1_000_000,
        defer_unknown_any: false,
    }
    .message(descriptor, json, 1)
}

pub(crate) fn validate_deferred(
    pool: &TranscodePool,
    descriptor: &MessageDescriptor,
    json: &Value,
) -> bool {
    Validator {
        pool,
        remaining: 1_000_000,
        defer_unknown_any: true,
    }
    .message(descriptor, json, 1)
}

struct Validator<'a> {
    pool: &'a TranscodePool,
    remaining: usize,
    defer_unknown_any: bool,
}

impl Validator<'_> {
    fn step(&mut self, depth: usize) -> bool {
        if depth > 32 || self.remaining == 0 {
            return false;
        }
        self.remaining -= 1;
        true
    }

    fn message(&mut self, descriptor: &MessageDescriptor, json: &Value, depth: usize) -> bool {
        if !self.step(depth) {
            return false;
        }
        match descriptor.full_name() {
            "google.protobuf.Value" => return self.json_value(json, depth),
            "google.protobuf.Struct" => return json.is_object() && self.json_value(json, depth),
            "google.protobuf.ListValue" => return json.is_array() && self.json_value(json, depth),
            "google.protobuf.Timestamp"
            | "google.protobuf.Duration"
            | "google.protobuf.FieldMask" => return json.is_string(),
            "google.protobuf.DoubleValue"
            | "google.protobuf.FloatValue"
            | "google.protobuf.Int64Value"
            | "google.protobuf.UInt64Value"
            | "google.protobuf.Int32Value"
            | "google.protobuf.UInt32Value"
            | "google.protobuf.BoolValue"
            | "google.protobuf.StringValue"
            | "google.protobuf.BytesValue" => {
                return descriptor
                    .get_field_by_name("value")
                    .is_some_and(|field| self.scalar(&field.kind(), json, depth));
            }
            "google.protobuf.Any" => return self.any(json, depth),
            _ => {}
        }
        let Some(object) = json.as_object() else {
            return false;
        };
        self.object(descriptor, object, depth)
    }

    fn object(
        &mut self,
        descriptor: &MessageDescriptor,
        object: &Map<String, Value>,
        depth: usize,
    ) -> bool {
        for oneof in descriptor.oneofs() {
            let count = oneof
                .fields()
                .filter(|field| object.contains_key(field.json_name()))
                .count();
            if count > 1 || (oneof.name() == "selection" && count != 1) {
                return false;
            }
        }
        object.iter().all(|(name, value)| {
            descriptor
                .fields()
                .find(|field| field.json_name() == name)
                .is_some_and(|field| self.field(&field, value, depth + 1))
        })
    }

    fn field(&mut self, field: &FieldDescriptor, json: &Value, depth: usize) -> bool {
        if !self.step(depth) || json.is_null() {
            return false;
        }
        if field.is_map() {
            let (Kind::Message(entry), Some(object)) = (field.kind(), json.as_object()) else {
                return false;
            };
            return object
                .values()
                .all(|value| self.scalar(&entry.map_entry_value_field().kind(), value, depth));
        }
        if field.is_list() {
            return json.as_array().is_some_and(|values| {
                values
                    .iter()
                    .all(|value| self.scalar(&field.kind(), value, depth))
            });
        }
        self.scalar(&field.kind(), json, depth)
    }

    fn scalar(&mut self, kind: &Kind, json: &Value, depth: usize) -> bool {
        if !self.step(depth) {
            return false;
        }
        match kind {
            Kind::Message(descriptor) => self.message(descriptor, json, depth),
            Kind::Enum(descriptor) => match json {
                Value::String(name) => descriptor.get_value_by_name(name).is_some(),
                Value::Number(number) => number
                    .as_i64()
                    .and_then(|value| i32::try_from(value).ok())
                    .and_then(|value| descriptor.get_value(value))
                    .is_some(),
                _ => false,
            },
            Kind::Bool => json.is_boolean(),
            Kind::String | Kind::Bytes => json.is_string(),
            Kind::Int32 | Kind::Sint32 | Kind::Sfixed32 => json
                .as_i64()
                .is_some_and(|value| i32::try_from(value).is_ok()),
            Kind::Uint32 | Kind::Fixed32 => json
                .as_u64()
                .is_some_and(|value| u32::try_from(value).is_ok()),
            Kind::Int64 | Kind::Sint64 | Kind::Sfixed64 => match json {
                Value::String(value) => integer_text(value) && value.parse::<i64>().is_ok(),
                _ => json.as_i64().is_some_and(|value| {
                    (-9_007_199_254_740_991..=9_007_199_254_740_991).contains(&value)
                }),
            },
            Kind::Uint64 | Kind::Fixed64 => match json {
                Value::String(value) => {
                    integer_text(value) && (value.parse::<u64>().is_ok() || value == "-0")
                }
                _ => json
                    .as_u64()
                    .is_some_and(|value| value <= 9_007_199_254_740_991),
            },
            Kind::Float | Kind::Double => {
                json.is_number() || matches!(json.as_str(), Some("NaN" | "Infinity" | "-Infinity"))
            }
        }
    }

    fn any(&mut self, json: &Value, depth: usize) -> bool {
        let Some(object) = json.as_object() else {
            return false;
        };
        let Some(type_url) = object.get("@type").and_then(Value::as_str) else {
            return false;
        };
        let Some(descriptor) = self
            .pool
            .descriptor_pool()
            .get_message_by_name(type_url.rsplit('/').next().unwrap_or_default())
        else {
            return self.defer_unknown_any && self.json_value(json, depth);
        };
        if descriptor.full_name().starts_with("google.protobuf.")
            && matches!(
                descriptor.name(),
                "Any"
                    | "Value"
                    | "Struct"
                    | "ListValue"
                    | "Timestamp"
                    | "Duration"
                    | "FieldMask"
                    | "DoubleValue"
                    | "FloatValue"
                    | "Int64Value"
                    | "UInt64Value"
                    | "Int32Value"
                    | "UInt32Value"
                    | "BoolValue"
                    | "StringValue"
                    | "BytesValue"
            )
        {
            return object.len() == 2
                && object
                    .get("value")
                    .is_some_and(|value| self.message(&descriptor, value, depth + 1));
        }
        let mut fields = object.clone();
        fields.remove("@type");
        self.object(&descriptor, &fields, depth)
    }

    fn json_value(&mut self, json: &Value, depth: usize) -> bool {
        if !self.step(depth) {
            return false;
        }
        match json {
            Value::Array(values) => values.iter().all(|value| self.json_value(value, depth + 1)),
            Value::Object(values) => values
                .values()
                .all(|value| self.json_value(value, depth + 1)),
            _ => true,
        }
    }
}

fn integer_text(value: &str) -> bool {
    let digits = value.strip_prefix('-').unwrap_or(value);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}
