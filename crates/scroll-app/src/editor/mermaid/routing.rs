//! Direct routes for flowchart edges. `mermaid-rs-renderer` places a flowchart's nodes well but
//! often takes its edges the long way round: out of a node, over the next one and in from its
//! far side, or two edges drawn along one line. None of its routing options cures that, so once
//! it has laid a flowchart out, `straighten_edges` routes again every edge that can run straight
//! from its node to one further along the flow. Any other edge keeps the route the crate gave
//! it: one that loops back, one with a node in its way, and one the crate already draws with
//! fewer bends.
//!
//! A direct edge leaves the side of its source that faces along the flow and enters the facing
//! side of its target, at ports spread evenly along each side. Where its two ports can line up
//! it is one straight line; otherwise it runs straight out, slants across and runs straight in,
//! and the renderer rounds the two bends. Its label goes on the line, or beside it when the
//! line is too short to show at both ends of the label.
//!
//! All of it is worked out in flow space (`Flow`), where every flowchart runs the same way, so
//! one implementation serves the four directions.

use std::collections::BTreeMap;

use mermaid_rs_renderer::layout::TextBlock;
use mermaid_rs_renderer::{Direction, EdgeLayout, Layout, NodeLayout, NodeShape, SubgraphLayout};

/// How far a bent edge runs straight out of one node and into the other before it turns: far
/// enough that the arrowhead sits on the straight part once the renderer has rounded the bend.
const STUB: f32 = 20.0;
/// The least room between two nodes, along the flow, for a direct edge and its arrowhead.
const ARROW_ROOM: f32 = 10.0;
/// The least room between two nodes for an edge to bend on its way across.
const BENDING_ROOM: f32 = 24.0;
/// How near a direct edge may pass a node or subgraph it does not belong to.
const CLEARANCE: f32 = 8.0;
/// The least distance between two ports on one side, so that their arrowheads stay apart.
const PORT_SPACING: f32 = 12.0;
/// How far a port stays from the ends of its side.
const CORNER_INSET: f32 = 8.0;
/// Ports this close across the flow count as lined up, and their edge is drawn straight.
const LINED_UP: f32 = 1.0;
/// A turn of more than this many degrees counts as a bend.
const BEND_DEGREES: f32 = 3.0;
/// The room the renderer leaves around the text of a flowchart edge label, across and down.
const LABEL_PADDING: (f32, f32) = (4.5, 2.2);
/// How far across the gap between its two nodes a label is tried, the middle first.
const LABEL_POSITIONS: [f32; 5] = [0.5, 0.35, 0.65, 0.2, 0.8];
/// How much of each end of an edge a label on it should leave showing: the arrowhead and a
/// little of the line.
const LINE_SHOWING: f32 = 12.0;
/// The room between a line and a label beside it. Any nearer and the renderer counts the label
/// as on the line, and draws a box behind it.
const BESIDE_LINE: f32 = 2.5;
/// The room left between two labels that had to be moved apart.
const LABEL_SPACING: f32 = 1.0;
/// How often overlapping labels are moved apart; each round can push one into another.
const SPREADING_ROUNDS: usize = 8;

/// Routes again the edges of a flowchart's `layout` that can run directly from one node to the
/// next, puts their labels on the new lines and cuts the page to fit.
pub(super) fn straighten_edges(layout: &mut Layout, direction: Direction) {
    let flow = Flow::of(direction);
    let direct = direct_routes(layout, flow);
    if direct.is_empty() {
        return;
    }
    let drawn_before = Bounds::of(layout);
    for (&index, direct) in &direct {
        let route = direct.route.iter();
        layout.edges[index].points = route.map(|point| flow.page(*point)).collect();
    }
    place_labels(layout, flow, &direct);
    fit_page(layout, drawn_before);
}

/// A point in flow space: `main` grows the way the diagram flows, `cross` runs across it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Point {
    main: f32,
    cross: f32,
}

/// A stretch of one axis. It is empty when it ends before it starts.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Span {
    start: f32,
    end: f32,
}

impl Span {
    fn between(a: f32, b: f32) -> Span {
        Span {
            start: a.min(b),
            end: a.max(b),
        }
    }

    fn length(self) -> f32 {
        self.end - self.start
    }

    fn centre(self) -> f32 {
        (self.start + self.end) / 2.0
    }

    fn is_empty(self) -> bool {
        self.start > self.end
    }

    fn overlaps(self, other: Span) -> bool {
        self.start < other.end && other.start < self.end
    }

    fn grown(self, by: f32) -> Span {
        Span {
            start: self.start - by,
            end: self.end + by,
        }
    }

    fn moved(self, by: f32) -> Span {
        Span {
            start: self.start + by,
            end: self.end + by,
        }
    }

    /// The stretch both spans cover.
    fn shared_with(self, other: Span) -> Span {
        Span {
            start: self.start.max(other.start),
            end: self.end.min(other.end),
        }
    }

    /// The part of the line from `from` to `to` that lies in this span, as fractions of the way
    /// along the line; it can reach beyond the line's two ends.
    fn part_of_line(self, from: f32, to: f32) -> Span {
        let travel = to - from;
        if travel != 0.0 {
            return Span::between((self.start - from) / travel, (self.end - from) / travel);
        }
        if self.start <= from && from <= self.end {
            return Span {
                start: f32::NEG_INFINITY,
                end: f32::INFINITY,
            };
        }
        Span {
            start: f32::INFINITY,
            end: f32::NEG_INFINITY,
        }
    }
}

/// A rectangle in flow space.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    main: Span,
    cross: Span,
}

impl Rect {
    fn around(centre: Point, size: Point) -> Rect {
        Rect {
            main: Span::between(centre.main - size.main / 2.0, centre.main + size.main / 2.0),
            cross: Span::between(
                centre.cross - size.cross / 2.0,
                centre.cross + size.cross / 2.0,
            ),
        }
    }

    fn centre(self) -> Point {
        Point {
            main: self.main.centre(),
            cross: self.cross.centre(),
        }
    }

    fn grown(self, by: f32) -> Rect {
        Rect {
            main: self.main.grown(by),
            cross: self.cross.grown(by),
        }
    }

