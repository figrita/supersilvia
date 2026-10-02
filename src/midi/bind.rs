// SPDX-License-Identifier: AGPL-3.0-or-later

//! The map: which message drives which control.
//!
//! **A binding is addressed by channel *and* number.** silvia keys its own map by the CC
//! number alone and throws the channel away, which is sixteen times fewer addresses and two
//! controllers colliding the moment both send CC 7. Channel is free to carry and it is what
//! every desk assumes.
//!
//! **A binding does not name a device.** Which box is on the table is the rig, and the rig is
//! written nowhere — so a patch opens the same whichever controller is plugged in, and every
//! source the sequencer offers is wired in at once. The cost is that two controllers sending
//! the same channel and CC drive the same control, which is what channel is for.
//!
//! The target is a [`Target`]: almost always a [`PortRef`], since a number control is keyed by
//! its node and key — a port's, or a hidden one a region draws, which has the same address
//! and no port — and an action input *is* a port, so one address covers all three. What is
//! not on a node is the Main Mixer's three show controls — the fade, Blackout and Freeze, the
//! rig's rather than a node's and the things a VJ most wants under a hand — so the target is
//! an enum with an arm for each rather than a second map kept in step with the first.

use crate::graph::PortRef;
use std::collections::BTreeMap;

/// What a binding drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    /// A number control or an action input on a node, a port's or one a region draws.
    Port(PortRef),
    /// The Main Mixer's A / B balance, −1 to +1. The rig's rather than the patch's, and the
    /// one control there that answers to a knob: which deck is on air is a button on the
    /// Output, and the method and resolution are nothing a hand turns mid-set.
    Balance,
    /// The Main Mixer's Blackout: the mix black until it is pressed again. A note toggles it;
    /// a CC holds it on at 64 and above.
    Blackout,
    /// The Main Mixer's Freeze: the mix's last frame until it is pressed again. Read as
    /// Blackout is.
    Freeze,
}

impl Target {
    /// The port, where this is one.
    pub fn port(self) -> Option<PortRef> {
        match self {
            Self::Port(p) => Some(p),
            Self::Balance | Self::Blackout | Self::Freeze => None,
        }
    }

    /// Whether this is one of the mixer's two presses, which a message switches rather than
    /// moves.
    pub fn is_switch(self) -> bool {
        matches!(self, Self::Blackout | Self::Freeze)
    }

    /// The mixer control's name, as the file writes it. `None` for a node's control.
    pub fn mixer_name(self) -> Option<&'static str> {
        match self {
            Self::Port(_) => None,
            Self::Balance => Some(BALANCE),
            Self::Blackout => Some(BLACKOUT),
            Self::Freeze => Some(FREEZE),
        }
    }

    /// Whether `graph` has this control and MIDI may drive it: the node, and a bindable input
    /// under the key — a port, or a number the node draws in a region of its own
    /// (`NodeDef::bindable`). The fade is always there.
    pub fn is_in(self, graph: &crate::graph::Graph) -> bool {
        match self {
            Self::Port(p) => graph
                .get(p.node)
                .is_some_and(|n| n.def.bindable(p.key).is_some()),
            Self::Balance | Self::Blackout | Self::Freeze => true,
        }
    }
}

impl From<PortRef> for Target {
    fn from(port: PortRef) -> Self {
        Self::Port(port)
    }
}

/// What a message is addressed by, which is what the map is keyed by.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Trigger {
    Control { channel: u8, cc: u8 },
    Note { channel: u8, note: u8 },
}

impl Trigger {
    /// How a person reads it: `CC 74 ch 1`, counting channels from one as every desk does.
    pub fn label(self) -> String {
        match self {
            Self::Control { channel, cc } => format!("CC {cc} ch {}", channel + 1),
            Self::Note { channel, note } => {
                format!("Note {} ch {}", note_name(note), channel + 1)
            }
        }
    }
}

/// One binding: a message, and the control it drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    pub target: Target,
}

/// Every binding in the project, keyed by what triggers it.
///
/// Keyed by the trigger rather than by the target, because that is the direction a message
/// travels: one message arrives and the map is asked what it drives. A `BTreeMap` so the file
/// is deterministic and so the window's list has an order without sorting one.
///
/// **One trigger drives one control.** silvia lets a CC drive a set, which is a feature
/// nobody asked for here and a rule that makes the window's table honest: one row per
/// binding, and learning a control that is already bound moves it rather than doubling it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bindings(BTreeMap<Trigger, Binding>);

impl Bindings {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// What this message drives, if anything.
    pub fn get(&self, trigger: Trigger) -> Option<Binding> {
        self.0.get(&trigger).copied()
    }

