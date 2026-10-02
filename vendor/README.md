# vendor/

## wgpu-core

`wgpu-core/` is [wgpu-core 30.0.1](https://crates.io/crates/wgpu-core/30.0.1) exactly as
published on crates.io, `.cargo_vcs_info.json` and `Cargo.toml.orig` included, with **one
change**: in `src/device/resource.rs`, `Surface::configure` accepts
`PollStatus::WaitSucceeded` from its wait for the device to go idle as it accepts
`PollStatus::QueueEmpty`, where 30.0.1 refuses the configuration with `GpuWaitTimeout`. A
comment beginning `supersilvia:` marks the arm. `Cargo.toml`'s `[patch.crates-io]` points the
build at this copy; it is not a workspace member, so `cargo fmt` and clippy do not read it.

**Why.** `WaitSucceeded` means another thread submitted during the wait, and on supersilvia's
one device the synth thread submits all the time. So every surface configure raced it — the
editor's at launch and on a resize, each picture window's — and wgpu's default error handler
panics on the refusal: about four launches in ten crashed on the Mac. Accepting it is safe
because a surface is used by one thread only and nothing another thread submits references
its textures. `tests/gpu_surface.rs` fails without the change. The decision and what lost are
in [docs/decisions.md](../docs/decisions.md#wgpu-core-carries-one-patch-so-a-surface-configures-beside-the-synth).

**Delete it** on the first wgpu-core release that contains
[gfx-rs/wgpu#10296](https://github.com/gfx-rs/wgpu/pull/10296), the same fix upstream: remove
the `[patch.crates-io]` table and this directory, move the lock to that release
(`cargo update -p wgpu-core` if it is a 30.0.x), and check that `tests/gpu_surface.rs` still
passes.

**Never edit it otherwise.** To see the one change:
`diff -r ~/.cargo/registry/src/*/wgpu-core-30.0.1 vendor/wgpu-core`, which also lists
`.cargo-ok`, the marker cargo leaves in its own extracted copy.

## Syphon

`syphon/Syphon.framework` is [Syphon](https://syphon.info/)'s framework built from
[Syphon-Framework](https://github.com/Syphon/Syphon-Framework) at commit
`f476167` on `main` (*Small fix so setting a SyphonServer.name correctly copies the new name*),
**arm64 only**, for macOS 14.2 and later, ad-hoc signed, with its licence beside it as
`syphon/LICENSE` (BSD). It is built from source because Syphon's last binary release (SDK 5,
2019) has no Metal and `main` has had it since 2023; `proposals/syphon.md` has the rulings.

`build.rs` links it on macOS alone: the framework search path is this folder, and the binary
carries two rpaths for it, `@executable_path/../Frameworks` for the `.app` and this folder for
`cargo run` and `cargo test`. `packaging/macos/build-app.sh` copies it into the bundle without
its headers and removes the second rpath there. Linux builds never read it.

**Rebuild it** with `syphon/build-framework.sh <checkout>`, from a clone of Syphon-Framework
at the commit above. The script compiles every source file with clang, the shaders with
`metal` and `metallib`, and links with the project's own export list, reproducing the Xcode
project's Release settings; it does not use `xcodebuild`, which will not start on a machine
whose Xcode first-launch resources are stale. With a working `xcodebuild`, the project's own
`Syphon` scheme with `ARCHS=arm64 ONLY_ACTIVE_ARCH=NO MACOSX_DEPLOYMENT_TARGET=14.2` builds
the same thing. Its install name is `@rpath/Syphon.framework/Versions/A/Syphon`.

**Move it** only to a later commit of `main`, rebuilt the same way, and check that
`tests/syphon.rs` still passes.