    fn overlaps(self, other: Rect) -> bool {
        self.main.overlaps(other.main) && self.cross.overlaps(other.cross)
    }

    /// Whether the straight line from `from` to `to` touches the rectangle.
    fn is_crossed_by(self, from: Point, to: Point) -> bool {
        let whole_line = Span {
            start: 0.0,
            end: 1.0,
        };
        let inside = self
            .main
            .part_of_line(from.main, to.main)
            .shared_with(self.cross.part_of_line(from.cross, to.cross))
            .shared_with(whole_line);
        !inside.is_empty()
    }

    fn is_crossed_by_route(self, route: &[Point]) -> bool {
        route
            .windows(2)
            .any(|line| self.is_crossed_by(line[0], line[1]))
    }
}

/// The way a flowchart runs. Flow space turns the page so that it runs towards greater `main`.
#[derive(Clone, Copy)]
struct Flow {
    is_horizontal: bool,
    is_reversed: bool,
}

impl Flow {
    fn of(direction: Direction) -> Flow {
        Flow {
            is_horizontal: matches!(direction, Direction::LeftRight | Direction::RightLeft),
            is_reversed: matches!(direction, Direction::RightLeft | Direction::BottomTop),
        }
    }

    fn point(self, (x, y): (f32, f32)) -> Point {
        let (main, cross) = if self.is_horizontal { (x, y) } else { (y, x) };
        Point {
            main: if self.is_reversed { -main } else { main },
            cross,
        }
    }

    fn page(self, point: Point) -> (f32, f32) {
        let main = if self.is_reversed {
            -point.main
        } else {
            point.main
        };
        if self.is_horizontal {
            (main, point.cross)
        } else {
            (point.cross, main)
        }
    }

    fn route(self, points: &[(f32, f32)]) -> Vec<Point> {
        points.iter().map(|point| self.point(*point)).collect()
    }

    /// A width and height on the page, as lengths along and across the flow.
    fn size(self, (width, height): (f32, f32)) -> Point {
        let (main, cross) = if self.is_horizontal {
            (width, height)
        } else {
            (height, width)
        };
        Point { main, cross }
    }

    fn rect(self, (x, y): (f32, f32), (width, height): (f32, f32)) -> Rect {
        let corner = self.point((x, y));
        let opposite = self.point((x + width, y + height));
        Rect {
            main: Span::between(corner.main, opposite.main),
            cross: Span::between(corner.cross, opposite.cross),
        }
    }

    fn node(self, node: &NodeLayout) -> Rect {
        self.rect((node.x, node.y), (node.width, node.height))
    }

    fn frame(self, subgraph: &SubgraphLayout) -> Rect {
        self.rect((subgraph.x, subgraph.y), (subgraph.width, subgraph.height))
    }
}

/// The outline of a side of a node that faces along the flow.
#[derive(Clone, Copy)]
enum Side {
    /// Straight, along the node's bounding box.
    Flat,
    /// Two straight edges that meet in a point `depth` further out, as on a diamond.
    Pointed { depth: f32 },
    /// An arc that bulges `depth` out, as on a circle.
    Rounded { depth: f32 },
}

/// What an edge starts or ends on, in flow space: a node, or the frame of a subgraph.
#[derive(Clone, Copy)]
struct End {
    bounds: Rect,
    side: Side,
    /// The stretch of the facing sides that ports are spread along.
    ports: Span,
}

impl End {
    /// `None` for a node that is not drawn, and for the shapes with slanted sides, whose
    /// outlines are not worked out here: their edges keep the crate's routes.
    fn of(layout: &Layout, id: &str, flow: Flow) -> Option<End> {
        let node = layout.nodes.get(id)?;
        if let Some(subgraph) = node.anchor_subgraph {
            // An edge to a whole subgraph, which the crate ends on a hidden node inside it.
            let bounds = flow.frame(layout.subgraphs.get(subgraph)?);
            return Some(End {
                bounds,
                side: Side::Flat,
                ports: bounds.cross,
            });
        }
        if node.hidden || node.width <= 0.0 || node.height <= 0.0 {
            return None;
        }
        let bounds = flow.node(node);
        let is_box = matches!(
            node.shape,
            NodeShape::Rectangle | NodeShape::RoundRect | NodeShape::Subroutine
        );
        // The other shapes narrow or curve towards the ends of a side, so their ports keep to
        // its middle half.
        let reach = bounds.cross.length() * if is_box { 0.5 } else { 0.25 };
        Some(End {
            bounds,
            side: facing_side(node, flow)?,
            ports: Span {
                start: bounds.cross.centre() - reach,
                end: bounds.cross.centre() + reach,
            },
        })
    }

    /// How far inside the bounding box the outline lies at `cross` on a facing side.
    fn recess(&self, cross: f32) -> f32 {
        let half_side = self.bounds.cross.length() / 2.0;
        let offset = ((cross - self.bounds.cross.centre()) / half_side).clamp(-1.0, 1.0);
        match self.side {
            Side::Flat => 0.0,
            Side::Pointed { depth } => depth * offset.abs(),
            Side::Rounded { depth } => depth * (1.0 - (1.0 - offset * offset).sqrt()),
        }
    }

    /// The point on the outline that an edge leaves by, at `cross`.
    fn exit(&self, cross: f32) -> Point {
        Point {
            main: self.bounds.main.end - self.recess(cross),
            cross,
        }
    }

    /// The point on the outline that an edge enters by, at `cross`.
    fn entry(&self, cross: f32) -> Point {
        Point {
            main: self.bounds.main.start + self.recess(cross),
            cross,
        }
    }
}

/// The outline the renderer draws for `node` on its sides that face along the flow.
fn facing_side(node: &NodeLayout, flow: Flow) -> Option<Side> {
    let size = flow.size((node.width, node.height));
    match (node.shape, flow.is_horizontal) {
        (NodeShape::Rectangle | NodeShape::RoundRect | NodeShape::Subroutine, _) => {
            Some(Side::Flat)
        }
        // Over its middle half the cap of a cylinder is within two pixels of flat.
        (NodeShape::Cylinder, _) | (NodeShape::Stadium | NodeShape::Hexagon, false) => {
            Some(Side::Flat)
        }
        (NodeShape::Diamond, _) => Some(Side::Pointed {
            depth: size.main / 2.0,
        }),
        (NodeShape::Hexagon, true) => Some(Side::Pointed {
            depth: size.main / 4.0,
        }),
        (NodeShape::Circle | NodeShape::DoubleCircle, _) => Some(Side::Rounded {
            depth: size.main / 2.0,
        }),
        (NodeShape::Stadium, true) => Some(Side::Rounded {
            depth: size.cross / 2.0,
        }),
        _ => None,
    }
}

