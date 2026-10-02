// SPDX-License-Identifier: AGPL-3.0-or-later

//! The WGSL gate: every module the compiler writes for wgpu reaches Metal and Vulkan.
//!
//! The corpus is every node in the registry built into an Output unconnected and with every
//! input driven, each connected build's cost probe, the workspace pass of each of those
//! graphs — its measurements and its thumbnails — and of one holding every node, every
//! choice of every option that changes the generated code, and a tap's pass with no
//! thumbnail. Each module goes through naga the way wgpu takes it: parse, validate with the capabilities wgpu gives a
//! device that asks for no optional features, then MSL out and SPIR-V out. Any error fails.
//!
//! Beside that, each module is held to what the renderer will assume of it without
//! reflection: its uniform struct sits at the offsets `compile::wgsl::uniform_layout` gives,
//! its bindings are the ones `compile::wgsl::bindings` lists, and it samples only with an
//! explicit level, since naga does not check WGSL's uniformity rule for implicit derivatives
//! and the drivers under it leave them undefined in divergent control flow.
//!
//! The renderer's own stages — the conversion pass, the resize's carry, the picture reads, the
//! mix and the viewer's blit, and the simulations' kernels — go through the same naga path
//! beside the corpus, held to the same rule on sampling and to their entry points.
//!
//! A node without WGSL still compiles, to a placeholder, and its module is held to all of the
//! above too. How many nodes have WGSL is printed, never asserted:
//! `cargo test --test shader_targets -- --nocapture`. `SHADER_TARGETS_DUMP=<dir>` writes each
//! module, its MSL, or its failure into that directory. `proposals/shader-path.md` is the
//! record of the decision this gate serves.

use emath::Pos2;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt::Write as _;
use supersilvia::compile::{Diagnostic, Shader, wgsl};
use supersilvia::graph::{Graph, NodeId, PortRef, PortType};
use supersilvia::nodes::{self, NodeDef};

// ------------------------------------------------------------------------------- the corpus

struct Entry {
    name: String,
    shader: Shader,
}

/// Every distinct module, by source.
fn corpus() -> Vec<Entry> {
    let mut entries = Vec::new();
    for def in nodes::REGISTRY {
        for wired in [false, true] {
            let Some((g, out)) = build_graph(def, wired, &[]) else {
                continue;
            };
            let wiring = if wired { "connected" } else { "unconnected" };
            if let Some(shader) = wgsl::build(&g, out) {
                entries.push(Entry {
                    name: format!("node:{}_{wiring}", def.slug),
                    shader,
                });
            }
            if wired && let Some(shader) = wgsl::build_probe(&g, out) {
                entries.push(Entry {
                    name: format!("probe:{}", def.slug),
                    shader,
                });
            }
            for shader in wgsl::build_pass(&g, g.default_workspace(), &measured(&g), true) {
                entries.push(Entry {
                    name: format!("pass:{}_{wiring}", def.slug),
                    shader,
                });
            }
        }
        for option in def.options.iter().filter(|o| o.rebuilds() && !o.is_asset()) {
            for (value, _) in option.choices {
                let Some((g, out)) = build_graph(def, false, &[(option.key, value)]) else {
                    continue;
                };
                if let Some(shader) = wgsl::build(&g, out) {
                    entries.push(Entry {
                        name: format!("option:{}.{}={value}", def.slug, option.key),
                        shader,
                    });
                }
            }
        }
    }
    entries.push(measured_alone());
    entries.extend(crowded());
    entries.push(everything());
    let mut seen = HashSet::new();
    entries.retain(|e| seen.insert(e.shader.body.clone()));
    entries
}

fn add(g: &mut Graph, slug: &str) -> NodeId {
    nodes::add_to_graph(g, slug, Pos2::ZERO).expect("slug is in the registry")
}