    /// Bind a trigger to a control, dropping whatever either of them was bound to.
    ///
    /// Both directions, because both are surprises: a trigger that drove something else stops
    /// driving it, and a control that was already learned moves rather than answering to two
    /// knobs. A hand that learns the same control twice meant the second one.
    pub fn bind(&mut self, trigger: Trigger, target: impl Into<Target>) {
        let target = target.into();
        self.0.retain(|_, b| b.target != target);
        self.0.insert(trigger, Binding { target });
    }

    /// Bind without dropping what either end was bound to.
    ///
    /// What a **file's** rows do. The learn gesture moves a binding rather than doubling it,
    /// which is what makes the window's table one row per binding; a file is not a gesture,
    /// and dropping one of two rows somebody saved would silently change their map. So two
    /// triggers on one control is a shape that exists, and everything reading the map has to
    /// hold for it — see the note counting in [`crate::synth`].
    pub fn bind_unchecked(&mut self, trigger: Trigger, target: impl Into<Target>) {
        let target = target.into();
        self.0.insert(trigger, Binding { target });
    }

    /// Drop the binding on this trigger.
    pub fn unbind(&mut self, trigger: Trigger) {
        self.0.remove(&trigger);
    }

    /// Drop whatever drives this control.
    pub fn unbind_target(&mut self, target: impl Into<Target>) {
        let target = target.into();
        self.0.retain(|_, b| b.target != target);
    }

    /// What drives this control, for the dot a bound control wears.
    pub fn trigger_of(&self, target: impl Into<Target>) -> Option<Trigger> {
        let target = target.into();
        self.0
            .iter()
            .find(|(_, b)| b.target == target)
            .map(|(t, _)| *t)
    }

    /// Every binding, trigger first, in the file's own order.
    pub fn iter(&self) -> impl Iterator<Item = (Trigger, Binding)> + '_ {
        self.0.iter().map(|(t, b)| (*t, *b))
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// The map as the project file writes it: a port's key is a definition's
    /// `&'static str`, which no file can hold, so it goes out as a string and comes back
    /// through the node's own port list — the same trip a cable makes. A mixer control goes
    /// out by name — `{"mixer": "balance"}`, `"blackout"`, `"freeze"` — rather than an id,
    /// since it is nothing the graph holds.
    pub fn saved(&self) -> Vec<Saved> {
        self.0
            .iter()
            .map(|(trigger, b)| Saved {
                trigger: *trigger,
                target: match b.target {
                    Target::Port(p) => SavedTarget::Port(crate::workspace::SavedPort {
                        node: p.node,
                        key: p.key.to_string(),
                    }),
                    Target::Balance | Target::Blackout | Target::Freeze => SavedTarget::Mixer {
                        mixer: b.target.mixer_name().unwrap_or(BALANCE).to_string(),
                    },
                },
            })
            .collect()
    }

    /// A saved map, back against a graph. A row naming a control this node does not have, or
    /// one MIDI does not bind, is dropped rather than reported: it is a binding to a control
    /// that no longer exists or never answered to a knob, which is the same thing
    /// [`Self::live`] leaves out and is not worth a warning of its own. A mixer row naming a
    /// control this build has no name for goes the same way.
    pub fn restored(saved: &[Saved], graph: &crate::graph::Graph) -> Self {
        let mut map = Self::default();
        for row in saved {
            match &row.target {
                SavedTarget::Port(target) => {
                    let Some(node) = graph.get(target.node) else {
                        continue;
                    };
                    let Some(key) = node.def.bindable(&target.key) else {
                        continue;
                    };
                    map.bind_unchecked(row.trigger, PortRef::new(target.node, key));
                }
                SavedTarget::Mixer { mixer } => {
                    if let Some(target) = [Target::Balance, Target::Blackout, Target::Freeze]
                        .into_iter()
                        .find(|t| t.mixer_name() == Some(mixer.as_str()))
                    {
                        map.bind_unchecked(row.trigger, target);
                    }
                }
            }
        }
        map
    }

    /// Every binding whose control `graph` has, in the map's own order.
    ///
    /// A deleted node's bindings stay in the map, so undoing the delete brings them back;
    /// what reads the map for a performance or a file reads this instead, so a row naming a
    /// node nobody has drives nothing, lists nothing and is not written. A node id is never
    /// handed out twice, so a binding left behind cannot come to name another node.
    pub fn live<'a>(
        &'a self,
        graph: &'a crate::graph::Graph,
    ) -> impl Iterator<Item = (Trigger, Binding)> + 'a {
        self.iter().filter(|(_, b)| b.target.is_in(graph))
    }

    /// [`Self::live`], as a map of its own.
    #[must_use]
    pub fn live_in(&self, graph: &crate::graph::Graph) -> Self {
        Self(self.live(graph).collect())
    }
}

