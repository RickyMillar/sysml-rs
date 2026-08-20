//! Spec-silent port-side fallback pass (#71 / G3 port-label clash).
//!
//! §8.2.3.12's `port-l/r/t/b` graphical productions carry NO `direction`
//! precondition, and `InterconnectionView` declares "Constraints: None"
//! (§9.2.20.2.6): which side of a node a port renders on is a RENDERING
//! FREEDOM, not a modeled property. (core-steward ruling 2026-07-22; recorded
//! as a numbered spec-silent decision in `sysml-graphical-notation-contract.md`
//! §A.5/F.6.)
//!
//! R4 correctly removed direction inference from the port NAME, so a bare
//! `port fuelIn;` now carries `direction: None` / `side: None`. The renderer
//! hands a side-less port to elk with `portConstraints: FREE`, and elk then
//! stacks same-node ports into overlapping label boxes — the standing G3 clash
//! on `AllPartsView` / `EmptyExposeView`, whose `Engine`/`Gearbox` ports are all
//! bare and land on the same side.
//!
//! This pass gives every displayed port a DETERMINISTIC side WITHOUT
//! re-introducing name inference and WITHOUT writing a `direction` back onto
//! the model:
//!   - a port with a DECLARED direction is placed by that direction
//!     (`In` → West, `Out`/`InOut` → East), matching the interconnection
//!     generator's existing inline logic (`interconnection.rs`);
//!   - a direction-less port gets a spec-silent side by alternating
//!     North/South across the node's remaining side-less ports in declaration
//!     order. (North/South, not West/East: §8.2.3.12 permits any of
//!     `port-l/r/t/b` equally for an undirected port, and placing bare ports on
//!     the top/bottom edges separates a same-side clash by adding HEIGHT rather
//!     than width — declared-direction ports still own the W/E flow axis, and
//!     the wide-ribbon views that fail the G5 aspect gate are helped, not hurt.)
//!
//! A port that already carries a side (set inline by a generator) is left
//! untouched, so the pass is idempotent and never fights a generator. It runs
//! as a scene post-pass — ONE home serving every view kind — rather than being
//! duplicated across each generator's tangled port-assembly branches (the
//! General generator has its own `make_port_ir`, the Interconnection generator
//! a separate `make_port_ir_recursive`).

use super::types::{DiagramChild, DiagramIR, DiagramNode, PortDirection, PortSide};
use crate::smodel::ViewType;

/// Ensure every displayed port has a placement side. See module docs.
pub(crate) fn assign_port_sides(scene: &mut DiagramIR) {
    // Interconnection views self-manage port placement: the generator sets
    // sides on DECLARED-direction ports inline and deliberately leaves bare
    // ports to elk `FREE`, which — under the IBD context frame's
    // INCLUDE_CHILDREN layout — spreads them without piling (PowertrainView
    // routes cleanly). Forcing sides there only overrides good placement and
    // lengthens routes. The clash this pass fixes is specific to the layered
    // General-view layout, where elk piles bare ports onto one side (G3).
    if scene.view_type == ViewType::Interconnection {
        return;
    }
    for node in &mut scene.nodes {
        assign_node_port_sides(node);
    }
}

fn assign_node_port_sides(node: &mut DiagramNode) {
    // Alternate W/E across this node's own side-less, direction-less ports.
    // The counter is per-node: the clash is per-node, so alternation resets
    // at each node boundary.
    let mut fallback_toggle: u32 = 0;
    for port in &mut node.ports {
        // Hidden routing ports never reach elk / the canvas (state generators
        // inject cardinal routing ports); leave them alone. A port that a
        // generator already placed keeps its side.
        if port.is_hidden || port.side.is_some() {
            continue;
        }
        port.side = Some(match port.direction {
            Some(PortDirection::In) => PortSide::West,
            Some(PortDirection::Out) | Some(PortDirection::InOut) => PortSide::East,
            None => {
                // SPEC-SILENT fallback: deterministic N/S alternation (adds
                // height, not width — helps the wide-ribbon G5 aspect gate).
                let s = if fallback_toggle % 2 == 0 {
                    PortSide::North
                } else {
                    PortSide::South
                };
                fallback_toggle += 1;
                s
            }
        });
    }

    // Recurse into nested child nodes (IBD internals, expanded packages).
    for child in &mut node.children {
        if let DiagramChild::Node(n) = child {
            assign_node_port_sides(n);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::types::{DiagramNode, DiagramPort};
    use crate::smodel::ViewType;
    use crate::visual_kind::VisualKind;

    fn port(id: &str, dir: Option<PortDirection>) -> DiagramPort {
        let mut p = DiagramPort::new(id.to_string(), id.to_string());
        p.direction = dir;
        p
    }

    fn scene_with(node: DiagramNode) -> DiagramIR {
        let mut scene = DiagramIR::new(ViewType::General);
        scene.nodes.push(node);
        scene
    }

    #[test]
    fn interconnection_views_are_left_to_elk() {
        let mut node = DiagramNode::new("engine".to_string(), VisualKind::Part, "engine".to_string());
        node.ports.push(port("fuelIn", None));
        node.ports.push(port("torqueOut", None));
        let mut scene = DiagramIR::new(ViewType::Interconnection);
        scene.nodes.push(node);

        assign_port_sides(&mut scene);

        // Bare ports stay side-less (elk FREE) in interconnection views.
        assert_eq!(scene.nodes[0].ports[0].side, None);
        assert_eq!(scene.nodes[0].ports[1].side, None);
    }

    #[test]
    fn bare_ports_alternate_north_south_in_order() {
        let mut node = DiagramNode::new("engine".to_string(), VisualKind::Part, "engine : Engine".to_string());
        node.ports.push(port("fuelIn", None));
        node.ports.push(port("torqueOut", None));
        let mut scene = scene_with(node);

        assign_port_sides(&mut scene);

        let ports = &scene.nodes[0].ports;
        assert_eq!(ports[0].side, Some(PortSide::North));
        assert_eq!(ports[1].side, Some(PortSide::South));
    }

    #[test]
    fn declared_direction_wins_over_alternation() {
        let mut node = DiagramNode::new("n".to_string(), VisualKind::Part, "n".to_string());
        node.ports.push(port("a", Some(PortDirection::Out))); // East by declared direction
        node.ports.push(port("b", None)); // bare fallback → North
        let mut scene = scene_with(node);

        assign_port_sides(&mut scene);

        let ports = &scene.nodes[0].ports;
        assert_eq!(ports[0].side, Some(PortSide::East));
        assert_eq!(ports[1].side, Some(PortSide::North));
    }

    #[test]
    fn generator_assigned_side_is_left_untouched() {
        let mut node = DiagramNode::new("n".to_string(), VisualKind::Part, "n".to_string());
        let mut p = port("a", None);
        p.side = Some(PortSide::North);
        node.ports.push(p);
        let mut scene = scene_with(node);

        assign_port_sides(&mut scene);

        assert_eq!(scene.nodes[0].ports[0].side, Some(PortSide::North));
    }

    #[test]
    fn hidden_routing_ports_are_skipped() {
        let mut node = DiagramNode::new("n".to_string(), VisualKind::Part, "n".to_string());
        let mut p = port("route", None);
        p.is_hidden = true;
        node.ports.push(p);
        let mut scene = scene_with(node);

        assign_port_sides(&mut scene);

        assert_eq!(scene.nodes[0].ports[0].side, None);
    }
}
