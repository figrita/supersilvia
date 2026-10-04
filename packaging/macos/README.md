# macOS

`build-app.sh` makes `dist/supersilvia.app`, a `.dmg` with an Applications link and a `.zip`,
for Apple Silicon Macs on macOS 14.2 or later that have nothing installed — no Homebrew, no
GStreamer — and beside them the source offer the LGPL asks of a download that carries
GStreamer, in one archive ([below](#the-source-offer)). What a tester reads is [TESTERS.md](TESTERS.md).

```sh
packaging/macos/build-app.sh
# dist/supersilvia.app
# dist/supersilvia-<version>-arm64.dmg
# dist/supersilvia-<version>-arm64.zip
# dist/gstreamer-1.28.7-source.tar
```

It needs Xcode's command-line tools, rustup's toolchain and a network connection the first
time. It installs nothing, needs no `sudo`, and never reads Homebrew's GStreamer, so the
development machine's stays exactly as it is. The first run downloads about 900 MB into
`~/Library/Caches/supersilvia-packaging/` (`SUPERSILVIA_PACKAGING_CACHE` moves it) and builds
the `dist` profile from scratch in `target/app/`; later runs reuse both.

| | |
| --- | --- |
| `build-app.sh` | the whole build, from the download to the `.zip` |
| `sources.sh` | GStreamer's source and cerbero's recipes, checked, and `SOURCE-OFFER.txt`, in one archive beside the download; `build-app.sh` runs it last |
| `Info.plist` | the bundle's; the script writes `Cargo.toml`'s version into its copy |
| `entitlements.plist` | what the hardened runtime is asked to allow, below |
| `Credits.rtf` | the standard About panel's text: the licences in brief and the NDI® line |
| `LGPL-2.1.txt` | GStreamer's licence, which the bundle must carry |
| `TESTERS.md` | the page a tester reads |

Every Rust crate in the binary and its licence text is written into the bundle by
[`scripts/crate-licenses.py`](../../scripts/crate-licenses.py) for `aarch64-apple-darwin`, the
same script that writes Linux's notices. The Mac's Help menu has no Licences entry:
`platform::macos::menu` answers Help's About and Licences with the standard About panel, whose
`Credits.rtf` names the folder below.

## The bundle

```
supersilvia.app/Contents/
  Info.plist
  MacOS/supersilvia
  Resources/supersilvia.icns            assets/icon/, rendered by scripts/make-icons.py
  Resources/Credits.rtf
  Resources/licenses/                   AGPL, Syphon's BSD, LGPL 2.1, what GStreamer carries,
                                        every Rust crate's licence, the repository's licenses/
  Frameworks/Syphon.framework           vendor/syphon, without its headers
  Frameworks/GStreamer/lib/             the libraries, @rpath/<name>
  Frameworks/GStreamer/lib/plugins/     the plugins; lib/gstreamer-1.0 is a symlink to it
  Frameworks/GStreamer/libexec/scanner/gst-plugin-scanner; libexec/gstreamer-1.0 likewise
```

The two symlinks are for codesign: it takes any folder inside `Frameworks` whose name has a
dot for a bundle, and refuses to sign the app because `gstreamer-1.0` is not one. It does not
enter a symlink, and GStreamer follows it.

Everything is arm64 alone. The NDI runtime is **never** bundled: its licence is the user's to
accept, from ndi.video, and `gst-plugin-ndi` is registered inside the binary and opens
`libndi.dylib` at run time. `Info.plist`'s `LSEnvironment` sets `NDI_RUNTIME_DIR_V6` to
`/usr/local/lib`, where NDI's installers put it, so a Finder launch, and a hardened build whose
library search may not reach that folder for a bare name, open it by its full path.

## GStreamer

**The official macOS release, unpacked, not installed.** The script downloads GStreamer's own
runtime and development `.pkg`s for the version it names, checks them against the SHA-256
written in it, and unpacks them with `pkgutil --expand-full` into the cache: every runtime
payload, and from the development ones only the `.pc` files and `pkg-config`. The binary is
built with `PKG_CONFIG` and `PKG_CONFIG_LIBDIR` pointing there and nowhere else, so a library
the release lacks fails the build rather than being found in Homebrew. The release's `.pc`
files are relocatable (`prefix=${pcfiledir}/../..`), and every library in it is named
`@rpath/<file>`, so the binary links them by that name; `build.rs` gives it the rpath
`@executable_path/../Frameworks/GStreamer/lib`. The development build is untouched: Homebrew's
libraries are linked by absolute path and never read that rpath.

**Why the release and not Homebrew's.** Homebrew's libraries name each other by absolute
`/opt/homebrew` paths, a few hundred of them, all of which would have to be rewritten with
`install_name_tool` and then kept rewritten on every `brew upgrade`; the release is built to be
moved. It carries every element the app makes — `applemedia` and `osxaudio` included — at the
same version Homebrew has, 1.28.7.

**What is carried.** `ELEMENTS` in the script is the inventory: every element `src/` names or
builds in a pipeline string, and the demuxers, parsers and decoders `decodebin` reaches for the
containers `nodes::Accepts::VIDEO` and `AUDIO` let in. Each element's plugin is carried, with
`typefindfunctions` and `pbtypes`, which no element names; then every library any of them links,
followed until nothing new appears. An element the release lacks stops the build. Add an
element there when `src/` makes a new one.

**Nothing is set at start-up.** GStreamer finds itself: `gst_init` asks `dladdr` where
`libgstreamer-1.0.0.dylib` was loaded from, scans `gstreamer-1.0/` beside it for plugins, and
runs `../libexec/gstreamer-1.0/gst-plugin-scanner` from there (`gstregistry.c` and
`gstpluginloader.c` in 1.28.7), which is why the libraries keep the release's `lib`/`libexec`
shape inside `Frameworks/GStreamer`, through the symlinks above. No environment variable is set and no `unsafe` was added.
The registry is GStreamer's usual cache, `~/.cache/gstreamer-1.0/registry.aarch64.bin`; on a
machine that also runs Homebrew's GStreamer the two rescan each other's plugins once when the
other runs, which costs a few seconds and nothing else. The Text node's `textoverlay` draws
through pango's CoreText backend, so the system's fonts are what it sees and fontconfig is
never configured.

**What the script proves before it signs.** The release's `gst-inspect-1.0` and
`gst-launch-1.0` are copied into the bundle for the length of the check and run from there,
with a registry of their own: the plugin scanner is found by relocation alone, every element
in `ELEMENTS` loads from the bundle, `avfdeviceprovider` is there, dyld reports no library
loaded from outside the bundle and the system, and `textoverlay` draws text with a system
font. Then every Mach-O in the bundle is audited with `otool`: arm64 alone, linking only
`/usr/lib`, `/System` and `@rpath`, with no absolute rpath and no mention of `/opt/homebrew`,
`/usr/local` or `/Library/Frameworks`.

## The source offer

The bundle carries GStreamer's LGPL libraries, so their source goes with every download of it.
`sources.sh`, the last step of `build-app.sh`, puts one archive beside the `.dmg` and the `.zip`,
`gstreamer-1.28.7-source.tar`, uncompressed since everything in it is compressed already, and
holding a folder of the same name with:

| | |
| --- | --- |
| `gstreamer`, `gst-plugins-base`, `-good`, `-bad`, `-ugly` and `gst-libav`, each `-1.28.7.tar.xz` | GStreamer's own release tarballs for every module a carried plugin or library comes from, from gstreamer.freedesktop.org/src, each checked against the SHA-256 written in the script — the one each is published with |
| `cerbero-1.28.7.tar.gz` | cerbero, the build system that made the release, at its tag 1.28.7, checked against the commit written in the script: the recipe of every other library in the release, naming its exact source and checksum, and every patch cerbero applies to one |
| `SOURCE-OFFER.txt` | what the files are, their checksums, and that the app changes none of GStreamer's |

**No patch is applied to the bundled GStreamer.** The files are the release's; the build keeps
each one's arm64 half with `lipo -thin` and signs it, neither of which changes its code, and
edits no install name but the app binary's own rpaths. Cerbero's recipes for the six modules
name the same tarballs by the same checksums and carry no patch for them, which is what makes
the six tarballs the carried code's source. The script runs anywhere with bash, curl, git and
`shasum` or `sha256sum`, and keeps what it downloads in the packaging cache beside the release.
A GStreamer bump changes `GST_VERSION` in both scripts, the six checksums and the cerbero commit
together; `sources.sh` refuses a version it has no checksums for.

## Signing

**Ad hoc by default.** Every library, the scanner and `Syphon.framework` are signed, then the
app with `entitlements.plist`, and `codesign --verify --deep --strict` checks the result. An
ad-hoc signature is enough for an Apple Silicon Mac to run the app once the person allows it
(TESTERS.md). macOS remembers a camera or microphone grant against the signature, and an ad-hoc
signature is a hash of this build, so every build asks again.

**With a Developer ID**, nothing is restructured:

```sh
xcrun notarytool store-credentials supersilvia-notary \
  --apple-id <apple id> --team-id <team id> --password <app-specific password>   # once
SUPERSILVIA_SIGN_IDENTITY="Developer ID Application: <name> (<team id>)" \
SUPERSILVIA_NOTARY_PROFILE=supersilvia-notary \
  packaging/macos/build-app.sh
```

The identity signs everything under the hardened runtime with a secure timestamp. The notary
profile then notarizes the app through a zip and staples it, builds the `.dmg` from the stapled
app, signs, notarizes and staples that, makes the zip again, and ends with `spctl --assess`.
Without the profile the build is signed but not notarized, which Gatekeeper still refuses.

**The entitlements**, which the hardened runtime reads and an ad-hoc build is not held to:

| | why |
| --- | --- |
| `device.camera` | AVFoundation's cameras, the iPhone's included |
| `device.audio-input` | a microphone, and the process tap that is System audio |
| `cs.allow-jit` | ORC, which `videoconvert`, `videoscale` and `audioconvert` compile their loops with at run time; without it ORC says *Failed to create write and exec mmap regions* and falls back to plain C, correct and several times slower |
| `cs.disable-library-validation` | `libndi.dylib`, the NDI runtime the user installs, is signed by NDI's team and not ours, and library validation refuses it in a hardened process; NDI's SDK documentation asks for the same. Nothing else needs it: every library the bundle carries is signed with the app's own identity |

No sandbox: supersilvia reads and writes project folders anywhere, which the sandbox would
turn into a security-scoped bookmark for every one.

## Info.plist

Every capability macOS asks the person about has its sentence, or macOS ends the app the moment
it reaches for it: the camera, `NSCameraUseContinuityCameraDeviceType` for an iPhone, the
microphone, `NSAudioCaptureUsageDescription` for the process tap, a screen (ScreenCaptureKit's
picker should need no grant; the sentence is there for a macOS that asks), and for NDI
`NSLocalNetworkUsageDescription` with `NSBonjourServices` holding `_ndi._tcp`, the service NDI's
SDK documentation names for discovery on Apple platforms
([Platform Considerations](https://docs.ndi.video/all/developing-with-ndi/sdk/platform-considerations)).
`LSMinimumSystemVersion` is 14.2, the first macOS with Core Audio's process taps, as
`.cargo/config.toml`'s deployment target is ([docs/decisions.md](../../docs/decisions.md)).

## What is left

- Tried on a Mac that has never had Homebrew or GStreamer, by a person.
- The third-party libraries' own tarballs — glib, pango, cairo, fribidi, FFmpeg, mpg123 and
  the rest the release is built with — are named by cerbero's recipes and mirrored at
  gstreamer.freedesktop.org/src/mirror/, not carried beside the download. Carrying them means
  reading each carried library's recipe for its source.
- `CFBundleDocumentTypes` for `.ssw`, so Finder opens a workspace in supersilvia.
