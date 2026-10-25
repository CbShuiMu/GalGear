//! Detail reading layout and local save/tool workflows.
use crate::{
    ai, cloud, detail, detail_form, library, models::Game, models::Metadata, models::SaveSnapshot,
    preferences, requests::Pending, requests::issue, state::App, state::show_toast, view::t,
    widgets::button, widgets::bytes, widgets::esc,
};
use galgear_i18n::Message;
use serde_json::json;
use std::fmt::Write;

pub(crate) fn form(app: &App, key: &str) -> String {
    app.form.get(key).cloned().unwrap_or_default()
}
pub(crate) fn setting(app: &App, key: &str, fallback: &str) -> String {
    app.form
        .get(key)
        .cloned()
        .unwrap_or_else(|| fallback.into())
}
fn request_history(app: &mut App, command: &str, payload: serde_json::Value) {
    if let Some(id) = app.detail.selected.clone() {
        issue(app, Pending::History(id), command, payload);
    }
}
fn game_tools(app: &mut App, id: &str, mode: Option<&str>, auto: Option<bool>) {
    let edit = app.settings.game_edits.get(id).cloned().unwrap_or_default();
    let profile = setting(app, "locale-profile", &edit.locale_profile);
    issue(
        app,
        Pending::Preferences,
        "set_game_tools",
        json!({"id":id,"launchMode":mode.unwrap_or(if edit.launch_mode=="locale" {"locale"} else {"normal"}),"localeProfile":profile,"autoBackup":auto.unwrap_or(edit.auto_backup),"upscale":edit.upscale}),
    );
}

