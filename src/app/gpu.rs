// SPDX-License-Identifier: AGPL-3.0-or-later

//! The GPU a start asks for: each of the environment's two variables over Preferences ▸
//! Performance ▸ GPU, and that over the rule, the strongest GPU the machine offers.
//!
//! `render::adapter` holds the rule and takes the setting as an [`Asked`]; this is where a
//! preference meets it, since nothing under `render/` reads one. `SUPERSILVIA_ADAPTER` and
//! `SUPERSILVIA_SOFTWARE_GPU` keep winning because scripts and tests pin the integrated GPU
//! with the first: a setting saved on a machine never moves a run that names its adapter.

use crate::preferences::Preferences;
use crate::render::adapter::{AdapterId, Asked, Choice};

/// The two answers Preferences ▸ Performance ▸ GPU keeps: the adapter, `None` for Automatic,
/// and whether a software one may be taken.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Setting {
    pub adapter: Option<AdapterId>,
    pub software: bool,
}

impl Setting {
    /// The setting as these preferences hold it.
    pub fn of(prefs: &Preferences) -> Self {
        Self {
            adapter: prefs.gpu.clone(),
            software: prefs.allow_software_gpu,
        }
    }
}

/// What a start asks of the rule: whatever the environment sets, and the setting where it sets
/// nothing. `SUPERSILVIA_ADAPTER` set leaves no adapter pinned; `SUPERSILVIA_SOFTWARE_GPU` set
/// decides the software answer, `0` as much as `1`.
pub fn asked(env: &Asked, setting: &Setting) -> Asked {
    Asked {
        adapter: env.adapter.clone(),
        software: env.software_env.unwrap_or(setting.software),
        software_env: env.software_env,
        pinned: if env.adapter.is_some() {
            None
        } else {
            setting.adapter.clone()
        },
    }
}

/// What a start says when the adapter the setting names is not the one it renders on: not on
/// this machine, or a software one with no software allowed. `None` where nothing was passed
/// over.
pub fn fallback(choice: &Choice) -> Option<String> {
    let pin = choice.pin_dropped()?;
    let why = if choice.offered.iter().any(|info| pin.is(info)) {
        "is a software GPU and Allow a software GPU is off"
    } else {
        "is not on this machine"
    };
    Some(format!(
        "the GPU chosen in Preferences, {}, {why}: rendering on {} instead",
        pin.name,
        choice.in_use().name
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::wgpu::{AdapterInfo, Backend, DeviceType};

    fn info(name: &str, kind: DeviceType) -> AdapterInfo {
        AdapterInfo {
            name: name.to_owned(),
            ..AdapterInfo::new(kind, Backend::Vulkan)
        }
    }

    fn intel() -> AdapterInfo {
        info("Intel(R) Graphics (RPL-S)", DeviceType::IntegratedGpu)
    }

    fn nvidia() -> AdapterInfo {
        info("NVIDIA GeForce RTX 3090", DeviceType::DiscreteGpu)
    }

    fn llvmpipe() -> AdapterInfo {
        info("llvmpipe (LLVM 22.1.8, 256 bits)", DeviceType::Cpu)
    }

    fn env(adapter: Option<&str>, software: Option<bool>) -> Asked {
        Asked {
            adapter: adapter.map(str::to_owned),
            software: software == Some(true),
            software_env: software,
            pinned: None,
        }
    }

    fn setting(adapter: Option<&AdapterInfo>, software: bool) -> Setting {
        Setting {
            adapter: adapter.map(AdapterId::of),
            software,
        }
    }

    /// The environment over the setting over the rule, for each of the two answers.
    #[test]
    fn the_environment_wins_over_the_setting_and_the_setting_over_automatic() {
        let pinned = setting(Some(&intel()), true);

        let neither = asked(&env(None, None), &Setting::default());
        assert_eq!(neither, Asked::default(), "nothing set is the rule");

        let set = asked(&env(None, None), &pinned);
        assert_eq!(set.pinned, Some(AdapterId::of(&intel())));
        assert!(set.software, "the setting allows software");

        let named = asked(&env(Some("3090"), None), &pinned);
        assert_eq!(named.adapter.as_deref(), Some("3090"));
        assert_eq!(named.pinned, None, "SUPERSILVIA_ADAPTER wins");
        assert!(
            named.software,
            "and leaves the software answer to the setting"
        );

        let refused = asked(&env(None, Some(false)), &pinned);
        assert!(
            !refused.software,
            "SUPERSILVIA_SOFTWARE_GPU=0 wins over the setting"
        );
        assert_eq!(refused.pinned, Some(AdapterId::of(&intel())));

        let allowed = asked(&env(None, Some(true)), &Setting::default());
        assert!(allowed.software, "SUPERSILVIA_SOFTWARE_GPU=1 wins too");
    }

    /// What the rule takes for each, over a machine of three.
    #[test]
    fn what_each_answer_renders_on() {
        use crate::render::adapter::choose;
        let machine = [intel(), nvidia(), llvmpipe()];
        let on = |env: Asked, setting: Setting| choose(&machine, &asked(&env, &setting));
        assert_eq!(on(env(None, None), Setting::default()), Ok(1), "automatic");
        assert_eq!(on(env(None, None), setting(Some(&intel()), false)), Ok(0));
        assert_eq!(
            on(env(Some("3090"), None), setting(Some(&intel()), false)),
            Ok(1),
            "the variable over the setting"
        );
        assert_eq!(on(env(None, None), setting(Some(&llvmpipe()), true)), Ok(2));
        assert_eq!(
            on(env(None, Some(false)), setting(Some(&llvmpipe()), true)),
            Ok(1),
            "software refused by the variable"
        );
    }

    /// An adapter the setting names that is not there is passed over for the strongest, and
    /// the start says which and why.
    #[test]
    fn a_setting_naming_an_adapter_not_there_falls_back_and_says_so() {
        use crate::render::adapter::choose;
        let choice = |offered: Vec<AdapterInfo>, setting: Setting| {
            let asked = asked(&env(None, None), &setting);
            let chosen = choose(&offered, &asked).unwrap();
            Choice {
                offered,
                chosen,
                asked,
            }
        };
        let radeon = info("AMD Radeon RX 7900 XTX", DeviceType::DiscreteGpu);

        let gone = choice(vec![intel(), nvidia()], setting(Some(&radeon), false));
        assert_eq!(gone.chosen, 1, "Automatic");
        assert_eq!(
            fallback(&gone).as_deref(),
            Some(
                "the GPU chosen in Preferences, AMD Radeon RX 7900 XTX, is not on this \
                 machine: rendering on NVIDIA GeForce RTX 3090 instead"
            )
        );

        let soft = choice(vec![intel(), llvmpipe()], setting(Some(&llvmpipe()), false));
        assert_eq!(soft.chosen, 0);
        assert!(
            fallback(&soft)
                .unwrap()
                .contains("is a software GPU and Allow a software GPU is off")
        );

        let there = choice(vec![intel(), nvidia()], setting(Some(&intel()), false));
        assert_eq!((there.chosen, fallback(&there)), (0, None));
        let automatic = choice(vec![intel(), nvidia()], Setting::default());
        assert_eq!(fallback(&automatic), None);
    }
}