/// `def` into an Output, every output of it folded into one picture, its inputs driven when
/// `wired`, and `options` set on it. `None` for a node with no picture and no field.
///
/// The same construction `tests/compile.rs` snapshots, tolerant of a cable a pin refuses:
/// the build is still a module worth checking.
fn build_graph(
    def: &'static NodeDef,
    wired: bool,
    options: &[(&'static str, &str)],
) -> Option<(Graph, NodeId)> {
    let roots_of = |ty: PortType| -> Vec<&'static str> {
        def.outputs
            .iter()
            .filter(|p| p.ty == ty)
            .map(|p| p.key)
            .collect()
    };
    let (pictures, fields) = (
        roots_of(PortType::VaryingColor),
        roots_of(PortType::VaryingNumber),
    );
    if pictures.is_empty() && fields.is_empty() || def.is_output && !wired {
        return None;
    }
    let mut g = Graph::new();
    let under = add(&mut g, def.slug);
    for (key, value) in options {
        g.get_mut(under)
            .expect("just added")
            .options
            .insert(key, (*value).to_string());
    }
    let out = add(&mut g, "output");
    if wired {
        for port in def.inputs {
            let (source, from) = match port.ty {
                PortType::VaryingColor => ("checkerboard", "output"),
                PortType::VaryingNumber => ("vignette", "mask"),
                PortType::UniformColor => ("color", "output"),
                PortType::UniformNumber | PortType::Action => continue,
            };
            let src = add(&mut g, source);
            let _ = g.connect(PortRef::new(src, from), PortRef::new(under, port.key));
        }
    }
    let mut roots: Vec<PortRef> = pictures.iter().map(|k| PortRef::new(under, k)).collect();
    if !fields.is_empty() {
        let rgba = add(&mut g, "rgba");
        for (field, channel) in fields.iter().zip(["r", "g", "b", "a"]) {
            let _ = g.connect(PortRef::new(under, field), PortRef::new(rgba, channel));
        }
        roots.push(PortRef::new(rgba, "output"));
    }
    let mut root = roots[0];
    for next in &roots[1..] {
        let blend = add(&mut g, "mix");
        let _ = g.connect(root, PortRef::new(blend, "a"));
        let _ = g.connect(*next, PortRef::new(blend, "b"));
        root = PortRef::new(blend, "output");
    }
    g.connect(root, PortRef::new(out, "input")).ok()?;
    Some((g, out))
}

/// One workspace holding every node in the registry, unconnected: the largest pass there
/// is, a thumbnail for every varying output of every kind.
fn everything() -> Entry {
    let mut g = Graph::new();
    for def in nodes::REGISTRY {
        add(&mut g, def.slug);
    }
    let shader = wgsl::build_pass(&g, g.default_workspace(), &measured(&g), true)
        .pop()
        .expect("varying outputs");
    let textures = wgsl::bindings(&shader)
        .iter()
        .filter(|(_, r)| matches!(r, wgsl::Resource::Texture(_)))
        .count();
    println!(
        "shader_targets: the pass of every node measures {}, holds {} thumbnails and binds \
         {textures} textures",
        shader.taps.len(),
        shader.thumbs.len()
    );
    Entry {
        name: "pass:everything".to_string(),
        shader,
    }
}

/// Every node in `g` with a measurement, in id order: what its workspace's pass measures.
fn measured(g: &Graph) -> Vec<NodeId> {
    g.iter()
        .filter(|(_, n)| n.def.measure_wgsl.is_some())
        .map(|(id, _)| id)
        .collect()
}

/// A workspace binding more textures than one pass may, in its batches.
fn crowded() -> Vec<Entry> {
    let mut g = Graph::new();
    for _ in 0..20 {
        add(&mut g, "cellularautomata");
    }
    let tap = add(&mut g, "tap");
    let batches = wgsl::build_pass(&g, g.default_workspace(), &[tap], true);
    assert!(batches.len() > 1, "crowded enough to split");
    batches
        .into_iter()
        .enumerate()
        .map(|(i, shader)| Entry {
            name: format!("pass:crowded_{i}"),
            shader,
        })
        .collect()
}

/// A tap on a chain that reaches no Output, measured with no thumbnail beside it: a pass on a
/// closed workspace that a deck keeps awake.
fn measured_alone() -> Entry {
    let mut g = Graph::new();
    let noise = add(&mut g, "perlin");
    let tap = add(&mut g, "tap");
    g.connect(PortRef::new(noise, "color"), PortRef::new(tap, "input"))
        .expect("color into color");
    Entry {
        name: "pass:tap_alone".to_string(),
        shader: wgsl::build_pass(&g, g.default_workspace(), &[tap], false)
            .pop()
            .expect("a tap"),
    }
}

// --------------------------------------------------------------------------------- the gate

/// The capabilities `wgpu_naga_bridge::create_validator` gives a device that asks for no
/// optional feature, on an adapter with every downlevel flag, which Vulkan and Metal are:
/// the two that come from downlevel flags rather than features.
fn capabilities() -> naga::valid::Capabilities {
    naga::valid::Capabilities::CUBE_ARRAY_TEXTURES | naga::valid::Capabilities::MULTISAMPLED_SHADING
}

/// What a module may not contain: sampling with implicit derivatives, and derivatives.
const FORBIDDEN: &[&str] = &[
    "textureSample(",
    "textureSampleBias(",
    "textureSampleCompare(",
    "dpdx",
    "dpdy",
    "fwidth",
];

/// A local named `u` would shadow the module's uniform struct instance, so every
/// `u.u_…` read in the rest of the body would silently resolve to the wrong thing and naga
/// would not catch it. Checked as a line-anchored declaration or a typed parameter/field,
/// rather than a bare-word search, so a name like `blur` is left alone.
fn check_no_local_u(source: &str) -> Result<(), String> {
    for line in source.lines() {
        let t = line.trim_start();
        if t.starts_with("let u ")
            || t.starts_with("let u=")
            || t.starts_with("var u ")
            || t.starts_with("var u=")
        {
            return Err(format!(
                "declares a local `u`, shadowing the uniform struct: {t}"
            ));
        }
        if !line.contains("var<uniform>") && (line.contains("(u:") || line.contains(" u:")) {
            return Err(format!(
                "names a parameter or field `u`, shadowing the uniform struct: {t}"
            ));
        }
    }
    Ok(())
}

/// One module through the path wgpu takes it: no word [`FORBIDDEN`], parsed, validated with
/// [`capabilities`], then MSL out and SPIR-V out. The module, and the MSL it wrote.
fn translate(source: &str) -> Result<(naga::Module, String), String> {
    for word in FORBIDDEN {
        if source.contains(word) {
            return Err(format!("names {word}"));
        }
    }
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|e| format!("parse: {}", e.emit_to_string(source)))?;
    let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), capabilities())
        .validate(&module)
        .map_err(|e| format!("validate: {}", e.emit_to_string(source)))?;
    let msl_options = naga::back::msl::Options {
        lang_version: (2, 4),
        fake_missing_bindings: true,
        ..Default::default()
    };
    let (msl, _) = naga::back::msl::write_string(
        &module,
        &info,
        &msl_options,
        &naga::back::msl::PipelineOptions::default(),
    )
    .map_err(|e| format!("msl-out: {}", error_chain(&e)))?;
    naga::back::spv::write_vec(&module, &info, &naga::back::spv::Options::default(), None)
        .map_err(|e| format!("spv-out: {}", error_chain(&e)))?;
    Ok((module, msl))
}