/// One binding as `project.ssp` holds it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Saved {
    pub trigger: Trigger,
    pub target: SavedTarget,
}

/// A binding's target as the file holds it: a port on a node, or a named control of the
/// mixer. Untagged, so a port row reads exactly as it did before the mixer had an address.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum SavedTarget {
    Port(crate::workspace::SavedPort),
    Mixer { mixer: String },
}

/// The mixer's controls' names in the file.
const BALANCE: &str = "balance";
const BLACKOUT: &str = "blackout";
const FREEZE: &str = "freeze";

/// A note number as a desk writes it: `C4` is 60, which is the convention every controller in
/// this room prints on its own keys.
fn note_name(note: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let octave = i16::from(note) / 12 - 1;
    format!("{}{octave}", NAMES[usize::from(note % 12)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::NodeId;

    fn port(n: u32, key: &'static str) -> Target {
        Target::Port(PortRef::new(NodeId(n), key))
    }

    #[test]
    fn the_fade_is_an_address_and_a_port_row_reads_as_it_always_did() {
        let mut map = Bindings::default();
        map.bind(Trigger::Control { channel: 0, cc: 1 }, Target::Balance);
        map.bind(Trigger::Control { channel: 0, cc: 2 }, port(1, "frequency"));
        map.bind(
            Trigger::Note {
                channel: 0,
                note: 36,
            },
            Target::Blackout,
        );
        map.bind(
            Trigger::Note {
                channel: 0,
                note: 37,
            },
            Target::Freeze,
        );
        let json = serde_json::to_string(&map.saved()).expect("serializes");
        assert!(json.contains(r#"{"mixer":"balance"}"#), "{json}");
        assert!(json.contains(r#"{"mixer":"blackout"}"#), "{json}");
        assert!(json.contains(r#"{"mixer":"freeze"}"#), "{json}");
        assert!(json.contains(r#"{"node":1,"key":"frequency"}"#), "{json}");

        let rows: Vec<Saved> = serde_json::from_str(&json).expect("parses");
        let graph = crate::graph::Graph::new();
        let back = Bindings::restored(&rows, &graph);
        assert_eq!(
            back.trigger_of(Target::Balance),
            Some(Trigger::Control { channel: 0, cc: 1 }),
            "the fade needs no node to come back"
        );
        assert_eq!(
            back.trigger_of(Target::Freeze),
            Some(Trigger::Note {
                channel: 0,
                note: 37
            })
        );
        assert_eq!(
            back.len(),
            3,
            "the port row named a node the graph does not have"
        );

        let kept = map.live_in(&graph);
        assert!(kept.trigger_of(Target::Balance).is_some());
        assert!(kept.trigger_of(port(1, "frequency")).is_none());
    }

    #[test]
    fn a_trigger_carries_its_channel() {
        let a = Trigger::Control { channel: 0, cc: 7 };
        let b = Trigger::Control { channel: 1, cc: 7 };
        assert_ne!(a, b, "the same CC on two channels is two addresses");

        let mut map = Bindings::default();
        map.bind(a, port(1, "frequency"));
        map.bind(b, port(2, "amplitude"));
        assert_eq!(map.len(), 2);
        assert_eq!(map.get(a).map(|x| x.target), Some(port(1, "frequency")));
        assert_eq!(map.get(b).map(|x| x.target), Some(port(2, "amplitude")));
    }

    #[test]
    fn learning_a_control_twice_moves_it_rather_than_doubling_it() {
        let mut map = Bindings::default();
        let target = port(1, "frequency");
        map.bind(Trigger::Control { channel: 0, cc: 7 }, target);
        map.bind(Trigger::Control { channel: 0, cc: 8 }, target);

        assert_eq!(map.len(), 1, "one control answers to one knob");
        assert_eq!(
            map.trigger_of(target),
            Some(Trigger::Control { channel: 0, cc: 8 }),
            "the second one"
        );
    }

    #[test]
    fn learning_a_trigger_twice_moves_it_too() {
        let mut map = Bindings::default();
        let trigger = Trigger::Control { channel: 0, cc: 7 };
        map.bind(trigger, port(1, "frequency"));
        map.bind(trigger, port(2, "amplitude"));

        assert_eq!(map.len(), 1, "one knob drives one control");
        assert_eq!(
            map.get(trigger).map(|x| x.target),
            Some(port(2, "amplitude"))
        );
    }

    #[test]
    fn a_note_reads_as_a_desk_writes_it() {
        assert_eq!(
            Trigger::Note {
                channel: 0,
                note: 60
            }
            .label(),
            "Note C4 ch 1"
        );
        assert_eq!(
            Trigger::Control {
                channel: 15,
                cc: 74
            }
            .label(),
            "CC 74 ch 16"
        );
    }
}
