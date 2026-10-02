// SPDX-License-Identifier: AGPL-3.0-or-later

//! The orders a graph is walked in, and the loops they have to break.
//!
//! **Components, then Kahn inside each.** A data loop is legal through a delayed port and an
//! action loop is legal outright, so neither order is a plain topological sort. Tarjan's walk
//! finds the strongly connected components, which are ordered producer first — a component
//! nothing outside feeds is ready, and ties go to the one holding the lowest id — and the
//! members of a component are ordered by Kahn over its own edges with its delayed ones left
//! out, since a delayed read inside a loop is last tick's whichever member goes first. So
//! everything downstream of a loop still follows it, and only a loop's own members read one
//! another a tick late. On a graph with no loop every component is one node and this is Kahn,
//! seeded in id order. See docs/architecture.md, *Delayed ports and feedback*.

use super::{Flow, Graph, NodeId};
use std::collections::{HashMap, HashSet, VecDeque};

impl Graph {
    /// Nodes in dependency order: every producer before its consumers. Data edges only.
    ///
    /// Cached; any structural change clears it. A loop through a delayed port is placed as
    /// one block after everything feeding it and before everything it feeds; inside it a
    /// member follows what feeds it this frame, and otherwise goes in id order.
    pub fn topological_order(&self) -> &[NodeId] {
        self.topo.get_or_init(|| self.order(Flow::is_data).into())
    }

    /// Nodes in the order `tick` runs them: `topological_order`, and **action edges too**.
    ///
    /// An event lives for one frame and is delivered inside the tick that fired it, so a node
    /// receiving one must run after the node firing it or the whole event half gains a frame
    /// of latency per hop. Action edges are not cycle-checked — a divider feeding a clock's
    /// reset is a legal graph — so a loop among them is broken at its lowest id, and the node
    /// the break lands on sees its input one frame late. That is what a loop means.
    ///
    /// Separate from `topological_order` because nothing else may see an action edge: the
    /// compiler does not walk them and `downstream_outputs` must not mark a recompile for one.
    pub fn tick_order(&self) -> &[NodeId] {
        self.tick_topo.get_or_init(|| self.order(|_| true).into())
    }

    /// Nodes in dependency order over immediate data edges alone, which the cycle check keeps
    /// acyclic: the order a dual output's type is worked out in, since a delayed port is
    /// never dual and so never carries a type that is still being decided.
    pub(super) fn immediate_order(&self) -> Vec<NodeId> {
        self.order(Flow::is_immediate)
    }

    /// Every node of `within` in a cycle of data edges among `within`, delayed ones included:
    /// a node whose value at one tick depends on its own at an earlier one. A component of
    /// one node counts where the node feeds itself.
    pub fn in_cycles(&self, within: &HashSet<NodeId>) -> HashSet<NodeId> {
        let inside = |id: NodeId| within.contains(&id);
        self.components(Flow::is_data, inside)
            .into_iter()
            .filter(|c| {
                c.len() > 1
                    || self
                        .cables
                        .leaving(c[0])
                        .iter()
                        .any(|e| e.flow.is_data() && e.to.node == c[0])
            })
            .flatten()
            .collect()
    }

    /// The order over the edges `keep` passes: components producer first, and Kahn inside
    /// each. See the module's own doc.
    fn order(&self, keep: fn(Flow) -> bool) -> Vec<NodeId> {
        let components = self.components(keep, |_| true);
        let mut component_of: HashMap<NodeId, usize> = HashMap::new();
        for (i, members) in components.iter().enumerate() {
            for id in members {
                component_of.insert(*id, i);
            }
        }
        // Every kept edge out of a node into another component.
        let crossing = |id: NodeId| {
            let from = component_of[&id];
            self.cables
                .leaving(id)
                .iter()
                .filter(|c| keep(c.flow))
                .map(|c| component_of[&c.to.node])
                .filter(move |to| *to != from)
        };

        // A component waits on every kept edge into it from another, counted per edge as
        // Kahn over single nodes counts them.
        let mut waiting: Vec<usize> = vec![0; components.len()];
        for id in self.nodes.keys().copied() {
            for to in crossing(id) {
                waiting[to] += 1;
            }
        }

        // Seeded in id order, which a BTreeMap gives for free: snapshot tests and the order a
        // tick submits in both depend on the result being deterministic.
        let mut queue: VecDeque<usize> = VecDeque::new();
        let mut queued = vec![false; components.len()];
        for id in self.nodes.keys() {
            let c = component_of[id];
            if waiting[c] == 0 && !queued[c] {
                queued[c] = true;
                queue.push_back(c);
            }
        }

        let mut order = Vec::with_capacity(self.nodes.len());
        while let Some(c) = queue.pop_front() {
            let start = order.len();
            self.order_within(&components[c], keep, &mut order);
            for id in &order[start..] {
                for next in crossing(*id) {
                    waiting[next] -= 1;
                    if waiting[next] == 0 {
                        queue.push_back(next);
                    }
                }
            }
        }
        debug_assert_eq!(order.len(), self.nodes.len(), "every component is placed");
        order
    }