/// An edge whose target lies further along the flow than its source, so it might run directly.
struct Forward<'a> {
    index: usize,
    edge: &'a EdgeLayout,
    source: End,
    target: End,
}

impl Forward<'_> {
    /// The room between the two nodes along the flow.
    fn gap(&self) -> Span {
        Span {
            start: self.source.bounds.main.end,
            end: self.target.bounds.main.start,
        }
    }

    /// Edges leaving one node are ordered by where they go, so that they do not cross; edges
    /// to the same node keep the order they were written in.
    fn leaves_before(&self, other: &Forward) -> bool {
        (self.target.bounds.cross.centre(), self.index)
            < (other.target.bounds.cross.centre(), other.index)
    }

    /// Edges entering one node are ordered by where they come from.
    fn enters_before(&self, other: &Forward) -> bool {
        (self.source.bounds.cross.centre(), self.index)
            < (other.source.bounds.cross.centre(), other.index)
    }

    /// Whether `route` may replace the crate's: it has the room to bend if it does, passes
    /// through nothing, and bends no more often than the crate's route.
    fn can_take(&self, route: &[Point], layout: &Layout, flow: Flow) -> bool {
        let is_straight = route.len() == 2;
        let is_clear = || {
            let obstacles = obstacles(self.edge, layout, flow);
            !obstacles.iter().any(|rect| rect.is_crossed_by_route(route))
        };
        let crate_route = flow.route(&self.edge.points);
        (is_straight || self.gap().length() >= BENDING_ROOM)
            && bend_count(route) <= bend_count(&crate_route)
            && is_clear()
    }
}

/// Where an edge attaches to its two nodes, across the flow.
#[derive(Clone, Copy)]
struct Ports {
    source: f32,
    target: f32,
}

/// The new route of an edge, and the room between its two nodes that the route crosses.
struct Direct {
    route: Vec<Point>,
    gap: Span,
}

impl Direct {
    /// The point of the route that is the fraction `along` of the way across the gap.
    fn point_across_gap(&self, along: f32) -> Option<Point> {
        let main = self.gap.start + self.gap.length() * along;
        // A direct route never turns back, so the first line to reach `main` is the one that
        // passes it.
        let line = self.route.windows(2).find(|line| main <= line[1].main)?;
        let covered = (main - line[0].main) / (line[1].main - line[0].main);
        Some(Point {
            main,
            cross: line[0].cross + (line[1].cross - line[0].cross) * covered,
        })
    }

    /// The places to try for a label of `size`, best first: on the line, then to either side of
    /// it, at each of `LABEL_POSITIONS`.
    fn label_spots(&self, size: Point) -> Vec<Rect> {
        let aside = size.cross / 2.0 + BESIDE_LINE;
        let along = LABEL_POSITIONS.iter();
        let on_line = along.filter_map(|along| self.point_across_gap(*along));
        on_line
            .flat_map(|spot| {
                [0.0, aside, -aside].map(|offset| {
                    let centre = Point {
                        cross: spot.cross + offset,
                        ..spot
                    };
                    Rect::around(centre, size)
                })
            })
            .collect()
    }
}

/// The new route of every edge that gets one, by the edge's place in `layout.edges`.
fn direct_routes(layout: &Layout, flow: Flow) -> BTreeMap<usize, Direct> {
    let mut forward = forward_edges(layout, flow);
    // An edge that cannot run directly gives up its ports, which moves the ports beside them,
    // and keeps its label where the crate put it, in the way of the others. So the rest are
    // routed again until every one of them can run directly.
    loop {
        let candidates = forward.len();
        let routes = routes_between_ports(&forward);
        let is_candidate = |index: usize| forward.iter().any(|edge| edge.index == index);
        let labels = kept_labels(layout, flow, |index| !is_candidate(index));
        let direct: Vec<(Forward, Vec<Point>)> = forward
            .into_iter()
            .zip(routes)
            .filter(|(edge, route)| {
                let strikes_a_label = labels.iter().any(|label| label.is_crossed_by_route(route));
                !strikes_a_label && edge.can_take(route, layout, flow)
            })
            .collect();
        if direct.len() == candidates {
            let direct = direct.into_iter();
            return direct
                .map(|(edge, route)| {
                    let gap = edge.gap();
                    (edge.index, Direct { route, gap })
                })
                .collect();
        }
        forward = direct.into_iter().map(|(edge, _)| edge).collect();
    }
}

/// The edges that run forwards between two ends whose outlines are known, with room between
/// them for at least an arrowhead.
fn forward_edges(layout: &Layout, flow: Flow) -> Vec<Forward<'_>> {
    let edges = layout.edges.iter().enumerate();
    edges
        .filter_map(|(index, edge)| {
            // Labels at the ends of an edge are placed against the crate's route.
            if edge.start_label.is_some() || edge.end_label.is_some() {
                return None;
            }
            let forward = Forward {
                index,
                edge,
                source: End::of(layout, &edge.from, flow)?,
                target: End::of(layout, &edge.to, flow)?,
            };
            (forward.gap().length() >= ARROW_ROOM).then_some(forward)
        })
        .collect()
}

/// A route for each of `forward`, in the same order.
fn routes_between_ports(forward: &[Forward]) -> Vec<Vec<Point>> {
    let mut ports = spread_ports(forward);
    line_up_ports(forward, &mut ports);
    let edges = forward.iter().zip(&ports);
    edges.map(|(edge, ports)| route(edge, *ports)).collect()
}