/// Whether `module` has an entry point `name` of `stage`.
fn has_entry(module: &naga::Module, name: &str, stage: naga::ShaderStage) -> Result<(), String> {
    if module
        .entry_points
        .iter()
        .any(|e| e.name == name && e.stage == stage)
    {
        Ok(())
    } else {
        Err(format!("no {stage:?} entry point {name}"))
    }
}

/// Everything the gate checks of one module, and the MSL it wrote.
fn check(entry: &Entry) -> Result<String, String> {
    let source = &entry.shader.body;
    check_no_local_u(source)?;
    let (module, msl) = translate(source)?;
    check_layout(&module, &entry.shader)?;
    check_bindings(&module, &entry.shader)?;
    has_entry(&module, wgsl::VERTEX_ENTRY, naga::ShaderStage::Vertex)?;
    has_entry(&module, wgsl::FRAGMENT_ENTRY, naga::ShaderStage::Fragment)?;
    Ok(msl)
}

/// The uniform struct naga laid out is the one `uniform_layout` says.
fn check_layout(module: &naga::Module, shader: &Shader) -> Result<(), String> {
    let layout = wgsl::uniform_layout(shader);
    let (members, span) = module
        .types
        .iter()
        .find_map(|(_, ty)| match &ty.inner {
            naga::TypeInner::Struct { members, span } if ty.name.as_deref() == Some("Uniforms") => {
                Some((members, *span))
            }
            _ => None,
        })
        .ok_or("no Uniforms struct")?;
    let naga: Vec<(&str, u32)> = members
        .iter()
        .map(|m| (m.name.as_deref().unwrap_or(""), m.offset))
        .collect();
    let ours: Vec<(&str, u32)> = layout.fields.iter().map(|f| (&*f.name, f.offset)).collect();
    if naga != ours || span != layout.size {
        return Err(format!(
            "uniform layout: naga {naga:?} size {span}, uniform_layout {ours:?} size {}",
            layout.size
        ));
    }
    Ok(())
}