    /// One component's members, appended to `order` by Kahn over the kept cables between
    /// them with the delayed ones left out, seeded in id order.
    ///
    /// **The fallback.** Where Kahn stalls, the lowest id that waits on no immediate data
    /// cable goes next, and the walk carries on from it. Over data edges it never stalls: what
    /// is left once the delayed edges are out is immediate, and an immediate loop is refused
    /// at connect time. In the tick's order an action loop is legal and lands here, and the
    /// break falls on an event rather than on a value read this frame, since the immediate
    /// cables alone never close a loop. A graph built by some path that skipped the check
    /// still gets every node, at the lowest id, in an order that depends on nothing but the
    /// graph.
    fn order_within(&self, members: &[NodeId], keep: fn(Flow) -> bool, order: &mut Vec<NodeId>) {
        if let [only] = members {
            order.push(*only);
            return;
        }
        let member: HashSet<NodeId> = members.iter().copied().collect();
        let inner = |c: &super::Cable| {
            keep(c.flow)
                && c.flow != Flow::Delayed
                && member.contains(&c.from.node)
                && member.contains(&c.to.node)
        };
        // Per member, the inner cables it waits on, and how many of those are immediate data.
        let mut waiting: HashMap<NodeId, (usize, usize)> = members
            .iter()
            .map(|id| {
                let cables = self.cables.entering(*id).iter().filter(|c| inner(c));
                let (all, immediate) = cables.fold((0, 0), |(a, i), c| {
                    (a + 1, i + usize::from(c.flow.is_immediate()))
                });
                (*id, (all, immediate))
            })
            .collect();
        let mut placed: HashSet<NodeId> = HashSet::new();
        let mut queue: VecDeque<NodeId> = members
            .iter()
            .copied()
            .filter(|id| waiting[id].0 == 0)
            .collect();
        while placed.len() < members.len() {
            let unplaced = || members.iter().filter(|id| !placed.contains(id));
            let next = queue
                .pop_front()
                .or_else(|| unplaced().find(|id| waiting[id].1 == 0).copied())
                .or_else(|| unplaced().next().copied())
                .expect("fewer placed than there are members");
            placed.insert(next);
            order.push(next);
            for c in self.cables.leaving(next).iter().filter(|c| inner(c)) {
                let d = waiting.get_mut(&c.to.node).expect("a member");
                d.0 -= 1;
                d.1 -= usize::from(c.flow.is_immediate());
                if d.0 == 0 && !placed.contains(&c.to.node) {
                    queue.push_back(c.to.node);
                }
            }
        }
    }

    /// The strongly connected components of the kept edges among the nodes `inside` passes,
    /// each one's members in id order. Tarjan's, walked without recursion so a long chain
    /// cannot overflow the stack.
    fn components(
        &self,
        keep: fn(Flow) -> bool,
        inside: impl Fn(NodeId) -> bool,
    ) -> Vec<Vec<NodeId>> {
        // The cable at `at` out of `v`: `None` past the last, and `Some(None)` for one not
        // kept.
        let successor = |v: NodeId, at: usize| {
            self.cables
                .leaving(v)
                .get(at)
                .map(|c| (keep(c.flow) && inside(c.to.node)).then_some(c.to.node))
        };
        let mut index: HashMap<NodeId, usize> = HashMap::new();
        let mut low: HashMap<NodeId, usize> = HashMap::new();
        let mut stack: Vec<NodeId> = Vec::new();
        let mut on_stack: HashSet<NodeId> = HashSet::new();
        // The walk's own stack: a node, and the position of the next cable out of it to look
        // at.
        let mut calls: Vec<(NodeId, usize)> = Vec::new();
        let mut components = Vec::new();
        for root in self.nodes.keys().copied() {
            if !inside(root) || index.contains_key(&root) {
                continue;
            }
            calls.push((root, 0));
            while let Some(&(v, at)) = calls.last() {
                if at == 0 && !index.contains_key(&v) {
                    let n = index.len();
                    index.insert(v, n);
                    low.insert(v, n);
                    stack.push(v);
                    on_stack.insert(v);
                }
                if let Some(next) = successor(v, at) {
                    if let Some(top) = calls.last_mut() {
                        top.1 += 1;
                    }
                    let Some(w) = next else { continue };
                    match index.get(&w) {
                        None => calls.push((w, 0)),
                        Some(&seen) if on_stack.contains(&w) => {
                            let l = low.entry(v).or_default();
                            *l = (*l).min(seen);
                        }
                        Some(_) => {}
                    }
                    continue;
                }
                calls.pop();
                let lv = low[&v];
                if let Some(&(parent, _)) = calls.last() {
                    let l = low.entry(parent).or_default();
                    *l = (*l).min(lv);
                }
                if lv == index[&v] {
                    let mut component = Vec::new();
                    while let Some(w) = stack.pop() {
                        on_stack.remove(&w);
                        component.push(w);
                        if w == v {
                            break;
                        }
                    }
                    component.sort_unstable();
                    components.push(component);
                }
            }
        }
        components
    }
}
