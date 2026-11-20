//! 模型只描述有证据的节点和连线；布局、转义和绘制由本地确定性完成。
#[path = "route_compact.rs"]
mod compact;
#[path = "route_focus.rs"]
mod focus;

use galgear_i18n::{Language, Message, tr};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::LazyLock;

pub(super) const RULES: &str = r#"Return ONLY compact JSON, never SVG or layout coordinates:
{"nodes":[{"id":"n1","label":"short scene/choice/ending label","kind":"scene","evidence":"source:position"}],"edges":[{"from":"n1","to":"n2","label":"choice or condition","inferred":true,"evidence":"source:position; missing condition"}],"uncertainties":["missing branch evidence"]}.
Use 1..256 nodes, at most 512 edges and 16 uncertainties, at most 262144 UTF-8 bytes overall. Node kinds: start, scene, choice, ending, extra, grand. IDs: 1..32 ASCII letters/digits/_/-. Labels: at most 80 characters; evidence: at most 240 characters; uncertainties: at most 240 characters. Every edge must reference existing unique node IDs. Preserve supported branches/endings; group minor scenes if necessary and disclose omitted detail. Store save positions separately from story nodes. A candidate byte pattern, function name, source adjacency or dialogue inference cannot prove a connection: set inferred=true. Evidence is mandatory for nodes and edges. Do not connect disconnected evidence merely to make a tree. Missing choices/conditions must remain in uncertainties. Use requested language for node titles and uncertainties. Edge labels for choices MUST preserve the verbatim option text in the analyzed script language; never combine multiple options with / into one edge. Keep each option as one edge, even when targets are identical. Options that rejoin the same main scene after a brief reaction use the SAME target ID and retain separate arrows. Keep the title specific to the shared scene. For every proven numeric change on a choice, add effects:[{stat,delta}] to its edge. Use the observed stat name or script variable and signed numeric delta, and cite the assignment in evidence. Preserve the verbatim option label. Put filenames, byte offsets and raw flags in evidence/optional pc, not visible titles. Edges may add kind: flow|choice|unlock|reference, pc:string, condition:string. Unlock edges MUST have kind=unlock and a human-readable condition stating prerequisites, and originate at the prerequisite ending. EXTRA/after-story nodes use kind=extra and attach below their prerequisite ending. GRAND/true endings use kind=grand; inspect title/menu dispatch and persistent flags to identify cross-playthrough entry, never infer unlocks from filenames. Back jumps keep the same target ID; renderer creates reference nodes, never duplicate a subtree. Use empty labels on automatic flow and put real branch conditions in condition. A mandatory choice is the only path from its scene to its option targets. Only draw a bypass with independent control-flow evidence. Describe the decision itself in its title. Use ordinary linear scenes only as source anchors or grouped passages; the map folds them into arrow details. Preserve decision, condition, distinct merge, ending and unlock boundaries. Preserve each known choice and ending. Check every ending's reachability from start nodes; investigate missing entry once, and explicitly disclose unresolved entries instead of inventing edges. Use official ending names when known."#;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Graph {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    uncertainties: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    guides: Vec<focus::Guide>,
    #[serde(default, skip_serializing)]
    characters: Vec<Character>,
}

#[derive(Clone, Debug, Deserialize)]
struct Character {
    id: String,
    name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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

#[derive(Clone, Debug, Deserialize, Serialize)]
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

#[derive(Clone, Debug, Deserialize, Serialize)]
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    effects: Vec<ChoiceEffect>,
    #[serde(skip)]
    scenes: Vec<FoldedScene>,
}

