//! Lossless packing of each independently usable MCP input schema.
//! Only schema locations are visited: defaults/examples are user data.
use serde_json::{Value, json};
use std::collections::BTreeMap;

const MAPS: &[&str] = &[
    "$defs",
    "properties",
    "patternProperties",
    "dependentSchemas",
];
const SINGLES: &[&str] = &[
    "items",
    "additionalProperties",
    "contains",
    "not",
    "if",
    "then",
    "else",
    "propertyNames",
];
const ARRAYS: &[&str] = &["oneOf", "anyOf", "allOf", "prefixItems"];

fn visit(schema: &Value, action: &mut impl FnMut(&Value)) {
    for key in MAPS {
        if let Some(map) = schema.get(key).and_then(Value::as_object) {
            for child in map.values() {
                visit(child, action);
            }
        }
    }
    for key in SINGLES {
        if let Some(child) = schema.get(key) {
            visit(child, action);
        }
    }
    for key in ARRAYS {
        if let Some(array) = schema.get(key).and_then(Value::as_array) {
            for child in array {
                visit(child, action);
            }
        }
    }
    action(schema);
}

fn visit_mut(schema: &mut Value, action: &mut impl FnMut(&mut Value)) {
    for key in MAPS {
        if let Some(map) = schema.get_mut(key).and_then(Value::as_object_mut) {
            for child in map.values_mut() {
                visit_mut(child, action);
            }
        }
    }
    for key in SINGLES {
        if let Some(child) = schema.get_mut(key) {
            visit_mut(child, action);
        }
    }
    for key in ARRAYS {
        if let Some(array) = schema.get_mut(key).and_then(Value::as_array_mut) {
            for child in array {
                visit_mut(child, action);
            }
        }
    }
    action(schema);
}

fn compact_unions(schema: &mut Value) {
    let Some(map) = schema.as_object_mut() else {
        return;
    };
    // A string const/enum already constrains the type.
    if map.get("type").is_some_and(|v| v == "string")
        && (map.get("const").is_some_and(Value::is_string)
            || map
                .get("enum")
                .and_then(Value::as_array)
                .is_some_and(|v| !v.is_empty() && v.iter().all(Value::is_string)))
    {
        map.remove("type");
    }
    // float/double are annotations, including on nullable number fields.
    if map
        .get("format")
        .is_some_and(|v| v == "float" || v == "double")
        && map
            .get("type")
            .is_some_and(|v| v == "number" || v == &json!(["number", "null"]))
    {
        map.remove("format");
    }
    for key in ["oneOf", "anyOf"] {
        let common_type = map.get(key).and_then(Value::as_array).and_then(|variants| {
            let kind = variants.first()?.get("type")?;
            (variants.len() > 1
                && variants.iter().all(|v| v.get("type") == Some(kind))
                && map.get("type").is_none_or(|v| v == kind))
            .then(|| kind.clone())
        });
        if let Some(kind) = common_type {
            map.insert("type".into(), kind);
            for variant in map[key].as_array_mut().unwrap() {
                variant.as_object_mut().unwrap().remove("type");
            }
        }
        let Some(variants) = map.get_mut(key).and_then(Value::as_array_mut) else {
            continue;
        };
        let mut groups = BTreeMap::<String, usize>::new();
        let mut packed: Vec<Value> = Vec::new();
        for variant in std::mem::take(variants) {
            let discriminator = &variant["properties"]["type"];
            let tag = discriminator["const"]
                .as_str()
                .filter(|_| discriminator.get("enum").is_none());
            let required = variant["required"]
                .as_array()
                .is_some_and(|v| v.iter().any(|v| v == "type"));
            if let Some(tag) = tag.filter(|_| required) {
                let mut template = variant.clone();
                template["properties"]["type"]
                    .as_object_mut()
                    .unwrap()
                    .remove("const");
                let signature = template.to_string();
                if let Some(&index) = groups.get(&signature) {
                    let discriminator =
                        packed[index]["properties"]["type"].as_object_mut().unwrap();
                    // Do not combine overlapping branches: oneOf must still reject them.
                    let duplicate = discriminator.get("const").is_some_and(|v| v == tag)
                        || discriminator
                            .get("enum")
                            .and_then(Value::as_array)
                            .is_some_and(|v| v.iter().any(|v| v == tag));
                    if !duplicate {
                        if let Some(first) = discriminator.remove("const") {
                            discriminator.insert("enum".into(), json!([first]));
                        }
                        discriminator["enum"]
                            .as_array_mut()
                            .unwrap()
                            .push(json!(tag));
                        continue;
                    }
                } else {
                    groups.insert(signature, packed.len());
                }
            }
            packed.push(variant);
        }
        *variants = packed;
        // (C & A) | (C & B) == C & (A | B), also for exclusive unions.
        // Keep additionalProperties on each branch: moving it would widen inputs.
        let common_required = variants.first().and_then(|first| {
            let required = first.get("required")?.as_array()?;
            variants
                .iter()
                .all(|v| v.get("required").is_some_and(Value::is_array))
                .then(|| {
                    required
                        .iter()
                        .filter(|field| {
                            field.is_string()
                                && variants
                                    .iter()
                                    .all(|v| v["required"].as_array().unwrap().contains(field))
                        })
                        .cloned()
                        .collect::<Vec<_>>()
                })
        });
        if let Some(common) = common_required.filter(|v| !v.is_empty() && variants.len() > 1) {
            for variant in variants {
                let fields = variant["required"].as_array_mut().unwrap();
                fields.retain(|v| !common.contains(v));
                if fields.is_empty() {
                    variant.as_object_mut().unwrap().remove("required");
                }
            }
            let fields = map
                .entry("required")
                .or_insert(json!([]))
                .as_array_mut()
                .unwrap();
            for field in common {
                if !fields.contains(&field) {
                    fields.push(field);
                }
            }
        }
    }
}