/// Every edge's ports, shared out evenly along the sides they are on.
fn spread_ports(forward: &[Forward]) -> Vec<Ports> {
    let ports_of = |edge: &Forward| {
        let leaving = forward
            .iter()
            .filter(|other| other.edge.from == edge.edge.from);
        let entering = forward.iter().filter(|other| other.edge.to == edge.edge.to);
        let leaving_before = leaving.clone().filter(|other| other.leaves_before(edge));
        let entering_before = entering.clone().filter(|other| other.enters_before(edge));
        Ports {
            source: evenly_spaced(edge.source.ports, leaving_before.count(), leaving.count()),
            target: evenly_spaced(edge.target.ports, entering_before.count(), entering.count()),
        }
    };
    forward.iter().map(ports_of).collect()
}

/// The middle of the `position`th of `count` equal parts of `span`.
fn evenly_spaced(span: Span, position: usize, count: usize) -> f32 {
    span.start + span.length() * (2 * position + 1) as f32 / (2 * count) as f32
}

/// Moves the two ports of an edge level with each other wherever both have the room, so that
/// the edge is one straight line.
fn line_up_ports(forward: &[Forward], ports: &mut [Ports]) {
    for (position, edge) in forward.iter().enumerate() {
        let others = || {
            let others = forward.iter().zip(ports.iter());
            others.filter(|(other, _)| other.index != edge.index)
        };
        let beside_source = others()
            .filter(|(other, _)| other.edge.from == edge.edge.from)
            .map(|(_, ports)| ports.source);
        let beside_target = others()
            .filter(|(other, _)| other.edge.to == edge.edge.to)
            .map(|(_, ports)| ports.target);
        let Ports { source, target } = ports[position];
        let shared = room_for_port(edge.source.ports, source, beside_source)
            .shared_with(room_for_port(edge.target.ports, target, beside_target));
        if shared.is_empty() {
            continue;
        }
        // The source's port moves to the target's, which gives way only as far as it must:
        // arrowheads then stay evenly spread along the side they point at.
        let level = target.max(shared.start).min(shared.end);
        ports[position] = Ports {
            source: level,
            target: level,
        };
    }
}

/// How far the port at `port` can move along its side: not into a corner, and not past or up
/// against the ports beside it.
fn room_for_port(side: Span, port: f32, beside: impl Iterator<Item = f32>) -> Span {
    let mut room = side.grown(-CORNER_INSET.min(side.length() / 4.0));
    for other in beside {
        if other < port {
            room.start = room.start.max(other + PORT_SPACING);
        } else {
            room.end = room.end.min(other - PORT_SPACING);
        }
    }
    room
}

/// The route between the two ports of `edge`: one straight line when they are level, and
/// otherwise straight out of the source, across on a slant and straight into the target.
fn route(edge: &Forward, ports: Ports) -> Vec<Point> {
    let end = edge.target.entry(ports.target);
    if (ports.source - ports.target).abs() < LINED_UP {
        return vec![edge.source.exit(ports.target), end];
    }
    let start = edge.source.exit(ports.source);
    let gap = edge.gap();
    let stub = STUB.min(gap.length() / 3.0);
    let turn_out = Point {
        main: gap.start + stub,
        cross: start.cross,
    };
    let turn_in = Point {
        main: gap.end - stub,
        cross: end.cross,
    };
    vec![start, turn_out, turn_in, end]
}

/// What a direct route for `edge` must keep clear of: every node and subgraph but the ones the
/// edge starts and ends in, with a little room around each.
fn obstacles(edge: &EdgeLayout, layout: &Layout, flow: Flow) -> Vec<Rect> {
    let is_end = |id: &String| *id == edge.from || *id == edge.to;
    let nodes = layout.nodes.values();
    let other_nodes = nodes
        .filter(|node| !node.hidden && !is_end(&node.id))
        .map(|node| flow.node(node));
    let is_anchor_of = |subgraph: usize, id: &String| {
        let node = layout.nodes.get(id);
        node.is_some_and(|node| node.anchor_subgraph == Some(subgraph))
    };
    let subgraphs = layout.subgraphs.iter().enumerate();
    let other_subgraphs = subgraphs
        .filter(|(index, subgraph)| {
            !subgraph.nodes.iter().any(is_end)
                && !is_anchor_of(*index, &edge.from)
                && !is_anchor_of(*index, &edge.to)
        })
        .map(|(_, subgraph)| flow.frame(subgraph));
    other_nodes
        .chain(other_subgraphs)
        .map(|rect| rect.grown(CLEARANCE))
        .collect()
}

/// How often `route` turns. A point the crate repeats is no turn, nor is a wobble of a few
/// degrees.
fn bend_count(route: &[Point]) -> usize {
    let lines = route.windows(2).filter(|line| line[0] != line[1]);
    let headings: Vec<f32> = lines
        .map(|line| (line[1].cross - line[0].cross).atan2(line[1].main - line[0].main))
        .collect();
    let turns = headings.windows(2).map(|pair| {
        let turn = (pair[1] - pair[0]).to_degrees().rem_euclid(360.0);
        turn.min(360.0 - turn)
    });
    turns.filter(|turn| *turn > BEND_DEGREES).count()
}

/// Gives the label of each re-routed edge a place on or beside its new line.
fn place_labels(layout: &mut Layout, flow: Flow, direct: &BTreeMap<usize, Direct>) {
    let surroundings = Surroundings::of(layout, flow, direct);
    let labelled = direct.iter().filter_map(|(&index, direct)| {
        let size = label_size(layout.edges[index].label.as_ref()?, flow);
        let spots = direct.label_spots(size).into_iter();
        // Of equally good places `min_by_key` takes the first, which is the one tried first.
        let best = spots.min_by_key(|spot| surroundings.clutter(*spot, index))?;
        Some((index, best))
    });
    let (edges, mut labels): (Vec<usize>, Vec<Rect>) = labelled.unzip();
    spread_labels(&mut labels);
    for (index, label) in edges.into_iter().zip(labels) {
        layout.edges[index].label_anchor = Some(flow.page(label.centre()));
    }
}

/// What the label of a re-routed edge has to find room among.
struct Surroundings {
    /// The nodes, and the labels of the edges that kept the crate's routes.
    boxes: Vec<Rect>,
    /// The route of every edge, in the order of `layout.edges`.
    lines: Vec<Vec<Point>>,
}

