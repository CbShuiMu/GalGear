//! Recent-game shelf. Selection is independent of library filters and detail state.
use crate::{
    game_actions,
    models::Game,
    playtime,
    state::App,
    view::{poster, t},
    widgets::{button, esc},
};
use std::fmt::Write;

fn ordered(app: &App) -> Vec<&Game> {
    let mut games: Vec<_> = app.catalog.games.iter().collect();
    games.sort_by(|a, b| {
        b.last_played
            .cmp(&a.last_played)
            .then_with(|| a.title.cmp(&b.title))
    });
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
        // Keep the shelf's siblings stable so changing artwork never remounts its animations.
        html.push_str(r#"<div class="home-backdrop" data-render-key="home-backdrop" aria-hidden="true">"#);
        if let Some(cover) = &game.cover {
            let background = game.cover_artwork.as_ref()
                .map(|art| format!("http://galgear-cover.localhost/{}", art.background));
            let _ = write!(
                html,
                r#"<img class="home-backdrop-bleed" src="{0}" alt="" decoding="async"/><img class="home-backdrop-soft" src="{0}" alt="" decoding="async"/><img class="home-backdrop-cover" src="{0}" alt="" decoding="async"/>"#,
                esc(background.as_deref().unwrap_or(cover)),
            );
        }
        html.push_str("</div>");
        let _ = write!(
            html,
            r#"<div class="home-shelf" data-render-key="home-shelf" data-focus-scroll="managed" aria-label="{}">"#,
            t(app, "home.recent")
        );
        for item in games.iter().take(24) {
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
                || game.executable.is_none()
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
        let _ = write!(
            html,
            r#"<span class="home-playtime">{}h</span></div>"#,
            playtime::hours(game.playtime_seconds)
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
        let mut html = String::new();
        render(&mut html, &app);
        assert!(html.contains("class=\"home-backdrop-cover\" src=\"http://galgear-cover.localhost/background\""));
        assert!(html.contains("src=\"http://galgear-cover.localhost/cover\""));
        app.catalog.games[0].cover_artwork = None;
        html.clear(); render(&mut html, &app);
        assert!(html.contains("class=\"home-backdrop-cover\" src=\"http://galgear-cover.localhost/cover\""));
    }
    #[test]
    fn home_uses_recent_play_and_ignores_library_filters() {
        let mut app = App::default();
        app.library.search = "no match".into();
        app.catalog.games = [1,3,2].into_iter().map(|n| serde_json::from_value(serde_json::json!({
            "id":n.to_string(),"title":n.to_string(),"folder":"X:/Fixture","engine":"unknown","modified":0,"last_played":n
        })).unwrap()).collect();
        assert_eq!(
            ordered(&app)
                .iter()
                .map(|game| game.id.as_str())
                .collect::<Vec<_>>(),
            vec!["3", "2", "1"]
        );
    }
    #[test]
    fn removed_home_selection_falls_back_to_an_existing_game() {
        let mut app = App {
            home_selected: Some("removed".into()),
            ..Default::default()
        };
        app.catalog.games.push(serde_json::from_value(serde_json::json!({"id":"live","title":"Live","folder":"X:/Fixture","engine":"unknown","modified":0})).unwrap());
        let mut html = String::new();
        render(&mut html, &app);
        assert!(html.contains("data-value=\"live\"") && !html.contains("removed"));
    }
}
