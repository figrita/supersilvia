// SPDX-License-Identifier: AGPL-3.0-or-later

//! The whole node registry as one JSON document, for comparing against silvia's.
//!
//! ```sh
//! cargo run --release --example registry > registry.json
//! ```
//!
//! One object per slug: the label, the icon, the category, the tooltip, every input and
//! output with its port type and its control's default, range, step and unit, every option
//! with its kind and choices, the values, the regions and what the CPU half declares. It is
//! the same data `scripts/node-shots/shots.sh` photographs, in the form a diff can read, and
//! `proposals/affordances/registry.json` is silvia's answer to the same question.

use serde_json::{Map, Value, json};
use supersilvia::graph::PortType;
use supersilvia::nodes::{Control, OutputKind, REGISTRY};

fn port_ty(ty: PortType) -> &'static str {
    match ty {
        PortType::VaryingNumber => "varying number",
        PortType::VaryingColor => "varying color",
        PortType::UniformNumber => "uniform number",
        PortType::UniformColor => "uniform color",
        PortType::Action => "action",
    }
}

fn control(c: &Control) -> Value {
    match c {
        Control::None => json!({ "kind": "none" }),
        Control::Number {
            default,
            min,
            max,
            step,
            unit,
            log,
            capped_by,
        } => json!({
            "kind": "number", "default": default, "min": min, "max": max,
            "step": step, "unit": unit, "log": log, "capped_by": capped_by,
        }),
        Control::Color { default } => json!({ "kind": "color", "default": default }),
        Control::Press => json!({ "kind": "press" }),
    }
}

fn main() {
    let mut all = Map::new();
    for d in REGISTRY {
        let inputs: Vec<Value> = d
            .inputs
            .iter()
            .chain(d.hidden.iter())
            .map(|i| {
                json!({
                    "key": i.key, "label": i.label, "type": port_ty(i.ty),
                    "control": control(&i.control),
                    "hidden": d.hidden.iter().any(|h| std::ptr::eq(h, i)),
                })
            })
            .collect();
        let outputs: Vec<Value> = d
            .outputs
            .iter()
            .map(|o| {
                json!({
                    "key": o.key, "label": o.label, "type": port_ty(o.ty),
                    "kind": match o.kind {
                        OutputKind::Shader => "shader",
                        OutputKind::Texture => "texture",
                        OutputKind::Uniform => "uniform",
                        OutputKind::Action => "action",
                    },
                    "range": o.range, "delayed": o.delayed, "dual": o.eval.is_some(),
                })
            })
            .collect();
        let options: Vec<Value> = d
            .options
            .iter()
            .map(|o| {
                json!({
                    "key": o.key, "label": o.label, "default": o.default,
                    "choices": o.choices.iter().map(|(v, l)| json!([v, l])).collect::<Vec<_>>(),
                    "kind": format!("{:?}", o.kind).to_lowercase(),
                    "checkbox": o.checkbox, "heading": o.heading,
                    "overridden_by": o.overridden_by, "placeholder": o.placeholder,
                })
            })
            .collect();
        let regions: Vec<Value> = d
            .regions
            .iter()
            .map(|r| {
                json!({
                    "heading": r.heading().map(|h| json!({ "key": h.key, "label": h.label, "open": h.open })),
                    "claims_pointer": supersilvia::widgets::def(*r).claims_pointer,
                    "width": supersilvia::widgets::def(*r).width,
                })
            })
            .collect();
        all.insert(
            d.slug.to_string(),
            json!({
                "label": d.label, "icon": d.icon, "tooltip": d.tooltip,
                "category": d.category.label(), "width": d.width, "is_output": d.is_output,
                "inputs": inputs, "outputs": outputs, "options": options,
                "values": d.values.iter().map(|v| json!({ "key": v.key, "label": v.label, "rows": v.rows() })).collect::<Vec<_>>(),
                "regions": regions,
                "cpu": d.cpu.as_ref().map(|c| json!({ "integrates": c.integrates, "live": c.live })),
                "measure": d.measure_wgsl.is_some(),
            }),
        );
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&Value::Object(all)).unwrap()
    );
}
