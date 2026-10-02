//! Layering rules from CONTRIBUTING.md, checked rather than remembered.
//!
//! These are the rules a compiler cannot state: nothing about them is a type error, and a
//! violation would build and pass every other test. They are asserted over the source text,
//! which is crude but is the only thing that sees them at all.

use std::path::Path;

/// `graph/`, `compile/`, `nodes/`, `audio/`, `video/` and `synth/` name no graphics crate.
///
/// It is what lets them be tested with a plain `cargo test` on a machine with no GPU, and
/// what keeps the renderer the one place that knows about the GPU. `emath` is deliberately
/// absent from the list: it is egui's math crate with no graphics in it.
///
/// `synth/` owns the renderer, and holds it on a `render::Gpu` — a handle it names
/// through the crate, never the `wgpu` crate itself — so it names no graphics crate either: it
/// never draws a widget, reads an event or touches a viewport. What a node computes still
/// does not depend on what draws it; see `proposals/deterministic-loop.md`.
#[test]
fn the_pure_modules_take_no_graphical_dependency() {
    const GRAPHICAL: [&str; 3] = ["egui", "eframe", "wgpu"];
    const PURE: [&str; 6] = ["graph", "compile", "nodes", "audio", "video", "synth"];

    let mut found = Vec::new();
    for module in PURE {
        for file in rust_files(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join(module),
        ) {
            let text = std::fs::read_to_string(&file).unwrap();
            for (n, line) in text.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                for crate_name in GRAPHICAL {
                    // The crate itself, not a path of the crate's own that ends in its name:
                    // `crate::render::` is a module of this crate and names no crate.
                    let named = code
                        .match_indices(&format!("{crate_name}::"))
                        .any(|(at, _)| {
                            code[..at]
                                .chars()
                                .next_back()
                                .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
                        });
                    if named {
                        found.push(format!("{}:{}: {}", file.display(), n + 1, line.trim()));
                    }
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "a pure module reached for a graphics crate:\n{}",
        found.join("\n")
    );
}

/// `graph/`, `compile/`, `nodes/`, `audio/` and `video/` reach no graphical crate through
/// the crate either: not through a module they name, nor one that module names, nor a
/// re-export on the way.
///
/// The test above reads a file for the crate's own name, and a path walks round it —
/// `crate::widgets::RegionDef` names no `egui::` and is a struct built on `egui::Ui`. So
/// this one follows every `crate::`, `super::`, `self::` and child-module path a pure file
/// names to the file that defines it, and every path that file names in turn, and fails on
/// the first file along the way that names a graphical crate. A path that stops at a
/// module's own file — an item it defines, or one it re-exports — reaches all of that file,
/// which is the conservative reading: a re-export is followed wherever it goes. The crate
/// root's `pub use` lines are resolved to the items they name, so `crate::App` reaches
/// `app/` and not every module `lib.rs` declares.
///
/// `synth/` keeps the direct rule only: it owns the renderer, and the renderer is wgpu, which
/// `render/` names.
#[test]
fn the_pure_modules_reach_no_graphical_crate_through_the_crate() {
    const GRAPHICAL: [&str; 3] = ["egui", "eframe", "wgpu"];
    const PURE: [&str; 5] = ["graph", "compile", "nodes", "audio", "video"];
    // The edges out of a pure module's reach known to break the rule, and not walked through:
    // each machine's video service, which `video/` and `audio/` reach through
    // `platform/mod.rs`, asks `render/dmabuf.rs` whether the renderer's device can import a
    // frame without a copy — Linux's a DMA-BUF, and in which formats, before a screen cast or a
    // clip asks its source for one, the Mac's an `IOSurface`, before a clip does. The question
    // needs the device, which only the renderer holds. Named by both files, so a second edge
    // between the same two modules is still caught.
    const KNOWN: [(&str, &str); 2] = [
        ("src/platform/linux/video.rs", "src/render/dmabuf.rs"),
        ("src/platform/macos/video.rs", "src/render/dmabuf.rs"),
    ];

    let crate_ = Crate::read();
    let mut found = Vec::new();
    for module in PURE {
        for start in crate_.files_in(module) {
            // Breadth first, remembering how each file was reached so a failure can say.
            let mut came_from: std::collections::BTreeMap<String, String> =
                std::collections::BTreeMap::new();
            let mut queue = std::collections::VecDeque::from([start.clone()]);
            came_from.insert(start.clone(), String::new());
            while let Some(file) = queue.pop_front() {
                if let Some(name) = crate_.names_any(&file, &GRAPHICAL) {
                    let mut chain = vec![file.clone()];
                    let mut at = file.clone();
                    while let Some(prev) = came_from.get(&at).filter(|p| !p.is_empty()) {
                        chain.push(prev.clone());
                        at = prev.clone();
                    }
                    chain.reverse();
                    found.push(format!("{} names {name}", chain.join(" -> ")));
                    continue;
                }
                for next in crate_.reached_from(&file) {
                    if KNOWN.contains(&(file.as_str(), next.as_str())) {
                        continue;
                    }
                    if !came_from.contains_key(&next) {
                        came_from.insert(next.clone(), file.clone());
                        queue.push_back(next);
                    }
                }
            }
        }
    }
    found.sort();
    found.dedup();
    assert!(
        found.is_empty(),
        "a pure module reaches a graphics crate through the crate:\n{}",
        found.join("\n")
    );
}

/// Each operating system's crates are named in its own backend and nowhere else: Linux's —
/// `libloading` among them, which opens the NDI® runtime by its path — in `platform/linux/`,
/// macOS's — `midir` among them — in `platform/macos/`, and `rfd`, which is
/// both machines' file dialogs, in those two. Apple's own bindings — every `objc2` crate,
/// `dispatch2` and `block2` — are macOS's, and `render/` may name them too, since the Metal
/// import of an `IOSurface` is the renderer's. `block2` is Syphon's, for the block a client is
/// handed its frames through, and `render/` names none. `gstndi`, the NDI® plugin, is both
/// machines' and is named by `video/ndi.rs` alone, which registers it: everything else asks for
/// its elements by name.
///
/// Everything the app asks of the machine goes through `platform/`'s narrow services, which
/// is what lets each backend answer the same names; a Linux crate named anywhere else is a
/// Linux dependency the macOS build cannot see coming, and the reverse, since `Cargo.toml`
/// declares each of these for its own machine alone. The Wayland crates `render::picture`
/// draws its windows with on Linux, and the winit it draws them with on macOS, are not on the
/// list: the picture windows are `render/`'s, not a service of `platform/`.
#[test]
fn the_os_crates_are_named_only_in_their_backends() {
    const LINUX: &str = "src/platform/linux/";
    const MACOS: &str = "src/platform/macos/";
    const RENDER: &str = "src/render/";
    const NDI: &str = "src/video/ndi.rs";
    // Each crate, and the backends that may name it; a name ending in `*` is every crate whose
    // name begins with the rest. A macOS crate goes in with `&[MACOS]`.
    const OS: [(&str, &[&str]); 12] = [
        ("alsa", &[LINUX]),
        ("libloading", &[LINUX]),
        ("ashpd", &[LINUX]),
        ("fontconfig", &[LINUX]),
        ("tokio", &[LINUX]),
        ("gstreamer_allocators", &[LINUX]),
        ("rfd", &[LINUX, MACOS]),
        ("midir", &[MACOS]),
        ("objc2*", &[MACOS, RENDER]),
        ("dispatch2", &[MACOS, RENDER]),
        ("block2", &[MACOS]),
        ("gstndi", &[NDI]),
    ];
    // Apple's frameworks, through `objc2` and every `objc2-*` binding, `dispatch2` and
    // `block2`: the services' backend, and the renderer on Metal.
    const APPLE: &[&str] = &[MACOS, "src/render/"];
    let apple = |name: &str| name.starts_with("objc2") || name == "dispatch2" || name == "block2";
    let crate_ = Crate::read();
    let mut found = Vec::new();
    for (f, tokens) in &crate_.files {
        for (name, backends) in OS {
            let named = tokens.windows(2).any(|w| {
                w[1] == "::"
                    && name
                        .strip_suffix('*')
                        .map_or(w[0] == name, |prefix| w[0].starts_with(prefix))
            });
            if named && !backends.iter().any(|b| f.starts_with(b)) {
                found.push(format!("{f} names {name}"));
            }
        }
        if !APPLE.iter().any(|b| f.starts_with(b))
            && let Some(w) = tokens.windows(2).find(|w| apple(&w[0]) && w[1] == "::")
        {
            found.push(format!("{f} names {}", w[0]));
        }
    }
    assert!(
        found.is_empty(),
        "an operating system's crate named outside its backend:\n{}",
        found.join("\n")
    );
}

/// `unsafe` is allowed on these modules and no others: the crate root denies it, and each is
/// re-allowed where its parent declares it. A new `#[allow(unsafe_code)]` fails here until it
/// is added to the list, and to CONTRIBUTING.md's and docs/invariants.md's.
#[test]
fn unsafe_is_allowed_only_where_the_rule_names() {
    const ALLOWED: [&str; 10] = [
        "src/render/mod.rs: dmabuf",
        "src/render/picture/mod.rs: thread",
        "src/render/picture/mod.rs: wayland",
        "src/platform/linux/mod.rs: ndi",
        "src/platform/macos/mod.rs: audio",
        "src/platform/macos/mod.rs: gpu",
        "src/platform/macos/mod.rs: menu",
        "src/platform/macos/mod.rs: pixels",
        "src/platform/macos/mod.rs: screen",
        "src/platform/macos/mod.rs: syphon",
    ];
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found = Vec::new();
    for file in rust_files(&root.join("src")) {
        let text = std::fs::read_to_string(&file).unwrap();
        let lines = production_lines(&text);
        let name = file
            .strip_prefix(root)
            .unwrap()
            .display()
            .to_string()
            .replace('\\', "/");
        for (i, &(_, line)) in lines.iter().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if !code.contains("allow(unsafe_code)") {
                continue;
            }
            // What it is on: the first line after it that is not another attribute.
            let on = lines[i + 1..]
                .iter()
                .map(|(_, l)| l.trim())
                .find(|l| !l.starts_with("#["))
                .unwrap_or("");
            let module = on
                .trim_start_matches("pub ")
                .strip_prefix("mod ")
                .map_or(on, |m| m.trim_end_matches(';'));
            found.push(format!("{name}: {module}"));
        }
    }
    found.sort();
    let mut allowed: Vec<String> = ALLOWED.iter().map(ToString::to_string).collect();
    allowed.sort();
    assert_eq!(
        found, allowed,
        "`unsafe` allowed somewhere the rule does not name"
    );
}

/// The walk above holds: a pure file that reached the toolkit only through another module's
/// re-export is caught, where the literal test is not.
#[test]
fn the_walk_sees_through_a_re_export() {
    let crate_ = Crate::from_files(&[
        ("src/lib.rs", "pub mod graph;\npub mod shim;\npub mod ui;\n"),
        ("src/graph/mod.rs", "use crate::shim::Thing;\n"),
        ("src/shim.rs", "pub use crate::ui::Thing;\n"),
        ("src/ui/mod.rs", "pub struct Thing(egui::Ui);\n"),
    ]);
    let reached = crate_.reached_from("src/graph/mod.rs");
    assert_eq!(reached, vec!["src/shim.rs".to_string()]);
    assert_eq!(
        crate_.reached_from("src/shim.rs"),
        vec!["src/ui/mod.rs".to_string()]
    );
    assert_eq!(
        crate_.names_any("src/ui/mod.rs", &["egui"]),
        Some("egui"),
        "and the file at the end of it names the toolkit"
    );
    assert_eq!(crate_.names_any("src/graph/mod.rs", &["egui"]), None);
}

/// A node already in the graph is never looked up by its slug.
///
/// `Node::def` is the kind, held; `nodes::find` scans the registry for a string, and is for
/// where a string is all there is — a node a file names, a node the library adds. Anything
/// else calling it is a search on a path that already had the answer, and the tick, the
/// compiler and the canvas run those paths for every node, every frame.
#[test]
fn a_node_in_the_graph_is_never_looked_up_by_its_slug() {
    // Where a slug arrives as a string: the two file formats, and the registry itself, whose
    // `add_to_graph_on` is what the library and the command bus add a node through.
    const ARRIVALS: [&str; 2] = ["src/workspace.rs", "src/project.rs"];
    let crate_ = Crate::read();
    let mut found = Vec::new();
    for (file, tokens) in &crate_.files {
        if ARRIVALS.contains(&file.as_str()) || file.starts_with("src/nodes/") {
            continue;
        }
        let named = tokens
            .windows(3)
            .any(|w| w[0] == "nodes" && w[1] == "::" && w[2] == "find")
            || paths_in(tokens)
                .iter()
                .any(|p| p.ends_with(&["nodes".to_string(), "find".to_string()]));
        if named {
            found.push(file.clone());
        }
    }
    assert!(
        found.is_empty(),
        "looked a node's kind up by slug where `Node::def` holds it:\n{}",
        found.join("\n")
    );
}

/// The crate's own source, as far as the walk needs it: every file's production code as
/// tokens, comments and literals taken out, keyed by its path from the crate root.
struct Crate {
    /// `src/graph/node.rs` to its tokens.
    files: std::collections::BTreeMap<String, Vec<String>>,
    /// What the crate root re-exports: `App` to `app::App`.
    root: std::collections::BTreeMap<String, Vec<String>>,
}

impl Crate {
    fn read() -> Self {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut owned = Vec::new();
        for file in rust_files(&src) {
            let text = std::fs::read_to_string(&file).unwrap();
            let rel = file
                .strip_prefix(Path::new(env!("CARGO_MANIFEST_DIR")))
                .unwrap()
                .to_string_lossy()
                .into_owned();
            owned.push((rel, text));
        }
        let borrowed: Vec<(&str, &str)> = owned
            .iter()
            .map(|(p, t)| (p.as_str(), t.as_str()))
            .collect();
        Self::from_files(&borrowed)
    }

    fn from_files(files: &[(&str, &str)]) -> Self {
        let files: std::collections::BTreeMap<String, Vec<String>> = files
            .iter()
            .map(|(path, text)| {
                let code: Vec<&str> = production_lines(text).into_iter().map(|(_, l)| l).collect();
                ((*path).to_string(), tokenize(&code.join("\n")))
            })
            .collect();
        let mut root = std::collections::BTreeMap::new();
        if let Some(lib) = files.get("src/lib.rs") {
            for path in paths_in(lib) {
                // `pub use app::App;` — a bare path out of the root, so the first segment is
                // a module and the last is the name the root gives it.
                if let (Some(first), Some(last)) = (path.first(), path.last())
                    && first != "crate"
                    && path.len() > 1
                {
                    root.insert(last.clone(), path.clone());
                }
            }
        }
        Self { files, root }
    }

    /// Every file under `src/<module>/`, or `src/<module>.rs`.
    fn files_in(&self, module: &str) -> Vec<String> {
        let dir = format!("src/{module}/");
        let file = format!("src/{module}.rs");
        self.files
            .keys()
            .filter(|p| p.starts_with(&dir) || **p == file)
            .cloned()
            .collect()
    }

    /// The first of `crates` this file's code names with a path, if it names one.
    fn names_any(&self, file: &str, crates: &[&'static str]) -> Option<&'static str> {
        let tokens = self.files.get(file)?;
        crates
            .iter()
            .copied()
            .find(|c| tokens.windows(2).any(|w| w[0] == *c && w[1] == "::"))
    }

    /// The module path a file defines: `src/nodes/area.rs` is `nodes::area`, and a `mod.rs`
    /// is its directory's.
    fn module_of(file: &str) -> Vec<String> {
        let rel = file.trim_start_matches("src/").trim_end_matches(".rs");
        let mut parts: Vec<String> = rel.split('/').map(str::to_string).collect();
        if parts.last().is_some_and(|p| p == "mod" || p == "lib") {
            parts.pop();
        }
        parts
    }

    /// The file that defines a module path, where there is one.
    fn file_of(&self, module: &[String]) -> Option<String> {
        if module.is_empty() {
            return Some("src/lib.rs".to_string()).filter(|p| self.files.contains_key(p));
        }
        let joined = module.join("/");
        [format!("src/{joined}.rs"), format!("src/{joined}/mod.rs")]
            .into_iter()
            .find(|p| self.files.contains_key(p))
    }

    /// Every other file one file's code reaches with a path, in path order, deduplicated.
    fn reached_from(&self, file: &str) -> Vec<String> {
        let Some(code) = self.files.get(file) else {
            return Vec::new();
        };
        let here = Self::module_of(file);
        let mut out = Vec::new();
        for path in paths_in(code) {
            if let Some(target) = self.resolve(&here, &path, 0)
                && target != file
                && !out.contains(&target)
            {
                out.push(target);
            }
        }
        out
    }

    /// The file a path out of module `here` ends in: the deepest module file it walks
    /// through, or for an item at the crate root, wherever the root's re-export points.
    fn resolve(&self, here: &[String], path: &[String], depth: usize) -> Option<String> {
        let (mut module, rest): (Vec<String>, &[String]) = match path.first()?.as_str() {
            "crate" | "$crate" => (Vec::new(), &path[1..]),
            "self" => (here.to_vec(), &path[1..]),
            "super" => {
                let mut m = here.to_vec();
                let mut rest = path;
                while rest.first().is_some_and(|s| s == "super") {
                    m.pop();
                    rest = &rest[1..];
                }
                (m, rest)
            }
            // A bare path is a child module of this one, or an extern crate this walk does
            // not follow.
            child => {
                let mut m = here.to_vec();
                m.push(child.to_string());
                self.file_of(&m)?;
                (m, &path[1..])
            }
        };
        let mut rest = rest.iter();
        for seg in rest.by_ref() {
            let mut next = module.clone();
            next.push(seg.clone());
            if self.file_of(&next).is_some() {
                module = next;
            } else {
                if module.is_empty() {
                    // An item at the crate root is a re-export; follow it once.
                    let target = self.root.get(seg)?;
                    return (depth < 4).then(|| self.resolve(&[], target, depth + 1))?;
                }
                break;
            }
        }
        (!module.is_empty()).then(|| self.file_of(&module))?
    }
}

/// Every path a file's tokens name, each as its segments: `crate::graph::{Node, NodeId}` is
/// two, `crate::graph::Node` and `crate::graph::NodeId`.
///
/// A path starts at `crate`, `$crate`, `super` or `self`, or at the first segment of a `use`
/// — which is how a module names its own children. A lowercase first segment anywhere else
/// is a local variable or an extern crate as often as a module, and a module named that way
/// is reached through its `use` first.
fn paths_in(tokens: &[String]) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let starts = matches!(tokens[i].as_str(), "crate" | "$crate" | "super" | "self")
            || (i > 0 && tokens[i - 1] == "use" && is_ident(&tokens[i]));
        let preceded = i > 0 && tokens[i - 1] == "::";
        if starts && !preceded && tokens.get(i + 1).is_some_and(|t| t == "::") {
            let mut j = i;
            out.extend(path_tree(tokens, &mut j));
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    out
}

/// One path and whatever braces it opens, from `tokens[*i]`, as the flat list of paths it
/// stands for. `self` inside braces names the prefix itself.
fn path_tree(tokens: &[String], i: &mut usize) -> Vec<Vec<String>> {
    let mut prefix = Vec::new();
    while let Some(t) = tokens.get(*i) {
        if t == "{" {
            *i += 1;
            let mut out = Vec::new();
            while tokens.get(*i).is_some_and(|t| t != "}") {
                for tail in path_tree(tokens, i) {
                    let mut whole = prefix.clone();
                    whole.extend(
                        tail.into_iter()
                            .filter(|s| s != "self" || prefix.is_empty()),
                    );
                    out.push(whole);
                }
                if tokens.get(*i).is_some_and(|t| t == ",") {
                    *i += 1;
                }
            }
            *i += 1;
            return out;
        }
        if !is_ident(t) {
            break;
        }
        prefix.push(t.clone());
        *i += 1;
        if tokens.get(*i).is_some_and(|t| t == "::") {
            *i += 1;
        } else {
            break;
        }
    }
    // `as` renames the last segment; what it names is unchanged.
    if tokens.get(*i).is_some_and(|t| t == "as") {
        *i += 2;
    }
    vec![prefix]
}

fn is_ident(t: &str) -> bool {
    t.chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
}

/// Identifiers, `::` and single punctuation, with comments and string and char literals
/// dropped — so a path in a doc comment or a word in a tooltip is not a dependency.
fn tokenize(code: &str) -> Vec<String> {
    let chars: Vec<char> = code.chars().collect();
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        let after_word = i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_');
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
        } else if c == '"' {
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                i += if chars[i] == '\\' { 2 } else { 1 };
            }
            i += 1;
        } else if c == 'r' && matches!(next, Some('#' | '"')) && !after_word {
            // A raw string, `r"..."` or `r#"..."#`, closed by a quote and as many hashes.
            let mut hashes = 0;
            i += 1;
            while chars.get(i) == Some(&'#') {
                hashes += 1;
                i += 1;
            }
            if chars.get(i) != Some(&'"') {
                continue;
            }
            i += 1;
            while i < chars.len()
                && !(chars[i] == '"' && (1..=hashes).all(|h| chars.get(i + h) == Some(&'#')))
            {
                i += 1;
            }
            i += 1 + hashes;
        } else if c == '\'' {
            // A char literal, `'x'` or `'\n'`; anything else is a lifetime's tick.
            if next == Some('\\') {
                i += 2;
                while i < chars.len() && chars[i] != '\'' {
                    i += 1;
                }
                i += 1;
            } else if chars.get(i + 2) == Some(&'\'') {
                i += 3;
            } else {
                i += 1;
            }
        } else if word(c) {
            let start = i;
            while i < chars.len() && word(chars[i]) {
                i += 1;
            }
            out.push(chars[start..i].iter().collect());
        } else if c == ':' && next == Some(':') {
            out.push("::".to_string());
            i += 2;
        } else if c.is_whitespace() {
            i += 1;
        } else {
            out.push(c.to_string());
            i += 1;
        }
    }
    out
}

