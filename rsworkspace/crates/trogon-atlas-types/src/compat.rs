use std::{collections::BTreeMap, fmt};

use prost_types::{
    field_descriptor_proto::Type, DescriptorProto, EnumDescriptorProto, FieldDescriptorProto,
    FileDescriptorSet,
};

/// A change that would stop existing binary or JSON payloads from decoding
/// the way their producers meant them to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum BreakingChange {
    PackageChanged {
        file: String,
        from: String,
        to: String,
    },
    MessageRemoved {
        message: String,
    },
    FieldRemoved {
        message: String,
        number: i32,
        name: String,
    },
    FieldNumberReused {
        message: String,
        number: i32,
        from: String,
        to: String,
    },
    FieldCardinalityChanged {
        message: String,
        field: String,
    },
    FieldOneofChanged {
        message: String,
        field: String,
        from: String,
        to: String,
    },
    FieldJsonNameChanged {
        message: String,
        field: String,
        from: String,
        to: String,
    },
    EnumRemoved {
        name: String,
    },
    EnumValueRemoved {
        name: String,
        value: String,
        number: i32,
    },
    EnumValueRenumbered {
        name: String,
        value: String,
        from: i32,
        to: i32,
    },
}

impl fmt::Display for BreakingChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PackageChanged { file, from, to } => {
                write!(f, "{file}: package changed from {from} to {to}")
            }
            Self::MessageRemoved { message } => write!(f, "message {message} was removed"),
            Self::FieldRemoved {
                message,
                number,
                name,
            } => write!(
                f,
                "{message}: field {name} = {number} was removed without reserving its number and name"
            ),
            Self::FieldNumberReused {
                message,
                number,
                from,
                to,
            } => write!(f, "{message}: field number {number} changed from {from} to {to}"),
            Self::FieldCardinalityChanged { message, field } => {
                write!(f, "{message}: field {field} changed cardinality")
            }
            Self::FieldOneofChanged {
                message,
                field,
                from,
                to,
            } => write!(
                f,
                "{message}: field {field} moved from oneof {from:?} to oneof {to:?}"
            ),
            Self::FieldJsonNameChanged {
                message,
                field,
                from,
                to,
            } => write!(
                f,
                "{message}: field {field} json_name changed from {from} to {to}"
            ),
            Self::EnumRemoved { name } => write!(f, "enum {name} was removed"),
            Self::EnumValueRemoved {
                name,
                value,
                number,
            } => write!(
                f,
                "{name}: value {value} = {number} was removed without reserving its number and name"
            ),
            Self::EnumValueRenumbered {
                name,
                value,
                from,
                to,
            } => write!(f, "{name}: value {value} renumbered from {from} to {to}"),
        }
    }
}

/// Every wire- or JSON-breaking difference from `previous` to `next`, in a
/// stable order.
pub fn breaking_changes(
    previous: &FileDescriptorSet,
    next: &FileDescriptorSet,
) -> Vec<BreakingChange> {
    let mut out = Vec::new();

    let next_packages: BTreeMap<&str, &str> =
        next.file.iter().map(|f| (f.name(), f.package())).collect();
    for file in &previous.file {
        if let Some(&to) = next_packages.get(file.name()) {
            if to != file.package() {
                out.push(BreakingChange::PackageChanged {
                    file: file.name().to_owned(),
                    from: file.package().to_owned(),
                    to: to.to_owned(),
                });
            }
        }
    }

    let (old_messages, old_enums) = index(previous);
    let (new_messages, new_enums) = index(next);
    for (name, old) in &old_messages {
        match new_messages.get(name) {
            Some(new) => compare_messages(name, old, new, &mut out),
            None => out.push(BreakingChange::MessageRemoved {
                message: name.clone(),
            }),
        }
    }
    for (name, old) in &old_enums {
        match new_enums.get(name) {
            Some(new) => compare_enums(name, old, new, &mut out),
            None => out.push(BreakingChange::EnumRemoved { name: name.clone() }),
        }
    }

    out.sort();
    out
}

type Index<'a> = (
    BTreeMap<String, &'a DescriptorProto>,
    BTreeMap<String, &'a EnumDescriptorProto>,
);

fn index(set: &FileDescriptorSet) -> Index<'_> {
    let mut messages = BTreeMap::new();
    let mut enums = BTreeMap::new();
    for file in &set.file {
        for message in &file.message_type {
            index_message(file.package(), message, &mut messages, &mut enums);
        }
        for e in &file.enum_type {
            enums.insert(qualify(file.package(), e.name()), e);
        }
    }
    (messages, enums)
}

