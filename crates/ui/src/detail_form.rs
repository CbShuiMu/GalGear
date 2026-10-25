//! One detail draft shared by the in-place editor and its save action.
use crate::{features, models::Metadata, requests::Pending, requests::issue, state::App};
use serde_json::json;
use std::collections::BTreeMap;

pub(crate) fn begin(app: &mut App) {
    app.form.retain(|key, _| !key.starts_with("draft-"));
    let Some(game) = app
        .detail
        .selected
        .as_ref()
        .and_then(|id| app.catalog.games.iter().find(|g| &g.id == id))
    else {
        return;
    };
    let edit = app
        .settings
        .game_edits
        .get(&game.id)
        .cloned()
        .unwrap_or_default();
    for (key, value) in [
        ("draft-title", game.title.clone()),
        ("draft-tags", game.tags.join(", ")),
        (
            "draft-executable",
            game.executable.clone().unwrap_or_default(),
        ),
        ("draft-arguments", edit.arguments.join("\n")),
        ("draft-workdir", edit.working_directory.unwrap_or_default()),
        ("draft-launcher", edit.launcher.unwrap_or_default()),
        ("draft-locale", edit.locale_profile),
    ] {
        app.form.insert(key.into(), value);
    }
    let empty = Metadata::default();
    let metadata = game.metadata.as_ref().unwrap_or(&empty);
    for (field, _) in features::METADATA_FIELDS {
        app.form.insert(
            format!("draft-meta-{field}"),
            features::meta_value(metadata, field).into(),
        );
    }
    app.detail.editing = true;
    app.choices.open = None;
}

pub(crate) fn cancel(app: &mut App) {
    app.detail.editing = false;
    app.form.retain(|key, _| !key.starts_with("draft-"));
}

pub(crate) fn save(app: &mut App) {
    let Some(id) = app.detail.selected.clone() else {
        return;
    };
    let value = |key: &str| features::form(app, key);
    let optional = |key: &str| {
        let text = value(key);
        (!text.trim().is_empty()).then_some(text)
    };
    let metadata: BTreeMap<_, _> = features::METADATA_FIELDS
        .iter()
        .map(|(field, _)| (field.to_string(), value(&format!("draft-meta-{field}"))))
        .collect();
    let resets: Vec<_> = features::METADATA_FIELDS
        .iter()
        .filter(|(field, _)| value(&format!("draft-reset-{field}")) == "true")
        .map(|(field, _)| field)
        .collect();
    let payload = json!({"id":id,"draft":{"title":value("draft-title"),"tags":value("draft-tags").split([',','，']).map(str::trim).collect::<Vec<_>>(),"executable":value("draft-executable"),"arguments":value("draft-arguments").lines().collect::<Vec<_>>(),"working_directory":optional("draft-workdir"),"launcher":optional("draft-launcher"),"locale_profile":value("draft-locale"),"metadata":metadata,"metadata_resets":resets,"reset_tag_rules":value("draft-reset-tags")=="true"}});
    issue(app, Pending::DetailSaved, "save_game_details", payload);
}
