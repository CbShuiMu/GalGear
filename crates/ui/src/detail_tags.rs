//! 详情顶部本地标签的删除交互；资料中的人物、品牌等标签仍仅用于筛选。
use crate::{
    requests::{Pending, issue},
    state::App,
    view::{f, t},
    widgets::{button, esc, icon_button},
};
use serde_json::json;
use std::fmt::Write;

pub(crate) fn dispatch(app: &mut App, action: &str, value: &str) -> bool {
    match action {
        "detail-tag-remove" => {
            if !app.busy()
                && !app.detail.editing
                && app.detail.editor.is_none()
                && app.detail.selected.as_ref().is_some_and(|id| {
                    app.catalog
                        .games
                        .iter()
                        .any(|game| &game.id == id && game.tags.iter().any(|tag| tag == value))
                })
            {
                app.detail.confirmation = Some((action.into(), value.into()));
            }
        }
        "detail-tag-remove-confirm" => {
            if app.busy() {
                return true;
            }
            let Some((kind, tag)) = app.detail.confirmation.as_ref() else {
                return true;
            };
            if kind != "detail-tag-remove" {
                return true;
            }
            // 确认时读取最新标签，保留弹窗打开期间资料刷新新增的名称。
            if let Some(game) = app.catalog.games.iter().find(|game| {
                Some(&game.id) == app.detail.selected.as_ref() && game.tags.contains(tag)
            }) {
                let key = galgear_tags::key(tag);
                let names: Vec<_> = game
                    .tags
                    .iter()
                    .filter(|name| galgear_tags::key(name) != key)
                    .collect();
                let payload = json!({"id": game.id, "names": names});
                issue(app, Pending::Edit, "edit_tags", payload);
            }
            app.detail.confirmation = None;
        }
        "detail-tag-remove-cancel" => {
            if app
                .detail
                .confirmation
                .as_ref()
                .is_some_and(|(kind, _)| kind == "detail-tag-remove")
            {
                app.detail.confirmation = None;
            }
        }
        _ => return false,
    }
    true
}

pub(crate) fn render_tag(html: &mut String, app: &App, tag: &str) {
    let label =
        galgear_i18n::format_text(app.settings.language, "tags.remove_label", &[("tag", tag)]);
    let _ = write!(
        html,
        r#"<span class="detail-tag-chip" data-render-key="detail-tag:{}">{}{}</span>"#,
        esc(tag),
        button("detail-tag", tag, &esc(tag), "", "tag-button", false),
        icon_button(
            "detail-tag-remove",
            tag,
            &label,
            "close",
            "tag-remove-button",
            app.busy() || app.detail.editing
        )
    );
}

pub(crate) fn render_dialog(html: &mut String, app: &App) {
    let Some((kind, tag)) = &app.detail.confirmation else {
        return;
    };
    if kind != "detail-tag-remove" {
        return;
    }
    let _ = write!(
        html,
        r#"<div class="editor-overlay"><section class="editor-dialog tag-remove-dialog" role="alertdialog" aria-modal="true" aria-labelledby="tag-remove-title" aria-describedby="tag-remove-description"><header><h2 id="tag-remove-title">{}</h2></header><div class="editor-body"><p id="tag-remove-description">{}</p></div><footer class="tag-remove-actions">{}{}</footer></section></div>"#,
        t(app, "tags.remove_title"),
        f(app, "tags.remove_confirm", &[("tag", tag)]),
        button(
            "detail-tag-remove-cancel",
            "",
            t(app, "common.cancel"),
            "",
            "quiet",
            false
        ),
        button(
            "detail-tag-remove-confirm",
            "",
            t(app, "common.confirm"),
            "",
            "primary",
            app.busy()
        )
    );
}

#[cfg(test)]
mod tests {
    use super::{dispatch, render_dialog, render_tag};
    use crate::{
        bridge::take_requests,
        responses,
        state::{APP, App},
    };
    use serde_json::{Value, json};

