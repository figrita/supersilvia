// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which GPU supersilvia renders on, as one rule over every adapter an instance offers.
//!
//! [`choose`] is pure: it reads a list of `wgpu::AdapterInfo` and what the environment asks,
//! and returns an index or a refusal naming every adapter and why each was passed over. The
//! app, every GPU test, every bench and kittest's shared device go through it.
//!
//! **The app renders on the strongest GPU the machine offers**: a discrete GPU of any vendor,
//! NVIDIA included, before an integrated one, before anything else. A software adapter
//! (llvmpipe, lavapipe, SwiftShader) is refused — a green run on it says nothing about the
//! pipeline — unless `SUPERSILVIA_SOFTWARE_GPU=1` opts a machine with no GPU in.
//! `SUPERSILVIA_ADAPTER` names an adapter outright and wins: `SUPERSILVIA_ADAPTER=integrated`
//! is how a machine with a discrete GPU renders on its integrated one instead.
//!
//! **Tests and benches ask for the integrated GPU**, [`Asked::integrated`], rather than taking
//! the app's default: a run on a discrete GPU would draw different pixels, and the
//! snapshots are an Intel iGPU's pixels (Mesa). See
//! [proposals/wgpu.md](../../proposals/wgpu.md#4-adapter-selection).

use wgpu::{AdapterInfo, Backend, DeviceType};

/// The environment variable that names an adapter: an index into the list [`describe_all`]
/// prints, `vendor:device` in hex, [`INTEGRATED`], or a case-insensitive substring of its name.
pub const ADAPTER_ENV: &str = "SUPERSILVIA_ADAPTER";

/// The environment variable that lets a software adapter be chosen, when set to `1`.
pub const SOFTWARE_ENV: &str = "SUPERSILVIA_SOFTWARE_GPU";

/// The override that names every integrated GPU, whatever its vendor or name.
pub const INTEGRATED: &str = "integrated";

/// PCI vendor id of NVIDIA.
pub const NVIDIA: u32 = 0x10de;

/// The backends an instance is made with: Vulkan on Linux and Windows, Metal on macOS, nothing
/// else.
pub const BACKENDS: wgpu::Backends = wgpu::Backends::VULKAN.union(wgpu::Backends::METAL);

/// What the environment asks of [`choose`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Asked {
    /// `SUPERSILVIA_ADAPTER`, if set and not empty.
    pub adapter: Option<String>,
    /// `SUPERSILVIA_SOFTWARE_GPU=1`.
    pub software: bool,
}

impl Asked {
    /// The two variables, read from this process's environment: what the app asks.
    pub fn from_env() -> Self {
        Self {
            adapter: std::env::var(ADAPTER_ENV)
                .ok()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty()),
            software: std::env::var(SOFTWARE_ENV).is_ok_and(|v| v.trim() == "1"),
        }
    }

    /// What a test or a bench asks: the environment's answers, with [`INTEGRATED`] standing in
    /// for an unset `SUPERSILVIA_ADAPTER`, so none of them renders on a discrete GPU unless
    /// the variable names one.
    pub fn integrated() -> Self {
        let mut asked = Self::from_env();
        asked.adapter.get_or_insert_with(|| INTEGRATED.to_owned());
        asked
    }
}

/// What the app was offered and what it took: every adapter the instance listed, in its order,
/// the index of the one rendered on, and what the environment asked. Read by the Preferences
/// window's GPU section; listing the adapters is the enumeration [`headless`] already did, and
/// nothing here opens a device on any of them.
#[derive(Clone, Debug)]
pub struct Choice {
    pub offered: Vec<AdapterInfo>,
    pub chosen: usize,
    pub asked: Asked,
}

impl Choice {
    /// The adapter rendered on.
    pub fn in_use(&self) -> &AdapterInfo {
        &self.offered[self.chosen]
    }

