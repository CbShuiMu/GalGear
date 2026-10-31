//! 读取 Magpie 缩放配置，修改默认配置时保留各应用配置及未知字段。
use galgear_i18n::Message;
use serde::Serialize;
use serde_json::Value;
use std::path::Path;

#[derive(Serialize)]
pub(crate) struct Mode {
    value: String,
    label: String,
}

#[derive(Serialize)]
pub(crate) struct Modes {
    options: Vec<Mode>,
    pub(crate) selected: String,
}

// Use the complete mode as its identity: a reordered or edited Magpie list must
// never silently select a different algorithm when a stale dropdown is saved.
fn identity(value: &Value) -> String {
    value.to_string()
}

pub(crate) fn read(executable: &Path) -> Result<Modes, Message> {
    let bytes = std::fs::read(super::magpie_config(executable))
        .map_err(|e| Message::detail("tools.error", e))?;
    let value: Value =
        serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes))
            .map_err(|e| Message::detail("tools.error", e))?;
    let modes = value["scalingModes"].as_array().ok_or_else(invalid)?;
    let options = modes
        .iter()
        .map(|mode| {
            Ok(Mode {
                value: identity(mode),
                label: mode["name"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(invalid)?
                    .into(),
            })
        })
        .collect::<Result<Vec<_>, Message>>()?;
    let selected = value["profiles"][0]["scalingMode"]
        .as_u64()
        .and_then(|index| modes.get(index as usize))
        .map(identity)
        .unwrap_or_default();
    Ok(Modes { options, selected })
}

fn invalid() -> Message {
    Message::new("tools.magpie_mode_invalid")
}

pub(super) fn apply(value: &mut Value, selected: Option<&str>) -> Result<(), Message> {
    let Some(selected) = selected else {
        return Ok(());
    };
    let index = value["scalingModes"]
        .as_array()
        .ok_or_else(invalid)?
        .iter()
        .position(|mode| identity(mode) == selected)
        .ok_or_else(invalid)?;
    let profile = value
        .get_mut("profiles")
        .and_then(Value::as_array_mut)
        .and_then(|p| p.first_mut())
        .and_then(Value::as_object_mut)
        .ok_or_else(invalid)?;
    profile.insert("scalingMode".into(), index.into());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_survives_reordering_and_preserves_application_profiles() {
        let mut value = serde_json::json!({"scalingModes":[{"name":"B"},{"name":"A"}],"profiles":[{"scalingMode":0,"extra":true},{"scalingMode":0,"name":"Game"}]});
        apply(&mut value, Some(r#"{"name":"A"}"#)).unwrap();
        assert_eq!(value["profiles"][0]["scalingMode"], 1);
        assert_eq!(value["profiles"][0]["extra"], true);
        assert_eq!(value["profiles"][1]["scalingMode"], 0);
        let previous = value.clone();
        assert!(apply(&mut value, Some(r#"{"name":"Removed"}"#)).is_err());
        assert_eq!(value, previous);
    }
}