    fn fixture(tags: &[&str]) -> App {
        take_requests();
        let mut app = App::default();
        app.catalog.games.push(
            serde_json::from_value(json!({
                "id":"game", "title":"Game", "folder":"X:/Fixture", "engine":"unknown",
                "modified":0, "tags":tags, "metadata":{"tags":["original"]}
            }))
            .unwrap(),
        );
        app.detail.selected = Some("game".into());
        app
    }

    #[test]
    fn cancel_and_unconfirmed_actions_never_save() {
        let mut app = fixture(&["ADV", "学园"]);
        dispatch(&mut app, "detail-tag-remove-confirm", "");
        dispatch(&mut app, "detail-tag-remove", "missing");
        assert!(app.detail.confirmation.is_none());
        dispatch(&mut app, "detail-tag-remove", "ADV");
        assert!(app.detail.confirmation.is_some());
        dispatch(&mut app, "detail-tag-remove-cancel", "");
        dispatch(&mut app, "detail-tag-remove-confirm", "");
        assert!(take_requests().is_empty());
        assert_eq!(app.catalog.games[0].tags, ["ADV", "学园"]);
    }

    #[test]
    fn confirmation_preserves_new_tags_and_waits_for_the_save_result() {
        let mut app = fixture(&["ADV", "学园"]);
        dispatch(&mut app, "detail-tag-remove", "ADV");
        app.catalog.games[0].tags.push("新增".into());
        dispatch(&mut app, "detail-tag-remove-confirm", "");
        dispatch(&mut app, "detail-tag-remove-confirm", "");
        let requests = take_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].1, "edit_tags");
        assert_eq!(
            serde_json::from_str::<Value>(&requests[0].2).unwrap(),
            json!({"id":"game", "names":["学园", "新增"]})
        );
        assert_eq!(app.catalog.games[0].tags, ["ADV", "学园", "新增"]);
    }

    #[test]
    fn failed_removal_keeps_the_tag_and_success_can_remove_the_last_tag() {
        for success in [false, true] {
            let mut app = fixture(&["ADV"]);
            let metadata = serde_json::to_value(&app.catalog.games[0].metadata).unwrap();
            let saved = json!({"id":"game", "title":"Game", "folder":"X:/Fixture", "engine":"unknown", "modified":0, "tags":[], "metadata":metadata});
            dispatch(&mut app, "detail-tag-remove", "ADV");
            dispatch(&mut app, "detail-tag-remove-confirm", "");
            let requests = take_requests();
            assert_eq!(
                serde_json::from_str::<Value>(&requests[0].2).unwrap()["names"],
                json!([])
            );
            APP.with(|state| *state.borrow_mut() = app);
            let payload = if success {
                saved.to_string()
            } else {
                "save failed".into()
            };
            responses::resolve(requests[0].0, success, &payload);
            APP.with(|state| {
                let app = state.borrow();
                assert_eq!(app.catalog.games[0].tags.is_empty(), success);
                assert_eq!(app.toast.as_ref().unwrap().1, !success);
                assert_eq!(
                    serde_json::to_value(&app.catalog.games[0].metadata).unwrap(),
                    saved["metadata"]
                );
            });
        }
    }

    #[test]
    fn tag_and_dialog_escape_names_once_and_remove_button_has_no_visible_text() {
        let mut app = fixture(&["A&B <tag> \"quoted\""]);
        let tag = app.catalog.games[0].tags[0].clone();
        let mut html = String::new();
        render_tag(&mut html, &app, &tag);
        assert!(!html.contains("&amp;amp;"));
        assert!(!html.contains("<tag>"));
        let remove = html.split("tag-remove-button").nth(1).unwrap();
        assert!(remove.contains("<svg"));
        assert!(!remove.contains("<span>"));
        dispatch(&mut app, "detail-tag-remove", &tag);
        html.clear();
        render_dialog(&mut html, &app);
        assert!(html.contains("role=\"alertdialog\""));
        assert!(html.contains("A&amp;B &lt;tag&gt; &quot;quoted&quot;"));
        assert!(html.contains("<footer class=\"tag-remove-actions\">"));
    }
}