fn share_arrays(schema: &mut Value) {
    let shapes = [(2, "NumberPair"), (3, "NumberTriple"), (4, "NumberQuad")]
        .into_iter()
        .map(|(length, name)| (name, json!({"type":"array","items":{"type":"number"},"minItems":length,"maxItems":length})))
        .chain(std::iter::once(("StringList", json!({"type":"array","items":{"type":"string"}}))));
    for (name, shape) in shapes {
        if schema["$defs"].get(name).is_some() {
            continue;
        }
        let reference = json!({"$ref":format!("#/$defs/{name}")});
        let matches = |value: &Value| {
            let Some(mut core) = value.as_object().cloned() else {
                return false;
            };
            core.remove("default");
            core.remove("description");
            Value::Object(core) == shape
        };
        let mut count = 0;
        visit(schema, &mut |v| count += usize::from(matches(v)));
        let saving = shape
            .to_string()
            .len()
            .saturating_sub(reference.to_string().len());
        if count * saving <= shape.to_string().len() + name.len() + 4 {
            continue;
        }
        visit_mut(schema, &mut |value| {
            if matches(value) {
                let mut replacement = reference.as_object().unwrap().clone();
                for key in ["default", "description"] {
                    if let Some(annotation) = value.get(key) {
                        replacement.insert(key.into(), annotation.clone());
                    }
                }
                *value = Value::Object(replacement);
            }
        });
        let definitions = schema
            .as_object_mut()
            .unwrap()
            .entry("$defs")
            .or_insert(json!({}));
        definitions[name] = shape;
    }
}

