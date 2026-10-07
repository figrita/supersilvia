# Trying supersilvia on Linux

Thank you for testing. This build is early. It is one file, an AppImage, that runs without
being installed and changes nothing on your system. What it does not carry is the media
framework and the drivers, which every video app on Linux takes from the system: you install
those from your distribution once, below, and `--check` tells you whether anything is missing.

**This is an alpha.** Projects you save with it may not open in the next build: the file
format can still change without warning, and nothing converts old files. Keep anything you
care about as a render or a Snap as well.

Questions and news are on the supersilvia Discord, https://discord.gg/3RMcCnHbf. Bugs go in its
bug report channel, https://discord.gg/prRuJ6rvy.

**You need**

- A 64-bit PC (x86_64).
- A distribution from 2024 or later: **Ubuntu 24.04** or newer, **Debian 13**, **Fedora 40**
  or newer, or **Arch**. The AppImage needs glibc 2.39 and GStreamer 1.24 or newer, so Ubuntu
  22.04 and Debian 12 are too old.
- A GPU with a **Vulkan** driver: Intel or AMD with Mesa, which your distribution installs.
  Software rendering is refused.
- A **Wayland** session — KDE Plasma or GNOME, which most distributions default to — for the
  pop-out and fullscreen picture windows. Under X11 the editor opens but those windows do not.

## Install what it uses

One command for your distribution. It is GStreamer and its plugins, PipeWire's GStreamer plugin
for screen capture, and the Vulkan driver.

**Ubuntu 24.04 and later, Debian 13:**

```sh
sudo apt install gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-plugins-bad \
  gstreamer1.0-plugins-ugly gstreamer1.0-libav gstreamer1.0-pipewire gstreamer1.0-pulseaudio \
  mesa-vulkan-drivers mesa-va-drivers
```

On an Intel GPU also install `intel-media-va-driver-non-free` (on Ubuntu, enable the
*multiverse* repository first: `sudo add-apt-repository multiverse`).

**Fedora:**

```sh
sudo dnf install gstreamer1-plugins-base gstreamer1-plugins-good gstreamer1-plugins-bad-free \
  gstreamer1-plugins-ugly-free gstreamer1-plugin-libav pipewire-gstreamer mesa-vulkan-drivers
```