/// Every global with a binding is in group 0 at the binding `bindings` gives its resource.
fn check_bindings(module: &naga::Module, shader: &Shader) -> Result<(), String> {
    let expected: BTreeMap<u32, String> = wgsl::bindings(shader)
        .into_iter()
        .map(|(b, r)| {
            let name = match r {
                wgsl::Resource::Uniforms => "u".to_string(),
                wgsl::Resource::Taps => "tap".to_string(),
                wgsl::Resource::Sampler(s) => s.name().to_string(),
                wgsl::Resource::Texture(t) => t.to_string(),
            };
            (b, name)
        })
        .collect();
    let mut found = BTreeMap::new();
    for (_, var) in module.global_variables.iter() {
        if let Some(binding) = &var.binding {
            if binding.group != 0 {
                return Err(format!("{:?} is in group {}", var.name, binding.group));
            }
            found.insert(binding.binding, var.name.clone().unwrap_or_default());
        }
    }
    if found != expected {
        return Err(format!(
            "bindings: module {found:?}, bindings() {expected:?}"
        ));
    }
    Ok(())
}

fn error_chain(e: &dyn std::error::Error) -> String {
    let mut out = e.to_string();
    let mut source = e.source();
    while let Some(s) = source {
        write!(out, ": {s}").unwrap();
        source = s.source();
    }
    out
}

/// Where `SHADER_TARGETS_DUMP` asks for the corpus to be written, if it does.
fn dump_dir() -> Option<std::path::PathBuf> {
    let dir = std::path::PathBuf::from(std::env::var_os("SHADER_TARGETS_DUMP")?);
    std::fs::create_dir_all(&dir).expect("the dump directory");
    Some(dir)
}

fn dump(dir: &std::path::Path, entry: &Entry, suffix: &str, text: &str) {
    let name = entry.name.replace([':', '/', '='], "_");
    std::fs::write(dir.join(format!("{name}.{suffix}")), text).expect("a dump file");
}

/// The slugs a module has no WGSL for.
fn untranslated(shader: &Shader) -> BTreeSet<&'static str> {
    shader
        .diagnostics
        .iter()
        .filter_map(|d| match d {
            Diagnostic::Untranslated { slug, .. } => Some(*slug),
            _ => None,
        })
        .collect()
}

#[test]
fn every_wgsl_module_reaches_msl_and_spirv() {
    let entries = corpus();
    assert!(
        entries.iter().any(|e| e.shader.body.contains("atomicAdd"))
            && entries.iter().any(|e| e.shader.body.contains("texture_2d")),
        "the corpus holds a tap and a texture"
    );
    let dir = dump_dir();
    let mut failures = Vec::new();
    for entry in &entries {
        if let Some(dir) = &dir {
            dump(dir, entry, "wgsl", &entry.shader.body);
        }
        let other: Vec<_> = entry
            .shader
            .diagnostics
            .iter()
            .filter(|d| !matches!(d, Diagnostic::Untranslated { .. }))
            .collect();
        if !other.is_empty() {
            failures.push(format!("{}: diagnostics {other:?}", entry.name));
        }
        match check(entry) {
            Ok(msl) => {
                if let Some(dir) = &dir {
                    dump(dir, entry, "metal", &msl);
                }
            }
            Err(e) => {
                if let Some(dir) = &dir {
                    dump(dir, entry, "fail", &e);
                }
                failures.push(format!("{}: {e}", entry.name));
            }
        }
    }

    // A node has WGSL when neither of its own builds reaches a placeholder of its own.
    let mut written = Vec::new();
    let mut not_yet = Vec::new();
    for def in nodes::REGISTRY {
        let builds: Vec<Shader> = [false, true]
            .into_iter()
            .filter_map(|wired| build_graph(def, wired, &[]))
            .filter_map(|(g, out)| wgsl::build(&g, out))
            .collect();
        if builds.is_empty() {
            continue;
        }
        if builds.iter().any(|s| untranslated(s).contains(def.slug)) {
            not_yet.push(def.slug);
        } else {
            written.push(def.slug);
        }
    }
    println!(
        "\nshader_targets: {} modules checked\nWGSL written for {} of {} nodes with a shader\n\
         not yet ({}): {}",
        entries.len(),
        written.len(),
        written.len() + not_yet.len(),
        not_yet.len(),
        not_yet.join(" ")
    );
    assert!(
        failures.is_empty(),
        "{} of {} WGSL modules fail:\n\n{}",
        failures.len(),
        entries.len(),
        failures.join("\n\n")
    );
}

