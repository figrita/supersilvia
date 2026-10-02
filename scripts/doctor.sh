#!/usr/bin/env bash
# doctor.sh — is this environment able to build and run supersilvia?
#
# One line per check, exit nonzero if any FAIL. Runs in about a second.
# check.sh calls this first; run it by hand after rebuilding the distrobox.
#
# On macOS the platform checks are Xcode, GStreamer by pkg-config and Metal in place of
# Vulkan, VA-API and the capture devices, and the fixes named are the Mac's
# (DEVSETUP.md, "macOS"). That branch runs under macOS's own bash 3.2.
#
# Version pins come from setup-supersilvia.sh (PIN_* variables) so there is one
# source of truth. Drift is a WARN, not a FAIL: an unexpected version usually
# still works, but it is the first thing to suspect when something breaks.

set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SETUP="$REPO/setup-supersilvia.sh"

MIN_RUST="1.92.0"
# gstreamer-app and gstreamer-video take the v1_24 feature.
MIN_GST="1.24"
# A device name that is a software rasterizer, whatever type the driver reports it as.
GPU_SOFTWARE_RE='llvmpipe|softpipe|swrast|lavapipe|SwiftShader'

fails=0
warns=0

if [[ -t 1 ]]; then C_OK=$'\033[32m'; C_NO=$'\033[31m'; C_WA=$'\033[33m'; C_Z=$'\033[0m'
else C_OK=""; C_NO=""; C_WA=""; C_Z=""; fi

pass() { printf '  %sPASS%s  %-22s %s\n' "$C_OK" "$C_Z" "$1" "${2-}"; }
fail() { printf '  %sFAIL%s  %-22s %s\n' "$C_NO" "$C_Z" "$1" "${2-}"; fails=$((fails + 1)); }
warn() { printf '  %sWARN%s  %-22s %s\n' "$C_WA" "$C_Z" "$1" "${2-}"; warns=$((warns + 1)); }
have() { command -v "$1" >/dev/null 2>&1; }