fn inline_small_definitions(schema: &mut Value) {
    loop {
        let Some(definitions) = schema.get("$defs").and_then(Value::as_object) else {
            return;
        };
        let candidate = definitions.iter().find_map(|(name, definition)| {
            let fields = definition.as_object()?;
            let reference = format!("#/$defs/{name}");
            let mut recursive = false;
            visit(definition, &mut |v| recursive |= v["$ref"] == reference);
            if recursive {
                return None;
            }
            let mut count = 0;
            let mut conflict = false;
            visit(schema, &mut |v| {
                if v["$ref"] == reference {
                    count += 1;
                    conflict |= v
                        .as_object()
                        .unwrap()
                        .keys()
                        .any(|k| k != "$ref" && fields.contains_key(k));
                }
            });
            let definition_size = definition.to_string().len();
            let reference_size = json!({"$ref":reference}).to_string().len();
            let original = count * reference_size + definition_size + name.len() + 4;
            (count > 0 && !conflict && count * definition_size < original)
                .then(|| (name.clone(), reference, definition.clone()))
        });
        let Some((name, reference, definition)) = candidate else {
            break;
        };
        visit_mut(schema, &mut |v| {
            if v["$ref"] == reference {
                let mut fields = definition.as_object().unwrap().clone();
                fields.extend(
                    v.as_object()
                        .unwrap()
                        .iter()
                        .filter(|(k, _)| *k != "$ref")
                        .map(|(k, v)| (k.clone(), v.clone())),
                );
                *v = Value::Object(fields);
            }
        });
        schema["$defs"].as_object_mut().unwrap().remove(&name);
    }
    if schema["$defs"].as_object().is_some_and(|v| v.is_empty()) {
        schema.as_object_mut().unwrap().remove("$defs");
    }
}

pub fn compact(schema: &mut Value) {
    visit_mut(schema, &mut compact_unions);
    share_arrays(schema);
    inline_small_definitions(schema);
}

/// Retain only definitions reachable from a narrowed tool schema.
pub fn prune_definitions(schema: &mut Value) {
    let Some(definitions) = schema.as_object_mut().unwrap().remove("$defs") else {
        return;
    };
    let mut pending = std::collections::BTreeSet::new();
    let mut refs = |value: &Value| {
        if let Some(reference) = value.get("$ref").and_then(Value::as_str) {
            pending.insert(
                reference
                    .strip_prefix("#/$defs/")
                    .expect("local definition")
                    .to_owned(),
            );
        }
    };
    visit(schema, &mut refs);
    let mut retained = serde_json::Map::new();
    while let Some(name) = pending.pop_first() {
        if retained.contains_key(&name) {
            continue;
        }
        let definition = definitions[&name].clone();
        visit(&definition, &mut |value| {
            if let Some(reference) = value.get("$ref").and_then(Value::as_str) {
                pending.insert(
                    reference
                        .strip_prefix("#/$defs/")
                        .expect("local definition")
                        .to_owned(),
                );
            }
        });
        retained.insert(name, definition);
    }
    if !retained.is_empty() {
        schema["$defs"] = Value::Object(retained);
    }
}

