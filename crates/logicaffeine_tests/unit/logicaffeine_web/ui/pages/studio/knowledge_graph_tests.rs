use super::kg_svg;
use logicaffeine_compile::codegen_sva::hw_pipeline::{KgLink, KgNode, KgSummary};

fn graph(links: &[(usize, usize, &str)]) -> KgSummary {
    let node = |name: &str, role: &str| KgNode { name: name.to_string(), role: role.to_string(), width: 1 };
    KgSummary {
        nodes: vec![node("Request", "input"), node("Acknowledge", "output"), node("high", "internal")],
        links: links.iter().map(|&(from, to, relation)| KgLink { from, to, relation: relation.to_string() }).collect(),
    }
}

fn relation_labels(svg: &str) -> Vec<String> {
    svg.split(r#"<text class="kgr""#)
        .skip(1)
        .map(|rest| rest.split('>').nth(1).unwrap_or("").split('<').next().unwrap_or("").to_string())
        .collect()
}

/// Two relations between one pair of signals share one line, so they share one label —
/// drawn apart at the same midpoint they overprinted into an unreadable smear.
#[test]
fn relations_between_the_same_signals_share_one_label() {
    let svg = kg_svg(&graph(&[(0, 1, "triggers"), (0, 1, "handshake")]));
    assert_eq!(relation_labels(&svg), ["triggers · handshake"]);
}

#[test]
fn relations_in_opposite_directions_share_one_label() {
    let svg = kg_svg(&graph(&[(0, 1, "triggers"), (1, 0, "acknowledges")]));
    assert_eq!(relation_labels(&svg), ["triggers · acknowledges"]);
}

#[test]
fn relations_between_different_signals_keep_their_own_labels() {
    let svg = kg_svg(&graph(&[(0, 1, "triggers"), (0, 2, "drives")]));
    assert_eq!(relation_labels(&svg), ["triggers", "drives"]);
}

#[test]
fn a_relation_named_twice_is_labelled_once() {
    let svg = kg_svg(&graph(&[(0, 1, "triggers"), (0, 1, "triggers")]));
    assert_eq!(relation_labels(&svg), ["triggers"]);
}
