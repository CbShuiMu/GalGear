//! The detail sidebar's local patch list and install/switch actions.
use crate::{
    models::Game, requests::Pending, requests::issue, state::App, state::show_toast, view::t,
    widgets::button, widgets::esc, widgets::icon,
};
use galgear_i18n::{Message, format_text};
use serde::Deserialize;
use serde_json::json;
use std::{collections::BTreeMap, fmt::Write};

#[derive(Default)]
pub(crate) struct State {
    reports: BTreeMap<String, Report>,
    errors: BTreeMap<String, Message>,
    latest: BTreeMap<String, u32>,
}

#[derive(Deserialize)]
struct Report {
    folder: String,
    patches: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    name: String,
    files: usize,
    replaces_existing: bool,
    error: Option<Message>,
}

pub(crate) fn load(app: &mut App, id: &str) {
    if !app
        .catalog
        .games
        .iter()
        .any(|game| game.id == id && game.available)
    {
        return;
    }
    app.local_patches.reports.remove(id);
    app.local_patches.errors.remove(id);
    issue(
        app,
        Pending::LocalPatchList(id.into()),
        "list_game_patches",
        json!({"id":id}),
    );
    app.local_patches
        .latest
        .insert(id.into(), app.requests.last_id());
}

fn pending(app: &App, id: &str) -> bool {
    app.requests.values().any(|request| {
        matches!(request, Pending::LocalPatchList(game) | Pending::LocalPatchApply(game, _) if game == id)
    })
}

pub(crate) fn refresh(app: &mut App, id: &str) {
    if !pending(app, id) {
        issue(app, Pending::LocalPatchList(id.into()), "list_game_patches", json!({"id":id}));
        app.local_patches.latest.insert(id.into(), app.requests.last_id());
    }
}

pub(crate) fn dispatch(app: &mut App, action: &str, value: &str) -> bool {
    if !matches!(
        action,
        "local-patches-refresh" | "local-patches-open" | "local-patch-apply"
    ) {
        return false;
    }
    let Some(id) = app.detail.selected.clone() else {
        return true;
    };
    if app.busy() || pending(app, &id) {
        return true;
    }
    match action {
        "local-patches-refresh" => load(app, &id),
        "local-patches-open" => issue(
            app,
            Pending::Open,
            "open_game_patch_folder",
            json!({"id":id}),
        ),
        "local-patch-apply" => {
            let switching = app.local_patches.reports.get(&id).and_then(|report| {
                report
                    .patches
                    .iter()
                    .find(|patch| patch.name == value && patch.error.is_none() && patch.files > 0)
                    .map(|patch| patch.replaces_existing)
            });
            let allowed = app
                .catalog
                .games
                .iter()
                .any(|game| game.id == id && game.available)
                && app
                    .runtime
                    .get(&id)
                    .is_none_or(|status| status != "running");
            if allowed && let Some(switching) = switching {
                show_toast(app, Message::new(if switching { "local_patches.switching" } else { "local_patches.installing" }), false);
                issue(
                    app,
                    Pending::LocalPatchApply(id.clone(), switching),
                    "apply_game_patch",
                    json!({"id":id,"name":value}),
                );
                app.local_patches.latest.insert(id, app.requests.last_id());
            }
        }
        _ => {}
    }
    true
}

pub(crate) fn resolve(
    app: &mut App,
    kind: &Pending,
    request_id: u32,
    success: bool,
    payload: &str,
) -> bool {
    let (Pending::LocalPatchList(id) | Pending::LocalPatchApply(id, _)) = kind else {
        return false;
    };
    if app.local_patches.latest.get(id) != Some(&request_id) {
        return true;
    }
    let applied = matches!(kind, Pending::LocalPatchApply(..));
    let result = if success {
        serde_json::from_str::<Report>(payload)
            .map_err(|error| Message::detail("error.patch", error))
    } else {
        Err(Message::from_error(payload))
    };
    match result {
        Ok(report) => {
            app.local_patches.reports.insert(id.clone(), report);
            app.local_patches.errors.remove(id);
            if applied {
                let key = if matches!(kind, Pending::LocalPatchApply(_, true)) {
                    "local_patches.switched"
                } else {
                    "local_patches.applied"
                };
                show_toast(app, Message::new(key), false);
            }
        }
        Err(error) => {
            if applied {
                let detail = error.args.get("detail").cloned().unwrap_or_else(|| error.render(app.settings.language));
                show_toast(app, Message::detail("local_patches.failed", detail), true);
                load(app, id);
            } else {
                app.local_patches.errors.insert(id.clone(), error);
            }
        }
    }
    true
}

