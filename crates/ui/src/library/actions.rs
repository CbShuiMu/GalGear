//! Library actions.
use crate::{
    bridge::reset_scroll, features, library::Dialog, library::clear_selection, library::matching,
    requests::Pending, requests::issue, state::App,
};
use serde_json::json;

fn batch(app: &mut App, operation: &str, ids: Vec<String>, collection: Option<String>) {
    if ids.is_empty() {
        return;
    }
    app.library.last_batch = Some((operation.into(), collection.clone()));
    issue(
        app,
        Pending::Batch,
        "batch_games",
        json!({"operation":operation,"ids":ids,"collection":collection}),
    );
}

fn open_edit(app: &mut App, id: Option<String>, parent: Option<String>) {
    let name = id
        .as_ref()
        .and_then(|id| app.settings.collections.iter().find(|f| &f.id == id))
        .map(|f| f.name.clone())
        .unwrap_or_default();
    app.form.insert("folder-name".into(), name);
    app.form
        .insert("folder-parent".into(), parent.unwrap_or_default());
    app.library.dialog = Some(Dialog::Edit { id });
}

pub(crate) fn dispatch(app: &mut App, action: &str, value: &str) -> bool {
    match action {
        "tag-clear" => {
            app.library.clear_filters();
            clear_selection(&mut app.library);
        }
        "tag-filter-open" => {
            app.library.dialog = Some(Dialog::Tags {
                selected: app.library.tag_filter.clone(),
                status: app.library.status_filter,
            });
            app.form.insert("tag-query".into(), String::new());
            app.choices.open = None;
        }
        "tag-filter" => {
            let tag = app
                .catalog
                .games
                .iter()
                .flat_map(|game| &game.tags)
                .find(|tag| galgear_tags::key(tag) == galgear_tags::key(value))
                .cloned();
            if let (Some(Dialog::Tags { selected, .. }), Some(tag)) = (&mut app.library.dialog, tag) {
                if !selected
                    .iter()
                    .any(|name| galgear_tags::key(name) == galgear_tags::key(&tag))
                {
                    selected.push(tag);
                }
                app.form.insert("tag-query".into(), String::new());
            }
        }
        "tag-filter-remove" => {
            if let Some(Dialog::Tags { selected, .. }) = &mut app.library.dialog {
                selected.retain(|tag| galgear_tags::key(tag) != galgear_tags::key(value));
            }
        }
        "tag-status" => {
            if let Some(Dialog::Tags { status, .. }) = &mut app.library.dialog {
                status.toggle(value);
            }
        }
        "tag-filter-apply" => {
            if let Some(Dialog::Tags { selected, status }) = app.library.dialog.take() {
                app.library.tag_filter = selected;
                app.library.status_filter = status;
                clear_selection(&mut app.library);
                #[cfg(target_arch = "wasm32")]
                reset_scroll();
            }
        }
        "detail-tag" => {
            clear_selection(&mut app.library);
            app.library.collection = None;
            app.library.batch = false;
            app.page = "games".into();
            app.detail.selected = None;
            app.detail.editing = false;
            app.detail.editor = None;
            app.library.dialog = None;
            app.choices.open = None;
            app.library.tag_filter = vec![value.into()];
            app.library.status_filter = Default::default();
            app.library.search.clear();
            #[cfg(target_arch = "wasm32")]
            reset_scroll();
        }
        "folder-tree" => app.library.tree_open = !app.library.tree_open,
        "folder-expand" | "folder-hover" => {
            if action == "folder-hover" || !app.library.expanded.remove(value) {
                app.library.expanded.insert(value.into());
            }
            app.library.tree_open = true;
        }
        "folder-open" => {
            if !app.settings.collections.iter().any(|f| f.id == value) {
                return true;
            }
            clear_selection(&mut app.library);
            app.library.batch = false;
            app.library.collection = Some(value.into());
            app.page = "collections".into();
            app.detail.selected = None;
            app.choices.open = None;
            app.library.search.clear();
            app.library.clear_filters();
            reset_scroll();
        }
        "folder-new" => open_edit(app, None, (!value.is_empty()).then(|| value.into())),
        "folder-edit" => {
            let parent = app
                .settings
                .collections
                .iter()
                .find(|f| f.id == value)
                .and_then(|f| f.parent.clone());
            open_edit(app, Some(value.into()), parent);
        }
        "folder-parent" => {
            app.form.insert("folder-parent".into(), value.into());
        }
        "folder-save" => {
            if let Some(Dialog::Edit { id }) = &app.library.dialog {
                let id = id.clone();
                let parent = features::form(app, "folder-parent");
                issue(
                    app,
                    Pending::CollectionSave,
                    "edit_collection",
                    json!({"operation":if id.is_some(){"update"}else{"create"},"id":id.unwrap_or_default(),"name":features::form(app,"folder-name"),"parent":if parent.is_empty(){None}else{Some(parent)}}),
                );
            }
        }
        "folder-delete" => app.library.dialog = Some(Dialog::Delete { id: value.into() }),
        "folder-delete-confirm" => {
            if let Some(Dialog::Delete { id }) = &app.library.dialog {
                let id = id.clone();
                issue(
                    app,
                    Pending::CollectionSave,
                    "edit_collection",
                    json!({"operation":"delete","id":id,"name":"","parent":null}),
                );
            }
        }
        "library-close" => {
            app.library.dialog = None;
            app.choices.open = None;
        }
        "batch-toggle" => {
            app.library.batch = !app.library.batch;
            clear_selection(&mut app.library);
        }
        "batch-select" => {
            if app.library.batch && !app.library.checked.remove(value) {
                app.library.checked.insert(value.into());
            }
        }
        "folder-select" => {
            if app.library.batch && !app.library.checked_folders.remove(value) {
                app.library.checked_folders.insert(value.into());
            }
        }
        "card-remove-select" => {
            app.library.batch = true;
            app.library.checked.insert(value.into());
        }
        "folder-remove-select" => {
            app.library.batch = true;
            app.library.checked_folders.insert(value.into());
        }
        "folders-delete-selected" => {
            app.library.dialog = Some(Dialog::DeleteFolders {
                ids: app.library.checked_folders.iter().cloned().collect(),
            });
        }
        "folders-delete-confirm" => {
            if let Some(Dialog::DeleteFolders { ids }) = &app.library.dialog {
                let ids = ids.clone();
                issue(
                    app,
                    Pending::CollectionSave,
                    "delete_collections",
                    json!({"ids": ids}),
                );
            }
        }
        "folder-add-games" => {
            app.library.picker_games.clear();
            app.form.insert("game-query".into(), String::new());
            app.library.dialog = Some(Dialog::AddGames);
        }
        "picker-game" => {
            if !app.library.picker_games.remove(value) {
                app.library.picker_games.insert(value.into());
            }
        }
        "picker-games-save" => batch(
            app,
            "add",
            app.library.picker_games.iter().cloned().collect(),
            app.library.collection.clone(),
        ),
        "batch-all" => {
            app.library.checked = matching(&app.catalog, &app.settings, &app.library, &app.page)
                .iter()
                .map(|g| g.id.clone())
                .collect();
            if app.page == "collections" {
                app.library.checked_folders = app
                    .settings
                    .collections
                    .iter()
                    .filter(|f| f.parent == app.library.collection)
                    .map(|f| f.id.clone())
                    .collect();
            }
        }
        "batch-clear" => {
            app.library.checked.clear();
            app.library.checked_folders.clear();
            app.library.batch_failures.clear();
        }
        "collection-picker" => {
            let ids = if value.is_empty() {
                app.library.checked.iter().cloned().collect()
            } else {
                vec![value.into()]
            };
            app.form.insert("folder-query".into(), String::new());
            app.library.dialog = Some(Dialog::Picker { ids });
        }
        "collection-pick" => {
            if let Some(Dialog::Picker { ids }) = &app.library.dialog {
                batch(app, "add", ids.clone(), Some(value.into()));
            }
        }
        "batch-exclude" => {
            app.library.dialog = Some(Dialog::Exclude {
                ids: app.library.checked.iter().cloned().collect(),
            })
        }
        "batch-exclude-confirm" => {
            if let Some(Dialog::Exclude { ids }) = &app.library.dialog {
                batch(app, "exclude", ids.clone(), None);
            }
        }
        "batch-detach" => batch(
            app,
            "detach",
            app.library.checked.iter().cloned().collect(),
            app.library.collection.clone(),
        ),
        "batch-failures" => app.library.dialog = Some(Dialog::Failures),
        "batch-retry" => {
            if let Some((operation, folder)) = app.library.last_batch.clone() {
                batch(
                    app,
                    &operation,
                    app.library
                        .batch_failures
                        .iter()
                        .map(|f| f.id.clone())
                        .collect(),
                    folder,
                );
            }
        }
        "library-warnings" => app.library.dialog = Some(Dialog::Warnings),
        "excluded-open" => {
            app.library.dialog = Some(Dialog::Restore);
            issue(app, Pending::Excluded, "excluded_entries", json!({}));
        }
        "excluded-restore" => batch(app, "restore", vec![value.into()], None),
        "excluded-restore-all" => batch(
            app,
            "restore",
            app.library.excluded.iter().map(|g| g.id.clone()).collect(),
            None,
        ),
        _ => return false,
    }
    true
}