#[derive(Clone, Debug, Serialize)]
struct FoldedScene {
    title: String,
    pc: String,
    evidence: String,
    transition_pc: String,
    transition_evidence: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
struct ChoiceEffect {
    stat: String,
    delta: serde_json::Number,
}

impl Edge {
    /// 旧图把简单赋值保存在选项证据末尾。只还原同一变量的常量增减。
    fn legacy_effect(&self, characters: &[Character]) -> Option<ChoiceEffect> {
        if self.kind != "choice" || self.inferred || !self.effects.is_empty() {
            return None;
        }
        static ASSIGNMENT: LazyLock<regex::Regex> = LazyLock::new(|| {
            regex::Regex::new(r"(?:^|\s)exp=([\w.]+)\s*=\s*([\w.]+)\s*([+-])\s*(\d+(?:\.\d+)?)\s*$")
                .expect("constant choice assignment pattern")
        });
        let capture = ASSIGNMENT.captures(&self.evidence)?;
        let variable = capture.get(1)?.as_str();
        if variable != &capture[2] {
            return None;
        }
        let name = variable.strip_prefix("f.").unwrap_or(variable);
        let stat = characters
            .iter()
            .find(|ch| ch.id == name && text_ok(&ch.name, 48, false))
            .map_or(variable, |ch| ch.name.as_str());
        let sign = if &capture[3] == "-" { "-" } else { "" };
        Some(ChoiceEffect {
            stat: stat.to_owned(),
            delta: format!("{sign}{}", &capture[4]).parse().ok()?,
        })
    }
}

fn text_ok(text: &str, limit: usize, empty: bool) -> bool {
    (empty || !text.trim().is_empty())
        && text.chars().count() <= limit
        && !text.chars().any(char::is_control)
}

impl Graph {
    /// 普通流程仅画箭头。兼容旧图将判定分支的条件只存在 label 中的情况。
    fn edge_caption<'a>(&self, edge: &'a Edge) -> (&'a str, &'a str) {
        let is_decision = self
            .nodes
            .iter()
            .any(|n| n.id == edge.from && matches!(n.kind, Kind::Choice));
        let filler = matches!(
            edge.label.trim(),
            "剧情推进" | "劇情推進" | "ストーリー進行" | "物語の進行" | "Story progression"
        );
        if edge.kind == "flow" {
            if edge.condition.is_empty()
                && !filler
                && (is_decision
                    || self.edges.iter().any(|other| {
                        other.from == edge.from
                            && other.kind == "flow"
                            && !other.condition.is_empty()
                    }))
            {
                return ("", &edge.label);
            }
            return ("", &edge.condition);
        }
        if edge.kind.is_empty() && !is_decision {
            // 早期图允许省略 kind，只有单出口或明确的占位文案可当作普通推进。
            let single_exit = self
                .edges
                .iter()
                .filter(|e| e.from == edge.from)
                .take(2)
                .count()
                == 1;
            if filler || single_exit {
                return ("", &edge.condition);
            }
        }
        (&edge.label, &edge.condition)
    }

