//! 详情顶部本地标签的增删改与排序；资料中的人物、品牌等标签仍仅用于筛选。
use crate::{
    requests::{Pending, issue},
    state::App,
    view::{f, t},
    widgets::{button, esc, icon_button},
};
use serde_json::json;
use std::fmt::Write;

#[derive(serde::Deserialize)]
struct Move {
    id: String,
    tag: String,
    before: Option<String>,
}

fn reorder(app: &mut App, value: &str) {
    if app.busy()
        || app.detail.editing
        || app.detail.editor.is_some()
        || app.detail.tag_draft.is_some()
        || app.detail.confirmation.is_some()
    {
        return;
    }
    let Ok(movement) = serde_json::from_str::<Move>(value) else {
        return;
    };
    if app.detail.selected.as_ref() != Some(&movement.id) {
        return;
    }
    let Some(game) = app.catalog.games.iter().find(|game| game.id == movement.id) else {
        return;
    };
    let Some(from) = game.tags.iter().position(|tag| tag == &movement.tag) else {
        return;
    };
    let target = match movement.before.as_ref() {
        Some(before) => match game.tags.iter().position(|tag| tag == before) {
            Some(index) => index,
            None => return,
        },
        None => game.tags.len(),
    };
    let to = target.saturating_sub(usize::from(from < target));
    if from == to {
        return;
    }
    let mut names = game.tags.clone();
    let tag = names.remove(from);
    names.insert(to, tag);
    // 只提交移动意图，使用最新标签计算；成功响应负责更新目录及后续编辑草稿。
    issue(
        app,
        Pending::Edit,
        "edit_tags",
        json!({"id":game.id,"names":names}),
    );
}

#[derive(Default)]
pub(crate) struct Draft {
    pub(crate) original: Option<String>,
    pub(crate) value: String,
}

pub(crate) fn input(app: &mut App, field: &str, value: &str) -> bool {
    if field != "detail-tag-input" {
        return false;
    }
    if let Some(draft) = &mut app.detail.tag_draft {
        draft.value = value.into();
    }
    true
}

