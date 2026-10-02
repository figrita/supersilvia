# Proposal: the frame where every node was culled

**Status: found, not agreed.** Seen once while the app was driven through the egui MCP: after three nodes
were added through the Nodes menu and two of them moved, one frame drew every cable and
not one node — the nodes were culled as off-screen while their cables, computed from the
same positions, were drawn. It reproduced only with that ordering and did not recur.

## Where to look

Culling and cables read the view transform and each node's rect in the same frame, so
for one to be wrong and the other right, one of them read a stale value. Two candidates,
both in `ui/canvas.rs` and `ui/mod.rs`:

- The viewport rect a fresh `Area` reports on its sizing pass — the trap `CONTRIBUTING.md`
  documents for kittest — read by the cull and not by the cables.
- The clamp that keeps a node's y inside the viewport, applied after a move to a rect the
  cull had already tested.

## The shape of the fix

Reproduce it in kittest first: the exact ordering, stepping one frame at a time, asserting
after every step that the number of node bodies in the accessibility tree equals the
number of nodes whose rect intersects the viewport. The failing step names the frame; the
fix is whichever read moves to the value the cables already use. Done when the kittest
passes and stays in `tests/ui.rs`.