/// No preference reaches the shader.
///
/// The tier test — a preference is about the person, a project is about the work — restated as
/// a code rule. `graph/`, `compile/`, `nodes/`, `render/`, `audio/` and `video/` decide what
/// the video is; if one of them could read a preference then two people opening the same
/// project would see two different pictures, and the file would no longer say what it means.
#[test]
fn no_preference_reaches_the_shader() {
    const SEALED: [&str; 6] = ["graph", "compile", "nodes", "render", "audio", "video"];

    let mut found = Vec::new();
    for module in SEALED {
        for file in rust_files(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join(module),
        ) {
            let text = std::fs::read_to_string(&file).unwrap();
            for (n, line) in production_lines(&text) {
                let code = line.split("//").next().unwrap_or("");
                if code.contains("preferences") || code.contains("Preferences") {
                    found.push(format!("{}:{}: {}", file.display(), n, line.trim()));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "a module that decides what the video is reached for a preference:\n{}",
        found.join("\n")
    );
}

/// Nothing that decides what the video is knows where the project is.
///
/// A node holds an asset reference — `assets/gumbasia.webm` — and asks `ctx.path` for a file
/// to open. The resolving is the project's, behind the `Assets` trait `nodes/cpu.rs` declares
/// and `project.rs` implements. A module here that reached `project.rs` directly would learn
/// the root, and "a project folder can be moved, zipped and sent" would then rest on every
/// node's care rather than on one place; the same holds for the cache directory, which is
/// handed in rather than looked up.
#[test]
fn nothing_that_decides_the_video_knows_where_the_project_is() {
    const SEALED: [&str; 6] = ["graph", "compile", "nodes", "render", "audio", "video"];

    let mut found = Vec::new();
    for module in SEALED {
        for file in rust_files(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join(module),
        ) {
            let text = std::fs::read_to_string(&file).unwrap();
            for (n, line) in production_lines(&text) {
                let code = line.split("//").next().unwrap_or("");
                if code.contains("project::") || code.contains("Project") {
                    found.push(format!("{}:{}: {}", file.display(), n, line.trim()));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "a module that decides what the video is reached for the project:\n{}",
        found.join("\n")
    );
}

/// `ui/` never mutates: the canvas returns `Command`s and the menu returns `MenuAction`s.
///
/// The command bus is the only mutation path, and it is what carries undo, coalescing and
/// the recompile boundary. A `ui/` function handed a `&mut Graph` would compile, would work,
/// and would take all three with it on the way out — there is no type error to catch it, and
/// the damage shows up as a missing undo step long after the change.
///
/// Test modules are exempt: they drive a real `App` to build the graph a layout test needs,
/// which is the command bus being used, not bypassed.
#[test]
fn ui_never_takes_a_mutable_reference_to_the_model() {
    const MODEL: [&str; 3] = ["App", "Graph", "Node"];

    let mut found = Vec::new();
    for file in rust_files(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("ui")) {
        let text = std::fs::read_to_string(&file).unwrap();
        for (n, line) in production_lines(&text) {
            for ty in MODEL {
                // `&mut Graph`, and `&mut crate::Graph` or any other path ending in it.
                if line.contains(&format!("&mut {ty}"))
                    || line.contains(&format!("::{ty}")) && line.contains("&mut ")
                {
                    found.push(format!("{}:{}: {}", file.display(), n, line.trim()));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "ui/ took a mutable reference to the model; it returns commands instead:\n{}",
        found.join("\n")
    );
}

/// A file's lines, numbered from 1, stopping at its test module.
///
/// Crude on purpose: it is looking at source text, and the alternative is a parser. A
/// `#[cfg(test)]` anywhere but before the trailing test module would end the scan early and
/// under-report, which is the failure direction worth choosing.
fn production_lines(text: &str) -> Vec<(usize, &str)> {
    text.lines()
        .take_while(|l| !l.trim_start().starts_with("#[cfg(test)]"))
        .enumerate()
        .map(|(i, l)| (i + 1, l))
        .collect()
}

fn rust_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).expect("a module directory") {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(rust_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out
}