/// MCP wording only; leave serialized data and developer API docs untouched.
pub fn simplify_descriptions(schema: &mut Value) {
    visit_mut(schema, &mut |value| {
        let replacement = match value.get("description").and_then(Value::as_str) {
            Some(
                "Legacy string configs remain byte-for-byte compatible in revision hashes."
                | "Versioned names make built-in configs reproducible across OCIO upgrades.",
            ) => "",
            Some("Immutable OCIOZ package, verified before any cache or processor is used.") => {
                "OCIOZ package path and content hash."
            }
            Some(
                "An input/display chain around scene-linear grading.\nDisplay outputs are tagged with the corresponding file colorimetry.",
            ) => "Input/display chain around scene-linear grading.",
            Some(
                "Kang 2002 Planckian-locus fit with signed normal offset in CIE 1960 uv.\nPositive Duv describes a greener source white, compensated by camera gains.",
            ) => {
                "Source white in kelvin and signed CIE 1960 uv tint. Positive Duv compensates a greener source."
            }
            Some("Native weight-free keying / interactive segmentation / alpha matting.") => {
                "Color keying, interactive segmentation or alpha matting."
            }
            _ => return,
        };
        if replacement.is_empty() {
            value.as_object_mut().unwrap().remove("description");
        } else {
            value["description"] = json!(replacement);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_required_fields_are_factored_without_opening_branches() {
        for key in ["oneOf", "anyOf"] {
            let branch = |tag: &str, field: &str| json!({"type":"object","properties":{"type":{"const":tag},"source":{"type":"string"},field:{"type":"number"}},"required":["type","source",field],"additionalProperties":false});
            let mut schema = json!({"required":["source"],key:[branch("a","x"),branch("b","y")]});
            compact(&mut schema);
            assert_eq!(schema["required"], json!(["source", "type"]));
            for (i, field) in ["x", "y"].iter().enumerate() {
                assert_eq!(schema[key][i]["required"], json!([field]));
                assert_eq!(schema[key][i]["additionalProperties"], false);
            }
        }
    }

    #[test]
    fn wording_changes_visit_schemas_only() {
        let text = "Legacy string configs remain byte-for-byte compatible in revision hashes.";
        let data = json!({"description":text});
        let mut schema = json!({"description":text,"default":data,"examples":[data],"properties":{"notes":{"description":text,"type":"string"}}});
        simplify_descriptions(&mut schema);
        assert!(schema.get("description").is_none());
        assert!(schema["properties"]["notes"].get("description").is_none());
        assert_eq!(schema["default"], data);
        assert_eq!(schema["examples"], json!([data]));
    }

    #[test]
    fn shared_string_lists_keep_defaults_and_descriptions() {
        let list =
            json!({"type":"array","items":{"type":"string"},"default":[],"description":"notes"});
        let mut schema = json!({"properties":{"a":list,"b":list,"c":list,"d":list,"e":list}});
        compact(&mut schema);
        for field in ["a", "b", "c", "d", "e"] {
            assert_eq!(schema["properties"][field]["default"], json!([]));
            assert_eq!(schema["properties"][field]["description"], "notes");
            assert_eq!(schema["properties"][field]["$ref"], "#/$defs/StringList");
        }
    }

    #[test]
    fn data_annotations_and_overlapping_branches_are_preserved() {
        let data = json!({"type":"string","const":"literal","$ref":"#/$defs/UserData"});
        let branch = json!({"type":"object","properties":{"type":{"const":"same","type":"string"}},"required":["type"],"additionalProperties":false});
        let mut schema = json!({"type":"object","default":data,"examples":[data],"properties":{"default":{"type":"string","const":"value"},"choice":{"oneOf":[branch,branch]}}});
        compact(&mut schema);
        assert_eq!(schema["default"], data);
        assert_eq!(schema["examples"], json!([data]));
        assert_eq!(schema["properties"]["default"], json!({"const":"value"}));
        assert_eq!(
            schema["properties"]["choice"]["oneOf"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn recursive_refs_and_sibling_constraints_survive() {
        let mut schema = json!({"type":"object","properties":{"mask":{"$ref":"#/$defs/Mask"},"value":{"$ref":"#/$defs/Bounded","minimum":2}},"$defs":{"Mask":{"type":"object","properties":{"children":{"type":"array","items":{"$ref":"#/$defs/Mask"}}}},"Bounded":{"type":"integer","minimum":0,"maximum":4}}});
        let before = schema.clone();
        compact(&mut schema);
        assert_eq!(schema, before);
    }

    #[test]
    fn existing_enum_still_restricts_a_const_discriminator() {
        let branch = |tag| json!({"type":"object","properties":{"type":{"const":tag,"enum":["a"]}},"required":["type"],"additionalProperties":false});
        let mut schema = json!({"oneOf":[branch("a"),branch("b")]});
        compact(&mut schema);
        for (index, tag) in ["a", "b"].iter().enumerate() {
            assert_eq!(schema["oneOf"][index]["properties"]["type"]["const"], *tag);
            assert_eq!(
                schema["oneOf"][index]["properties"]["type"]["enum"],
                json!(["a"])
            );
        }
    }

    #[test]
    fn vector_lengths_defaults_and_descriptions_survive_sharing() {
        let vector = json!({"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3,"default":[1,1,1],"description":"RGB gains"});
        let mut schema =
            json!({"type":"object","properties":{"a":vector,"b":vector,"c":vector,"d":vector}});
        compact(&mut schema);
        for key in ["a", "b", "c", "d"] {
            let property = &schema["properties"][key];
            assert_eq!(property["default"], json!([1, 1, 1]));
            assert_eq!(property["description"], "RGB gains");
            let target = schema
                .pointer(
                    property["$ref"]
                        .as_str()
                        .unwrap()
                        .strip_prefix('#')
                        .unwrap(),
                )
                .unwrap();
            assert_eq!(target["minItems"], 3);
            assert_eq!(target["maxItems"], 3);
            assert_eq!(target["items"]["type"], "number");
        }
    }
}