Fedora leaves the H.264 and HEVC codecs out, and importing a video file needs your GPU's video
engine to encode one. Enable [RPM Fusion](https://rpmfusion.org/Configuration), free and
nonfree, then:

```sh
sudo dnf swap ffmpeg-free ffmpeg --allowerasing            # H.264 and HEVC files
sudo dnf swap mesa-va-drivers mesa-va-drivers-freeworld    # AMD
sudo dnf install intel-media-driver                        # Intel
```

RPM Fusion's [multimedia page](https://rpmfusion.org/Howto/Multimedia) has the rest.

**Arch:**

```sh
sudo pacman -S gstreamer gst-plugins-base gst-plugins-good gst-plugins-bad gst-plugins-ugly \
  gst-libav gst-plugin-pipewire gst-plugin-va vulkan-icd-loader
sudo pacman -S vulkan-intel intel-media-driver   # Intel
sudo pacman -S vulkan-radeon                     # AMD
```

**NVIDIA** is untested. supersilvia renders on the strongest GPU the machine has — a discrete
card, NVIDIA or AMD, before an integrated one — so a machine with an NVIDIA card renders on it
by default. To render on another GPU, start it with `SUPERSILVIA_ADAPTER` in front of the
command (below): `SUPERSILVIA_ADAPTER=integrated` for the integrated GPU, or a piece of a GPU's
name, such as `intel`, `radeon` or `nvidia`. `--check` names the GPU it picked. Install
`gstreamer1.0-plugins-bad` / `gst-plugins-bad` for the NVENC codecs.

## Run it

1. Download `supersilvia-<version>-linux-x86_64.AppImage`.
2. Make it executable — in a terminal in the folder you downloaded it to:

   ```sh
   chmod +x supersilvia-*-linux-x86_64.AppImage
   ```

   or in the file manager: right-click it, **Properties ▸ Permissions ▸ Allow executing as a
   program**.
3. **Run the check first:**

   ```sh
   ./supersilvia-*-linux-x86_64.AppImage --check
   ```

4. Then open it by double-clicking, or from the terminal without `--check`.

If supersilvia cannot start — there is no GPU it can use, or its window does not open — a box
says why and where its log is. The box needs `zenity` (GNOME has it) or `kdialog` (KDE has
it); without either, only the terminal and the log say it.

If it prints a line about **FUSE** and stops, install the `fuse3` package, or run it without
FUSE by adding `--appimage-extract-and-run` straight after the file name
(`./supersilvia-*-linux-x86_64.AppImage --appimage-extract-and-run --check`).

## What `--check` says

It asks your system everything supersilvia uses, without opening a window, and prints a line
for each. On an Ubuntu 24.04 machine with an Intel GPU and the packages above, the start of it
looks like this:

```
supersilvia 0.9.0-alpha.3 --check

  PASS  GStreamer         1.24.2
  PASS  pipelines         all 16
  PASS  video import      all 5
  PASS  PNG and JPEG      all 3
  PASS  Text node         textoverlay
  PASS  cameras           v4l2src
  PASS  microphones       pulsesrc
  PASS  screen capture    pipewiresrc
  PASS  zero-copy video   vapostproc
  PASS  clip formats      all 24
  PASS  hardware codec    h264: vah264enc and vah264dec
  PASS  GPU               Intel(R) Graphics (RPL-S) (8086:a780, IntegratedGpu, Intel open-source Mesa driver …, Vulkan)
  PASS  session           Wayland
  ...
```

- **PASS** — there.
- **WARN** — missing, and supersilvia runs without it, with one feature off: the line says
  which, and the plugin set in brackets is the package to install (`gst-plugins-good` is
  `gstreamer1.0-plugins-good` on Ubuntu, `gstreamer1-plugins-good` on Fedora and
  `gst-plugins-good` on Arch).
- **FAIL** — supersilvia cannot run until it is fixed. The check then exits with 1.

The lines that matter most:

| line | what it means |
| --- | --- |
| **GStreamer**, **pipelines** | the media framework and the elements every picture goes through. A FAIL here is a missing `gst-plugins-base` |
| **hardware codec** | importing a video file re-encodes it on your GPU's video engine, through VA-API. A WARN means video files cannot be imported until the VA-API driver above is installed; everything else works |
| **video import**, **clip formats** | the containers and decoders for files you import; a WARN names which formats are off |
| **cameras**, **microphones**, **screen capture** | the Main Input's three kinds of source |
| **GPU** | the graphics card supersilvia draws on, or every card it was offered and why each was refused |
| **session** | Wayland, or what X11 costs |
| **Vulkan loader**, **window libraries** | libraries opened while running rather than at start |
| **NDI® runtime** | only needed for NDI, below |

Paste the whole output into a bug report.

If the AppImage will not start at all — not even `--check` — the terminal says which library
it could not find. It needs these from your system: GStreamer 1.24 or newer (`libgstreamer`,
`libgstbase`, `libgstapp`, `libgstaudio`, `libgstvideo`, `libgstpbutils`, `libgstallocators`),
GLib, ALSA (`libasound`), udev (`libudev`) and fontconfig, all of which a desktop has once the
packages above are in, and glibc 2.39 or newer.

## NDI

To send or receive NDI video, get the free **NDI SDK for Linux** from
[ndi.video](https://ndi.video), which carries the runtime, and copy its `libndi.so.6` into
`/usr/local/lib` or `/usr/lib64` (as root), keeping the name `libndi.so.6` — a symbolic link
to the versioned file is fine. Then restart supersilvia; `--check`'s **NDI® runtime** line
says whether it loads. supersilvia does not include the runtime, because NDI's licence has you
install it yourself. NDI finds other machines through Avahi, which most desktops run.

NDI® is a registered trademark of Vizrt NDI AB. [https://ndi.video](https://ndi.video)

## Where your things are

| | |
| --- | --- |
| projects | `~/Documents/supersilvia/`, one folder each — your desktop's documents folder, by whatever name it has in your language. **Edit ▸ Preferences… ▸ Files** shows it, opens it, and **Change…** picks another. A project saved somewhere else stays where you saved it |
| inside a project | `assets/` the media you imported, `renders/` your renders, `snaps/` your Snaps, `cache/` the re-encoded copies of your clips (made again if you delete it), `.autosave/` unsaved changes (below) |
| preferences | `~/.config/supersilvia/preferences.json`. Preferences ▸ Files opens it; edit it with supersilvia closed, since it writes its own copy while it runs. A file it cannot read is kept as `preferences.json.bad` |
| the log | `~/.local/share/supersilvia/logs/`: `supersilvia.log` for the run going on, `previous.log` for the one before. `--check` names it |
| GStreamer's plugin list | `~/.cache/gstreamer-1.0/`, shared with every GStreamer app |

**Project ▸ Show project folder** opens the folder of the project you are in. After a Snap or
a render, the note at the foot of the window that says where it went has a **Show** that
opens the folder with the file in it. The **file** line at the foot of the Status box's Project section
(Preferences ▸ Performance ▸ Show the Status box) keeps a **▸ show** that does the same.

## If supersilvia closes unexpectedly

The next time it starts, it says so first — **supersilvia closed unexpectedly** — with the
reason its log gives, if it wrote one, and **Show log**, which opens the folder with that log
selected.

While a project has unsaved changes, supersilvia writes them every 30 seconds into the
project's own `.autosave/` folder, and a real Save deletes it. The next time that project
opens — and supersilvia opens your last project when it starts — it asks whether to recover
the changes, saying how old they are. **Recover** puts them on screen, still unsaved, so save
to keep them; **Discard** throws them away. At most the last 30 seconds of work are lost.

## Sending a bug report

Choose **Help ▸ Report a problem…**. A small window asks **What's up?** — say what you did,
what you expected and what happened instead — and, if you like, your name or Discord handle. Under that it shows what it adds by itself: the
version, your distribution, desktop and GPU, what `--check` says, and the end of the log, and
of the last run's log if that one closed unexpectedly.

Press **Copy report**, then **Open the Discord bug channel**
(https://discord.gg/prRuJ6rvy) and paste it in a new thread, one thread a bug. **Open a
GitHub issue** is the other place to paste it.

If supersilvia cannot start at all, send the output of `--check` and the log from
`~/.local/share/supersilvia/logs/` instead.

If it crashed, the log says the most. On a system with `systemd-coredump`,
`coredumpctl list supersilvia` shows the crash and `coredumpctl info supersilvia` its details.

## Removing it

Delete the AppImage. Nothing else was installed. To remove what it saved too, delete
`~/.config/supersilvia/`, `~/.local/share/supersilvia/` and — only if you do not want your
projects, renders and Snaps —
`~/Documents/supersilvia/`, or the projects folder Preferences ▸ Files names if you changed it.
The packages you installed above are ordinary system packages other apps use as well; leave
them, or remove them with your package manager.