ver_ge() { [[ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | head -1)" == "$1" ]]; }

# Load the pinned versions. Only PIN_* assignments are evaluated.
PIN_CARGO_INSTA=""
if [[ -f "$SETUP" ]]; then
    eval "$(grep -E '^PIN_[A-Z_]+="[^"]*"$' "$SETUP")"
fi
# check_pin LABEL PINNED ACTUAL
check_pin() {
    local label="$1" pinned="$2" actual="$3"
    [[ -z "$pinned" ]] && { warn "$label pin" "no PIN_ entry in setup-supersilvia.sh"; return; }
    [[ "$pinned" == "$actual" ]] && return
    warn "$label version drift" "have $actual, setup-supersilvia.sh pins $pinned"
}

# The fixes each shared check names. setup-supersilvia.sh installs all of it on Linux; on a
# Mac it checks only the command line tools, so the commands are spelled out.
darwin=0; [[ "$(uname -s)" == Darwin ]] && darwin=1
if [[ $darwin -eq 1 ]]; then
    fix_rust="not on PATH — run: source ~/.cargo/env, or install rustup (DEVSETUP.md, "macOS")"
    fix_insta="not on PATH — run: cargo install cargo-insta --version $PIN_CARGO_INSTA"
    fix_cc="no C compiler — rustc cannot link; run: xcode-select --install"
else
    fix_rust="not on PATH"
    fix_insta="not on PATH — run setup-supersilvia.sh"
    fix_cc="no C compiler — rustc cannot link"
fi

printf '\nsupersilvia doctor — %s\n' "$(date '+%Y-%m-%d %H:%M:%S')"
printf 'repo: %s\ncontainer: %s\n\n' "$REPO" "${CONTAINER_ID:-none (running on the host)}"

# --- identity ---------------------------------------------------------------------------
# Running as root leaves subuid-owned files in the shared $HOME that the host user cannot
# read or delete. This is what a bare `podman exec` / Dev Containers attach without
# remoteUser does.
if [[ "$(id -u)" -eq 0 ]]; then
    fail "not root" "running as uid 0 — see DEVSETUP.md, set remoteUser in the attached-container config"
else
    pass "not root" "$(id -un) uid=$(id -u)"
fi

# `-ef`, not `!=`: on an atomic Fedora host `/home` is a symlink to `/var/home`, so `$HOME` is
# `/var/home/<user>` while passwd says `/home/<user>` and both name one directory. Comparing
# the strings failed a correct environment, and the workaround — running the gate under a
# corrected `$HOME` — silently repointed `CARGO_HOME` with it and rebuilt every dependency on
# each flip. What this is actually for is a container that did not adopt the host user, and
# that still fails: a different directory is a different inode.
expected_home="$(getent passwd "$(id -un)" 2>/dev/null | cut -d: -f6)"
if [[ "$HOME" == "/root" || "$HOME" == "" ]]; then
    fail "\$HOME" "$HOME — the container did not adopt the host user"
elif [[ ! -d "$HOME" || ! -w "$HOME" ]]; then
    fail "\$HOME" "$HOME is not a writable directory"
elif [[ -n "$expected_home" && ! "$HOME" -ef "$expected_home" ]]; then
    fail "\$HOME" "$HOME but passwd says $expected_home, and they are different directories"
else
    pass "\$HOME" "$HOME"
fi

# --- rust -------------------------------------------------------------------------------
if have rustc; then
    rust_ver="$(rustc --version 2>/dev/null | awk '{print $2}')"
    if ver_ge "$MIN_RUST" "$rust_ver"; then pass "rustc >= $MIN_RUST" "$rust_ver"
    else fail "rustc >= $MIN_RUST" "$rust_ver — run: rustup update stable"; fi
else
    fail "rustc >= $MIN_RUST" "$fix_rust"
fi

# --- cargo tools ------------------------------------------------------------------------
if have cargo-insta; then
    insta_ver="$(cargo insta --version 2>/dev/null | awk '{print $2}')"
    pass "cargo-insta" "$insta_ver"
    check_pin cargo-insta "$PIN_CARGO_INSTA" "$insta_ver"
else
    fail "cargo-insta" "$fix_insta"
fi

# Only a speed-up: setup-supersilvia.sh installs through it on Linux, and nothing uses it on a Mac.
if have cargo-binstall; then
    pass "cargo-binstall" "$(cargo binstall -V 2>/dev/null)"
elif [[ $darwin -eq 0 ]]; then
    fail "cargo-binstall" "not on PATH — prebuilt tool installs will fall back to slow source builds"
fi

# --- linker -----------------------------------------------------------------------------
# rustc cannot link without this. Installing cargo tools before it exists is the failure
# mode setup-supersilvia.sh now guards against.
if have cc; then pass "cc (linker)" "$(cc --version 2>/dev/null | head -1)"
else fail "cc (linker)" "$fix_cc"; fi

if [[ $darwin -eq 1 ]]; then
    # --- Xcode --------------------------------------------------------------------------
    # The command line tools give the linker and clang. Only full Xcode has `metal`, which the
    # MSL compile check needs (DEVSETUP.md, "The MSL check") and nothing else does, so it is a WARN.
    if xcode_dir="$(xcode-select -p 2>/dev/null)"; then pass "xcode-select" "$xcode_dir"
    else fail "xcode-select" "no developer tools — run: xcode-select --install"; fi
    if metal_bin="$(xcrun -f metal 2>/dev/null)"; then
        pass "Metal compiler" "$metal_bin"
    elif [[ -d /Applications/Xcode.app ]]; then
        warn "Metal compiler" "xcrun finds no metal — run: sudo xcode-select -s /Applications/Xcode.app/Contents/Developer"
    else
        warn "Metal compiler" "the command line tools have none — install Xcode from the App Store (DEVSETUP.md, "macOS")"
    fi

    # --- GStreamer ----------------------------------------------------------------------
    # The gstreamer -sys crates find the libraries through pkg-config at build time. Homebrew's
    # .pc files are on pkg-config's default path; the official framework's need
    # PKG_CONFIG_PATH (DEVSETUP.md, "macOS").
    gst_pcs="gstreamer-1.0 gstreamer-video-1.0 gstreamer-app-1.0 gstreamer-pbutils-1.0"
    if ! have pkg-config; then
        fail "GStreamer >= $MIN_GST" "pkg-config missing — run: brew install pkgconf"
    elif ! pkg-config --exists $gst_pcs; then
        fail "GStreamer >= $MIN_GST" "pkg-config finds no $gst_pcs — run: brew install gstreamer"
    else
        gst_ver="$(pkg-config --modversion gstreamer-1.0)"
        if ver_ge "$MIN_GST" "$gst_ver"; then pass "GStreamer >= $MIN_GST" "$gst_ver (pkg-config)"
        else fail "GStreamer >= $MIN_GST" "$gst_ver — run: brew upgrade gstreamer"; fi
    fi

    # The elements the app's pipelines name. A missing one is a status-line error at run
    # time and nothing at build time. avfvideosrc is the camera, from the applemedia plugin.
    if ! have gst-inspect-1.0; then
        fail "GStreamer elements" "gst-inspect-1.0 missing — run: brew install gstreamer"
    else
        gst_missing=""
        for el in videotestsrc decodebin videoconvert appsink mp4mux avfvideosrc; do
            gst-inspect-1.0 "$el" >/dev/null 2>&1 || gst_missing="$gst_missing $el"
        done
        if [[ -z "$gst_missing" ]]; then
            pass "GStreamer elements" "$(gst-inspect-1.0 --version 2>/dev/null | head -1)"
        else
            fail "GStreamer elements" "missing$gst_missing — see DEVSETUP.md, "macOS""
        fi
        # The video node imports through whichever VideoToolbox pair exists, from applemedia,
        # in the order src/platform/macos/video.rs's CODECS prefers. No pair, no import, and
        # layer 1's clip tests transcode through it, as on Linux.
        hw_codec=""
        for pair in "vtenc_h264_hw vtdec_hw h264parse" "vtenc_h265_hw vtdec_hw h265parse"; do
            ok=1
            for el in $pair; do gst-inspect-1.0 "$el" >/dev/null 2>&1 || ok=0; done
            if [[ $ok -eq 1 ]]; then hw_codec="$pair"; break; fi
        done
        if [[ -n "$hw_codec" ]]; then pass "hardware video codec" "$hw_codec"
        else fail "hardware video codec" "no VideoToolbox encoder+decoder pair — video import needs one; see DEVSETUP.md, "macOS""; fi
    fi

    # --- GPU ----------------------------------------------------------------------------
    # Metal is the one backend the app enumerates on macOS. Which adapter it takes is
    # `render::adapter::choose`'s rule, held by the tests; this only asks that there is one.
    metal_gpus="$(system_profiler SPDisplaysDataType 2>/dev/null | awk -F': ' '
        /^[[:space:]]+Chipset Model:/ { n = $2 }
        /^[[:space:]]+Metal( Support| Family)?:/ && n != "" { printf "%s%s (%s)", sep, n, $2; sep = "; "; n = "" }')"
    if [[ -n "$metal_gpus" ]]; then pass "Metal GPU" "$metal_gpus"
    else fail "Metal GPU" "system_profiler lists no GPU with Metal support — the app refuses to start without one"; fi
else
    # Linux: the GPU through Vulkan and /dev/dri, VA-API, GStreamer's Linux elements, the
    # capture devices and the box's packages.
    # --- GPU ----------------------------------------------------------------------------
    render_nodes="$(ls /dev/dri/renderD* 2>/dev/null | tr '\n' ' ')"
    if [[ -n "$render_nodes" ]]; then pass "/dev/dri render node" "$render_nodes"
    else fail "/dev/dri render node" "not visible — the container cannot reach the GPU"; fi

    # The adapter the app renders on, by the rule `render::adapter::choose` applies to wgpu's
    # list, applied here to vulkaninfo's: Vulkan is the one backend the app enumerates on Linux.
    # Never a software device unless SUPERSILVIA_SOFTWARE_GPU=1; otherwise the strongest GPU,
    # a discrete one before an integrated one before anything else, the first listed of a kind.
    # SUPERSILVIA_ADAPTER names one instead: an index into this list, `vendor:device` in hex,
    # `integrated`, or a piece of a device's name, as the app reads it; check.sh sets it to
    # `integrated`, the GPU the tests ask for. vulkaninfo runs with no display in its environment:
    # it needs none to list devices, and with an unreachable one inherited from whatever spawned
    # the box it asks the X server for authorization and lists nothing.
    vk() { env -u DISPLAY -u WAYLAND_DISPLAY vulkaninfo "$@"; }
    vk_gpu=""
    if ! have vulkaninfo; then
        fail "Vulkan adapter" "vulkaninfo missing — install vulkan-tools (see distrobox.ini)"
    else
        mapfile -t vk_devs < <(vk --summary 2>/dev/null | awk '
            function flush() { if (g != "") printf "%s\t%s\t%s\t%s\t%s\n", g, t, v, d, n }
            /^GPU[0-9]+:/ { flush(); g = $1; sub(/^GPU/, "", g); sub(/:$/, "", g); t = v = d = n = "" }
            /^[[:space:]]+deviceType[[:space:]]*=/ { t = $3; sub(/^PHYSICAL_DEVICE_TYPE_/, "", t) }
            /^[[:space:]]+vendorID[[:space:]]*=/ { v = $3 }
            /^[[:space:]]+deviceID[[:space:]]*=/ { d = $3 }
            /^[[:space:]]+deviceName[[:space:]]*=/ { n = $0; sub(/^[^=]*=[[:space:]]*/, "", n) }
            END { flush() }')
        offered="$(for row in "${vk_devs[@]}"; do IFS=$'\t' read -r i t v d n <<<"$row"; printf '%s: %s (%s, %s:%s); ' "$i" "$n" "$t" "$v" "$d"; done)"
        offered="${offered%; }"
        asked="${SUPERSILVIA_ADAPTER:-}"
        software_ok=0; [[ "${SUPERSILVIA_SOFTWARE_GPU:-}" == 1 ]] && software_ok=1
        chosen=""
        if [[ ${#vk_devs[@]} -eq 0 ]]; then
            fail "Vulkan adapter" "vulkaninfo lists no device — the app refuses to start without one"
        elif [[ -n "$asked" ]]; then
            # The override: an index, then vendor:device, then `integrated`, then a piece of
            # the name.
            for row in "${vk_devs[@]}"; do
                IFS=$'\t' read -r i t v d n <<<"$row"
                if [[ "$asked" =~ ^[0-9]+$ && "$asked" -lt ${#vk_devs[@]} ]]; then
                    [[ "$i" == "$asked" ]] && { chosen="$row"; break; }
                elif [[ "$asked" =~ ^(0x)?([0-9a-fA-F]+):(0x)?([0-9a-fA-F]+)$ ]]; then
                    (( 16#${BASH_REMATCH[2]} == v && 16#${BASH_REMATCH[4]} == d )) && { chosen="$row"; break; }
                elif [[ "${asked,,}" == integrated ]]; then
                    [[ "$t" == INTEGRATED_GPU ]] && { chosen="$row"; break; }
                elif [[ "${n,,}" == *"${asked,,}"* ]]; then
                    chosen="$row"; break
                fi
            done
            [[ -z "$chosen" ]] && fail "Vulkan adapter" "SUPERSILVIA_ADAPTER=$asked names nothing; offered: $offered"
        else
            for want in DISCRETE_GPU INTEGRATED_GPU '' CPU; do
                for row in "${vk_devs[@]}"; do
                    IFS=$'\t' read -r i t v d n <<<"$row"
                    [[ "$t" == CPU && $software_ok -eq 0 ]] && continue
                    [[ -n "$want" && "$t" != "$want" ]] && continue
                    [[ -z "$want" && "$t" == CPU ]] && continue
                    chosen="$row"; break 2
                done
            done
            [[ -z "$chosen" ]] && fail "Vulkan adapter" "no adapter the app may use (never software unless SUPERSILVIA_SOFTWARE_GPU=1); offered: $offered"
        fi
        if [[ -n "$chosen" ]]; then
            IFS=$'\t' read -r i t v d n <<<"$chosen"
            if [[ $software_ok -eq 0 ]] && { [[ "$t" == CPU ]] || grep -qE "$GPU_SOFTWARE_RE" <<<"$n"; }; then
                fail "Vulkan adapter" "SOFTWARE RASTERIZER: $n — set SUPERSILVIA_SOFTWARE_GPU=1 only for CI"
            else
                pass "Vulkan adapter" "GPU$i $n (${t,,}, $v:$d)"
                vk_gpu="$i"
            fi
        fi
    fi

    # --- hardware video decode -----------------------------------------------------------
    # A WARN, not a FAIL, and it stays one now that video nodes have landed. The `hardware video
    # codec` check below FAILs on the same underlying gap and names the pair the video node
    # actually needs, so promoting this one would report one missing capability twice under two
    # names. This check earns its place by naming VA-API specifically: an NVIDIA box has perfectly
    # good decode through NVDEC and simply does not answer to vainfo without the
    # nvidia-vaapi-driver bridge, which is a different remedy from installing a Mesa driver.
    if ! have vainfo; then
        warn "VA-API" "vainfo missing — install libva-utils (see distrobox.ini)"
    else
        va="$(vainfo 2>/dev/null)"
        va_missing=()
        for profile in H264 HEVC; do
            grep -q "VAProfile${profile}" <<<"$va" || va_missing+=("$profile")
        done
        if [[ ${#va_missing[@]} -eq 0 ]]; then
            pass "VA-API H264 + HEVC" "$(sed -n 's/^vainfo: Driver version: //p' <<<"$va" | head -1)"
        else
            warn "VA-API H264 + HEVC" "missing ${va_missing[*]} — RPM Fusion freeworld driver (AMD/Intel) or nvidia-vaapi-driver (NVIDIA)"
        fi
    fi

    # --- GStreamer ----------------------------------------------------------------------
    # The camera node is a pipeline string; every element it names has to be installed, and
    # `v4l2src` in particular lives in plugins-good rather than base. A missing element is a
    # status-line error at run time and nothing at build time, so it is checked here.
    if ! have gst-inspect-1.0; then
        fail "GStreamer" "gst-inspect-1.0 missing — install gstreamer1-devel (see distrobox.ini)"
    else
        gst_missing=()
        for el in v4l2src videotestsrc decodebin videoconvert appsink mp4mux; do
            gst-inspect-1.0 "$el" >/dev/null 2>&1 || gst_missing+=("$el")
        done
        if [[ ${#gst_missing[@]} -eq 0 ]]; then
            pass "GStreamer elements" "$(gst-inspect-1.0 --version 2>/dev/null | head -1)"
        else
            fail "GStreamer elements" "missing ${gst_missing[*]} — see distrobox.ini"
        fi
        # The video node imports through whichever hardware codec pair exists, in the order
        # src/video/clip.rs prefers: VA-API on Mesa, NVENC on NVIDIA. No pair, no import.
        hw_codec=""
        for pair in "vah264enc vah264dec h264parse" "vah265enc vah265dec h265parse" "vaav1enc vaav1dec av1parse" \
                    "nvh264enc nvh264dec h264parse" "nvh265enc nvh265dec h265parse" "nvav1enc nvav1dec av1parse"; do
            ok=1
            for el in $pair; do gst-inspect-1.0 "$el" >/dev/null 2>&1 || ok=0; done
            if [[ $ok -eq 1 ]]; then hw_codec="$pair"; break; fi
        done
        if [[ -n "$hw_codec" ]]; then
            pass "hardware video codec" "$hw_codec"
        else
            fail "hardware video codec" "no VA-API or NVENC encoder+decoder pair — video import needs one (see distrobox.ini)"
        fi
    fi

    # --- capture devices -----------------------------------------------------------------
    # The camera and microphone nodes open real devices from inside the box. distrobox shares
    # /dev and the user's session by default, but a box built before those nodes existed, or a
    # host with no camera, gives a node that opens nothing and says so only in its status line.
    # Naming the missing device here is cheaper than finding it mid-set.
    video_devs=(/dev/video*)
    if [[ -e "${video_devs[0]}" ]]; then
        pass "video device" "${video_devs[*]}"
    else
        warn "video device" "no /dev/video* — the camera node will find nothing"
    fi

    # What cpal actually needs is a socket or an ALSA device, not a command-line tool. The first
    # version of this check asked `pactl` and `arecord`, neither of which the box installs, and
    # warned on a machine whose microphone worked perfectly.
    audio_rt="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
    if [[ -S "$audio_rt/pipewire-0" ]]; then
        pass "audio server" "PipeWire ($audio_rt/pipewire-0)"
    elif [[ -e "$audio_rt/pulse/native" ]]; then
        pass "audio server" "PulseAudio ($audio_rt/pulse/native)"
    elif compgen -G "/dev/snd/pcmC*c" >/dev/null; then
        pass "audio server" "ALSA capture device"
    else
        warn "audio server" "no PipeWire or PulseAudio socket and no ALSA capture device — the microphone node will find nothing"
    fi

    # --- zero-copy video ----------------------------------------------------------------------
    # A decoder's frame reaches a texture without a copy when the chosen Vulkan device can import
    # a DMA-BUF with its modifier and vapostproc can export one. Without either the video node
    # falls back to bytes and says so in the overlay, so this is a WARN: slower, not broken.
    if [[ -n "$vk_gpu" ]]; then
        vk_ext="$(vk 2>/dev/null | awk -v g="GPU$vk_gpu:" '
            /^GPU[0-9]+:/ { on = ($1 == g) }
            on && /VK_EXT_external_memory_dma_buf|VK_EXT_image_drm_format_modifier/ { print $1 }' | sort -u | wc -l)"
        if [[ "$vk_ext" -eq 2 ]] && gst-inspect-1.0 vapostproc >/dev/null 2>&1; then
            pass "zero-copy video" "VK_EXT_external_memory_dma_buf + VK_EXT_image_drm_format_modifier + vapostproc"
        else
            warn "zero-copy video" "no DMA-BUF import on GPU$vk_gpu or no vapostproc — video frames will be copied"
        fi
    fi

    # --- declared packages --------------------------------------------------------------
    # distrobox.ini is the package list. Verify the box still has everything it declares: a
    # `dnf swap --allowerasing` for the RPM Fusion freeworld drivers can be solved by removing
    # the dependents instead of substituting, which silently guts the container long before
    # anything notices. init_hooks run after additional_packages, so this is checkable here.
    if ! have rpm; then
        :
    elif [[ ! -f "$REPO/distrobox.ini" ]]; then
        warn "declared packages" "distrobox.ini not found"
    else
        mapfile -t declared < <(sed -n 's/^additional_packages[[:space:]]*=[[:space:]]*//p' \
            "$REPO/distrobox.ini" | tr ' ' '\n' | sed '/^$/d')
        if [[ ${#declared[@]} -eq 0 ]]; then
            warn "declared packages" "no additional_packages in distrobox.ini"
        else
            missing=()
            for pkg in "${declared[@]}"; do
                rpm -q "$pkg" >/dev/null 2>&1 || missing+=("$pkg")
            done
            if [[ ${#missing[@]} -eq 0 ]]; then
                pass "declared packages" "${#declared[@]} from distrobox.ini, all installed"
            else
                fail "declared packages" "missing ${missing[*]} — rebuild: distrobox assemble create --replace --file distrobox.ini"
            fi
        fi
    fi
fi

# ----------------------------------------------------------------------------------------
printf '\n'
if [[ $fails -gt 0 ]]; then
    printf '%sdoctor: %d check(s) FAILED%s, %d warning(s). See DEVSETUP.md.\n\n' "$C_NO" "$fails" "$C_Z" "$warns"
    exit 1
fi
printf '%sdoctor: all checks passed%s (%d warning(s)).\n\n' "$C_OK" "$C_Z" "$warns"