pub(crate) fn dispatch(app: &mut App, action: &str, value: &str) -> bool {
    if matches!(action, "launch-normal" | "open-folder") {
        app.context_menu = None;
    }
    match action {
        "game-settings" => app.detail.editor = Some(detail::Editor::Tools),
        "toggle-locale" => {
            if let Some(id) = app.detail.selected.clone() {
                let locale = app
                    .settings
                    .game_edits
                    .get(&id)
                    .is_some_and(|edit| edit.launch_mode == "locale");
                game_tools(
                    app,
                    &id,
                    Some(if locale { "normal" } else { "locale" }),
                    None,
                );
            }
        }
        "toggle-upscale" => {
            if let Some(id) = app.detail.selected.clone() {
                let edit = app
                    .settings
                    .game_edits
                    .get(&id)
                    .cloned()
                    .unwrap_or_default();
                issue(
                    app,
                    Pending::Preferences,
                    "set_game_tools",
                    json!({"id":id,"launchMode":if edit.launch_mode=="locale"{"locale"}else{"normal"},"localeProfile":edit.locale_profile,"autoBackup":edit.auto_backup,"upscale":!edit.upscale}),
                );
            }
        }
        "metadata-provider"
            if preferences::SOURCES
                .iter()
                .any(|(source, _)| *source == value) =>
        {
            app.detail.metadata_source = value.into();
            if let Some(id) = &app.detail.selected {
                app.detail.candidates.remove(id);
            }
        }
        "source-default" => issue(
            app,
            Pending::Preferences,
            "set_data_source",
            json!({"source":value}),
        ),
        "source-save" => {
            issue(
                app,
                Pending::DataSources(value.into()),
                "save_data_source",
                json!({"source":value,"token":form(app,&format!("source-{value}-token")),"clientId":form(app,"source-ymgal-client-id"),"clientSecret":form(app,"source-ymgal-client-secret")}),
            );
        }
        "source-clear" => issue(
            app,
            Pending::DataSources(value.into()),
            "clear_data_source",
            json!({"source":value}),
        ),
        "detail-edit" => detail_form::begin(app),
        "detail-save" => detail_form::save(app),
        "detail-cancel" => detail_form::cancel(app),
        "detail-field-reset" => {
            if let Some(game) = app
                .detail
                .selected
                .as_ref()
                .and_then(|id| app.catalog.games.iter().find(|g| &g.id == id))
            {
                let empty = Metadata::default();
                let cached = app
                    .settings
                    .game_edits
                    .get(&game.id)
                    .and_then(|edit| edit.metadata.as_ref())
                    .or(game.metadata.as_ref())
                    .unwrap_or(&empty);
                if METADATA_FIELDS.iter().any(|(field, _)| *field == value) {
                    app.form.insert(
                        format!("draft-meta-{value}"),
                        meta_value(cached, value).into(),
                    );
                    app.form
                        .insert(format!("draft-reset-{value}"), "true".into());
                }
            }
        }
        "detail-tags-reset" => {
            if let Some(game) = app
                .detail
                .selected
                .as_ref()
                .and_then(|id| app.catalog.games.iter().find(|g| &g.id == id))
            {
                let source_tags = app
                    .settings
                    .game_edits
                    .get(&game.id)
                    .and_then(|edit| edit.source_tags.as_ref());
                let mut imported = source_tags.cloned().unwrap_or_default();
                if source_tags.is_none()
                    && let Some(metadata) = &game.metadata
                {
                    imported.extend(metadata.tags.iter().cloned());
                    for field in galgear_tags::CREDIT_FIELDS {
                        imported.extend(galgear_tags::credit_names(
                            meta_value(metadata, field),
                            metadata
                                .links
                                .iter()
                                .filter(|link| link.field == *field)
                                .map(|link| link.name.as_str()),
                        ));
                    }
                }
                let tags = galgear_tags::normalize(game.tags.iter().cloned().chain(imported));
                app.form.insert("draft-tags".into(), tags.join(", "));
                app.form.insert("draft-reset-tags".into(), "true".into());
            }
        }
        "settings-section"
            if matches!(
                value,
                "account" | "ai" | "cloud" | "library" | "display" | "system" | "paths" | "about"
            ) =>
        {
            app.settings_section = value.into();
            app.choices.open = None;
            if value == "ai" {
                ai::load(app);
            }
            if value == "cloud" {
                cloud::load(app);
            }
        }
        "patch-tab" => issue(
            app,
            Pending::Preferences,
            "set_patch_tab",
            json!({"visible":value=="true"}),
        ),
        "context-close" => app.context_menu = None,
        "play-status" => {
            if let Some(menu) = app.context_menu.take() {
                issue(
                    app,
                    Pending::PlayStatus,
                    "set_play_status",
                    json!({"id":menu.id,"status":value}),
                );
            }
        }
        "context-delete" => {
            if let Some(menu) = app.context_menu.take() {
                app.library.dialog = Some(library::Dialog::Exclude { ids: vec![menu.id] });
            }
        }
        "card-finished" => {
            if let Some(game) = app.catalog.games.iter().find(|game| game.id == value) {
                let status = if game.play_status == "finished" {
                    "unmarked"
                } else {
                    "finished"
                };
                issue(
                    app,
                    Pending::PlayStatus,
                    "set_play_status",
                    json!({"id":value,"status":status}),
                );
            }
        }
        "card-delete" => {
            if app.catalog.games.iter().any(|game| game.id == value) {
                app.library.dialog = Some(library::Dialog::Exclude {
                    ids: vec![value.into()],
                });
            }
        }
        "editor-close" => {
            app.detail.editor = None;
            app.form.remove("title-draft");
            app.form.remove("locale-profile");
            app.detail.confirmation = None;
            app.choices.open = None;
        }
        "metadata-association" | "tags-association" => {
            app.detail.editor = Some(if action == "tags-association" {
                detail::Editor::TagSource
            } else {
                detail::Editor::Association
            });
            app.detail.metadata_source = "vndb".into();
            if let Some(game) = app
                .detail
                .selected
                .as_ref()
                .and_then(|id| app.catalog.games.iter().find(|g| &g.id == id))
            {
                app.detail.candidates.remove(&game.id);
                app.form.insert("metadata-query".into(), game.title.clone());
            }
        }
        "metadata-association-close" => app.detail.editor = None,
        "reset-cover" => issue(
            app,
            Pending::SelectMetadata,
            "reset_cover",
            json!({"id":value}),
        ),
        "save-tags" => {
            if let Some(id) = app.detail.selected.clone() {
                let tags: Vec<_> = form(app, "edit-tags")
                    .split([',', '，'])
                    .map(str::to_owned)
                    .collect();
                issue(
                    app,
                    Pending::Edit,
                    "edit_tags",
                    json!({"id":id,"names":tags}),
                );
            }
        }
        "font-size" => {
            if let Ok(size) = value.parse::<u32>() {
                issue(
                    app,
                    Pending::Display,
                    "set_display",
                    json!({"coverScale":120,"fontSize":size}),
                );
            }
        }
        "meta-save" | "meta-reset" => {
            if let Some(id) = app.detail.selected.clone() {
                let text = (action == "meta-save").then(|| form(app, &format!("meta-{value}")));
                issue(
                    app,
                    Pending::MetadataField(value.into()),
                    "edit_metadata",
                    json!({"id":id,"field":value,"value":text}),
                );
            }
        }
        "restore-game-tags" => {
            app.detail.confirmation = Some(("restore-game-tags".into(), value.into()));
        }
        "confirm-action" => {
            if let Some((action, value)) = app.detail.confirmation.clone() {
                match action.as_str() {
                    "restore-game-tags" => {
                        issue(app, Pending::Edit, "reset_game_tags", json!({"id":value}))
                    }
                    "snapshot-delete" => {
                        if let Some(id) = app.detail.selected.clone() {
                            request_history(
                                app,
                                "update_snapshot",
                                json!({"id":id,"snapshotId":value,"locked":false,"note":"","delete":true}),
                            );
                        }
                    }
                    "launch" | "launch-locale" => {
                        launch(app, &value, action == "launch-locale", true)
                    }
                    _ => {}
                }
                app.detail.confirmation = None;
            }
        }
        "cancel-action" => app.detail.confirmation = None,
        "save-history" => {
            app.detail.history_open = true;
            app.detail.restore_preview = None;
            request_history(app, "save_history", json!({"id":value}));
        }
        "close-history" => {
            app.detail.history_open = false;
            app.detail.restore_preview = None;
            app.detail.confirmation = None;
        }
        "backup-saves" => {
            let note = form(app, "backup-note");
            request_history(app, "backup_saves", json!({"id":value,"note":note}));
        }
        "confirm-save" => {
            if let Some(id) = app.detail.selected.clone() {
                issue(
                    app,
                    Pending::ConfirmSave(id.clone()),
                    "confirm_save",
                    json!({"id":id,"path":value}),
                );
            }
        }
        "preview-restore" => {
            if let Some(id) = app.detail.selected.clone() {
                app.detail.history_open = true;
                app.detail.restore_preview = None;
                issue(
                    app,
                    Pending::RestorePreview,
                    "preview_save_restore",
                    json!({"id":id,"snapshotId":value}),
                );
            }
        }
        "restore-saves" => {
            if let Some(plan) = app.detail.restore_preview.clone()
                && let Some(id) = app.detail.selected.clone()
            {
                request_history(
                    app,
                    "restore_saves",
                    json!({"id":id,"snapshotId":plan.snapshot.id}),
                );
            }
        }
        "snapshot-delete" => {
            app.detail.confirmation = Some(("snapshot-delete".into(), value.into()))
        }
        "snapshot-lock" | "snapshot-note" => {
            if let Some(snapshot) = app.detail.history.iter().find(|s| s.id == value).cloned()
                && let Some(id) = app.detail.selected.clone()
            {
                let note = if action == "snapshot-note" {
                    form(app, &format!("edit-note-{value}"))
                } else {
                    snapshot.note
                };
                request_history(
                    app,
                    "update_snapshot",
                    json!({"id":id,"snapshotId":value,"locked":if action=="snapshot-lock" {!snapshot.locked} else {snapshot.locked},"note":note,"delete":false}),
                );
            }
        }
        "auto-backup" => {
            if let Some(id) = app.detail.selected.clone() {
                game_tools(app, &id, None, Some(value == "true"));
            }
        }
        "save-profile" => {
            if let Some(id) = app.detail.selected.clone() {
                game_tools(app, &id, None, None);
            }
        }
        "default-launch" => {
            if let Some(id) = app.detail.selected.clone() {
                game_tools(app, &id, Some(value), None);
            }
        }
        "launch" | "launch-normal" | "launch-locale" => {
            let locale = action == "launch-locale"
                || (action == "launch"
                    && app
                        .settings
                        .game_edits
                        .get(value)
                        .is_some_and(|e| e.launch_mode == "locale"));
            match app.runtime.get(value).map(String::as_str) {
                Some("running") => show_toast(app, Message::new("error.already_running"), true),
                Some("stopped") => launch(app, value, locale, false),
                _ => {
                    app.detail.confirmation = Some((
                        (if locale { "launch-locale" } else { "launch" }).into(),
                        value.into(),
                    ))
                }
            }
        }
        "detect-runtime" => issue(
            app,
            Pending::Runtime(value.into()),
            "game_runtime",
            json!({"id":value}),
        ),
        "upscale" => issue(app, Pending::Upscale, "upscale_game", json!({"id":value})),
        "configure-tools" => {
            let settings = &app.settings;
            let locale = setting(app, "tool-locale", &settings.locale_emulator);
            let magpie = setting(app, "tool-magpie", &settings.magpie);
            let hotkey = setting(
                app,
                "tool-hotkey",
                if settings.magpie_hotkey.is_empty() {
                    "Ctrl+Alt+A"
                } else {
                    &settings.magpie_hotkey
                },
            );
            if let Ok(retention) = setting(
                app,
                "backup-retention",
                &settings.backup_retention.to_string(),
            )
            .parse::<usize>()
            {
                issue(
                    app,
                    Pending::Tools,
                    "configure_tools",
                    json!({"localeEmulator":locale,"magpie":magpie,"magpieHotkey":hotkey,"backupRetention":retention}),
                );
            } else {
                show_toast(app, Message::new("backup.retention_error"), true);
            }
        }
        _ => return false,
    }
    true
}