impl Surroundings {
    fn of(layout: &Layout, flow: Flow, direct: &BTreeMap<usize, Direct>) -> Surroundings {
        let nodes = layout.nodes.values().filter(|node| !node.hidden);
        let kept_labels = kept_labels(layout, flow, |index| !direct.contains_key(&index));
        let edges = layout.edges.iter();
        Surroundings {
            boxes: nodes
                .map(|node| flow.node(node))
                .chain(kept_labels)
                .collect(),
            lines: edges.map(|edge| flow.route(&edge.points)).collect(),
        }
    }

    /// How much is wrong with `spot` as the place for the label of the edge at `index`; less is
    /// better. The best place is on the edge's line, leaving both ends of it showing and
    /// covering nothing else; then beside the line, where that is clear; then on the line
    /// regardless.
    fn clutter(&self, spot: Rect, index: usize) -> (usize, usize, bool) {
        let boxes = self.boxes.iter().filter(|taken| spot.overlaps(**taken));
        let other_lines = self.lines.iter().enumerate();
        let crossed = other_lines
            .filter(|(other, line)| *other != index && spot.is_crossed_by_route(line))
            .count();
        let own_line = &self.lines[index];
        if !spot.is_crossed_by_route(own_line) {
            // The renderer draws nothing behind a label that is off its line, so an edge
            // across one strikes its text through: as bad as covering a node.
            return (boxes.count() + crossed, 0, false);
        }
        (boxes.count(), crossed, hides_an_end(spot, own_line))
    }
}

/// Whether a label on `route` covers the arrowhead, or all but a stump, at either end of it.
fn hides_an_end(label: Rect, route: &[Point]) -> bool {
    let ends = route.first().zip(route.last());
    ends.is_some_and(|(start, end)| {
        label.main.start < start.main + LINE_SHOWING || label.main.end > end.main - LINE_SHOWING
    })
}

/// Moves apart the labels that overlap, each pair the short way: along the axis on which they
/// overlap least, which leaves them on or right beside their own edges.
fn spread_labels(labels: &mut [Rect]) {
    for _ in 0..SPREADING_ROUNDS {
        for later in 1..labels.len() {
            let (earlier, rest) = labels.split_at_mut(later);
            let label = &mut rest[0];
            for other in earlier {
                if !label.overlaps(*other) {
                    continue;
                }
                let overlap_along = label.main.shared_with(other.main).length();
                let overlap_across = label.cross.shared_with(other.cross).length();
                if overlap_along < overlap_across {
                    push_apart(&mut other.main, &mut label.main);
                } else {
                    push_apart(&mut other.cross, &mut label.cross);
                }
            }
        }
    }
}

/// Moves two overlapping spans equally far apart, to leave `LABEL_SPACING` between them.
fn push_apart(first: &mut Span, second: &mut Span) {
    let mut each = (first.shared_with(*second).length() + LABEL_SPACING) / 2.0;
    if first.centre() > second.centre() {
        each = -each;
    }
    *first = first.moved(-each);
    *second = second.moved(each);
}

/// The labels, where the crate put them, of the edges that `is_kept` picks out by their place
/// in `layout.edges`.
fn kept_labels(layout: &Layout, flow: Flow, is_kept: impl Fn(usize) -> bool) -> Vec<Rect> {
    let edges = layout.edges.iter().enumerate();
    edges
        .filter(|(index, _)| is_kept(*index))
        .filter_map(|(_, edge)| {
            let size = label_size(edge.label.as_ref()?, flow);
            Some(Rect::around(flow.point(edge.label_anchor?), size))
        })
        .collect()
}

/// The size of the box the renderer draws behind `label`, along and across the flow.
fn label_size(label: &TextBlock, flow: Flow) -> Point {
    flow.size(label_box(label))
}

/// The width and height of the box the renderer draws behind `label`.
fn label_box(label: &TextBlock) -> (f32, f32) {
    (
        label.width + 2.0 * LABEL_PADDING.0,
        label.height + 2.0 * LABEL_PADDING.1,
    )
}

/// The part of the page that is drawn on: its top left and bottom right corners.
struct Bounds {
    least: (f32, f32),
    most: (f32, f32),
}

impl Bounds {
    /// The bounds of every node, subgraph, edge and edge label.
    fn of(layout: &Layout) -> Bounds {
        let nodes = layout.nodes.values();
        let nodes = nodes.map(|node| ((node.x, node.y), (node.width, node.height)));
        let subgraphs = layout.subgraphs.iter();
        let subgraphs = subgraphs.map(|frame| ((frame.x, frame.y), (frame.width, frame.height)));
        let labels = layout.edges.iter().filter_map(|edge| {
            let (width, height) = label_box(edge.label.as_ref()?);
            let (x, y) = edge.label_anchor?;
            Some(((x - width / 2.0, y - height / 2.0), (width, height)))
        });
        let boxes = nodes.chain(subgraphs).chain(labels);
        let corners = boxes.flat_map(|((x, y), (width, height))| [(x, y), (x + width, y + height)]);
        let edges = layout.edges.iter();
        let corners = corners.chain(edges.flat_map(|edge| edge.points.iter().copied()));
        let none = Bounds {
            least: (f32::INFINITY, f32::INFINITY),
            most: (f32::NEG_INFINITY, f32::NEG_INFINITY),
        };
        corners.fold(none, |bounds, (x, y)| Bounds {
            least: (bounds.least.0.min(x), bounds.least.1.min(y)),
            most: (bounds.most.0.max(x), bounds.most.1.max(y)),
        })
    }
}