pub(crate) fn render(html: &mut String, app: &App, game: &Game) {
    let loading = pending(app, &game.id);
    let disabled = app.busy() || loading || !game.available;
    write!(html, r#"<section class="detail-card patch-card" data-render-key="local-patches"><div class="utility-heading"><h3>{}</h3><div class="card-heading-actions">{}<button class="icon-button" data-action="local-patches-refresh" aria-label="{}" title="{}" {}>{}</button></div></div>"#,
        t(app,"local_patches.title"), button("local-patches-open", "", t(app,"local_patches.open"), "folder", "quiet", disabled), t(app,"local_patches.refresh"), t(app,"local_patches.refresh"), if disabled {"disabled"} else {""}, icon("refresh")).unwrap();
    let report = app.local_patches.reports.get(&game.id);
    let folder = report
        .map(|report| report.folder.clone())
        .unwrap_or_else(|| format!("{}/patch", game.folder.trim_end_matches(['/', '\\'])));
    write!(
        html,
        r#"<p class="path-value">{}</p><p class="patch-help">{}</p>"#,
        esc(&folder),
        t(app, "local_patches.help")
    )
    .unwrap();
    if !game.available {
        write!(
            html,
            r#"<p class="muted">{}</p>"#,
            t(app, "games.unavailable")
        )
        .unwrap();
    } else if let Some(error) = app.local_patches.errors.get(&game.id) {
        write!(
            html,
            r#"<p class="patch-error" role="status">{}</p>"#,
            esc(&error.render(app.settings.language))
        )
        .unwrap();
    } else if let Some(report) = report {
        if report.patches.is_empty() {
            write!(
                html,
                r#"<p class="muted patch-empty">{}</p>"#,
                t(app, "local_patches.empty")
            )
            .unwrap();
        }
        for patch in &report.patches {
            let description = patch
                .error
                .as_ref()
                .map(|error| error.render(app.settings.language))
                .unwrap_or_else(|| {
                    format_text(
                        app.settings.language,
                        "local_patches.files",
                        &[("count", &patch.files.to_string())],
                    )
                });
            write!(html, r#"<div class="local-patch-entry"><div class="local-patch-copy"><h4>{}</h4><p>{}</p></div>{}</div>"#,
                esc(&patch.name), esc(&description), button("local-patch-apply", &patch.name, t(app, if patch.replaces_existing {"local_patches.switch"} else {"local_patches.install"}), "", "secondary", disabled || patch.error.is_some() || patch.files == 0 || app.runtime.get(&game.id).is_some_and(|status| status == "running"))).unwrap();
        }
        if app
            .runtime
            .get(&game.id)
            .is_some_and(|status| status == "running")
        {
            write!(
                html,
                r#"<p class="patch-help" role="status">{}</p>"#,
                t(app, "local_patches.running")
            )
            .unwrap();
        }
    } else {
        write!(
            html,
            r#"<p class="muted" role="status">{}</p>"#,
            t(app, "local_patches.loading")
        )
        .unwrap();
    }
    html.push_str("</section>");
}

#[cfg(test)]
mod tests {
    use super::*;
    use galgear_i18n::Language;

    #[test]
    fn renders_install_switch_and_empty_patches_with_escaped_names() {
        let mut app = App {
            ..Default::default()
        };
        let game: Game = serde_json::from_value(json!({"id":"game","title":"Game","folder":"F:/Game","executable":null,"engine":"unknown","cover":null,"modified":0})).unwrap();
        app.local_patches.reports.insert(
            "game".into(),
            Report {
                folder: "F:/Game/patch".into(),
                patches: vec![
                    Entry {
                        name: "<new>".into(),
                        files: 1,
                        replaces_existing: false,
                        error: None,
                    },
                    Entry {
                        name: "Existing".into(),
                        files: 2,
                        replaces_existing: true,
                        error: None,
                    },
                    Entry {
                        name: "Empty".into(),
                        files: 0,
                        replaces_existing: false,
                        error: Some(Message::new("local_patches.empty_patch")),
                    },
                ],
            },
        );
        for language in [Language::Chinese, Language::Japanese, Language::English] {
            app.settings.language = language;
            let mut html = String::new();
            render(&mut html, &app, &game);
            assert!(html.contains("&lt;new&gt;"));
            assert!(html.contains(t(&app, "local_patches.install")));
            assert!(html.contains(t(&app, "local_patches.switch")));
            assert_eq!(html.matches("data-action=\"local-patch-apply\"").count(), 3);
            assert_eq!(html.matches(" disabled").count(), 1);
        }
        app.runtime.insert("game".into(), "running".into());
        let mut html = String::new();
        render(&mut html, &app, &game);
        assert_eq!(html.matches(" disabled").count(), 3);
    }

