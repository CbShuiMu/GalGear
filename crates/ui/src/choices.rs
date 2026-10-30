use crate::{
    ai, cloud, game_setup, library, state::App, view::engine, view::t, widgets::esc, widgets::icon,
};
use std::{collections::BTreeSet, fmt::Write};

#[derive(Default)]
pub(crate) struct State {
    pub(crate) open: Option<String>,
    pub(crate) query: String,
    pub(crate) active: usize,
}

#[derive(Clone)]
pub struct OptionItem {
    pub value: String,
    pub label: String,
    pub subtitle: String,
    pub image: Option<String>,
}
pub(crate) fn item(value: &str, label: &str) -> OptionItem {
    OptionItem {
        value: value.into(),
        label: label.into(),
        subtitle: String::new(),
        image: None,
    }
}

pub fn options(app: &App, field: &str) -> Vec<OptionItem> {
    match field {
        "folder-parent" => {
            let mut values = vec![item("", t(app, "folders.root"))];
            let excluded = if let Some(library::Dialog::Edit { id: Some(id) }) = &app.library.dialog
            {
                library::subtree(&app.settings.collections, id)
            } else {
                BTreeSet::new()
            };
            for (folder, _) in
                library::ordered(&app.settings.collections, &app.library.expanded, false)
            {
                let path = library::folder_path(&app.settings.collections, &folder.id);
                if !excluded.contains(&folder.id) && folder.parent.is_none() && excluded.len() <= 1 {
                    values.push(item(&folder.id, &path));
                }
            }
            values
        }
        "sort" => vec![
            item("title", t(app, "games.sort_title")),
            item("recent", t(app, "games.sort_recent")),
            item("release", t(app, "games.sort_release")),
        ],
        "fan-mode" => vec![
            item("csrf", "authenticity_token / CSRF"),
            item("bearer", "Bearer token"),
        ],
        "cloud-game" => cloud::choice_options(app),
        "patch-game" | "patch-search-game" => {
            let mut games: Vec<_> = app
                .catalog
                .games
                .iter()
                .filter(|g| field == "patch-search-game" || g.executable.is_some() && g.available)
                .collect();
            games.sort_by_cached_key(|g| g.title.to_lowercase());
            let mut values = vec![item(
                "",
                t(
                    app,
                    if field == "patch-search-game" {
                        "patches.library_placeholder"
                    } else {
                        "choice.game_placeholder"
                    },
                ),
            )];
            values.extend(games.into_iter().map(|g| {
                OptionItem {
                    value: g.id.clone(),
                    label: g.title.clone(),
                    subtitle: engine(app, g).into(),
                    image: app
                        .game_icons
                        .get(&g.id)
                        .cloned()
                        .or_else(|| g.cover.clone()),
                }
            }));
            values
        }
        field if field.starts_with("ai-model:") => ai::choice_options(app, field),
        _ => game_setup::choice_options(app, field),
    }
}
pub fn current<'a>(app: &'a App, field: &str) -> &'a str {
    match field {
        "folder-parent" => app.form.get("folder-parent").map_or("", String::as_str),
        "sort" => &app.settings.sort_by,
        "patch-game" => &app.patches.patch_game,
        "patch-search-game" => &app.patches.patch_search_game,
        "fan-mode" => &app.account.credentials.mode,
        "cloud-game" => cloud::choice_current(app),
        field if field.starts_with("ai-model:") => ai::choice_current(app, field),
        _ => game_setup::choice_current(app, field),
    }
}
pub fn filtered(app: &App, field: &str) -> Vec<OptionItem> {
    let query = app.choices.query.trim().to_lowercase();
    let all = options(app, field);
    let mut values: Vec<_> = all
        .iter()
        .filter(|o| {
            query.is_empty()
                || o.label.to_lowercase().contains(&query)
                || o.subtitle.to_lowercase().contains(&query)
        })
        .take(80)
        .cloned()
        .collect();
    if query.is_empty()
        && !values.iter().any(|o| o.value == current(app, field))
        && let Some(selected) = all.iter().find(|o| o.value == current(app, field))
    {
        values.truncate(79);
        values.insert(0, selected.clone());
    }
    values
}

fn visual(html: &mut String, item: &OptionItem, game: bool) {
    if game {
        if let Some(image) = &item.image {
            write!(
                html,
                r#"<span class="choice-game-icon"><img src="{}" alt="" loading="lazy"/></span>"#,
                esc(image)
            )
            .unwrap();
        } else {
            write!(
                html,
                r#"<span class="choice-game-icon fallback">{}</span>"#,
                icon("games")
            )
            .unwrap();
        }
    }
    write!(
        html,
        r#"<span class="choice-copy"><span>{}</span>"#,
        esc(&item.label)
    )
    .unwrap();
    if !item.subtitle.is_empty() {
        write!(html, "<small>{}</small>", esc(&item.subtitle)).unwrap();
    }
    html.push_str("</span>");
}