/// Cuts the page to fit the diagram again, with the margins the crate left round what was
/// `drawn_before`. The crate sized the page for its own routes, which may have reached further
/// out than anything that is left.
fn fit_page(layout: &mut Layout, drawn_before: Bounds) {
    let drawn = Bounds::of(layout);
    let shift = (
        drawn_before.least.0 - drawn.least.0,
        drawn_before.least.1 - drawn.least.1,
    );
    layout.width += drawn.most.0 + shift.0 - drawn_before.most.0;
    layout.height += drawn.most.1 + shift.1 - drawn_before.most.1;
    let moved = |(x, y): (f32, f32)| (x + shift.0, y + shift.1);
    for node in layout.nodes.values_mut() {
        (node.x, node.y) = moved((node.x, node.y));
    }
    for subgraph in &mut layout.subgraphs {
        (subgraph.x, subgraph.y) = moved((subgraph.x, subgraph.y));
    }
    for edge in &mut layout.edges {
        edge.points = edge.points.iter().map(|point| moved(*point)).collect();
        edge.label_anchor = edge.label_anchor.map(moved);
        edge.start_label_anchor = edge.start_label_anchor.map(moved);
        edge.end_label_anchor = edge.end_label_anchor.map(moved);
    }
}

#[cfg(test)]
mod tests {
    use mermaid_rs_renderer::{LayoutConfig, Theme, compute_layout, parse_mermaid};

    use super::*;

    /// The flowchart whose edges were reported as roundabout.
    const REPORTED: &str = "flowchart LR
    A[Pull request] --> B{Who opened it?}
    B -->|a person| C[One approval, then merge]
    B -->|Renovate, major| C
    B -->|Renovate, patch/minor| D[Merges automatically]";
    const LOOP_BACK: &str = "flowchart LR
    A[Idea] --> B{Ready?}
    B -->|Yes| C[Publish]
    B -->|No| A";
    const SKIPPED_RANK: &str = "flowchart LR
    A[Start] --> B[Middle]
    B --> C[End]
    A -->|skip| C";
    const FAN_OUT_AND_IN: &str = "flowchart LR
    S[Source] --> A[Alpha] & B[Beta] & C[Gamma] & D[Delta]
    A & B & C & D --> T[Target]";
    const PARALLEL: &str = "flowchart LR
    A[Client] -->|request| B[Server]
    A -->|retry| B
    A -->|cancel| B";
    const SUBGRAPH: &str = "flowchart LR
    U[User] --> G[Gateway]
    subgraph Backend
        G --> S1[Auth service] & S2[Page service]
        S1 & S2 --> DB[(Database)]
    end
    DB --> L([Logs])";
    const BRANCH_AND_JOIN: &str = "flowchart LR
    A[Start] --> B{Check}
    B -->|yes| C[Left]
    B -->|no| D[Right]
    C --> E[Join]
    D --> E
    E --> F([End])
    A -->|skip| E
    F -->|again| A";
    const DIRECTIONS: [&str; 4] = ["LR", "RL", "TD", "BT"];

    /// The crate's own layout of `source` at the app's label size, and the way it flows.
    fn crate_layout(source: &str) -> (Layout, Direction) {
        let theme = Theme {
            font_size: 14.0,
            ..Theme::modern()
        };
        let graph = parse_mermaid(source).unwrap().graph;
        let layout = compute_layout(&graph, &theme, &LayoutConfig::default());
        (layout, graph.direction)
    }

    /// The crate's layout of `source`, and the same layout with its edges straightened.
    fn before_and_after(source: &str) -> (Layout, Layout, Flow) {
        let (crate_layout, direction) = crate_layout(source);
        let mut layout = crate_layout.clone();
        straighten_edges(&mut layout, direction);
        (crate_layout, layout, Flow::of(direction))
    }

    fn straightened(source: &str) -> (Layout, Flow) {
        let (_, layout, flow) = before_and_after(source);
        (layout, flow)
    }

    fn is_close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    fn label_rect(edge: &EdgeLayout, flow: Flow) -> Rect {
        let size = label_size(edge.label.as_ref().unwrap(), flow);
        Rect::around(flow.point(edge.label_anchor.unwrap()), size)
    }

    /// The edges whose points are no longer the crate's. Cutting the page to fit moves every
    /// node and edge by the same amount, which does not count.
    fn rerouted(before: &Layout, after: &Layout) -> Vec<usize> {
        let (id, node) = before.nodes.iter().next().unwrap();
        let moved = (after.nodes[id].x - node.x, after.nodes[id].y - node.y);
        let edges = before.edges.iter().zip(&after.edges).enumerate();
        edges
            .filter(|(_, (before, after))| {
                let is_same = before.points.len() == after.points.len()
                    && before
                        .points
                        .iter()
                        .zip(&after.points)
                        .all(|(before, after)| {
                            is_close(before.0 + moved.0, after.0)
                                && is_close(before.1 + moved.1, after.1)
                        });
                !is_same
            })
            .map(|(index, _)| index)
            .collect()
    }

    /// Fails unless `edge` leaves the side of its source that faces along the flow and enters
    /// the facing side of its target, squarely and away from the corners, runs forwards all the
    /// way with at most two bends, and passes through no other node.
    fn assert_direct(layout: &Layout, flow: Flow, edge: &EdgeLayout) {
        let name = format!("{} to {}", edge.from, edge.to);
        let route = flow.route(&edge.points);
        let (start, end) = (route[0], route[route.len() - 1]);
        let source = End::of(layout, &edge.from, flow).unwrap();
        let target = End::of(layout, &edge.to, flow).unwrap();
        assert!(
            is_close(start.main, source.exit(start.cross).main),
            "{name}"
        );
        assert!(is_close(end.main, target.entry(end.cross).main), "{name}");
        let is_on = |ports: Span, port: f32| ports.start < port && port < ports.end;
        assert!(
            is_on(source.ports, start.cross),
            "{name} leaves by a corner"
        );
        assert!(is_on(target.ports, end.cross), "{name} enters by a corner");
        assert!(is_close(route[1].cross, start.cross), "{name} leaves askew");
        assert!(
            is_close(route[route.len() - 2].cross, end.cross),
            "{name} enters askew"
        );
        let runs_forwards = route.windows(2).all(|line| line[0].main < line[1].main);
        assert!(runs_forwards, "{name} turns back: {route:?}");
        assert!(bend_count(&route) <= 2, "{name}: {route:?}");
        let drawn = layout.nodes.values().filter(|node| !node.hidden);
        for node in drawn.filter(|node| node.id != edge.from && node.id != edge.to) {
            let is_crossed = flow.node(node).is_crossed_by_route(&route);
            assert!(!is_crossed, "{name} crosses {}", node.id);
        }
    }

