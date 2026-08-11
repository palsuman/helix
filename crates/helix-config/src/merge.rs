//! Tree primitives behind the layered merge (REQ-CONFIG-001.1).
//!
//! Settings are stored as a JSON tree and addressed by dotted key, which is
//! what lets `"editor.fontSize": 14` and `{ "editor": { "fontSize": 14 } }`
//! mean the same thing — the way every editor's users already expect them to.
//! [`expand_dotted`] normalizes the authored form into the tree, and the rest
//! of this module reads, writes, merges, and diffs that tree.
//!
//! The merge rule is deliberately narrow, because "merge" is ambiguous for
//! arrays and ambiguity in a settings system is a bug report waiting to
//! happen:
//!
//! | Shape | Behaviour |
//! |-------|-----------|
//! | object vs object | deep merge, key by key |
//! | array vs anything | replaced wholesale, never concatenated |
//! | scalar vs anything | replaced (last layer wins) |
//! | object vs scalar | replaced, because the shapes disagree |
//!
//! Arrays replacing rather than concatenating is the load-bearing decision.
//! Concatenation would make a workspace unable to *shorten* a user's list,
//! and would silently duplicate entries every time a layer repeated one.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

/// Deep-merge `overlay` onto `base` in place.
///
/// Objects merge key by key; everything else replaces. See the module docs
/// for why arrays replace.
pub fn deep_merge(base: &mut Value, overlay: &Value) {
    match (base, overlay) {
        (Value::Object(base_map), Value::Object(overlay_map)) => {
            for (key, overlay_value) in overlay_map {
                match base_map.get_mut(key) {
                    Some(existing) => deep_merge(existing, overlay_value),
                    None => {
                        base_map.insert(key.clone(), overlay_value.clone());
                    }
                }
            }
        }
        (base, overlay) => *base = overlay.clone(),
    }
}

/// Read a dotted key out of a tree.
pub fn get_path<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    let mut cursor = value;
    for segment in key.split('.') {
        cursor = cursor.as_object()?.get(segment)?;
    }
    Some(cursor)
}

/// Write a dotted key into a tree, creating intermediate objects and
/// replacing any non-object that sits in the way.
pub fn set_path(value: &mut Value, key: &str, new_value: Value) {
    let segments: Vec<&str> = key.split('.').collect();
    if segments.is_empty() {
        return;
    }
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    let mut cursor = value;
    for segment in &segments[..segments.len() - 1] {
        let map = cursor.as_object_mut().expect("cursor is an object");
        let entry = map
            .entry(segment.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        cursor = entry;
    }
    cursor
        .as_object_mut()
        .expect("cursor is an object")
        .insert(segments[segments.len() - 1].to_string(), new_value);
}

/// Remove a dotted key from a tree, pruning objects the removal empties.
/// Returns whether anything was removed.
pub fn remove_path(value: &mut Value, key: &str) -> bool {
    let segments: Vec<&str> = key.split('.').collect();
    remove_segments(value, &segments)
}

fn remove_segments(value: &mut Value, segments: &[&str]) -> bool {
    let Some(map) = value.as_object_mut() else {
        return false;
    };
    let Some((head, rest)) = segments.split_first() else {
        return false;
    };
    if rest.is_empty() {
        return map.remove(*head).is_some();
    }
    let Some(child) = map.get_mut(*head) else {
        return false;
    };
    let removed = remove_segments(child, rest);
    if removed && child.as_object().map(Map::is_empty).unwrap_or(false) {
        map.remove(*head);
    }
    removed
}

/// Flatten a tree to its leaves, keyed by dotted path.
///
/// A leaf is anything that is not a non-empty object: scalars, arrays, and
/// empty objects. Arrays are leaves because they replace wholesale, so
/// descending into them would produce keys nothing can address.
pub fn flatten_leaves(value: &Value) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    flatten_into(value, String::new(), &mut out);
    out
}

fn flatten_into(value: &Value, prefix: String, out: &mut BTreeMap<String, Value>) {
    match value {
        Value::Object(map) if !map.is_empty() => {
            for (key, child) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten_into(child, path, out);
            }
        }
        other => {
            if !prefix.is_empty() {
                out.insert(prefix, other.clone());
            }
        }
    }
}

/// Keys whose values differ between two flattened views, including keys
/// present in only one of them. Sorted, so a change notification is
/// deterministic.
pub fn changed_keys(
    before: &BTreeMap<String, Value>,
    after: &BTreeMap<String, Value>,
) -> Vec<String> {
    let mut changed: Vec<String> = before
        .iter()
        .filter(|(key, value)| after.get(*key) != Some(value))
        .map(|(key, _)| key.clone())
        .chain(
            after
                .iter()
                .filter(|(key, _)| !before.contains_key(*key))
                .map(|(key, _)| key.clone()),
        )
        .collect();
    changed.sort_unstable();
    changed.dedup();
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn objects_merge_key_by_key_rather_than_replacing_wholesale() {
        let mut base = json!({ "editor": { "fontSize": 14, "tabSize": 4 } });
        deep_merge(&mut base, &json!({ "editor": { "tabSize": 2 } }));
        assert_eq!(base, json!({ "editor": { "fontSize": 14, "tabSize": 2 } }));
    }

    #[test]
    fn scalars_are_last_wins() {
        let mut base = json!({ "a": 1 });
        deep_merge(&mut base, &json!({ "a": 2 }));
        assert_eq!(base["a"], 2);
    }

    #[test]
    fn arrays_replace_and_are_never_concatenated() {
        let mut base = json!({ "files": { "watchers": ["a", "b", "c"] } });
        deep_merge(&mut base, &json!({ "files": { "watchers": ["z"] } }));
        assert_eq!(
            base["files"]["watchers"],
            json!(["z"]),
            "a higher layer must be able to shorten a list, not only extend it"
        );
    }

    #[test]
    fn an_object_replaces_a_scalar_when_the_shapes_disagree() {
        let mut base = json!({ "a": 1 });
        deep_merge(&mut base, &json!({ "a": { "b": 2 } }));
        assert_eq!(base, json!({ "a": { "b": 2 } }));
    }

    #[test]
    fn a_path_can_be_read_written_and_removed() {
        let mut tree = json!({});
        set_path(&mut tree, "a.b.c", json!(1));
        assert_eq!(get_path(&tree, "a.b.c"), Some(&json!(1)));

        assert!(remove_path(&mut tree, "a.b.c"));
        assert_eq!(get_path(&tree, "a.b.c"), None);
        assert_eq!(tree, json!({}), "emptied parents are pruned");
        assert!(!remove_path(&mut tree, "a.b.c"));
    }

    #[test]
    fn leaves_flatten_to_dotted_keys_and_arrays_stay_whole() {
        let leaves = flatten_leaves(&json!({
            "editor": { "fontSize": 14, "rulers": [80, 120] },
            "empty": {}
        }));
        assert_eq!(leaves["editor.fontSize"], json!(14));
        assert_eq!(leaves["editor.rulers"], json!([80, 120]));
        assert_eq!(leaves["empty"], json!({}));
        assert_eq!(leaves.len(), 3);
    }

    #[test]
    fn a_diff_reports_added_removed_and_altered_keys() {
        let before = flatten_leaves(&json!({ "a": 1, "b": 2 }));
        let after = flatten_leaves(&json!({ "a": 1, "b": 3, "c": 4 }));
        assert_eq!(changed_keys(&before, &after), vec!["b", "c"]);
    }
}
