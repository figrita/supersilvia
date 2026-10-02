# Proposal: the port tooltip over the conversion menu

**Status: found, not agreed.** When a cable is released on a convertible port, the
conversion menu opens beside the port, and the port's own hover text draws over the
menu's first two rows until the pointer moves toward a row. Cosmetic, seen every time.

## The fix

`ui/mod.rs` draws port hover text from the port's response. While `state.bridging` is
set, the pointer is by definition on the port that opened the menu, and the text says
what the menu already shows. Suppress the port tooltip while a bridge menu is open — one
condition at the draw site — and, for the same reason, while the node browser is open at
a port. A kittest opens the menu and asserts no tooltip node is in the accessibility tree.
