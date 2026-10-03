# Proposal: the rest of Windows

**Status: proposed.** What is built is in [docs/rendering.md](../docs/rendering.md),
[docs/media.md](../docs/media.md), `src/platform/windows/` and
[packaging/windows/README.md](../packaging/windows/README.md); this is what is left between
that and a Windows release, in the order it should be done, with the few choices that are the
owner's marked **Ruling**.

## Where it stands

supersilvia builds for Windows from Linux: `packaging/windows/build.sh` cross-compiles with
cargo-xwin against GStreamer's official MSVC release and stages a relocatable folder and a
`.zip`. Every service in `src/platform/` has a Windows answer. The renderer is wgpu on Direct3D
12, its HLSL compiled by a DXC linked into the binary. A clip decoded by GStreamer's Direct3D 12
decoders is sampled where the decoder wrote it, with the renderer's queue waiting on
GStreamer's fence.

**All of it has run under Wine and none of it on Windows.** Under Wine 11, `--check` passes,
the editor renders on Direct3D 12 through vkd3d, and `tests/gpu_d3d12.rs` and the GPU suites
pass but for the four failures named below. Wine cannot prove the rest: vkd3d has no Direct3D
12 video decoding, makes a new device on every call rather than one per adapter, and shares no
texture by handle, so a clip's frames reach the renderer as bytes there.

## Stage 1: what is known broken

Each of these is small and can be checked under Wine.

1. **Saving a project fails on Windows** with *Access denied (os error 5)*, which is three of
   `tests/gpu_app.rs`' failures. Two calls in `src/project.rs` are what Windows refuses:
   `sync_dir` opens a folder with `File::open`, which Windows allows only with
   `FILE_FLAG_BACKUP_SEMANTICS`, and `write_painting` syncs a file opened for reading, where
   `FlushFileBuffers` needs a handle that can write. The file is opened for writing to be
   synced, and a folder is not synced on Windows, whose file systems journal a rename's
   entries themselves; `sync_dir` says so beside its Unix body. Done when the three tests pass
   under Wine.
2. **`cargo fmt --check` fails on `main`**, in `src/platform/linux/filedrop.rs` and
   `src/ui/start.rs`. One `cargo fmt`.
3. **A file dragged onto the window.** winit hears it through OLE on Windows, so
   `platform::windows::filedrop` is empty. Two things are unknown: whether it arrives at all,
   which Wine's X11 driver can show (its Wayland driver has no drag and drop, which is why
   `wine.sh` now chooses X11), and where it lands. winit 0.30 tells egui nothing of the pointer
   while the shell holds a drag, so a drop is expected to land where the pointer last was, not
   under it. If so, the Windows `FileDrop` reads the cursor with `GetCursorPos` and
   `ScreenToClient` while egui holds hovered files, and hands it in as `Drag::Moved`, which is
   what the Linux half does with Wayland's position.
4. **The harness's adapter test under Wine.** vkd3d reports an integrated GPU as discrete,
   so `the_harness_renders_on_the_igpu` fails there. It is Wine's answer, not Windows': the
   test is made to accept the adapter `SUPERSILVIA_ADAPTER` names when it is set.
5. **`create_factory_media failed: 0x80004002`** is logged at start-up under Wine: wgpu asks
   for `IDXGIFactoryMedia`, which Wine's DXGI lacks and Windows has. It changes nothing. It is
   left alone, and named in `packaging/windows/README.md` so it is not chased.

## Stage 2: a folder a stranger can run

6. **The Visual C++ runtime.** The binary and every GStreamer DLL import `VCRUNTIME140.dll`;
   Wine has its own, and a clean Windows does not. The options:
   - *Require the redistributable*, as now: one more download and a cryptic error without it.
   - *Carry it in `bin/`*: `vcruntime140.dll`, `vcruntime140_1.dll` and `msvcp140.dll` beside
     the binary, which Microsoft's redistribution terms allow, and which works for the `.zip`
     and any installer alike. On Linux they come from installing `vc_redist.x64.exe` into the
     build's Wine prefix once, as GStreamer is, and copying them out.
   - *Have the installer run the redistributable*: needs an installer, and administrator rights.
   - *A static C runtime*: possible for our binary, not for GStreamer's DLLs, so it does not
     answer.

   **Proposed: carry it.** `build.sh` checks they are the real Microsoft files and not Wine's.
7. **Only the plugins the app uses.** The folder is 315 MB because it carries every plugin of
   the release. `build-app.sh` on the Mac carries the plugins its elements are in and the
   libraries they reach; Windows does the same from `platform::windows::check::GROUPS` and the
   clip formats, mapping each element to its plugin with the release's own `gst-inspect-1.0`
   run under Wine, and stopping the build when one is missing.
8. **The folder tries itself.** The last step of `build.sh` runs the staged `supersilvia.exe
   --check` under Wine with nothing of the build's environment, so a missing DLL or plugin fails
   the build rather than a tester's first launch, as `build-app.sh` tries the bundle.
9. **An installer.** **Ruling.** The options:
   - *The `.zip` alone*: unzip and run. No Start menu, no `.ssw` association, no uninstall.
   - *An MSI made with `wixl`* (msitools, already on the build machine): a per-user install
     into `%LOCALAPPDATA%\Programs`, a Start menu entry, `.ssw` opening in supersilvia, and an
     entry in *Installed apps*, built on Linux with no Wine.
   - *NSIS*: `makensis` runs natively on Linux; a scripted `.exe`, the most flexible and the
     most code.
   - *Inno Setup*: what GStreamer itself ships; needs Wine to build.
   - *MSIX*: the Store's format; needs signing, and its sandbox fights GStreamer's plugin
     scanner and registry cache.

   **Proposed: an MSI from `wixl`, beside the `.zip` for those who want it portable.**
