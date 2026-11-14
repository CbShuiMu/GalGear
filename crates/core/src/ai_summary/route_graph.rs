//! 模型只描述有证据的节点和连线；布局、转义和绘制由本地确定性完成。
use galgear_i18n::{Language, Message, tr};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub(super) const RULES: &str = r#"Return ONLY compact JSON, never SVG or layout coordinates:
{"nodes":[{"id":"n1","label":"short scene/choice/ending label","kind":"scene","evidence":"source:position"}],"edges":[{"from":"n1","to":"n2","label":"choice or condition","inferred":true,"evidence":"source:position; missing condition"}],"uncertainties":["missing branch evidence"]}.
Use 1..64 nodes, at most 128 edges and 16 uncertainties, at most 24000 UTF-8 bytes overall. Node kinds: start, scene, choice, save, ending, extra, grand. IDs: 1..32 ASCII letters/digits/_/-. Labels: at most 80 characters; evidence: at most 240 characters; uncertainties: at most 240 characters. Every edge must reference existing unique node IDs. Preserve supported branches/endings; group minor scenes if necessary and disclose omitted detail. Do not invent save points. A candidate byte pattern, function name, source adjacency or dialogue inference cannot prove a connection: set inferred=true. Evidence is mandatory for nodes and edges. Do not connect disconnected evidence merely to make a tree. Missing choices/conditions must remain in uncertainties. Use requested language for node titles and uncertainties. Edge labels for choices MUST preserve the verbatim option text in the analyzed script language; never combine multiple options with / into one edge. Keep each option as one edge, even when targets are identical. Shared targets use the SAME node ID. Put filenames, byte offsets and raw flags in evidence/optional pc, not visible titles. Edges may add kind: flow|choice|unlock|reference, pc:string, condition:string. Unlock edges MUST have kind=unlock and a human-readable condition stating prerequisites, and originate at the prerequisite ending. EXTRA/after-story nodes use kind=extra and attach below their prerequisite ending. GRAND/true endings use kind=grand; inspect title/menu dispatch and persistent flags to identify cross-playthrough entry, never infer unlocks from filenames. Back jumps keep the same target ID; renderer creates reference nodes, never duplicate a subtree. Group ordinary linear scenes between choices, not the choices themselves. Preserve each known choice and ending. Check every ending's reachability from start nodes; investigate missing entry once, and explicitly disclose unresolved entries instead of inventing edges. Use official ending names when known."#;

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct Graph {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    uncertainties: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Node {
    id: String,
    #[serde(alias = "title")]
    label: String,
    #[serde(alias = "type")]
    kind: Kind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pc: String,
    evidence: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Start,
    Scene,
    Choice,
    Save,
    Ending,
    Extra,
    Grand,
}

#[derive(Debug, Deserialize, Serialize)]
struct Edge {
    from: String,
    to: String,
    label: String,
    inferred: bool,
    evidence: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pc: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    condition: String,
}

fn text_ok(text: &str, limit: usize, empty: bool) -> bool {
    (empty || !text.trim().is_empty())
        && text.chars().count() <= limit
        && !text.chars().any(char::is_control)
}

impl Graph {
    pub(super) fn validate(&self) -> Result<(), Message> {
        let mut ids = BTreeSet::new();
        let valid = !self.nodes.is_empty()
            && self.nodes.len() <= 64
            && self.edges.len() <= 128
            && self.uncertainties.len() <= 16
            && serde_json::to_vec(self).map_err(super::io)?.len() <= 24_000
            && self.nodes.iter().all(|node| {
                !node.id.is_empty()
                    && node.id.len() <= 32
                    && node
                        .id
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
                    && ids.insert(node.id.as_str())
                    && text_ok(&node.label, 80, false)
                    && text_ok(&node.evidence, 240, false)
                    && text_ok(&node.pc, 240, true)
            })
            && self.edges.iter().all(|edge| {
                ids.contains(edge.from.as_str())
                    && ids.contains(edge.to.as_str())
                    && text_ok(&edge.label, 80, true)
                    && text_ok(&edge.evidence, 240, false)
                    && matches!(
                        edge.kind.as_str(),
                        "" | "flow" | "choice" | "unlock" | "reference"
                    )
                    && text_ok(&edge.pc, 240, true)
                    && text_ok(&edge.condition, 160, true)
                    && (edge.kind != "unlock" || !edge.condition.trim().is_empty())
            })
            && self
                .uncertainties
                .iter()
                .all(|text| text_ok(text, 240, false));
        if !valid {
            return Err(Message::detail(
                "route.invalid_analysis",
                "Graph needs unique IDs and existing edge endpoints, evidence, bounded text, 1..64 nodes, <=128 edges, <=16 uncertainties and <=24000 UTF-8 bytes. Correct only invalid fields; preserve evidence.",
            ));
        }
        Ok(())
    }

    /// 按有向可达层级排列；循环和回边单独绕行，不删除分支或补造连接。
    pub(super) fn render(&self, title: &str, language: Language) -> Result<String, Message> {
        self.validate()?;
        let indices: BTreeMap<_, _> = self
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.as_str(), i))
            .collect();
        let mut ranks = vec![None; self.nodes.len()];
        let mut next_rank = 0;
        let mut roots: Vec<_> = (0..self.nodes.len())
            .filter(|&i| !self.edges.iter().any(|e| e.to == self.nodes[i].id))
            .collect();
        let root_count = roots.len();
        roots.extend(0..self.nodes.len());
        for (root_index, root) in roots.into_iter().enumerate() {
            if ranks[root].is_some() {
                continue;
            }
            ranks[root] = Some(if root_index < root_count {
                0
            } else {
                next_rank
            });
            let mut queue = VecDeque::from([root]);
            while let Some(from) = queue.pop_front() {
                let rank = ranks[from].unwrap_or(0);
                next_rank = next_rank.max(rank + 1);
                for edge in self
                    .edges
                    .iter()
                    .filter(|edge| edge.from == self.nodes[from].id)
                {
                    let to = indices[edge.to.as_str()];
                    if ranks[to].is_none() {
                        ranks[to] = Some(rank + 1);
                        queue.push_back(to);
                    }
                }
            }
        }
        let mut levels = vec![Vec::new(); next_rank];
        for (i, rank) in ranks.iter().enumerate() {
            levels[rank.unwrap_or(0)].push(i);
        }
        let columns = levels.iter().map(Vec::len).max().unwrap_or(1);
        let width = (columns * 340 + 180).max(880);
        // 同一目标的多个选项标签纵向排列；层间距按真实文字高度计算，
        // 避免并行选项与合流边的标签挤在同一个中点。图的节点和边不合并。
        let edge_lines: Vec<_> = self.edges.iter().map(|e| wrap(&e.label, 20)).collect();
        let mut label_heights = vec![0; self.nodes.len()];
        for (edge, lines) in self.edges.iter().zip(&edge_lines) {
            if !lines.is_empty() {
                label_heights[indices[edge.to.as_str()]] += lines.len() * 18 + 12;
            }
        }
        let mut positions = vec![(0, 0); self.nodes.len()];
        let mut y = 100;
        for level in &levels {
            let label_height = level.iter().map(|&i| label_heights[i]).max().unwrap_or(0);
            y += (label_height + 36).max(136);
            let left = (width - level.len() * 340) / 2;
            for (column, &i) in level.iter().enumerate() {
                positions[i] = (left + column * 340 + 20, y);
            }
            y += 104;
        }
        let footer_y = y + 80;
        let footer: Vec<_> = self
            .uncertainties
            .iter()
            .flat_map(|s| wrap(s, 54))
            .collect();
        let height = footer_y + 70 + footer.len() * 24;
        let mut svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" role="img"><title>{}</title><defs><marker id="arrow" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto"><path d="M0 0 L8 4 L0 8" fill="#6298ef"/></marker></defs><rect width="{width}" height="{height}" fill="#f7f7ff"/><g font-family="sans-serif" fill="#263249"><text x="40" y="44" font-size="22">{}</text><text x="40" y="78" font-size="14">{}</text>"##,
            xml(title),
            xml(title),
            xml(tr(language, "route.graph_legend"))
        );
        for (number, edge) in self.edges.iter().enumerate() {
            let from = indices[edge.from.as_str()];
            let to = indices[edge.to.as_str()];
            let (fx, fy) = positions[from];
            let (tx, ty) = positions[to];
            let (x1, y1, x2, y2) = (fx + 150, fy + 104, tx + 150, ty);
            let path = if ty > fy {
                format!("M{x1} {y1} C{x1} {} {x2} {} {x2} {y2}", y1 + 60, y2 - 60)
            } else {
                let lane = width - 16 - (number % 8) * 8;
                format!(
                    "M{} {} H{lane} V{} H{}",
                    fx + 300,
                    fy + 52,
                    ty + 52,
                    tx + 300
                )
            };
            let dash = if edge.inferred { "7 5" } else { "none" };
            svg.push_str(&format!(r##"<path d="{path}" fill="none" stroke="#6298ef" stroke-width="2" stroke-dasharray="{dash}" marker-end="url(#arrow)"><title>{}</title></path>"##, xml(&edge.evidence)));
        }
        // 标签在各目标节点上方占用独立矩形区域。先画全部连线，再画标签，
        // 以背景隔开穿过文字的曲线；每条边仍保留自己的证据提示。
        let mut label_offsets = vec![0; self.nodes.len()];
        for (number, (edge, lines)) in self.edges.iter().zip(&edge_lines).enumerate() {
            if lines.is_empty() {
                continue;
            }
            let target = indices[edge.to.as_str()];
            let (x, y) = positions[target];
            let top = y - 24 - label_heights[target] + label_offsets[target];
            let height = lines.len() * 18 + 6;
            svg.push_str(&format!(r##"<g id="edge-label-{number}"><title>{}</title><rect x="{}" y="{top}" width="288" height="{height}" rx="6" fill="#f7f7ff" fill-opacity="0.97"/>"##, xml(&edge.evidence), x + 6));
            for (line, text) in lines.iter().enumerate() {
                svg.push_str(&format!(r##"<text x="{}" y="{}" text-anchor="middle" font-size="14" fill="#3565a6">{}</text>"##, x + 150, top + 16 + line * 18, xml(text)));
            }
            svg.push_str("</g>");
            label_offsets[target] += lines.len() * 18 + 12;
        }
        for (i, node) in self.nodes.iter().enumerate() {
            let (x, y) = positions[i];
            let (fill, stroke) = match node.kind {
                Kind::Start => ("#bdffd0", "#64cc83"),
                Kind::Ending => ("#e0edff", "#a7cbff"),
                Kind::Choice => ("#fff3ce", "#dfc574"),
                _ => ("#fdfdff", "#cdd2df"),
            };
            svg.push_str(&format!(r##"<g><title>{}</title><rect x="{x}" y="{y}" width="300" height="104" rx="24" fill="{fill}" stroke="{stroke}"/>"##, xml(&node.evidence)));
            let lines = wrap(&node.label, 16);
            for (line, text) in lines.iter().enumerate() {
                svg.push_str(&format!(
                    r#"<text x="{}" y="{}" text-anchor="middle" font-size="16">{}</text>"#,
                    x + 150,
                    y + 58 - (lines.len().saturating_sub(1) * 10) + line * 20,
                    xml(text)
                ));
            }
            svg.push_str("</g>");
        }
        for (line, text) in footer.iter().enumerate() {
            svg.push_str(&format!(
                r##"<text x="40" y="{}" font-size="14" fill="#616a7a">{}</text>"##,
                footer_y + line * 24,
                xml(text)
            ));
        }
        svg.push_str("</g></svg>");
        Ok(svg)
    }
}

fn xml(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn wrap(value: &str, width: usize) -> Vec<String> {
    value
        .chars()
        .collect::<Vec<_>>()
        .chunks(width)
        .map(|part| part.iter().collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn fixture() -> Graph {
        serde_json::from_value(serde_json::json!({"nodes":[{"id":"a","label":"Start <&>","kind":"start","evidence":"s:0"},{"id":"b","label":"End","kind":"ending","evidence":"s:42"}],"edges":[{"from":"a","to":"b","label":"Maybe","inferred":true,"evidence":"s:42 candidate"}],"uncertainties":["Unknown condition"]})).unwrap()
    }

    #[test]
    fn validates_references_limits_and_evidence() {
        let mut graph = fixture();
        graph.nodes[1].id = "a".into();
        assert!(graph.validate().is_err());
        let mut graph = fixture();
        graph.edges[0].to = "missing".into();
        assert!(graph.validate().is_err());
        let mut graph = fixture();
        graph.edges[0].evidence.clear();
        assert!(graph.validate().is_err());
    }

    #[test]
    fn parallel_choice_labels_have_separate_boxes() {
        let mut graph = fixture();
        for label in ["选择甲", "选择乙", "很长的条件说明".repeat(10).as_str()] {
            graph.edges.push(Edge {
                from: "a".into(),
                to: "b".into(),
                label: label.into(),
                inferred: false,
                evidence: "menu:1".into(),
                kind: String::new(),
                pc: String::new(),
                condition: String::new(),
            });
        }
        let svg = graph.render("choices", Language::Chinese).unwrap();
        let document = roxmltree::Document::parse(&svg).unwrap();
        let boxes: Vec<_> = document
            .descendants()
            .filter(|node| {
                node.attribute("id")
                    .is_some_and(|id| id.starts_with("edge-label-"))
            })
            .map(|node| {
                let rect = node.children().find(|n| n.has_tag_name("rect")).unwrap();
                (
                    rect.attribute("y").unwrap().parse::<usize>().unwrap(),
                    rect.attribute("height").unwrap().parse::<usize>().unwrap(),
                )
            })
            .collect();
        assert_eq!(boxes.len(), 4);
        for pair in boxes.windows(2) {
            assert!(pair[0].0 + pair[0].1 < pair[1].0);
        }
        assert_eq!(svg.matches("marker-end=").count(), 4);
    }

    #[test]
    fn cycles_disconnected_nodes_and_xml_are_rendered_without_invented_links() {
        let mut graph = fixture();
        graph.edges.push(Edge {
            from: "b".into(),
            to: "a".into(),
            label: "loop".into(),
            inferred: true,
            evidence: "s:44".into(),
            kind: String::new(),
            pc: String::new(),
            condition: String::new(),
        });
        graph.nodes.push(Node {
            id: "c".into(),
            label: "isolated".into(),
            kind: Kind::Scene,
            pc: String::new(),
            evidence: "s:99".into(),
        });
        let svg = graph
            .render("<script>alert('x')</script>", Language::Chinese)
            .unwrap();
        super::super::route_map::validate_svg(&svg).unwrap();
        assert!(svg.contains("Start &lt;&amp;&gt;"));
        assert!(svg.contains("isolated"));
        assert_eq!(svg.matches("marker-end=").count(), 2);
        assert!(svg.contains("stroke-dasharray=\"7 5\""));
    }
}
