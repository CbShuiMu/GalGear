//! 批量库目录添加结果与分页失败报告。
use crate::{
    game_setup,
    library::Dialog,
    models::{LibraryFolderFailure, LibraryFoldersResult},
    requests::{Pending, issue},
    state::{App, apply_settings, show_toast},
    view::t,
    widgets::{button, esc},
};
use galgear_i18n::Message;
use serde_json::json;
use std::fmt::Write;

const FAILURE_PAGE_SIZE: usize = 40;

pub(crate) fn added(app: &mut App, id: u32, result: LibraryFoldersResult) {
    apply_settings(app, result.settings, id);
    show_toast(
        app,
        Message::new("library.folders_added")
            .arg("added", result.added)
            .arg("skipped", result.unchanged)
            .arg("failed", result.failed.len()),
        !result.failed.is_empty(),
    );
    if !result.failed.is_empty() {
        app.library.dialog = Some(Dialog::FolderFailures {
            failed: result.failed,
            page: 0,
        });
    }
    if result.added == 0 {
        return;
    }
    game_setup::refreshed(app);
    // 正在扫描的任务持有旧目录快照，等它结束再合并补扫。
    if app.library.scanning {
        app.library.rescan_pending = true;
    } else {
        issue(app, Pending::BackgroundScan, "scan_library", json!({}));
    }
}

pub(crate) fn change_page(app: &mut App, value: &str) {
    if let Some(Dialog::FolderFailures { failed, page }) = &mut app.library.dialog
        && let Ok(next) = value.parse::<usize>()
        && next < failed.len().div_ceil(FAILURE_PAGE_SIZE)
    {
        *page = next;
    }
}

pub(crate) fn render_failures(
    html: &mut String,
    app: &App,
    failed: &[LibraryFolderFailure],
    page: usize,
) {
    let pages = failed.len().div_ceil(FAILURE_PAGE_SIZE);
    html.push_str("<ul class=\"library-folder-failures\">");
    for failure in failed
        .iter()
        .skip(page * FAILURE_PAGE_SIZE)
        .take(FAILURE_PAGE_SIZE)
    {
        let _ = write!(
            html,
            "<li><strong>{}</strong><p>{}</p></li>",
            esc(&failure.path),
            esc(&failure.error.render(app.settings.language))
        );
    }
    html.push_str("</ul>");
    let _ = write!(
        html,
        "<div class=\"library-dialog-footer\">{}<span>{} / {}</span>{}</div>",
        button(
            "folder-failures-page",
            &page.saturating_sub(1).to_string(),
            t(app, "library.previous_page"),
            "",
            "quiet",
            page == 0
        ),
        page + 1,
        pages,
        button(
            "folder-failures-page",
            &(page + 1).to_string(),
            t(app, "library.next_page"),
            "",
            "quiet",
            page + 1 >= pages
        )
    );
}