10. **Signing.** **Ruling.** An unsigned download gets SmartScreen's *Windows protected your
    PC*, passed with *More info ▸ Run anyway*. The options are to stay unsigned for testers
    and say so in a `TESTERS.md`; an OV or EV certificate; or Microsoft's Trusted Signing, a
    subscription. Whichever certificate, `osslsigncode` signs the binary and the MSI on
    Linux. **Proposed: unsigned with a `packaging/windows/TESTERS.md` until a public release,
    then Trusted Signing.**

## Stage 3: Windows itself

Nothing below can be learned under Wine. **Ruling: which Windows machine** — a tester's, a
machine of the owner's, or a rented cloud GPU machine; each wants the release folder and this
list, and an hour.

- **It starts.** On NVIDIA, AMD and Intel, the clean machine of stage 2: `--check`, then the
  editor. The log says `Dx12` and the adapter the app chose.
- **Picture windows.** Open, move, the resize band, `Shift` resize, `F` and double-click
  fullscreen, `Escape` twice, minimize the editor and the window keeps moving. Their DXGI
  swapchains are made and presented on a thread that is not the window's, which Wine allows
  and Windows may not.
- **Zero-copy.** An H.264, HEVC and AV1 clip each plays with `video import` reporting the
  Direct3D 12 decoder, and Help ▸ Status showing no upload. That checks four things Wine could
  not: the decoder's device being the renderer's, NV12 as one texture with plane views, a
  chroma view's `GetDimensions` answering half the size, which the conversion shader assumes,
  and the shared-handle fallback on a machine with two GPUs.
- **Encoders.** A clip's cache written with each vendor's encoder plays back frame-accurate.
  The Quick Sync, AMF and Direct3D 12 rows ask for every frame a keyframe with
  `gop-size=1`; that the encoder honours it is unchecked.
- **Cameras.** Listed by name, opened, and found again by device path after a replug and a
  reboot. What a webcam's MJPEG decodes with.
- **Audio.** Microphones listed and heard; the loopback hears what is playing.
- **Screen capture.** The primary monitor captured, the pointer in it.
- **MIDI.** A controller heard, unplugged and replugged with no Rescan; held by another app,
  said once in the log.
- **A game controller**, through gilrs' Windows backend.
- **NDI®**, with NDI 6 Runtime installed: a source received, an Output sent.
- **Files.** Open and save dialogs, *Show in File Explorer*, a `.ssw` and a picture dragged
  in, and where it lands (stage 1, item 3). A project saved into Documents under *Controlled
  folder access* says what `DENIED_HINT` says.
- **The Text node** lists the installed fonts and draws in them.
- **Scale and screens.** 100, 150 and 200% display scaling; a window dragged between screens
  of different scales.
- **The keyboard.** The shortcut legend's keys, and an AltGr layout typing into a text field.

## Stage 4: Windows on every change

There is no CI today, so this is one part of deciding whether there is any. **Ruling.** If
there is, Windows wants two jobs: the cross build and Windows clippy on Linux, which needs
nothing new, and `cargo test` on a hosted Windows runner, built natively against the same
GStreamer release, where the GPU tests run on WARP, the software Direct3D 12 adapter every
Windows has. The harness pins the integrated GPU today and is taught to take WARP when asked.

## Stage 5: Windows' own features

Each is its own proposal when it comes; this is the list, so none is forgotten.

- **Choosing what the screen capture shows.** Only the primary monitor is captured. Windows'
  own picker, `GraphicsCapturePicker`, is what the portal is on Linux and
  `SCContentSharingPicker` on the Mac: the system's dialog, a monitor or a window. Its frames
  are already Direct3D textures, so `d3d12screencapturesrc` feeding the clip's import path
  makes a screen zero-copy too.
- **The GPU's busy share** in the Status box, which reads "— %" today: PDH's *GPU Engine*
  counters, filtered to this process's 3D engine, which is what the Linux half reads from
  DRM.
- **Spout**, which is Windows' Syphon. Spout 2 shares Direct3D 11 textures by legacy handles,
  which Direct3D 12 cannot open, so it needs a Direct3D 11-on-12 bridge on the renderer's
  device. The largest item here.
- **Hardware MJPEG** for webcams, if the release has a Direct3D 12 or Media Foundation
  decoder for it, preferred as Linux prefers VA-API's.
- **Several apps on one MIDI device** through Windows MIDI Services, where WinMM gives a
  device to one app at a time.
- **One application's sound**, through WASAPI's process loopback, as the Mac's process tap.
- **Windows on ARM**, which GStreamer and cargo-xwin both build for.

## Risks

- `mach-dxcompiler-rs`, the DXC linked in, calls itself experimental and signs DXIL itself.
  If it fails on Windows, the fallback is the dynamic DXC, two DLLs in `bin/`.
- The decoders' rank is raised for the renderer's adapter process-wide, which is right while
  every decode is ours.
- Wine and Windows differ exactly where this port is least tested — devices, handles,
  swapchains off the window's thread. Stage 3 is not optional.

## The rulings, in short

1. **Installer**: an MSI built on Linux, plus the zip — or the zip alone, NSIS or Inno Setup.
2. **Signing**: unsigned for testers now and Trusted Signing at release — or a certificate now.
3. **A Windows machine** for stage 3: a tester's, the owner's, or a rented one.
4. **CI**: whether there is any; Windows' jobs follow from that.
5. **Order within stage 5**: proposed screen choice, then the GPU share, then Spout.