    /// How it was picked, in a few words: by the variable that named it, or by the rule.
    pub fn how(&self) -> String {
        let how = match &self.asked.adapter {
            Some(wanted) => format!("{ADAPTER_ENV}={wanted}"),
            None => "the strongest: a discrete GPU before an integrated one".to_owned(),
        };
        if self.asked.software {
            format!("{how}, {SOFTWARE_ENV}=1")
        } else {
            how
        }
    }
}

/// What kind of adapter it is, in a word.
pub fn kind(device_type: DeviceType) -> &'static str {
    match device_type {
        DeviceType::DiscreteGpu => "discrete",
        DeviceType::IntegratedGpu => "integrated",
        DeviceType::Cpu => "software",
        DeviceType::Other | DeviceType::VirtualGpu => "other",
    }
}

/// The adapter supersilvia renders on, out of every adapter the instance offers.
///
/// In order: an adapter on another backend than Vulkan or Metal is never taken; an override
/// leaves only the adapters it names and refuses to start if it names none; a software adapter
/// needs `software`, named or not; of what is left, a discrete GPU before an integrated one
/// before any other hardware before a software adapter, and among adapters of one kind the
/// first in the order the instance lists them. `Err` names every adapter offered and why each
/// was passed over.
pub fn choose(adapters: &[AdapterInfo], asked: &Asked) -> Result<usize, String> {
    let mut passed = Vec::new();
    let mut candidates = Vec::new();

    for (i, info) in adapters.iter().enumerate() {
        let refusal = if !matches!(info.backend, Backend::Vulkan | Backend::Metal) {
            Some(format!(
                "its backend is {:?}, not Vulkan or Metal",
                info.backend
            ))
        } else if let Some(wanted) = &asked.adapter
            && !names(wanted, i, adapters)
        {
            Some(format!("{ADAPTER_ENV} does not name it"))
        } else if info.device_type == DeviceType::Cpu && !asked.software {
            Some(format!(
                "it is a software adapter; {SOFTWARE_ENV}=1 allows one"
            ))
        } else {
            None
        };
        match refusal {
            Some(why) => passed.push(format!("  [{i}] {}: {why}", describe(info))),
            None => candidates.push(i),
        }
    }

    let tier = |i: &usize| match adapters[*i].device_type {
        DeviceType::DiscreteGpu => 0,
        DeviceType::IntegratedGpu => 1,
        DeviceType::Other | DeviceType::VirtualGpu => 2,
        DeviceType::Cpu => 3,
    };
    // `min_by_key` keeps the first of equals, so the list's own order breaks a tie.
    if let Some(&chosen) = candidates.iter().min_by_key(|i| tier(i)) {
        return Ok(chosen);
    }

    let offered = if adapters.is_empty() {
        "  none".to_owned()
    } else {
        passed.join("\n")
    };
    let (asked_for, advice) = match &asked.adapter {
        Some(wanted) => (
            format!(" ({ADAPTER_ENV}={wanted})"),
            format!(
                "\n{ADAPTER_ENV} takes an index into this list, `vendor:device` in hex, \
                 `{INTEGRATED}`, or a piece of an adapter's name."
            ),
        ),
        None => (String::new(), String::new()),
    };
    Err(format!(
        "no GPU supersilvia will render on{asked_for}. Offered:\n{offered}{advice}"
    ))
}

/// Whether an override names adapter `i`: its index, its `vendor:device` in hex, [`INTEGRATED`]
/// for every integrated GPU, or a case-insensitive substring of its name. A number is an index
/// when the list is that long, and a piece of a name otherwise, so `3090` names the card.
fn names(wanted: &str, i: usize, adapters: &[AdapterInfo]) -> bool {
    let info = &adapters[i];
    if let Ok(index) = wanted.parse::<usize>()
        && index < adapters.len()
    {
        return index == i;
    }
    if let Some((vendor, device)) = wanted.split_once(':')
        && let (Ok(vendor), Ok(device)) = (
            u32::from_str_radix(vendor.trim_start_matches("0x"), 16),
            u32::from_str_radix(device.trim_start_matches("0x"), 16),
        )
    {
        return info.vendor == vendor && info.device == device;
    }
    if wanted.eq_ignore_ascii_case(INTEGRATED) {
        return info.device_type == DeviceType::IntegratedGpu;
    }
    info.name.to_lowercase().contains(&wanted.to_lowercase())
}