/// WGSL has no overloading, so two helpers of one name — in two nodes' `wgsl_utils`, or a
/// util and the prelude — are a module that fails to parse on the day a graph holds both.
/// Every distinct util in the registry goes into one module with the prelude, which parses
/// only if every name is unique.
#[test]
fn every_wgsl_util_name_is_unique() {
    let mut seen = HashSet::new();
    let mut source = format!(
        "{}\nstruct Uniforms {{ u_resolution: vec2f, u_time: f32, }}\n\
         @group(0) @binding(0) var<uniform> u: Uniforms;\n",
        wgsl::PRELUDE
    );
    for def in nodes::REGISTRY {
        for util in def.wgsl_utils {
            if seen.insert(*util) {
                write!(source, "\n// {}\n{util}\n", def.slug).unwrap();
            }
        }
    }
    let module = naga::front::wgsl::parse_str(&source)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), capabilities())
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
}

// ---------------------------------------------------------------------- the compute kernels

/// One compute module through the path wgpu takes it, with its one compute entry point where
/// the renderer asks for it.
fn check_compute(source: &str) -> Result<String, String> {
    let (module, msl) = translate(source)?;
    has_entry(
        &module,
        supersilvia::render::sims::ENTRY,
        naga::ShaderStage::Compute,
    )?;
    Ok(msl)
}

/// Every kernel `slimemold` steps its world with, as `render::sims::source` assembles it
/// after the renderer's prelude with the numbers the node hands it, and the renderer's own
/// resample stage, reach Metal and Vulkan. `SHADER_TARGETS_DUMP` writes them beside the rest.
#[test]
fn every_simulation_kernel_reaches_msl_and_spirv() {
    use supersilvia::nodes::slimemold::{KERNELS, PARAMS};
    use supersilvia::render::sims;

    let mut modules: Vec<(String, String)> = KERNELS
        .iter()
        .map(|k| (format!("kernel_{}", k.name), sims::source(k, &PARAMS)))
        .collect();
    modules.push(("stage_sim_resample".to_string(), sims::RESAMPLE.to_string()));
    assert!(
        modules
            .iter()
            .any(|(_, s)| s.contains("workgroupBarrier()"))
            && modules.iter().any(|(_, s)| s.contains("atomicAdd"))
            && modules
                .iter()
                .any(|(_, s)| s.contains("r32float, read_write")),
        "the kernels share memory, count with atomics and write the field in place"
    );
    let dir = dump_dir();
    let mut failures = Vec::new();
    for (name, source) in &modules {
        if let Some(dir) = &dir {
            std::fs::write(dir.join(format!("{name}.wgsl")), source).expect("a dump file");
        }
        match check_compute(source) {
            Ok(msl) => {
                if let Some(dir) = &dir {
                    std::fs::write(dir.join(format!("{name}.metal")), msl).expect("a dump file");
                }
            }
            Err(e) => failures.push(format!("{name}: {e}")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} compute modules fail:\n\n{}",
        failures.len(),
        modules.len(),
        failures.join("\n\n")
    );
}

// ------------------------------------------------------------------- the renderer's stages

/// Every render stage the renderer writes for itself rather than compiling from a graph: the
/// conversion pass a CPU node's planes go through, the carry a resize scales the old frame
/// with, the pass every picture read draws, the mix and the viewer's blit. Each reaches Metal
/// and Vulkan and has the vertex and fragment entry points its pipeline names, and none
/// samples with implicit derivatives. `SHADER_TARGETS_DUMP` writes them beside the rest.
#[test]
fn every_renderer_stage_reaches_msl_and_spirv() {
    use supersilvia::render::{mixer, readback, shared, sources, viewer};

    let stages = [
        ("stage_convert", sources::CONVERT),
        ("stage_carry", shared::CARRY),
        ("stage_picture", readback::PICTURE),
        ("stage_mix", mixer::MIX),
        ("stage_blit", viewer::BLIT),
    ];
    let dir = dump_dir();
    let mut failures = Vec::new();
    for (name, source) in stages {
        if let Some(dir) = &dir {
            std::fs::write(dir.join(format!("{name}.wgsl")), source).expect("a dump file");
        }
        let checked = translate(source).and_then(|(module, msl)| {
            has_entry(&module, wgsl::VERTEX_ENTRY, naga::ShaderStage::Vertex)?;
            has_entry(&module, wgsl::FRAGMENT_ENTRY, naga::ShaderStage::Fragment)?;
            Ok(msl)
        });
        match checked {
            Ok(msl) => {
                if let Some(dir) = &dir {
                    std::fs::write(dir.join(format!("{name}.metal")), msl).expect("a dump file");
                }
            }
            Err(e) => failures.push(format!("{name}: {e}")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} renderer stages fail:\n\n{}",
        failures.len(),
        stages.len(),
        failures.join("\n\n")
    );
}
