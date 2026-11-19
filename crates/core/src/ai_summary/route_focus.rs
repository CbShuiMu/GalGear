//! 将有出处的入线走法绑定到具体选项。缺少攻略的旧图仅展示能够连到结局的剧情。
use super::{Edge, Graph, Kind, text_ok};
use galgear_i18n::Message;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Guide {
    name: String,
    start: String,
    path: Vec<Step>,
    evidence: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Step {
    to: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    via: String,
}

fn story_edge(e: &Edge) -> bool {
    !matches!(e.kind.as_str(), "unlock" | "reference")
}

fn trace<'a>(
    starts: impl IntoIterator<Item = &'a str>,
    links: &BTreeMap<&str, Vec<&'a str>>,
    excluded: &BTreeSet<&str>,
) -> BTreeSet<&'a str> {
    let mut result = BTreeSet::new();
    let mut queue: Vec<_> = starts.into_iter().collect();
    while let Some(id) = queue.pop() {
        if !excluded.contains(id) && result.insert(id) {
            queue.extend(links.get(id).into_iter().flatten());
        }
    }
    result
}

impl Guide {
    fn resolve(&self, graph: &Graph) -> Result<Vec<usize>, Message> {
        let invalid = || {
            Message::detail(
                "route.invalid_analysis",
                "Guide needs a source, start node and continuous path to its route entry. Identify parallel edges by exact option or condition.",
            )
        };
        if !text_ok(&self.name, 80, false)
            || !text_ok(&self.evidence, 240, false)
            || self.path.is_empty()
            || self.path.len() > 512
            || !graph
                .nodes
                .iter()
                .any(|n| n.id == self.start && matches!(n.kind, Kind::Start))
        {
            return Err(invalid());
        }
        let mut from = &self.start;
        let mut result = Vec::new();
        for step in &self.path {
            let mut edges = graph.edges.iter().enumerate().filter(|(_, e)| {
                e.from == *from
                    && e.to == step.to
                    && story_edge(e)
                    && (step.via.is_empty() || e.label == step.via || e.condition == step.via)
            });
            let (index, _) = edges.next().ok_or_else(invalid)?;
            if edges.next().is_some() {
                return Err(invalid());
            }
            result.push(index);
            from = &step.to;
        }
        Ok(result)
    }
}

pub(super) fn routes(graph: &Graph) -> Result<Value, Message> {
    let mut forward = BTreeMap::<_, Vec<_>>::new();
    let mut backward = BTreeMap::<_, Vec<_>>::new();
    for edge in graph.edges.iter().filter(|e| story_edge(e)) {
        forward
            .entry(edge.from.as_str())
            .or_default()
            .push(edge.to.as_str());
        backward
            .entry(edge.to.as_str())
            .or_default()
            .push(edge.from.as_str());
    }
    let empty = BTreeSet::new();
    let starts = graph
        .nodes
        .iter()
        .filter(|n| matches!(n.kind, Kind::Start))
        .map(|n| n.id.as_str());
    let accessible = trace(starts, &forward, &empty);
    let mut routes = Vec::new();
    let mut covered = BTreeSet::new();
    let entries: BTreeSet<_> = graph
        .guides
        .iter()
        .filter_map(|g| g.path.last().map(|s| s.to.as_str()))
        .collect();
    if entries.len() != graph.guides.len() {
        return Err(super::super::invalid());
    }
    for guide in &graph.guides {
        let walk = guide.resolve(graph)?;
        let entry = guide
            .path
            .last()
            .ok_or_else(super::super::invalid)?
            .to
            .as_str();
        let choices: Vec<_> = walk
            .iter()
            .copied()
            .filter(|&i| graph.edges[i].kind == "choice")
            .collect();
        let mut edges: BTreeSet<_> = walk.into_iter().collect();
        let mut others = entries.clone();
        others.remove(entry);
        let members = trace([entry], &forward, &others);
        covered.extend(members.iter().copied());
        edges.extend(
            graph
                .edges
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    story_edge(e)
                        && members.contains(e.from.as_str())
                        && members.contains(e.to.as_str())
                })
                .map(|(i, _)| i),
        );
        let mut nodes = members.clone();
        nodes.insert(&guide.start);
        nodes.extend(guide.path.iter().map(|s| s.to.as_str()));
        routes.push(json!({"name":guide.name,"entry":entry,"nodes":nodes,"edges":edges,"members":members,"guided":true,"choices":choices}));
    }
    for end in graph.nodes.iter().filter(|n| {
        matches!(n.kind, Kind::Ending | Kind::Grand)
            && accessible.contains(n.id.as_str())
            && !covered.contains(n.id.as_str())
    }) {
        let ancestors = trace([end.id.as_str()], &backward, &empty);
        let nodes: BTreeSet<_> = ancestors.intersection(&accessible).copied().collect();
        let edges: Vec<_> = graph
            .edges
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                story_edge(e) && nodes.contains(e.from.as_str()) && nodes.contains(e.to.as_str())
            })
            .map(|(i, _)| i)
            .collect();
        routes.push(json!({"name":end.label,"entry":end.id,"nodes":nodes,"edges":edges,"members":nodes,"guided":false}));
    }
    // 共通章和合流场景仍可点击展开。仅属于一条线的剧情框才触发选线。
    let mut membership = BTreeMap::new();
    for route in &routes {
        for id in route["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            *membership.entry(id.to_owned()).or_insert(0) += 1;
        }
    }
    for route in &mut routes {
        let entry = route["entry"].clone();
        if let Some(members) = route["members"].as_array_mut() {
            members.retain(|id| {
                id == &entry || id.as_str().is_some_and(|s| membership.get(s) == Some(&1))
            });
        }
    }
    if graph.guides.is_empty() && routes.len() < 2 {
        routes.clear();
    }
    Ok(Value::Array(routes))
}