/// One adapter on one line: name, ids, kind, driver and backend.
pub fn describe(info: &AdapterInfo) -> String {
    format!(
        "{} ({:04x}:{:04x}, {:?}, {} {}, {:?})",
        info.name,
        info.vendor,
        info.device,
        info.device_type,
        info.driver,
        info.driver_info,
        info.backend
    )
}

/// Every adapter offered, one per line, indexed as `SUPERSILVIA_ADAPTER` counts them.
pub fn describe_all(adapters: &[AdapterInfo]) -> String {
    adapters
        .iter()
        .enumerate()
        .map(|(i, info)| format!("  [{i}] {}", describe(info)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// [`choose`] over live adapters with what `asked` asks, returning the adapter itself. Logs
/// the choice; a refusal names every adapter offered. The app passes [`Asked::from_env`], a
/// test or a bench [`Asked::integrated`].
pub fn pick(adapters: &[wgpu::Adapter], asked: &Asked) -> Result<wgpu::Adapter, String> {
    let infos: Vec<AdapterInfo> = adapters.iter().map(wgpu::Adapter::get_info).collect();
    let i = choose(&infos, asked)?;
    log::info!("rendering on {}", describe(&infos[i]));
    Ok(adapters[i].clone())
}

/// An instance over [`BACKENDS`] with no display handle, the adapter [`pick`] takes from it with
/// what `asked` asks — what the app, a headless test and a bench render on — and the
/// [`Choice`] it made.
pub fn headless(asked: &Asked) -> Result<(wgpu::Instance, wgpu::Adapter, Choice), String> {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = BACKENDS;
    let instance = wgpu::Instance::new(desc);
    let adapters = block_on(instance.enumerate_adapters(BACKENDS));
    let offered: Vec<AdapterInfo> = adapters.iter().map(wgpu::Adapter::get_info).collect();
    let chosen = choose(&offered, asked)?;
    log::info!("rendering on {}", describe(&offered[chosen]));
    let choice = Choice {
        offered,
        chosen,
        asked: asked.clone(),
    };
    Ok((instance, adapters[chosen].clone(), choice))
}

/// Run a future to completion on this thread. wgpu's native futures are ready when made, so
/// this polls once in practice; it parks between polls otherwise.
pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
    struct Unpark(std::thread::Thread);
    impl std::task::Wake for Unpark {
        fn wake(self: std::sync::Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = std::task::Waker::from(std::sync::Arc::new(Unpark(std::thread::current())));
    let mut cx = std::task::Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        if let std::task::Poll::Ready(out) = future.as_mut().poll(&mut cx) {
            return out;
        }
        std::thread::park();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter(name: &str, vendor: u32, device: u32, kind: DeviceType) -> AdapterInfo {
        AdapterInfo {
            name: name.to_owned(),
            vendor,
            device,
            ..AdapterInfo::new(kind, Backend::Vulkan)
        }
    }

    fn uhd770() -> AdapterInfo {
        adapter(
            "Intel(R) Graphics (RPL-S)",
            0x8086,
            0xa780,
            DeviceType::IntegratedGpu,
        )
    }

    fn rtx3090() -> AdapterInfo {
        adapter(
            "NVIDIA GeForce RTX 3090",
            NVIDIA,
            0x2204,
            DeviceType::DiscreteGpu,
        )
    }

    fn llvmpipe() -> AdapterInfo {
        adapter(
            "llvmpipe (LLVM 22.1.8, 256 bits)",
            0x10005,
            0,
            DeviceType::Cpu,
        )
    }

    fn radeon() -> AdapterInfo {
        adapter(
            "AMD Radeon RX 7900 XTX",
            0x1002,
            0x744c,
            DeviceType::DiscreteGpu,
        )
    }

    fn apu() -> AdapterInfo {
        adapter(
            "AMD Radeon Graphics (RADV RAPHAEL_MENDOCINO)",
            0x1002,
            0x164e,
            DeviceType::IntegratedGpu,
        )
    }

    fn none() -> Asked {
        Asked::default()
    }

    fn named(wanted: &str) -> Asked {
        Asked {
            adapter: Some(wanted.to_owned()),
            software: false,
        }
    }

    fn software() -> Asked {
        Asked {
            adapter: None,
            software: true,
        }
    }

    /// The rule as a table: what each machine renders on, by default and when told.
    #[test]
    fn the_strongest_gpu_unless_the_variable_names_another() {
        let cases: [(&str, Vec<AdapterInfo>, Asked, Option<usize>); 17] = [
            (
                "discrete NVIDIA and an iGPU",
                vec![uhd770(), rtx3090(), llvmpipe()],
                none(),
                Some(1),
            ),
            (
                "discrete NVIDIA first",
                vec![rtx3090(), uhd770()],
                none(),
                Some(0),
            ),
            (
                "discrete AMD and an iGPU",
                vec![uhd770(), radeon()],
                none(),
                Some(1),
            ),
            (
                "two discrete GPUs: the first listed",
                vec![uhd770(), radeon(), rtx3090()],
                none(),
                Some(1),
            ),
            (
                "two discrete GPUs, NVIDIA listed first",
                vec![rtx3090(), radeon()],
                none(),
                Some(0),
            ),
            ("an iGPU alone", vec![uhd770()], none(), Some(0)),
            (
                "an iGPU and llvmpipe",
                vec![llvmpipe(), uhd770()],
                none(),
                Some(1),
            ),
            ("NVIDIA alone", vec![rtx3090()], none(), Some(0)),
            ("software alone", vec![llvmpipe()], none(), None),
            ("nothing", vec![], none(), None),
            (
                "`intel` on this box",
                vec![uhd770(), rtx3090(), llvmpipe()],
                named("intel"),
                Some(0),
            ),
            (
                "`integrated` on this box, NVIDIA first",
                vec![rtx3090(), llvmpipe(), uhd770()],
                named("integrated"),
                Some(2),
            ),
            (
                "`INTEGRATED` on an AMD APU and an NVIDIA card",
                vec![rtx3090(), apu()],
                named("INTEGRATED"),
                Some(1),
            ),
            (
                "`integrated` with no iGPU",
                vec![rtx3090(), radeon()],
                named("integrated"),
                None,
            ),
            (
                "`radeon` naming nothing",
                vec![uhd770(), rtx3090()],
                named("radeon"),
                None,
            ),
            (
                "`3090` naming the card",
                vec![uhd770(), rtx3090()],
                named("3090"),
                Some(1),
            ),
            (
                "an index past the list is a piece of a name",
                vec![uhd770()],
                named("7"),
                None,
            ),
        ];
        for (what, adapters, asked, chosen) in cases {
            assert_eq!(choose(&adapters, &asked).ok(), chosen, "{what}");
        }
    }

    #[test]
    fn an_override_names_an_adapter_every_way_it_can() {
        let list = [uhd770(), rtx3090(), llvmpipe()];
        for wanted in ["3090", "nvidia", "10de:2204", "0x10de:0x2204", "1"] {
            assert_eq!(choose(&list, &named(wanted)), Ok(1), "{wanted}");
        }
        for wanted in ["intel", "Intel", "integrated", "8086:a780", "0"] {
            assert_eq!(choose(&list, &named(wanted)), Ok(0), "{wanted}");
        }
    }

    #[test]
    fn an_override_naming_nothing_refuses_and_says_what_it_takes() {
        let err = choose(&[uhd770(), rtx3090()], &named("radeon")).unwrap_err();
        assert!(err.contains("SUPERSILVIA_ADAPTER=radeon"), "{err}");
        assert!(err.contains("Intel(R) Graphics (RPL-S)"), "{err}");
        assert!(err.contains("NVIDIA GeForce RTX 3090"), "{err}");
        assert!(err.contains("does not name it"), "{err}");
        assert!(err.contains("`integrated`"), "{err}");
    }

    #[test]
    fn a_refusal_with_no_override_gives_no_override_advice() {
        let err = choose(&[llvmpipe()], &none()).unwrap_err();
        assert!(err.contains("SUPERSILVIA_SOFTWARE_GPU=1"), "{err}");
        assert!(!err.contains("takes an index"), "{err}");
    }

    #[test]
    fn software_is_taken_only_when_allowed_and_only_after_a_real_gpu() {
        assert_eq!(choose(&[llvmpipe()], &software()), Ok(0));
        assert_eq!(choose(&[llvmpipe(), uhd770()], &software()), Ok(1));
        assert_eq!(choose(&[llvmpipe(), radeon()], &software()), Ok(1));
        let err = choose(&[uhd770(), llvmpipe()], &named("llvmpipe")).unwrap_err();
        assert!(err.contains("software adapter"), "{err}");
    }

    #[test]
    fn a_test_asks_for_the_integrated_gpu_unless_the_variable_names_one() {
        // `Asked::integrated` is `from_env` with `integrated` for an unset variable; the
        // environment is the process's, so this checks the substitution rather than a value.
        let asked = Asked::integrated();
        match Asked::from_env().adapter {
            Some(set) => assert_eq!(asked.adapter.as_deref(), Some(set.as_str())),
            None => assert_eq!(asked.adapter.as_deref(), Some(INTEGRATED)),
        }
        let list = [rtx3090(), uhd770(), llvmpipe()];
        assert_eq!(choose(&list, &named(INTEGRATED)), Ok(1));
    }

    #[test]
    fn a_backend_other_than_vulkan_or_metal_is_never_taken() {
        let gl = AdapterInfo {
            name: "Mesa Intel(R) Graphics (RPL-S)".to_owned(),
            vendor: 0x8086,
            ..AdapterInfo::new(DeviceType::IntegratedGpu, Backend::Gl)
        };
        assert_eq!(choose(&[gl.clone(), uhd770()], &none()), Ok(1));
        let err = choose(&[gl], &named("intel")).unwrap_err();
        assert!(err.contains("not Vulkan or Metal"), "{err}");
        let metal = AdapterInfo {
            name: "Apple M2".to_owned(),
            ..AdapterInfo::new(DeviceType::IntegratedGpu, Backend::Metal)
        };
        assert_eq!(choose(std::slice::from_ref(&metal), &none()), Ok(0));
        assert_eq!(choose(&[metal], &named(INTEGRATED)), Ok(0));
    }

    #[test]
    fn a_choice_says_how_it_was_made() {
        let choice = |asked| Choice {
            offered: vec![uhd770(), rtx3090()],
            chosen: 1,
            asked,
        };
        assert_eq!(choice(none()).in_use().name, "NVIDIA GeForce RTX 3090");
        assert!(choice(none()).how().starts_with("the strongest"));
        assert_eq!(choice(named("intel")).how(), "SUPERSILVIA_ADAPTER=intel");
        assert!(
            choice(software())
                .how()
                .ends_with("SUPERSILVIA_SOFTWARE_GPU=1")
        );
        assert_eq!(kind(DeviceType::IntegratedGpu), "integrated");
    }

    #[test]
    fn nothing_offered_says_so() {
        let err = choose(&[], &none()).unwrap_err();
        assert!(err.contains("none"), "{err}");
    }
}