    /// 旧分析曾将存档附在章节旁或插在流程中。投影时收起这些节点，保留选项和条件。
    fn story(&self) -> Result<std::borrow::Cow<'_, Self>, Message> {
        if !self.nodes.iter().any(|n| matches!(n.kind, Kind::Save)) {
            return Ok(std::borrow::Cow::Borrowed(self));
        }
        let mut graph = self.clone();
        graph.nodes.retain(|n| !matches!(n.kind, Kind::Save));
        for save in self.nodes.iter().filter(|n| matches!(n.kind, Kind::Save)) {
            let edges = std::mem::take(&mut graph.edges);
            let exits: Vec<_> = edges
                .iter()
                .filter(|e| e.from == save.id && e.to != save.id)
                .collect();
            for before in edges.iter().filter(|e| e.from != save.id) {
                if before.to != save.id {
                    graph.edges.push(before.clone());
                    continue;
                }
                for &after in &exits {
                    let mut edge =
                        if matches!(after.kind.as_str(), "choice" | "unlock" | "reference") {
                            after.clone()
                        } else {
                            before.clone()
                        };
                    edge.from.clone_from(&before.from);
                    edge.to.clone_from(&after.to);
                    edge.condition = [&*before.condition, &*after.condition]
                        .into_iter()
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(" ∧ ");
                    edge.inferred |= before.inferred || after.inferred;
                    let duplicate_flow = matches!(edge.kind.as_str(), "" | "flow")
                        && edges.iter().any(|e| {
                            e.from == edge.from
                                && e.to == edge.to
                                && e.kind == edge.kind
                                && e.label == edge.label
                                && e.condition == edge.condition
                                && e.effects == edge.effects
                        });
                    if !duplicate_flow {
                        graph.edges.push(edge);
                        if graph.edges.len() > 512 {
                            return Err(super::invalid());
                        }
                    }
                }
            }
        }
        if graph.nodes.is_empty() || graph.edges.len() > 512 {
            return Err(super::invalid());
        }
        Ok(std::borrow::Cow::Owned(graph))
    }

    /// 优先采用与已分析存档绑定的剧情节点；旧格式只接受唯一可达的剧情邻居。
    pub(super) fn cached_story_node(&self, bound: Option<&str>) -> Result<&str, Message> {
        let reachable = self.reachable();
        let story = |id: &str| {
            self.nodes
                .iter()
                .find(|n| n.id == id && !matches!(n.kind, Kind::Save) && reachable.contains(id))
        };
        if let Some(id) = bound {
            return story(id)
                .map(|n| n.id.as_str())
                .ok_or_else(|| Message::new("route.locate_unmatched"));
        }
        let mut candidates = BTreeSet::new();
        for (save, pc, _) in self.location_nodes(true) {
            let neighbors: Vec<_> = self
                .edges
                .iter()
                .filter_map(|edge| {
                    if edge.from == save {
                        story(&edge.to)
                    } else if edge.to == save {
                        story(&edge.from)
                    } else {
                        None
                    }
                })
                .collect();
            let exact: Vec<_> = neighbors
                .iter()
                .filter(|n| !pc.is_empty() && n.pc == pc)
                .collect();
            if exact.is_empty() {
                candidates.extend(neighbors.iter().map(|n| n.id.as_str()));
            } else {
                candidates.extend(exact.iter().map(|n| n.id.as_str()));
            }
        }
        let mut candidates = candidates.into_iter();
        let node = candidates
            .next()
            .ok_or_else(|| Message::new("route.locate_unmatched"))?;
        if candidates.next().is_some() {
            return Err(Message::new("route.locate_ambiguous"));
        }
        Ok(node)
    }

    pub(super) fn display_data(&self) -> Result<serde_json::Value, Message> {
        self.compact()?.graph.compact_data()
    }

    fn compact(&self) -> Result<compact::Projection, Message> {
        Ok(compact::project(self.story()?.into_owned()))
    }

    /// 折叠的存档位置落在其前方可见节点，实际坐标仍由原始图验证。
    pub(super) fn visible_node(&self, id: &str) -> Result<Option<String>, Message> {
        Ok(self.compact()?.anchors.remove(id))
    }

    fn compact_data(&self) -> Result<serde_json::Value, Message> {
        let mut data = serde_json::to_value(self).map_err(super::io)?;
        for (edge, output) in self
            .edges
            .iter()
            .zip(data["edges"].as_array_mut().into_iter().flatten())
        {
            let (label, condition) = self.edge_caption(edge);
            output["label"] = label.into();
            if !condition.is_empty() {
                output["condition"] = condition.into();
            }
            if let Some(effect) = edge.legacy_effect(&self.characters) {
                output["effects"] = serde_json::json!([effect]);
            }
            if !edge.scenes.is_empty() {
                output["scenes"] = serde_json::to_value(&edge.scenes).map_err(super::io)?;
            }
        }
        data["focus_routes"] = focus::routes(self)?;
        Ok(data)
    }
    pub(super) fn reachable(&self) -> BTreeSet<&str> {
        let mut reached: BTreeSet<_> = self
            .nodes
            .iter()
            .filter(|node| matches!(node.kind, Kind::Start))
            .map(|node| node.id.as_str())
            .collect();
        loop {
            let before = reached.len();
            for edge in &self.edges {
                if reached.contains(edge.from.as_str()) {
                    reached.insert(edge.to.as_str());
                }
            }
            if before == reached.len() {
                return reached;
            }
        }
    }

    pub(super) fn location_nodes(&self, save: bool) -> impl Iterator<Item = (&str, &str, &str)> {
        let reached = self.reachable();
        self.nodes
            .iter()
            .filter(move |node| {
                matches!(node.kind, Kind::Save) == save && reached.contains(node.id.as_str())
            })
            .map(|node| (node.id.as_str(), node.pc.as_str(), node.evidence.as_str()))
    }

    pub(super) fn revision(&self) -> Result<String, Message> {
        use sha2::{Digest, Sha256};
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(self).map_err(super::io)?)
        ))
    }

    pub(super) fn validate(&self) -> Result<(), Message> {
        let mut ids = BTreeSet::new();
        let valid = !self.nodes.is_empty()
            && self.nodes.len() <= 256
            && self.edges.len() <= 512
            && self.uncertainties.len() <= 16
            && serde_json::to_vec(self).map_err(super::io)?.len() <= 262_144
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
                    && (edge.effects.is_empty()
                        || (edge.kind == "choice"
                            && edge
                                .effects
                                .iter()
                                .all(|effect| text_ok(&effect.stat, 48, false))))
                    && (edge.kind != "unlock" || !edge.condition.trim().is_empty())
            })
            && self
                .uncertainties
                .iter()
                .all(|text| text_ok(text, 240, false));
        if !valid {
            return Err(Message::detail(
                "route.invalid_analysis",
                "Graph needs unique IDs and existing edge endpoints, evidence, bounded text, 1..256 nodes, <=512 edges, <=16 uncertainties and <=262144 UTF-8 bytes. Correct only invalid fields; preserve evidence.",
            ));
        }
        Ok(())
    }

    /// 按有向可达层级排列；循环和回边单独绕行，不删除分支或补造连接。
    pub(super) fn render(&self, title: &str, language: Language) -> Result<String, Message> {
        self.validate()?;
        self.compact()?.graph.render_story(title, language)
    }

    fn render_story(&self, title: &str, language: Language) -> Result<String, Message> {
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
        // 避免并行选项与合流边的标签挤在同一个中点。
        let edge_lines: Vec<_> = self
            .edges
            .iter()
            .map(|edge| {
                let (caption, condition) = self.edge_caption(edge);
                let mut label = [caption, condition]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(" · ");
                for effect in edge
                    .effects
                    .iter()
                    .chain(edge.legacy_effect(&self.characters).as_ref())
                {
                    let delta = effect.delta.to_string();
                    let sign = if delta.starts_with('-') || effect.delta.as_f64() == Some(0.0) {
                        ""
                    } else {
                        "+"
                    };
                    label.push_str(&format!(" · {} {sign}{delta}", effect.stat));
                }
                wrap(&label, 20)
            })
            .collect();
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
        let mut incoming = vec![0; self.nodes.len()];
        let mut slots = vec![0; self.nodes.len()];
        for edge in &self.edges {
            incoming[indices[edge.to.as_str()]] += 1;
        }
        for (number, edge) in self.edges.iter().enumerate() {
            let from = indices[edge.from.as_str()];
            let to = indices[edge.to.as_str()];
            let (fx, fy) = positions[from];
            let (tx, ty) = positions[to];
            slots[to] += 1;
            let port = f64::from(slots[to]) / f64::from(incoming[to] + 1);
            let (x1, y1, x2, y2) = (fx + 150, fy + 104, tx as f64 + 30.0 + port * 240.0, ty);
            let path = if ranks[to] == ranks[from].map(|rank| rank + 1) {
                format!("M{x1} {y1} V{} H{x2} V{y2}", y1 + 18)
            } else {
                // 跨层或回跳沿右侧空白绕行，避开中间层的节点。
                let lane = width - 16 - (number % 8) * 8;
                format!("M{x1} {y1} V{} H{lane} V{} H{x2} V{y2}", y1 + 18, y2 - 12,)
            };
            let dash = if edge.inferred { "7 5" } else { "none" };
            let evidence = std::iter::once(edge.evidence.as_str())
                .chain(edge.scenes.iter().flat_map(|scene| {
                    [
                        scene.title.as_str(),
                        scene.pc.as_str(),
                        scene.evidence.as_str(),
                        scene.transition_pc.as_str(),
                        scene.transition_evidence.as_str(),
                    ]
                }))
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            svg.push_str(&format!(r##"<path d="{path}" fill="none" stroke="#6298ef" stroke-width="2" stroke-dasharray="{dash}" marker-end="url(#arrow)"><title>{}</title></path>"##, xml(&evidence)));
        }
        // 标签在各目标节点上方占用独立矩形区域。先画全部连线，再画标签，
        // 以背景隔开穿过文字的线段；每条边仍保留自己的证据提示。
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
    fn legacy_save_is_transparent_to_choices_and_attaches_location_to_story() {
        let graph: Graph = serde_json::from_value(serde_json::json!({
            "nodes": ([("s","start"),("m","choice"),("slot","save"),("a","scene"),("b","ending")]
                .map(|(id,kind)| serde_json::json!({"id":id,"label":id,"kind":kind,"evidence":"script"}))),
            "edges": ([("s","m","flow",""),("m","slot","choice","选甲"),("slot","a","flow",""),("m","a","choice","选乙"),("a","b","flow","")]
                .map(|(from,to,kind,label)| serde_json::json!({"from":from,"to":to,"kind":kind,"label":label,"evidence":"script","inferred":false}))),
            "uncertainties":[]
        })).unwrap();
        let display = graph.display_data().unwrap();
        assert_eq!(display["nodes"].as_array().unwrap().len(), 3);
        let labels: Vec<_> = display["edges"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["to"] == "b" && e["kind"] == "choice")
            .map(|e| e["label"].as_str().unwrap())
            .collect();
        assert_eq!(labels, ["选甲", "选乙"]);
        assert_eq!(
            graph.cached_story_node(None).unwrap_err().key,
            "route.locate_ambiguous"
        );
        assert_eq!(graph.cached_story_node(Some("a")).unwrap(), "a");
        assert!(graph.cached_story_node(Some("slot")).is_err());
    }

    #[test]
    fn route_guide_selects_a_specific_parallel_choice_and_excludes_sibling_routes() {
        let mut value = serde_json::json!({
            "nodes": ([("s","start"),("m","choice"),("gate","choice"),("bridge","scene"),("a","ending"),("b","ending")]
                .map(|(id,kind)| serde_json::json!({"id":id,"label":id,"kind":kind,"evidence":"script"}))),
            "edges": ([("s","m","flow",""),("m","gate","choice","Left"),("m","bridge","choice","Right"),("gate","a","flow",""),("gate","b","flow",""),("bridge","gate","flow","")]
                .map(|(from,to,kind,label)| serde_json::json!({"from":from,"to":to,"kind":kind,"label":label,"evidence":"script","inferred":false}))),
            "guides":[{"name":"A","start":"s","path":[{"to":"m"},{"to":"bridge","via":"Right"},{"to":"gate"},{"to":"a"}],"evidence":"local guide","prerequisite":"Finish B and start a new game"}],
            "uncertainties":[]
        });
        let graph: Graph = serde_json::from_value(value.clone()).unwrap();
        let display = graph.display_data().unwrap();
        assert_eq!(graph.visible_node("bridge").unwrap().as_deref(), Some("m"));
        assert_eq!(display["edges"][2]["scenes"][0]["title"], "bridge");
        assert_eq!(display["edges"][2]["to"], "gate");
        let routes = &display["focus_routes"];
        assert_eq!(routes[0]["choices"], serde_json::json!([2]));
        assert_eq!(routes[0]["edges"], serde_json::json!([0, 2, 3]));
        assert_eq!(routes[0]["members"], serde_json::json!(["a"]));
        assert_eq!(routes[0]["prerequisite"], "Finish B and start a new game");
        assert_eq!(routes[1]["guided"], false);
        let mut speculative = value.clone();
        speculative["edges"][5]["inferred"] = true.into();
        let speculative: Graph = serde_json::from_value(speculative).unwrap();
        let display = speculative.display_data().unwrap();
        let routes = display["focus_routes"].as_array().unwrap();
        assert_eq!(routes.len(), 2);
        assert!(
            routes
                .iter()
                .all(|route| route["guided"] == false && route.get("choices").is_none())
        );
        value["guides"][0]["path"][1] = serde_json::json!({"to":"bridge","via":"Wrong"});
        assert!(
            serde_json::from_value::<Graph>(value)
                .unwrap()
                .display_data()
                .is_err()
        );
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
    fn preserves_repeated_map_decisions_beyond_the_old_compact_graph_limit() {
        let nodes: Vec<_> = (0..100)
            .map(|index| {
                serde_json::json!({
                    "id":format!("map_{index}"), "label":format!("地图选择 {index}"),
                    "kind":if index==0 {"start"} else {"choice"}, "evidence":"script menu"
                })
            })
            .collect();
        let edges: Vec<_> = (0..99)
            .map(|index| {
                serde_json::json!({
                    "from":format!("map_{index}"), "to":format!("map_{}", index+1),
                    "label":"一緒に行く", "kind":"choice", "inferred":false,
                    "evidence":"script menu option"
                })
            })
            .collect();
        let graph: Graph = serde_json::from_value(serde_json::json!({
            "nodes":nodes,"edges":edges,"uncertainties":[]
        }))
        .unwrap();
        assert!(graph.validate().is_ok());
    }

    #[test]
    fn legacy_choice_effect_requires_an_unambiguous_assignment_on_that_option() {
        let mut graph = fixture();
        graph.characters.push(Character {
            id: "eve".into(),
            name: "蓼科伊舞".into(),
        });
        let edge = &mut graph.edges[0];
        edge.kind = "choice".into();
        edge.inferred = false;
        edge.evidence = "menu[1] exp=f.eve=f.eve+1".into();
        assert_eq!(
            graph.display_data().unwrap()["edges"][0]["effects"],
            serde_json::json!([{"stat":"蓼科伊舞","delta":1}])
        );
        for evidence in [
            "menu[0] exp=None",
            "exp=f.eve=f.other+1",
            "exp=f.eve=f.eve+1*2",
        ] {
            graph.edges[0].evidence = evidence.into();
            assert!(
                graph.display_data().unwrap()["edges"][0]
                    .get("effects")
                    .is_none()
            );
        }
        graph.edges[0].evidence = "exp=f.eve=f.eve-2".into();
        assert_eq!(
            graph.display_data().unwrap()["edges"][0]["effects"][0]["delta"],
            -2
        );
        graph.edges[0].inferred = true;
        assert!(
            graph.display_data().unwrap()["edges"][0]
                .get("effects")
                .is_none()
        );
    }

    #[test]
    fn automatic_flow_has_no_caption_but_choices_and_branch_conditions_survive() {
        let graph: Graph = serde_json::from_value(serde_json::json!({
            "nodes":[{"id":"s","kind":"start","label":"入口","evidence":"s:1"},
                     {"id":"m","kind":"choice","label":"是否等待","evidence":"s:2"},
                     {"id":"b","kind":"choice","label":"判定","evidence":"s:3"},
                     {"id":"e","kind":"ending","label":"结束","evidence":"s:4"}],
            "edges":[{"from":"s","to":"m","kind":"flow","label":"剧情推进","evidence":"s:1","inferred":false},
                     {"from":"m","to":"b","kind":"choice","label":"剧情推进","evidence":"s:2","inferred":false},
                     {"from":"b","to":"e","kind":"flow","label":"到达","condition":"分数 ≥ 2","evidence":"s:3","inferred":false},
                     {"from":"b","to":"s","kind":"flow","label":"其余情况","evidence":"s:3","inferred":false}],
            "uncertainties":[]
        })).unwrap();
        let edges = &graph.display_data().unwrap()["edges"];
        assert_eq!(edges[0]["label"], "");
        assert_eq!(edges[1]["label"], "剧情推进");
        assert_eq!(edges[2]["condition"], "分数 ≥ 2");
        assert_eq!(edges[3]["condition"], "其余情况");
        let svg = graph.render("路线", Language::Chinese).unwrap();
        assert_eq!(svg.matches("剧情推进").count(), 1);
        assert!(svg.contains("分数 ≥ 2") && svg.contains("其余情况"));
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
                effects: Vec::new(),
                scenes: Vec::new(),
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
        let paths: BTreeSet<_> = document
            .descendants()
            .filter(|node| node.attribute("marker-end").is_some())
            .filter_map(|node| node.attribute("d"))
            .collect();
        assert_eq!(paths.len(), 4);
        for path in paths {
            let commands: Vec<_> = path.chars().filter(char::is_ascii_alphabetic).collect();
            assert!(
                commands
                    .iter()
                    .all(|command| matches!(command, 'M' | 'H' | 'V'))
            );
            assert_eq!(commands.last(), Some(&'V'));
        }
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
            effects: Vec::new(),
            scenes: Vec::new(),
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