fn index_message<'a>(
    scope: &str,
    message: &'a DescriptorProto,
    messages: &mut BTreeMap<String, &'a DescriptorProto>,
    enums: &mut BTreeMap<String, &'a EnumDescriptorProto>,
) {
    let name = qualify(scope, message.name());
    for nested in &message.nested_type {
        index_message(&name, nested, messages, enums);
    }
    for e in &message.enum_type {
        enums.insert(qualify(&name, e.name()), e);
    }
    messages.insert(name, message);
}

fn qualify(scope: &str, name: &str) -> String {
    if scope.is_empty() {
        name.to_owned()
    } else {
        format!("{scope}.{name}")
    }
}

fn compare_messages(
    name: &str,
    old: &DescriptorProto,
    new: &DescriptorProto,
    out: &mut Vec<BreakingChange>,
) {
    for old_field in &old.field {
        let number = old_field.number();
        let Some(new_field) = new.field.iter().find(|f| f.number() == number) else {
            let number_reserved = new
                .reserved_range
                .iter()
                .any(|r| r.start() <= number && number < r.end());
            let name_reserved = new.reserved_name.iter().any(|n| n == old_field.name());
            if !(number_reserved && name_reserved) {
                out.push(BreakingChange::FieldRemoved {
                    message: name.to_owned(),
                    number,
                    name: old_field.name().to_owned(),
                });
            }
            continue;
        };

        let (from, to) = (field_signature(old_field), field_signature(new_field));
        if from != to {
            out.push(BreakingChange::FieldNumberReused {
                message: name.to_owned(),
                number,
                from,
                to,
            });
            continue;
        }
        if old_field.label() != new_field.label() {
            out.push(BreakingChange::FieldCardinalityChanged {
                message: name.to_owned(),
                field: old_field.name().to_owned(),
            });
        }
        let (from, to) = (oneof_name(old, old_field), oneof_name(new, new_field));
        if from != to {
            out.push(BreakingChange::FieldOneofChanged {
                message: name.to_owned(),
                field: old_field.name().to_owned(),
                from: from.unwrap_or_default().to_owned(),
                to: to.unwrap_or_default().to_owned(),
            });
        }
        let (from, to) = (json_name(old_field), json_name(new_field));
        if from != to {
            out.push(BreakingChange::FieldJsonNameChanged {
                message: name.to_owned(),
                field: old_field.name().to_owned(),
                from,
                to,
            });
        }
    }
}

fn field_signature(field: &FieldDescriptorProto) -> String {
    let ty = match field.r#type() {
        Type::Message | Type::Enum | Type::Group => {
            field.type_name().trim_start_matches('.').to_owned()
        }
        other => other
            .as_str_name()
            .trim_start_matches("TYPE_")
            .to_lowercase(),
    };
    format!("{ty} {}", field.name())
}

fn oneof_name<'a>(message: &'a DescriptorProto, field: &FieldDescriptorProto) -> Option<&'a str> {
    if field.proto3_optional() {
        return None;
    }
    let index = usize::try_from(field.oneof_index?).ok()?;
    message
        .oneof_decl
        .get(index)
        .map(prost_types::OneofDescriptorProto::name)
}

fn json_name(field: &FieldDescriptorProto) -> String {
    match &field.json_name {
        Some(name) if !name.is_empty() => name.clone(),
        _ => default_json_name(field.name()),
    }
}

fn default_json_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut upper = false;
    for c in name.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

fn compare_enums(
    name: &str,
    old: &EnumDescriptorProto,
    new: &EnumDescriptorProto,
    out: &mut Vec<BreakingChange>,
) {
    for value in &old.value {
        match new.value.iter().find(|v| v.name() == value.name()) {
            Some(same) if same.number() != value.number() => {
                out.push(BreakingChange::EnumValueRenumbered {
                    name: name.to_owned(),
                    value: value.name().to_owned(),
                    from: value.number(),
                    to: same.number(),
                });
            }
            Some(_) => {}
            None => {
                let number = value.number();
                let number_reserved = new
                    .reserved_range
                    .iter()
                    .any(|r| r.start() <= number && number <= r.end());
                let name_reserved = new.reserved_name.iter().any(|n| n == value.name());
                if !(number_reserved && name_reserved) {
                    out.push(BreakingChange::EnumValueRemoved {
                        name: name.to_owned(),
                        value: value.name().to_owned(),
                        number,
                    });
                }
            }
        }
    }
}
