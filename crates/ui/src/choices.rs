use super::*;

#[derive(Clone)]
pub struct OptionItem {
    pub value: String,
    pub label: String,
    pub subtitle: String,
    pub image: Option<String>,
}
fn item(value: &str, label: &str) -> OptionItem {
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
            let excluded = if let Some(library::Dialog::Edit { id: Some(id) }) = &app.library_dialog
            {
                library::subtree(app, id)
            } else {
                BTreeSet::new()
            };
            for (folder, _) in library::ordered(app, false) {
                let path = library::folder_path(app, &folder.id);
                if !excluded.contains(&folder.id) {
                    values.push(item(&folder.id, &path));
                }
            }
            values
        }
        "sort" => vec![
            item("title", t(app, "games.sort_title")),
            item("recent", t(app, "games.sort_recent")),
        ],
        "fan-mode" => vec![
            item("csrf", "authenticity_token / CSRF"),
            item("bearer", "Bearer token"),
        ],
        "tag-filter" => {
            let mut values = vec![item("", t(app, "games.all_tags"))];
            let tags: std::collections::BTreeSet<_> = app
                .catalog
                .games
                .iter()
                .flat_map(|g| g.tags.iter())
                .collect();
            values.extend(tags.into_iter().map(|tag| item(tag, tag)));
            values
        }
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
        _ => vec![],
    }
}
pub fn current<'a>(app: &'a App, field: &str) -> &'a str {
    match field {
        "folder-parent" => app.form.get("folder-parent").map_or("", String::as_str),
        "sort" => &app.settings.sort_by,
        "tag-filter" => &app.tag_filter,
        "patch-game" => &app.patch_game,
        "patch-search-game" => &app.patch_search_game,
        "fan-mode" => &app.credentials.mode,
        _ => "",
    }
}
pub fn filtered(app: &App, field: &str) -> Vec<OptionItem> {
    let query = app.choice_query.trim().to_lowercase();
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
    let Some(selected) = values
        .iter()
        .find(|v| v.value == current(app, field))
        .or_else(|| values.first())
    else {
        return;
    };
    let open = app.choice.as_deref() == Some(field);
    write!(html,r#"<div class="choice-control {}"><button type="button" id="{field}" class="choice-trigger {}" role="combobox" aria-label="{}" aria-expanded="{open}" aria-haspopup="listbox" aria-controls="choice-list" data-action="choice-toggle" data-value="{field}" {}>"#,if matches!(field,"patch-game"|"patch-search-game"){"game-choice"}else{""},if open{"open"}else{""},esc(label),if disabled{"disabled"}else{""}).unwrap();
    visual(
        html,
        selected,
        matches!(field, "patch-game" | "patch-search-game"),
    );
    html.push_str(&icon("chevron"));
    html.push_str("</button></div>");
}

pub fn popup(html: &mut String, app: &App) {
    let Some(field) = app.choice.as_deref() else {
        return;
    };
    let values = filtered(app, field);
    let searchable = matches!(
        field,
        "patch-game" | "patch-search-game" | "tag-filter" | "folder-parent"
    );
    let label = match field {
        "patch-game" => t(app, "patches.target"),
        "patch-search-game" => t(app, "patches.library_search"),
        "tag-filter" => t(app, "edit.tags"),
        "folder-parent" => t(app, "folders.parent"),
        "sort" => t(app, "games.sort_label"),
        _ => t(app, "auth.token_type"),
    };
    write!(
        html,
        r#"<section class="choice-popup {}" data-choice-anchor="{}" aria-label="{}">"#,
        if matches!(field, "patch-game" | "patch-search-game") {
            "game-menu"
        } else {
            ""
        },
        field,
        esc(label)
    )
    .unwrap();
    if searchable {
        write!(html,r#"<div class="choice-search">{}<input id="choice-query" data-input="choice-query" type="search" autocomplete="off" value="{}" placeholder="{}" aria-label="{}" aria-controls="choice-list" aria-activedescendant="choice-option-{}"/></div>"#,icon("search"),esc(&app.choice_query),t(app,"choice.search"),t(app,"choice.search"),app.choice_active).unwrap();
    }
    write!(html,r#"<div class="choice-list" id="choice-list" role="listbox" tabindex="-1" aria-label="{}" aria-activedescendant="choice-option-{}">"#,esc(label),app.choice_active).unwrap();
    for (index, option) in values.iter().enumerate() {
        let selected = option.value == current(app, field);
        let payload = serde_json::to_string(&[field, option.value.as_str()]).unwrap();
        write!(html,r#"<button type="button" id="choice-option-{index}" class="choice-option {} {}" role="option" aria-selected="{selected}" tabindex="-1" data-action="choice-pick" data-value="{}" title="{}">"#,if selected{"selected"}else{""},if app.choice_active==index{"active"}else{""},esc(&payload),esc(&option.label)).unwrap();
        visual(
            html,
            option,
            matches!(field, "patch-game" | "patch-search-game"),
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
    if options(app, field).len() > 80 && app.choice_query.is_empty() {
        write!(
            html,
            "<p class=\"choice-limit\">{}</p>",
            t(app, "choice.more")
        )
        .unwrap();
    }
    html.push_str("</section>");
}