pub(crate) fn dispatch(app: &mut App, action: &str, value: &str) -> bool {
    match action {
        "detail-tag-reorder" => reorder(app, value),
        "detail-tag-add" | "detail-tag-edit" => {
            if app.busy() || app.detail.editing || app.detail.editor.is_some() {
                return true;
            }
            let Some(game) = app
                .catalog
                .games
                .iter()
                .find(|game| Some(&game.id) == app.detail.selected.as_ref())
            else {
                return true;
            };
            if action == "detail-tag-edit" && !game.tags.iter().any(|tag| tag == value) {
                return true;
            }
            app.detail.tag_draft = Some(Draft {
                original: (action == "detail-tag-edit").then(|| value.into()),
                value: if action == "detail-tag-edit" {
                    value.into()
                } else {
                    String::new()
                },
            });
        }
        "detail-tag-cancel" => app.detail.tag_draft = None,
        "detail-tag-save" => {
            if app.busy() {
                return true;
            }
            let Some(draft) = &app.detail.tag_draft else {
                return true;
            };
            if draft.value.trim().is_empty() {
                return true;
            }
            let Some(game) = app
                .catalog
                .games
                .iter()
                .find(|game| Some(&game.id) == app.detail.selected.as_ref())
            else {
                return true;
            };
            let mut names = game.tags.clone();
            if let Some(original) = &draft.original {
                let Some(name) = names.iter_mut().find(|name| *name == original) else {
                    return true;
                };
                *name = draft.value.trim().into();
            } else {
                names.push(draft.value.trim().into());
            }
            issue(
                app,
                Pending::Edit,
                "edit_tags",
                json!({"id":game.id,"names":galgear_tags::normalize(names)}),
            );
        }
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
    if app
        .detail
        .tag_draft
        .as_ref()
        .is_some_and(|draft| draft.original.as_deref() == Some(tag))
    {
        render_input(html, app);
        return;
    }
    let label =
        galgear_i18n::format_text(app.settings.language, "tags.remove_label", &[("tag", tag)]);
    let _ = write!(
        html,
        r#"<span class="detail-tag-chip" data-render-key="detail-tag:{}" title="{}">{}{}{}</span>"#,
        esc(tag),
        esc(t(app, "tags.reorder_hint")),
        icon_button(
            "detail-tag-edit",
            tag,
            &galgear_i18n::format_text(app.settings.language, "tags.edit_label", &[("tag", tag)]),
            "edit",
            "tag-edit-button",
            app.busy() || app.detail.editing
        ),
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

fn render_input(html: &mut String, app: &App) {
    let Some(draft) = &app.detail.tag_draft else {
        return;
    };
    let _ = write!(
        html,
        r#"<span class="detail-tag-chip tag-input-chip" data-render-key="detail-tag-input"><span class="tag-input-wrap" data-value="{}"><input id="detail-tag-input" size="1" data-input="detail-tag-input" value="{}" aria-label="{}" autocomplete="off" {}/></span>{}</span>"#,
        esc(&draft.value),
        esc(&draft.value),
        t(app, "tags.name"),
        if app.busy() { "disabled" } else { "" },
        icon_button(
            "detail-tag-cancel",
            "",
            t(app, "common.cancel"),
            "close",
            "tag-cancel-button",
            app.busy()
        )
    );
}

pub(crate) fn render_add(html: &mut String, app: &App) {
    if app
        .detail
        .tag_draft
        .as_ref()
        .is_some_and(|draft| draft.original.is_none())
    {
        render_input(html, app);
    } else {
        html.push_str(&icon_button(
            "detail-tag-add",
            "",
            t(app, "tags.add"),
            "plus",
            "tag-add-button",
            app.busy(),
        ));
    }
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
        r#"<div class="editor-overlay"><section class="editor-dialog tag-remove-dialog" role="alertdialog" aria-modal="true" aria-labelledby="tag-remove-title" aria-describedby="tag-remove-description"><header><h2 id="tag-remove-title">{}</h2>{}</header><div class="editor-body"><p id="tag-remove-description">{}</p></div><footer class="tag-remove-actions">{}{}</footer></section></div>"#,
        t(app, "tags.remove_title"),
        icon_button(
            "detail-tag-remove-cancel",
            "",
            t(app, "common.close"),
            "close",
            "",
            false
        ),
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
    fn reorder_moves_to_front_middle_and_end_without_changing_names() {
        for (tag, before, expected) in [
            ("剧情", Some("ADV"), vec!["剧情", "ADV", "学园"]),
            ("ADV", Some("剧情"), vec!["学园", "ADV", "剧情"]),
            ("ADV", None, vec!["学园", "剧情", "ADV"]),
        ] {
            let mut app = fixture(&["ADV", "学园", "剧情"]);
            dispatch(
                &mut app,
                "detail-tag-reorder",
                &json!({"id":"game","tag":tag,"before":before}).to_string(),
            );
            let requests = take_requests();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].1, "edit_tags");
            assert_eq!(
                serde_json::from_str::<Value>(&requests[0].2).unwrap()["names"],
                json!(expected)
            );
            assert_eq!(app.catalog.games[0].tags, ["ADV", "学园", "剧情"]);
        }
    }

    #[test]
    fn reorder_ignores_invalid_stale_and_unchanged_moves() {
        let mut app = fixture(&["ADV", "学园"]);
        for movement in [
            json!({"id":"other","tag":"ADV","before":null}),
            json!({"id":"game","tag":"missing","before":null}),
            json!({"id":"game","tag":"ADV","before":"missing"}),
            json!({"id":"game","tag":"ADV","before":"ADV"}),
            json!({"id":"game","tag":"ADV","before":"学园"}),
            json!({"id":"game","tag":"学园","before":null}),
            json!("invalid"),
        ] {
            dispatch(&mut app, "detail-tag-reorder", &movement.to_string());
            assert!(take_requests().is_empty());
        }
        let movement = json!({"id":"game","tag":"ADV","before":null}).to_string();
        crate::detail_form::begin(&mut app);
        dispatch(&mut app, "detail-tag-reorder", &movement);
        assert!(take_requests().is_empty());
        crate::detail_form::cancel(&mut app);
        dispatch(&mut app, "detail-tag-add", "");
        dispatch(&mut app, "detail-tag-reorder", &movement);
        assert!(take_requests().is_empty());
    }

    #[test]
    fn reorder_success_and_failure_are_reflected_in_the_edit_field() {
        for success in [false, true] {
            let mut app = fixture(&["ADV", "学园", "剧情"]);
            let movement = json!({"id":"game","tag":"ADV","before":null}).to_string();
            dispatch(&mut app, "detail-tag-reorder", &movement);
            dispatch(&mut app, "detail-tag-reorder", &movement);
            let requests = take_requests();
            assert_eq!(requests.len(), 1);
            let saved = json!({"id":"game", "title":"Game", "folder":"X:/Fixture",
                "engine":"unknown", "modified":0, "tags":["学园", "剧情", "ADV"]});
            APP.with(|state| *state.borrow_mut() = app);
            responses::resolve(
                requests[0].0,
                success,
                &if success {
                    saved.to_string()
                } else {
                    "save failed".into()
                },
            );
            APP.with(|state| {
                let mut app = state.borrow_mut();
                crate::detail_form::begin(&mut app);
                assert_eq!(
                    crate::features::form(&app, "draft-tags"),
                    if success {
                        "学园, 剧情, ADV"
                    } else {
                        "ADV, 学园, 剧情"
                    }
                );
            });
        }
    }

    #[test]
    fn inline_add_and_rename_preserve_other_tags_and_wait_for_success() {
        let mut app = fixture(&["ADV", "学园"]);
        dispatch(&mut app, "detail-tag-add", "");
        super::input(&mut app, "detail-tag-input", "  剧情  ");
        dispatch(&mut app, "detail-tag-save", "");
        let request = take_requests().remove(0);
        let payload: Value = serde_json::from_str(&request.2).unwrap();
        assert_eq!(payload["names"], json!(["ADV", "学园", "剧情"]));
        assert_eq!(app.catalog.games[0].tags, ["ADV", "学园"]);
        assert!(app.detail.tag_draft.is_some());
        app.requests.complete(request.0);
        dispatch(&mut app, "detail-tag-cancel", "");
        dispatch(&mut app, "detail-tag-edit", "学园");
        super::input(&mut app, "detail-tag-input", "校园");
        dispatch(&mut app, "detail-tag-save", "");
        let request = take_requests().remove(0);
        let payload: Value = serde_json::from_str(&request.2).unwrap();
        assert_eq!(payload["names"], json!(["ADV", "校园"]));
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