fn launch(app: &mut App, id: &str, locale: bool, confirmed: bool) {
    issue(
        app,
        Pending::Launch,
        "launch_game",
        json!({"id":id,"mode":if locale {"locale"} else {"normal"},"confirmedUnknown":confirmed}),
    );
}

pub(crate) fn confirmation(html: &mut String, app: &App) {
    if let Some((action, _value)) = &app.detail.confirmation {
        let message = if action.starts_with("launch") {
            t(app, "runtime.confirm").to_owned()
        } else if action == "restore-game-tags" {
            t(app, "tags.restore_confirm").into()
        } else {
            t(app, "backup.delete_confirm").into()
        };
        write!(html,r#"<section class="confirmation" role="alert"><p>{}</p><div class="detail-actions">{}{}</div></section>"#,esc(&message),button("confirm-action","",t(app,"common.confirm"),"check","primary",app.busy()),button("cancel-action","",t(app,"common.cancel"),"close","quiet",false)).unwrap();
    }
}

pub(crate) const METADATA_FIELDS: [(&str, &str); 10] = [
    ("description", "metadata.story"),
    ("brand", "metadata.brand"),
    ("developer", "metadata.developer"),
    ("publisher", "metadata.publisher"),
    ("release_date", "metadata.date"),
    ("artists", "metadata.artists"),
    ("voices", "metadata.voices"),
    ("writers", "metadata.writers"),
    ("music", "metadata.music"),
    ("singers", "metadata.singers"),
];

pub(crate) fn meta_value<'a>(meta: &'a Metadata, field: &str) -> &'a str {
    match field {
        "description" => &meta.description,
        "brand" => &meta.brand,
        "developer" => &meta.developer,
        "publisher" => &meta.publisher,
        "release_date" => &meta.release_date,
        "artists" => &meta.artists,
        "voices" => &meta.voices,
        "writers" => &meta.writers,
        "music" => &meta.music,
        "singers" => &meta.singers,
        _ => "",
    }
}

