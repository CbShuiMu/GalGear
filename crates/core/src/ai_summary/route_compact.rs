//! 将连续剧情收进箭头详情，保留真正影响走法的节点。
use super::{Edge, FoldedScene, Graph, Kind, Node};
use std::collections::BTreeMap;

pub(super) struct Projection {
    pub(super) graph: Graph,
    pub(super) anchors: BTreeMap<String, String>,
}

fn simple_flow(graph: &Graph, edge: &Edge) -> bool {
    matches!(edge.kind.as_str(), "" | "flow")
        && graph.edge_caption(edge) == ("", "")
        && edge.effects.is_empty()
}

fn fold_target<'a>(graph: &'a Graph, node: &Node) -> Option<(&'a str, usize)> {
    let id = node.id.as_str();
    if !matches!(node.kind, Kind::Scene) || graph.guides.iter().any(|guide| guide.ends_at(id)) {
        return None;
    }
    let mut exits = graph.edges.iter().enumerate().filter(|(_, e)| e.from == id);
    let (exit_index, exit) = exits.next()?;
    if exits.next().is_some() || !simple_flow(graph, exit) || exit.to == id {
        return None;
    }
    let arrivals: Vec<_> = graph.edges.iter().filter(|e| e.to == id).collect();
    let origin = arrivals.first()?.from.as_str();
    if origin == exit.to
        || arrivals
            .iter()
            .any(|e| e.from != origin || matches!(e.kind.as_str(), "unlock" | "reference"))
    {
        return None;
    }
    // 如果同一菜单的同名选项原先靠目标区分，保留目标之间的区别。
    if graph.edges.iter().any(|e| {
        e.from == origin
            && e.to == exit.to
            && arrivals.iter().any(|before| {
                before.label == e.label
                    || (!before.condition.is_empty()
                        && (before.condition == e.label || before.condition == e.condition))
                    || (!e.condition.is_empty() && e.condition == before.label)
            })
    }) {
        return None;
    }
    Some((origin, exit_index))
}

pub(super) fn project(mut graph: Graph) -> Projection {
    let mut anchors = BTreeMap::<String, String>::new();
    while let Some((index, origin, exit_index)) =
        graph.nodes.iter().enumerate().find_map(|(index, node)| {
            fold_target(&graph, node).map(|(origin, exit)| (index, origin.to_owned(), exit))
        })
    {
        let node = graph.nodes.remove(index);
        let exit = graph.edges.remove(exit_index);
        let detail = FoldedScene {
            title: node.label,
            pc: node.pc,
            evidence: node.evidence,
            transition_pc: exit.pc,
            transition_evidence: exit.evidence,
        };
        for edge in graph.edges.iter_mut().filter(|edge| edge.to == node.id) {
            edge.to.clone_from(&exit.to);
            edge.inferred |= exit.inferred;
            edge.scenes.push(detail.clone());
            edge.scenes.extend(exit.scenes.iter().cloned());
        }
        for guide in &mut graph.guides {
            guide.fold(&node.id, &exit.to);
        }
        for anchor in anchors.values_mut().filter(|anchor| **anchor == node.id) {
            anchor.clone_from(&origin);
        }
        anchors.insert(node.id, origin);
    }
    Projection { graph, anchors }
}
