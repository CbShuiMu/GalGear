//! 将已校验的路线数据和固定视图打包为离线单文件，不允许模型提供可执行 HTML。
use super::{io, route_graph::Graph};
use galgear_i18n::{Language, Message, tr};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub(super) fn data(graph: &Graph, title: &str, language: Language) -> Result<Value, Message> {
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
    Ok(data)
}

pub(super) fn render(graph: &Graph, title: &str, language: Language) -> Result<String, Message> {
    let data = data(graph, title, language)?;
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
    let template = include_str!("../../../../web/route-map.html");
    let (_, view) = template
        .split_once("<!-- route view -->")
        .ok_or_else(super::invalid)?;
    let (view, _) = view
        .split_once("<!-- route scripts -->")
        .ok_or_else(super::invalid)?;
    Ok(format!(
        r#"<!doctype html><html lang="{lang}"><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; connect-src 'none'; base-uri 'none'; form-action 'none'">
<title>{escaped_title}</title><style>{style}</style>{view}<script id="route-data" type="application/json">{encoded}</script><script>{script}</script></html>"#,
        lang = language.code(),
        style = include_str!("../../../../web/route-map.css"),
        script = include_str!("../../../../web/route-map.js")
    ))
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
