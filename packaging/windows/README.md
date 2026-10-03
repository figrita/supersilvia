# Windows

A folder that runs on 64-bit Windows, and the `.zip` that carries it, cross-compiled from
Linux:

```sh
packaging/windows/build.sh                     # dist/windows/supersilvia/ and its .zip
SUPERSILVIA_PROFILE=release packaging/windows/build.sh   # quicker, for trying it
packaging/windows/wine.sh --check              # the folder's supersilvia.exe, under Wine
```

**The toolchain** is Microsoft's: `cargo-xwin` downloads the MSVC C runtime and the Windows
SDK the first time (accepting their licence for you, `XWIN_ACCEPT_LICENSE`) and builds with
`clang-cl` and `lld-link`. `build.sh` names every tool it needs and stops if one is missing.
The renderer is wgpu on Direct3D 12, whose HLSL is compiled by the DirectX Shader Compiler,
linked into the binary whole by `mach-dxcompiler-rs` (about 22 MB of it), and that library
takes Microsoft's ATL with it, which cargo-xwin leaves out unless asked: so `build.sh` keeps a
CRT and SDK of its own, with ATL, under `~/.cache/supersilvia-windows/xwin`. A `cargo xwin
clippy` links nothing and needs neither.

**GStreamer** is the official MSVC release, at the version the macOS bundle uses. It ships
only as an Inno Setup installer, so `build.sh` installs it once into a Wine prefix of its own
under `~/.cache/supersilvia-windows` and builds against that copy through a link without the
space in `Program Files`. pkg-config is pointed at the release's `.pc` files alone, so the
machine's own GStreamer never reaches the build.

**The folder** is GStreamer's own relocatable layout: `bin/` holds `supersilvia.exe` and the
DLLs it and the plugins import, `lib/gstreamer-1.0/` the plugins and
`libexec/gstreamer-1.0/` the plugin scanner, so GStreamer finds everything from where
`gstreamer-1.0-0.dll` is and the app sets nothing at start-up. Every plugin of the release is
carried for now; the macOS bundle's list of the plugins the app's elements are in is the next
step. `licenses/` holds supersilvia's licence, GStreamer's, the shader compiler's, every Rust
crate's and the list of GStreamer files carried.

**The Visual C++ runtime** (`VCRUNTIME140.dll` and its siblings) is not in the folder. Both
the binary and GStreamer's DLLs import it, and Wine supplies its own, but a clean Windows
machine needs Microsoft's *Visual C++ Redistributable* (x64) installed until the folder
carries it.

**The icon.** `build.rs` writes a resource script naming `assets/icon/supersilvia.ico` and the
version, compiles it with `llvm-rc` (or the SDK's `rc` on Windows), and links the `.res` into
the executable, so Explorer shows the icon and the Details tab the version. The window's own
icon, which the taskbar and Alt-Tab use, is `src/main.rs`'s `ViewportBuilder::with_icon`.

**Under Wine** the app draws on Direct3D 12 through vkd3d, Wine's own, over the machine's
Vulkan driver, so what picks a GPU on Linux picks it there: `SUPERSILVIA_ADAPTER`, and
`VK_DRIVER_FILES` to limit which drivers Vulkan loads. `wine.sh` passes both through. vkd3d
lists adapters only through Wine's own DXGI, so where the system's Wine installs DXVK's in its
place, as Fedora's does, `wine.sh` puts Wine's in the prefix as a native library, prefers it,
and has wined3d under it find the GPU through Vulkan rather than OpenGL. No video is decoded
under Wine: it has no Direct3D 12 video decoder, so `--check` warns of it.

**The installer** — MSI, NSIS, or the zip as it is — is not made yet.
