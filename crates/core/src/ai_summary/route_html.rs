//! 将已校验的路线数据和固定视图打包为离线单文件，不允许模型提供可执行 HTML。
use super::{io, route_graph::Graph};
use galgear_i18n::{Language, Message, tr};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub(super) fn render(graph: &Graph, title: &str, language: Language) -> Result<String, Message> {
    graph.validate()?;
    let mut data = serde_json::to_value(graph).map_err(io)?;
    let mut reached = BTreeSet::new();
    for node in data["nodes"].as_array().into_iter().flatten() {
        if node["kind"] == "start" {
            reached.insert(node["id"].as_str().unwrap_or_default().to_owned());
        }
    }
    loop {
        let count = reached.len();
        for edge in data["edges"].as_array().into_iter().flatten() {
            if reached.contains(edge["from"].as_str().unwrap_or_default()) {
                reached.insert(edge["to"].as_str().unwrap_or_default().to_owned());
            }
        }
        if count == reached.len() {
            break;
        }
    }
    let mut validation = Vec::new();
    for node in data["nodes"].as_array_mut().into_iter().flatten() {
        if !reached.contains(node["id"].as_str().unwrap_or_default()) {
            validation.push(format!(
                "{}: {}",
                tr(language, "route.html.unreachable"),
                node["label"].as_str().unwrap_or_default()
            ));
        }
        node["title"] = node["label"].take();
        node["type"] = node["kind"].take();
        if let Some(object) = node.as_object_mut() {
            object.remove("label");
            object.remove("kind");
        }
    }
    let mut ui = serde_json::Map::new();
    for key in [
        "hint",
        "canvas",
        "legend",
        "review",
        "expand",
        "collapse",
        "in",
        "out",
        "fit",
        "theme",
        "json",
        "draft",
        "draftNote",
        "next",
        "inferred",
    ] {
        ui.insert(
            key.into(),
            Value::String(tr(language, &format!("route.html.{key}")).into()),
        );
    }
    data["ui"] = Value::Object(ui);
    data["title"] = json!(title);
    data["language"] = json!(language.code());
    data["validation"] = json!(validation);
    // JSON script contents must not be able to terminate the script element, even inside strings.
    let encoded = serde_json::to_string(&data)
        .map_err(io)?
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");
    let escaped_title = title
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    // 仅扫描固定模板一次，输入中的占位符文本永远不参与下一轮替换。
    let mut html = String::new();
    for (index, part) in include_str!("../../../../web/route-map.html")
        .split("__")
        .enumerate()
    {
        if index % 2 == 0 {
            html.push_str(part);
            continue;
        }
        html.push_str(match part {
            "STYLE" => include_str!("../../../../web/route-map.css"),
            "SCRIPT" => include_str!("../../../../web/route-map.js"),
            "LANG" => language.code(),
            "TITLE" => &escaped_title,
            "DATA" => &encoded,
            _ => return Err(super::invalid()),
        });
    }
    Ok(html)
}

#[cfg(test)]
mod tests {
    use super::render;
    use crate::ai_summary::route_graph::Graph;
    use galgear_i18n::Language;
    use serde_json::json;

    #[test]
    fn offline_document_escapes_script_termination_and_reports_unreachable_endings() {
        let graph:Graph=serde_json::from_value(json!({"nodes":[
            {"id":"s","title":"Start","type":"start","evidence":"s:1"},
            {"id":"e","title":"</script><script>alert(1)</script>","type":"grand","evidence":"s:2"}],
            "edges":[],"uncertainties":[]})).unwrap();
        let html = render(&graph, "Title", Language::Chinese).unwrap();
        assert!(!html.contains("</script><script>alert"));
        assert!(html.contains("\\u003c/script\\u003e"));
        assert!(html.contains("connect-src 'none'"));
        assert!(html.contains("未确认从开始入口可达"));
        assert!(!html.contains("__DATA__"));
    }
}
