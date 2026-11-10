//! Home game shelf. Selection is independent of library filters and detail state.
use crate::{
    game_actions,
    models::Game,
    playtime,
    state::App,
    view::{poster, t},
    widgets::{button, esc, icon},
};
use std::fmt::Write;

fn ordered(app: &App) -> Vec<&Game> {
    let mut games: Vec<_> = app.catalog.games.iter().filter(|game| crate::game_setup::visible(&app.settings, game)).collect();
    crate::library::sort_games(&mut games, &app.settings);
    games
}

pub(crate) fn render(html: &mut String, app: &App) {
    let games = ordered(app);
    let selected = games
        .iter()
        .copied()
        .find(|game| Some(&game.id) == app.home_selected.as_ref())
        .or_else(|| games.first().copied());
    let _ = write!(
        html,
        r#"<section class="home-page" aria-label="{}">"#,
        t(app, "nav.home")
    );
    if let Some(game) = selected {
        let background = game
            .cover_artwork
            .as_ref()
            .map(|art| format!("http://galgear-cover.localhost/{}", art.background));
        let cover = game
            .cover
            .as_deref()
            .map(|cover| background.as_deref().unwrap_or(cover))
            .unwrap_or("");
        // Rust supplies the target; JavaScript retains decoded layers until their transitions finish.
        let _ = write!(
            html,
            r#"<div class="home-backdrop" data-render-key="home-backdrop" data-cover="{}" aria-hidden="true"></div>"#,
            esc(cover)
        );
        let _ = write!(
            html,
            r#"<div class="home-shelf" data-render-key="home-shelf" data-focus-scroll="managed" aria-label="{}">"#,
            t(app, "home.recent")
        );
        for item in games {
            let _ = write!(
                html,
                r#"<button id="home-game-{}" class="home-game {}" data-action="home-select" data-value="{}" aria-label="{}" aria-pressed="{}"><span class="poster">"#,
                esc(&item.id),
                if item.id == game.id { "selected" } else { "" },
                esc(&item.id),
                esc(&item.title),
                item.id == game.id
            );
            poster(html, app, item);
            if !item.available {
                let _ = write!(html, r#"<span class="path-warning" role="img" aria-label="{}" title="{}">{}</span>"#, t(app,"library.path_warning"), t(app,"library.path_warning"), icon("error"));
            }
            html.push_str("</span></button>");
        }
        html.push_str(r#"</div><div class="home-feature" data-render-key="home-feature"><div class="home-copy">"#);
        let _ = write!(html, "<h2>{}</h2>", esc(&game.title));
        if let Some(metadata) = &game.metadata {
            let _ = write!(
                html,
                "<p class=\"home-description\">{}</p>",
                esc(&metadata.description)
            );
        }
        html.push_str("<div class=\"home-actions\">");
        html.push_str(&button(
            "launch",
            &game.id,
            t(app, "detail.launch"),
            "play",
            "primary game-launch-button",
            app.busy()
                || !game.available
                || app
                    .runtime
                    .get(&game.id)
                    .is_some_and(|status| status == "running"),
        ));
        html.push_str(&button(
            "game-detail",
            &game.id,
            t(app, "context.detail"),
            "about",
            "secondary icon-only",
            false,
        ));
        html.push_str(r#"<div class="home-game-status">"#);
        game_actions::render(html, app, game);
        let last_played = game
            .last_played
            .map(|seconds| playtime::recent_date(app.settings.language, seconds))
            .filter(|date| !date.is_empty());
        let _ = write!(
            html,
            r#"<span class="home-playtime">{}</span></div>"#,
            last_played
                .as_deref()
                .map(|date| crate::view::f(app, "home.last_played", &[("date", date)]))
                .unwrap_or_else(|| t(app, "home.never_played").into())
        );
        html.push_str("</div></div></div>");
    } else {
        let _ = write!(
            html,
            "<div class=\"home-empty\"><h1 id=\"home-title\">{}</h1><p>{}</p>{}</div>",
            t(app, "games.none"),
            t(app, "games.none_description"),
            button(
                "choose-folder",
                "",
                t(app, "games.add_library"),
                "plus",
                "primary",
                app.busy()
            )
        );
    }
    html.push_str("</section>");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_background_is_used_only_on_home_and_original_is_the_fallback() {
        let mut app = App::default();
        let game = serde_json::json!({"id":"art","title":"Art","folder":"X:/Fixture","engine":"unknown","modified":0,
            "cover":"http://galgear-cover.localhost/cover", "cover_artwork":{"background":"background"}});
        app.catalog.games.push(serde_json::from_value(game).unwrap());
        for game in &app.catalog.games { app.settings.game_edits.entry(game.id.clone()).or_default().setup_confirmed = true; }
        let mut html = String::new();
        render(&mut html, &app);
        assert!(html.contains("data-cover=\"http://galgear-cover.localhost/background\""));
        assert!(html.contains("src=\"http://galgear-cover.localhost/cover\""));
        app.catalog.games[0].cover_artwork = None;
        html.clear(); render(&mut html, &app);
        assert!(html.contains("data-cover=\"http://galgear-cover.localhost/cover\""));
    }
    #[test]
    fn home_uses_library_order_and_ignores_library_filters() {
        let mut app = App::default();
        app.library.search = "no match".into();
        app.catalog.games = [1,3,2].into_iter().map(|n| serde_json::from_value(serde_json::json!({
            "id":n.to_string(),"title":n.to_string(),"folder":"X:/Fixture","engine":"unknown","modified":0,"last_played":n
        })).unwrap()).collect();
        for game in &app.catalog.games { app.settings.game_edits.entry(game.id.clone()).or_default().setup_confirmed = true; }
        assert_eq!(
            ordered(&app)
                .iter()
                .map(|game| game.id.as_str())
                .collect::<Vec<_>>(),
            vec!["1", "2", "3"]
        );
    }
    #[test]
    fn removed_home_selection_falls_back_to_an_existing_game() {
        let mut app = App {
            home_selected: Some("removed".into()),
            ..Default::default()
        };
        app.catalog.games.push(serde_json::from_value(serde_json::json!({"id":"live","title":"Live","folder":"X:/Fixture","engine":"unknown","modified":0})).unwrap());
        for game in &app.catalog.games { app.settings.game_edits.entry(game.id.clone()).or_default().setup_confirmed = true; }
        let mut html = String::new();
        render(&mut html, &app);
        assert!(html.contains("data-value=\"live\"") && !html.contains("removed"));
    }

    #[test]
    fn home_renders_every_visible_game() {
        let mut app = App::default();
        for index in 1..=25 {
            let id = format!("game-{index:02}");
            let game = serde_json::from_value(serde_json::json!({
                "id": id,
                "title": id,
                "folder": "X:/Fixture",
                "engine": "unknown",
                "modified": 0
            }))
            .unwrap();
            app.catalog.games.push(game);
            app.settings
                .game_edits
                .entry(id)
                .or_default()
                .setup_confirmed = true;
        }

        let mut html = String::new();
        render(&mut html, &app);

        assert_eq!(html.matches(r#"data-action="home-select""#).count(), 25);
    }
}