pub(crate) fn save_busy(app: &App) -> bool {
    app.requests
        .values()
        .any(|p| matches!(p, Pending::History(_) | Pending::RestorePreview))
}
pub(crate) fn snapshot_name(game: &Game, snapshot: &SaveSnapshot) -> String {
    if !snapshot.name.is_empty() && !snapshot.name.starts_with('_') {
        snapshot.name.clone()
    } else {
        format!(
            "{}_{}_{}",
            game.title,
            snapshot.created,
            if snapshot.sequence > 0 {
                snapshot.sequence.to_string()
            } else {
                snapshot.id.clone()
            }
        )
    }
}
pub(crate) fn history(html: &mut String, app: &App, game: &Game) {
    write!(html,r#"<div class="editor-overlay"><section class="editor-dialog restore-dialog" role="dialog" aria-modal="true" aria-labelledby="editor-title"><header><h2 id="editor-title">{}</h2>{}</header><div class="editor-body">"#,t(app,"backup.restore_preview"),button("close-history","",t(app,"common.close"),"close","quiet",save_busy(app))).unwrap();
    if let Some(plan) = &app.detail.restore_preview {
        write!(
            html,
            "<strong>{}</strong><p>{}</p><details><summary>{}</summary><ul>",
            esc(&snapshot_name(game, &plan.snapshot)),
            bytes(plan.snapshot.size),
            t(app, "backup.files")
        )
        .unwrap();
        for file in &plan.snapshot.files {
            write!(
                html,
                "<li>{}: {} · {}</li>",
                file.root + 1,
                esc(&file.path),
                bytes(file.size)
            )
            .unwrap();
        }
        html.push_str("</ul></details>");
        write!(
            html,
            "<section class=\"confirmation\"><h3>{}</h3><p>{}</p><ul>",
            t(app, "backup.restore_preview"),
            t(app, "backup.restore_help")
        )
        .unwrap();
        for target in &plan.targets {
            write!(html, "<li>{}</li>", esc(target)).unwrap();
        }
        html.push_str("</ul><ul class=\"restore-changes\">");
        for change in &plan.changes {
            write!(
                html,
                "<li>{}: {} · {}</li>",
                change.root + 1,
                esc(&change.path),
                t(
                    app,
                    match change.action.as_str() {
                        "remove" => "backup.remove",
                        "add" => "backup.add",
                        _ => "backup.overwrite",
                    }
                )
            )
            .unwrap();
        }
        write!(
            html,
            "</ul>{}</section>",
            button(
                "restore-saves",
                "",
                t(app, "common.confirm"),
                "check",
                "primary",
                save_busy(app)
            )
        )
        .unwrap();
    } else {
        write!(
            html,
            "<p class=\"muted\">{}</p>",
            t(app, "backup.preview_loading")
        )
        .unwrap();
    }
    html.push_str("</div></section></div>");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saving_tools_submits_the_draft_and_waits_for_save_confirmation() {
        let mut app = App::default();
        let path = "F:/Locale Emulator/LEProc.exe";
        app.form.insert("tool-locale".into(), path.into());

        dispatch(&mut app, "configure-tools", "");

        let requests = crate::bridge::take_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].1, "configure_tools");
        let payload: serde_json::Value = serde_json::from_str(&requests[0].2).unwrap();
        assert_eq!(payload["localeEmulator"], path);
        assert_eq!(payload["magpieHotkey"], "Ctrl+Alt+A");
        assert!(matches!(
            app.requests.complete(requests[0].0),
            Some(Pending::Tools)
        ));
        assert!(app.settings.locale_emulator.is_empty());
        assert!(app.toast.is_none());
    }
}