    #[test]
    fn the_reported_flowchart_is_routed_directly_whichever_way_it_runs() {
        for direction in DIRECTIONS {
            let source = REPORTED.replace("LR", direction);
            let (before, after, flow) = before_and_after(&source);
            assert_eq!(rerouted(&before, &after), [0, 1, 2, 3], "{direction}");
            for edge in &after.edges {
                assert_direct(&after, flow, edge);
            }
        }
    }

    #[test]
    fn labels_sit_on_or_beside_their_own_edges_and_clear_of_each_other() {
        for direction in DIRECTIONS {
            let (layout, flow) = straightened(&REPORTED.replace("LR", direction));
            let labelled: Vec<&EdgeLayout> = layout
                .edges
                .iter()
                .filter(|edge| edge.label.is_some())
                .collect();
            assert_eq!(labelled.len(), 3);
            for (position, edge) in labelled.iter().enumerate() {
                let label = label_rect(edge, flow);
                let reach = label.grown(BESIDE_LINE + 0.5);
                let route = flow.route(&edge.points);
                assert!(reach.is_crossed_by_route(&route), "{direction} {label:?}");
                for other in &labelled[..position] {
                    let other = label_rect(other, flow);
                    assert!(!label.overlaps(other), "{direction} {label:?} {other:?}");
                }
            }
        }
    }

    #[test]
    fn ports_on_a_diamond_sit_on_its_slanted_edges() {
        let (layout, _) = straightened(REPORTED);
        let diamond = &layout.nodes["B"];
        let centre = (
            diamond.x + diamond.width / 2.0,
            diamond.y + diamond.height / 2.0,
        );
        let leaving: Vec<(f32, f32)> = layout
            .edges
            .iter()
            .filter(|edge| edge.from == "B")
            .map(|edge| edge.points[0])
            .collect();
        for (x, y) in &leaving {
            let across = (x - centre.0).abs() / (diamond.width / 2.0);
            let down = (y - centre.1).abs() / (diamond.height / 2.0);
            assert!(
                is_close(across + down, 1.0),
                "({x}, {y}) is off the outline"
            );
        }
        let tip = diamond.x + diamond.width;
        let on_a_slant = leaving.iter().filter(|(x, _)| *x < tip - 1.0);
        assert!(on_a_slant.count() >= 2, "{leaving:?}");
    }

    #[test]
    fn edges_between_the_same_two_nodes_stay_apart_each_with_its_label() {
        let (layout, flow) = straightened(PARALLEL);
        let routes: Vec<Vec<Point>> = layout
            .edges
            .iter()
            .map(|edge| flow.route(&edge.points))
            .collect();
        for (position, route) in routes.iter().enumerate() {
            assert_eq!(route.len(), 2, "side by side, so each is one straight line");
            for other in &routes[..position] {
                assert!((route[0].cross - other[0].cross).abs() >= PORT_SPACING);
                assert!((route[1].cross - other[1].cross).abs() >= PORT_SPACING);
            }
        }
        let labels: Vec<Rect> = layout
            .edges
            .iter()
            .map(|edge| label_rect(edge, flow))
            .collect();
        for (position, label) in labels.iter().enumerate() {
            for other in &labels[..position] {
                assert!(!label.overlaps(*other), "{label:?} {other:?}");
            }
        }
        // The labels are stacked in the order of their lines.
        assert!(labels[0].cross.centre() < labels[1].cross.centre());
        assert!(labels[1].cross.centre() < labels[2].cross.centre());
    }

    #[test]
    fn an_edge_that_cannot_run_directly_keeps_the_crates_route() {
        // The edge that skips a rank has the middle node in its way.
        let (before, after, _) = before_and_after(SKIPPED_RANK);
        assert!(!rerouted(&before, &after).contains(&2));
        // The edge back to the start runs against the flow.
        let (before, after, _) = before_and_after(LOOP_BACK);
        assert!(!rerouted(&before, &after).contains(&2));
        // A parallelogram's slanted sides are not worked out.
        let slanted = "flowchart LR\n    A[/Input/] --> B[/Output/]";
        let (before, after, _) = before_and_after(slanted);
        assert!(rerouted(&before, &after).is_empty());
    }

    #[test]
    fn every_edge_is_either_direct_or_just_as_the_crate_routed_it() {
        let flowcharts = [
            REPORTED,
            LOOP_BACK,
            SKIPPED_RANK,
            FAN_OUT_AND_IN,
            PARALLEL,
            SUBGRAPH,
            BRANCH_AND_JOIN,
        ];
        for flowchart in flowcharts {
            for direction in DIRECTIONS {
                let source = flowchart.replace("LR", direction);
                let (before, after, flow) = before_and_after(&source);
                for index in rerouted(&before, &after) {
                    let edge = &after.edges[index];
                    assert_direct(&after, flow, edge);
                    let bends = bend_count(&flow.route(&edge.points));
                    let crate_bends = bend_count(&flow.route(&before.edges[index].points));
                    assert!(bends <= crate_bends, "{source}: edge {index}");
                }
            }
        }
    }

    #[test]
    fn a_direct_route_does_not_strike_the_label_of_an_edge_that_kept_its_route() {
        let source = format!("{REPORTED}\n    D -->|reopened| A");
        let (mut before, direction) = crate_layout(&source);
        let mut after = before.clone();
        straighten_edges(&mut after, direction);
        assert_eq!(rerouted(&before, &after), [0, 1, 2, 3]);

        // The same again, with the label of the edge back to the start put between B and C.
        let (from, to) = (&before.nodes["B"], &before.nodes["C"]);
        let in_the_way = ((from.x + from.width + to.x) / 2.0, to.y + to.height / 2.0);
        before.edges[4].label_anchor = Some(in_the_way);
        let mut after = before.clone();
        straighten_edges(&mut after, direction);
        let rerouted = rerouted(&before, &after);
        assert!(
            !rerouted.contains(&1) && !rerouted.contains(&2),
            "{rerouted:?}"
        );
    }