    #[test]
    fn ignores_responses_from_older_requests_for_the_same_game() {
        let mut app = App::default();
        app.local_patches.latest.insert("game".into(), 2);
        assert!(resolve(
            &mut app,
            &Pending::LocalPatchList("game".into()),
            1,
            true,
            r#"{"folder":"old","patches":[]}"#
        ));
        assert!(app.local_patches.reports.is_empty());
        assert!(resolve(
            &mut app,
            &Pending::LocalPatchList("game".into()),
            2,
            true,
            r#"{"folder":"new","patches":[]}"#
        ));
        assert_eq!(app.local_patches.reports["game"].folder, "new");
    }

    #[test]
    fn returning_to_details_refreshes_install_to_switch_without_duplicate_requests() {
        let mut app = App::default();
        let game: Game = serde_json::from_value(json!({"id":"game","title":"Game","folder":"F:/Game","engine":"unknown","modified":0})).unwrap();
        app.catalog.games.push(game.clone());
        app.detail.selected = Some(game.id.clone());
        crate::bridge::take_requests();
        for replaces in [false, true] {
            crate::features::refresh_detail_files(&mut app);
            crate::features::refresh_detail_files(&mut app);
            let requests = crate::bridge::take_requests();
            assert_eq!(requests.iter().filter(|(_, command, _)| command == "list_game_patches").count(), 1);
            assert_eq!(requests.iter().filter(|(_, command, _)| command == "detect_saves").count(), 1);
            for (id, command, _) in requests {
                let pending = app.requests.complete(id).unwrap();
                if command == "list_game_patches" {
                    resolve(&mut app, &pending, id, true, &json!({"folder":"F:/Game/patch","patches":[{"name":"Patch","files":108,"replaces_existing":replaces,"error":null}]}).to_string());
                }
            }
            let mut html = String::new();
            render(&mut html, &app, &game);
            assert!(html.contains(t(&app, if replaces {"local_patches.switch"} else {"local_patches.install"})));
        }
    }

    #[test]
    fn successful_apply_shows_the_message_for_the_original_install_or_switch_action() {
        for switching in [false, true] {
            let mut app = App::default();
            app.catalog.games.push(
                serde_json::from_value(json!({
                    "id":"game", "title":"Game", "folder":"F:/Game", "available":true,
                    "executable":null, "engine":"unknown", "cover":null, "modified":0
                }))
                .unwrap(),
            );
            app.detail.selected = Some("game".into());
            app.local_patches.reports.insert(
                "game".into(),
                Report {
                    folder: "F:/Game/patch".into(),
                    patches: vec![Entry {
                        name: "Patch".into(),
                        files: 1,
                        replaces_existing: switching,
                        error: None,
                    }],
                },
            );
            assert!(dispatch(&mut app, "local-patch-apply", "Patch"));
            assert_eq!(app.toast.as_ref().unwrap().0.key, if switching {"local_patches.switching"} else {"local_patches.installing"});
            let request_id = app.requests.last_id();
            let request = app.requests.complete(request_id).unwrap();
            assert!(resolve(
                &mut app,
                &request,
                request_id,
                true,
                r#"{"folder":"F:/Game/patch","patches":[{"name":"Patch","files":1,"replaces_existing":true,"error":null}]}"#
            ));
            let (message, error) = app.toast.as_ref().unwrap();
            assert!(!error);
            assert_eq!(
                message.key,
                if switching {
                    "local_patches.switched"
                } else {
                    "local_patches.applied"
                }
            );
            assert_eq!(app.toast_generation, 2);
            for language in [Language::Chinese, Language::Japanese, Language::English] {
                assert!(!message.render(language).is_empty());
            }
        }
    }

    #[test]
    fn failed_switch_shows_an_error_instead_of_a_success_message() {
        let mut app = App::default();
        app.local_patches.latest.insert("game".into(), 1);
        assert!(resolve(
            &mut app,
            &Pending::LocalPatchApply("game".into(), true),
            1,
            false,
            r#"{"key":"error.patch","args":{"detail":"failed"}}"#
        ));
        let (message, error) = app.toast.as_ref().unwrap();
        assert!(*error);
        assert_eq!(message.key, "local_patches.failed");
    }
}
