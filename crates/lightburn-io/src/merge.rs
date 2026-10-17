use serde_json::Value;

/// Deep-merge `overlay` on top of `base`, returning the result.
///
/// This is the "defaults layer" from LightBurn's I/O: `base` is the
/// `defaultprefs.ini` layer, `overlay` is the user's `prefs.ini`. User
/// values win; keys present only in the defaults are kept.
///
/// Merge rules:
/// - object + object  → merged key-by-key, recursively
/// - anything else     → `overlay` replaces `base` wholesale
///
/// Arrays are replaced, not concatenated: a user's `CutList` or `DeviceList`
/// fully supersedes the default list rather than appending to it, which
/// matches how LightBurn treats those collections.
pub fn merge_defaults(base: &Value, overlay: &Value) -> Value {
    match (base, overlay) {
        (Value::Object(base_map), Value::Object(overlay_map)) => {
            let mut out = base_map.clone();
            for (k, v) in overlay_map {
                let merged = match out.get(k) {
                    Some(existing) => merge_defaults(existing, v),
                    None => v.clone(),
                };
                out.insert(k.clone(), merged);
            }
            Value::Object(out)
        }
        // Non-object overlay (scalar, array, null) replaces the base value.
        (_, overlay) => overlay.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn user_value_overrides_default() {
        let base = json!({ "AutoSaveSeconds": 120, "units": 0 });
        let overlay = json!({ "AutoSaveSeconds": 300 });
        let merged = merge_defaults(&base, &overlay);
        assert_eq!(merged["AutoSaveSeconds"], 300);
        assert_eq!(merged["units"], 0); // default survives
    }

    #[test]
    fn nested_objects_merge_recursively() {
        let base = json!({ "DefaultCutSetting": { "speed": 100, "power": 20 } });
        let overlay = json!({ "DefaultCutSetting": { "power": 80 } });
        let merged = merge_defaults(&base, &overlay);
        assert_eq!(merged["DefaultCutSetting"]["speed"], 100);
        assert_eq!(merged["DefaultCutSetting"]["power"], 80);
    }

    #[test]
    fn arrays_are_replaced_not_concatenated() {
        let base = json!({ "CutList": [1, 2, 3] });
        let overlay = json!({ "CutList": [9] });
        let merged = merge_defaults(&base, &overlay);
        assert_eq!(merged["CutList"], json!([9]));
    }
}