    #[test]
    fn a_chain_is_drawn_with_straight_lines_on_a_page_of_the_same_size() {
        for direction in DIRECTIONS {
            let source =
                format!("flowchart {direction}\n    A[Write] --> B[Review] --> C[Publish]");
            let (before, after, flow) = before_and_after(&source);
            for edge in &after.edges {
                let route = flow.route(&edge.points);
                assert_eq!(route.len(), 2);
                assert!(is_close(route[0].cross, route[1].cross));
            }
            assert!(is_close(after.width, before.width));
            assert!(is_close(after.height, before.height));
        }
    }

    #[test]
    fn a_label_too_long_for_its_edge_goes_beside_the_line() {
        let (layout, flow) = straightened(LOOP_BACK);
        let edge = &layout.edges[1];
        let (label, route) = (label_rect(edge, flow), flow.route(&edge.points));
        assert!(!label.is_crossed_by_route(&route), "the whole line shows");
        assert!(label.grown(BESIDE_LINE + 0.5).is_crossed_by_route(&route));
    }

    #[test]
    fn an_edge_to_a_subgraph_ends_on_its_frame() {
        let source = "flowchart TD
    A[Start] --> G
    subgraph G[Group]
        X[One] --> Y[Two]
    end";
        let (layout, _) = straightened(source);
        let edge = &layout.edges[0];
        assert_eq!((edge.from.as_str(), edge.to.as_str()), ("A", "G"));
        assert_eq!(edge.points.len(), 2);
        let start = &layout.nodes["A"];
        assert!(is_close(edge.points[0].1, start.y + start.height));
        assert!(is_close(edge.points[1].1, layout.subgraphs[0].y));
    }

    #[test]
    fn the_page_is_cut_to_fit_once_the_long_routes_are_gone() {
        let source = REPORTED.replace("LR", "TD");
        let (before, after, _) = before_and_after(&source);
        assert!(after.width < before.width - 20.0);
        assert!(after.height < before.height - 20.0);
        let leftmost = after.nodes.values().map(|node| node.x);
        assert!(is_close(leftmost.fold(f32::INFINITY, f32::min), 8.0));
        let points = after.edges.iter().flat_map(|edge| &edge.points);
        for (x, y) in points {
            assert!((0.0..after.width).contains(x) && (0.0..after.height).contains(y));
        }
    }

    #[test]
    fn flow_space_runs_the_same_way_for_every_direction() {
        let directions = [
            (Direction::LeftRight, (10.0, 0.0)),
            (Direction::RightLeft, (-10.0, 0.0)),
            (Direction::TopDown, (0.0, 10.0)),
            (Direction::BottomTop, (0.0, -10.0)),
        ];
        for (direction, downstream) in directions {
            let flow = Flow::of(direction);
            assert!(flow.point(downstream).main > flow.point((0.0, 0.0)).main);
            assert_eq!(flow.page(flow.point((3.0, 4.0))), (3.0, 4.0));
            let rect = flow.rect((1.0, 2.0), (30.0, 40.0));
            let along = if flow.is_horizontal { 30.0 } else { 40.0 };
            assert_eq!(
                (rect.main.length(), rect.cross.length()),
                (along, 70.0 - along)
            );
        }
    }

    #[test]
    fn a_rect_is_crossed_by_a_line_through_it_and_not_by_one_past_it() {
        let rect = Rect {
            main: Span::between(10.0, 20.0),
            cross: Span::between(10.0, 20.0),
        };
        let point = |main, cross| Point { main, cross };
        assert!(rect.is_crossed_by(point(0.0, 15.0), point(30.0, 15.0)));
        assert!(rect.is_crossed_by(point(0.0, 0.0), point(30.0, 30.0)));
        assert!(
            rect.is_crossed_by(point(12.0, 12.0), point(14.0, 18.0)),
            "inside"
        );
        assert!(!rect.is_crossed_by(point(0.0, 5.0), point(30.0, 5.0)));
        assert!(
            !rect.is_crossed_by(point(0.0, 15.0), point(5.0, 15.0)),
            "stops short"
        );
        assert!(!rect.is_crossed_by(point(0.0, 25.0), point(25.0, 50.0)));
        assert!(!rect.is_crossed_by(point(25.0, 0.0), point(25.0, 30.0)));
    }

    #[test]
    fn bends_are_counted_where_a_route_turns() {
        let route = |points: &[(f32, f32)]| -> Vec<Point> {
            let points = points.iter();
            points.map(|&(main, cross)| Point { main, cross }).collect()
        };
        assert_eq!(bend_count(&route(&[(0.0, 0.0), (5.0, 0.0), (9.0, 0.0)])), 0);
        assert_eq!(bend_count(&route(&[(0.0, 0.0), (5.0, 0.0), (5.0, 5.0)])), 1);
        assert_eq!(bend_count(&route(&[(0.0, 0.0), (5.0, 0.0), (2.0, 0.0)])), 1);
        let crooked = route(&[(0.0, 0.0), (5.0, 0.0), (9.0, 4.0), (14.0, 4.0)]);
        assert_eq!(bend_count(&crooked), 2);
        let repeated = route(&[(0.0, 0.0), (5.0, 0.0), (5.0, 0.0), (9.0, 0.1)]);
        assert_eq!(bend_count(&repeated), 0, "a wobble is not a bend");
    }

    #[test]
    fn ports_share_a_side_evenly() {
        let side = Span::between(10.0, 70.0);
        let ports: Vec<f32> = (0..3).map(|port| evenly_spaced(side, port, 3)).collect();
        assert_eq!(ports, [20.0, 40.0, 60.0]);
        assert_eq!(evenly_spaced(side, 0, 1), 40.0);
    }

    #[test]
    fn overlapping_labels_are_moved_apart_the_short_way() {
        let label = |main: f32, cross: f32| Rect {
            main: Span::between(main, main + 60.0),
            cross: Span::between(cross, cross + 20.0),
        };
        let mut labels = [label(0.0, 0.0), label(10.0, 15.0), label(100.0, 0.0)];
        spread_labels(&mut labels);
        assert!(!labels[0].overlaps(labels[1]));
        assert_eq!(
            labels[0].main,
            label(0.0, 0.0).main,
            "apart across, not along"
        );
        assert!(labels[0].cross.centre() < labels[1].cross.centre());
        assert_eq!(labels[2], label(100.0, 0.0), "it overlapped nothing");
    }
}
