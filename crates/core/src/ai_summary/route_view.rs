//! 供内置窗口使用的路线数据。窗口只运行打包资源，旧版 SVG 单独校验。
use super::{read, route_html, route_map, skill_analysis};
use crate::{Game, Settings};
use galgear_i18n::Message;
use serde::Serialize;
use serde_json::Value;
use std::path::Path;

/// 已校验的交互图数据，或旧解析器生成的静态 SVG。
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Document {
    Interactive(Value),
    Svg(String),
}

/// 从当前游戏和语言的缓存读取已校验的图。旧版 SVG 沿用原生成与缓存策略。
pub fn load(game: &Game, settings: &Settings, config: &Path) -> Result<Document, Message> {
    if !settings
        .game_edits
        .get(&game.id)
        .is_some_and(|e| e.ai_summary)
    {
        return Err(Message::new("summary.disabled"));
    }
    if skill_analysis::exists(game, settings) {
        let graph = skill_analysis::graph(game, settings)?;
        return route_html::data(&graph, &game.title, settings.language).map(Document::Interactive);
    }
    let path = route_map::route_map(game, settings, config)?;
    let svg = String::from_utf8(read(&path, 2 * 1024 * 1024)?).map_err(super::io)?;
    route_map::validate_svg(&svg)?;
    Ok(Document::Svg(svg))
}