pub fn control(html: &mut String, app: &App, field: &str, label: &str, disabled: bool) {
    let values = options(app, field);
    let placeholder = game_setup::choice_placeholder(app, field).map(|label| item("", label));
    let Some(selected) = values
        .iter()
        .find(|v| v.value == current(app, field))
        .or(placeholder.as_ref())
        .or_else(|| values.first())
    else {
        return;
    };
    let open = app.choices.open.as_deref() == Some(field);
    write!(html,r#"<div class="choice-control {}"><button type="button" id="{}" class="choice-trigger {}" role="combobox" aria-label="{}" aria-expanded="{open}" aria-haspopup="listbox" aria-controls="choice-list" data-action="choice-toggle" data-value="{}" {}>"#,if matches!(field,"patch-game"|"patch-search-game"|"cloud-game"){"game-choice"}else{""},esc(field),if open{"open"}else{""},esc(label),esc(field),if disabled{"disabled"}else{""}).unwrap();
    visual(
        html,
        selected,
        matches!(field, "patch-game" | "patch-search-game" | "cloud-game"),
    );
    html.push_str(&icon("chevron"));
    html.push_str("</button></div>");
}

pub fn popup(html: &mut String, app: &App) {
    let Some(field) = app.choices.open.as_deref() else {
        return;
    };
    let values = filtered(app, field);
    let searchable = matches!(
        field,
        "patch-game" | "patch-search-game" | "folder-parent" | "cloud-game"
    ) || field.starts_with("ai-model:");
    let label = match field {
        "patch-game" => t(app, "patches.target"),
        "cloud-game" => t(app, "cloud.game"),
        "patch-search-game" => t(app, "patches.library_search"),
        "folder-parent" => t(app, "folders.parent"),
        "sort" => t(app, "games.sort_label"),
        field if field.starts_with("ai-model:") => t(app, "ai.model"),
        field if field.starts_with("setup-source:") => t(app, "setup.source"),
        field if field.starts_with("setup-candidate:") => t(app, "setup.entry"),
        _ => t(app, "auth.token_type"),
    };
    write!(
        html,
        r#"<section class="choice-popup {}" data-render-key="choice-popup" data-choice-anchor="{}" aria-label="{}">"#,
        if matches!(field, "patch-game" | "patch-search-game" | "cloud-game") {
            "game-menu"
        } else if field.starts_with("setup-") {
            "setup-menu"
        } else {
            ""
        },
        esc(field),
        esc(label)
    )
    .unwrap();
    if searchable {
        write!(html,r#"<div class="choice-search">{}<input id="choice-query" data-input="choice-query" type="search" autocomplete="off" value="{}" placeholder="{}" aria-label="{}" aria-controls="choice-list" aria-activedescendant="choice-option-{}"/></div>"#,icon("search"),esc(&app.choices.query),t(app,"choice.search"),t(app,"choice.search"),app.choices.active).unwrap();
    }
    write!(html,r#"<div class="choice-list" id="choice-list" role="listbox" tabindex="-1" aria-label="{}" aria-activedescendant="choice-option-{}">"#,esc(label),app.choices.active).unwrap();
    for (index, option) in values.iter().enumerate() {
        let selected = option.value == current(app, field);
        let payload = serde_json::to_string(&[field, option.value.as_str()]).unwrap();
        write!(html,r#"<button type="button" id="choice-option-{index}" class="choice-option {} {}" role="option" aria-selected="{selected}" tabindex="-1" data-action="choice-pick" data-value="{}" title="{}">"#,if selected{"selected"}else{""},if app.choices.active==index{"active"}else{""},esc(&payload),esc(&option.label)).unwrap();
        visual(
            html,
            option,
            matches!(field, "patch-game" | "patch-search-game" | "cloud-game"),
        );
        if selected {
            html.push_str(&icon("check"));
        } else {
            html.push_str("<span class=\"choice-check-space\"></span>");
        }
        html.push_str("</button>");
    }
    if values.is_empty() {
        write!(
            html,
            "<p class=\"choice-empty\">{}</p>",
            t(app, "choice.empty")
        )
        .unwrap();
    }
    html.push_str("</div>");
    if options(app, field).len() > 80 && app.choices.query.is_empty() {
        write!(
            html,
            "<p class=\"choice-limit\">{}</p>",
            t(app, "choice.more")
        )
        .unwrap();
    }
    html.push_str("</section>");
}
