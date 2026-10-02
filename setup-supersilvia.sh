#!/usr/bin/env bash
# setup-supersilvia.sh — prepare a machine to build, test and run supersilvia.
#
# Run from a clone of the repository:
#     bash setup-supersilvia.sh
#
# Idempotent: safe to re-run. Installs nothing globally except Rust tooling via
# rustup/cargo and (optionally) system packages via apt/brew with your consent.
#
# What it does:
#   1. Rust toolchain >= 1.92, rustfmt, clippy
#   2. system deps for native egui + the Vulkan renderer
#      - immutable/ostree hosts (Aurora, Bluefin, Silverblue, Kinoite): assembles the
#        `supersilviabox` distrobox from the committed distrobox.ini and re-runs itself inside
#        it. You then do all work inside `distrobox enter supersilviabox`.
#      - mutable Fedora: dnf.  Debian/Ubuntu: apt.  macOS: Xcode CLT check.
#   3. cargo tools: cargo-insta (snapshot review)
#   4. clones silvia, supersilvia's predecessor and its behavioral reference, next to the repo
#   5. runs scripts/doctor.sh

set -euo pipefail

MIN_RUST="1.92.0"

# Pinned tool versions. These are the ones this environment is known to work with; the
# script installs exactly these, and scripts/doctor.sh (which parses these same lines)
# warns when what is on PATH drifts from them. Bump deliberately, then re-run both.
PIN_CARGO_INSTA="1.48.0"

SCRIPT_DIR="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
# distrobox.ini declares the container: image, packages, init hooks, userns. It is the
# single source of truth for both the box and (on plain Fedora) the dnf package list.
BOX_INI="${BOX_INI:-$SCRIPT_DIR/distrobox.ini}"

bold()  { printf '\033[1m%s\033[0m\n' "$*"; }
ok()    { printf '  \033[32m✓\033[0m %s\n' "$*"; }
warn()  { printf '  \033[33m!\033[0m %s\n' "$*"; }
die()   { printf '  \033[31m✗\033[0m %s\n' "$*" >&2; exit 1; }
have()  { command -v "$1" >/dev/null 2>&1; }

# ---------------------------------------------------------------------------
bold "0. Sanity"
# ---------------------------------------------------------------------------
cd "$SCRIPT_DIR"
[[ -f Cargo.toml ]] || die "no Cargo.toml beside this script — run it from a clone of the repository"

OS="$(uname -s)"
case "$OS" in
    Linux)  PLATFORM=linux ;;
    Darwin) PLATFORM=mac ;;
    *)      PLATFORM=other; warn "untested OS: $OS" ;;
esac
ok "platform: $PLATFORM"

# Immutable host?  (Aurora/Bluefin/Silverblue/Kinoite all boot via ostree.)
IN_CONTAINER=0; [[ -n "${CONTAINER_ID:-}" || -f /run/.containerenv ]] && IN_CONTAINER=1
OSTREE_HOST=0;  [[ $IN_CONTAINER -eq 0 && -f /run/ostree-booted ]] && OSTREE_HOST=1
# The box name comes from the [section] header in distrobox.ini so the two cannot drift.
BOX="${BOX:-$(sed -n 's/^\[\(.*\)\]$/\1/p' "$BOX_INI" 2>/dev/null | head -1)}"
BOX="${BOX:-supersilviabox}"

if [[ $OSTREE_HOST -eq 1 ]]; then
    bold "0b. Immutable host detected → assembling distrobox '$BOX' from distrobox.ini"
    have distrobox || die "distrobox not found. On Aurora/Bluefin: 'ujust' has it, or 'brew install distrobox'."
    [[ -f "$BOX_INI" ]] || die "$BOX_INI not found — it is what declares the box's contents."
    # `assemble create` prints "already exists" and returns 0 when the box is present, so
    # re-running this script is a no-op. To pick up an edit to distrobox.ini, rebuild:
    #     distrobox assemble create --replace --file distrobox.ini
    distrobox assemble create --file "$BOX_INI"
    distrobox list 2>/dev/null | grep -q " $BOX " \
        || die "distrobox assemble produced no box named '$BOX' — check $BOX_INI"
    ok "distrobox '$BOX' ready (declared by $BOX_INI)"
    echo "  re-running this script inside the box …"
    exec distrobox enter "$BOX" -- bash "$(readlink -f "$0")"
fi
[[ $IN_CONTAINER -eq 1 ]] && ok "running inside container '${CONTAINER_ID:-?}' (shares \$HOME and the display with the host)"

# ---------------------------------------------------------------------------
bold "1. Rust toolchain"
# ---------------------------------------------------------------------------
if ! have rustup; then
    warn "rustup not found — installing (https://rustup.rs)"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile default
    # shellcheck disable=SC1090
    source "$HOME/.cargo/env"
fi
rustup update stable >/dev/null
rustup default stable >/dev/null

RUST_VER="$(rustc --version | awk '{print $2}')"
if [[ "$(printf '%s\n%s\n' "$MIN_RUST" "$RUST_VER" | sort -V | head -1)" != "$MIN_RUST" ]]; then
    die "rustc $RUST_VER < $MIN_RUST (eframe 0.36 / edition 2024 need it). Run: rustup update"
fi
ok "rustc $RUST_VER"

rustup component add rustfmt clippy || die "rustup component add rustfmt clippy failed"
ok "rustfmt + clippy"

# ---------------------------------------------------------------------------
bold "2. System dependencies (before cargo tools: rustc needs a linker)"
# ---------------------------------------------------------------------------
if [[ "$PLATFORM" == linux ]]; then
    if have dnf; then
        # Fedora, either as the host (non-immutable) or as the assembled distrobox. The
        # package list lives in distrobox.ini so the box and a bare Fedora install cannot
        # disagree about what supersilvia needs.
        if [[ -f "$BOX_INI" ]]; then
            mapfile -t PKGS < <(sed -n 's/^additional_packages[[:space:]]*=[[:space:]]*//p' "$BOX_INI" \
                                | tr ' ' '\n' | sed '/^$/d')
            [[ ${#PKGS[@]} -gt 0 ]] || die "no additional_packages found in $BOX_INI"
            ok "package list: ${#PKGS[@]} packages from distrobox.ini"
        else
            die "$BOX_INI not found — it holds the dnf package list."
        fi
        echo "  dnf install (sudo is passwordless inside distrobox) …"
        sudo dnf install -y -q "${PKGS[@]}" || die "dnf install failed (see above)"
        ok "dnf packages"

        # --- GPU video decode ---------------------------------------------------------
        # Mirrors the init_hooks in distrobox.ini; the long comment there explains why
        # --allowerasing belongs on the ffmpeg swap and must never touch the mesa one.
        # This path is what a non-immutable Fedora host uses, where there is no box.
        FEDORA_VER="$(rpm -E %fedora)"
        if ! rpm -q rpmfusion-free-release >/dev/null 2>&1; then
            echo "  enabling RPM Fusion (free + nonfree) for hardware video decode …"
            for v in "$FEDORA_VER" "$((FEDORA_VER - 1))"; do
                if sudo dnf install -y -q \
                    "https://mirrors.rpmfusion.org/free/fedora/rpmfusion-free-release-${v}.noarch.rpm" \
                    "https://mirrors.rpmfusion.org/nonfree/fedora/rpmfusion-nonfree-release-${v}.noarch.rpm"
                then ok "rpmfusion repos (f${v})"; break
                else warn "no RPM Fusion release rpm for Fedora ${v}"; fi
            done
        fi
        rpm -q rpmfusion-free-release >/dev/null 2>&1 \
            || warn "RPM Fusion absent — VA-API will lack H264/HEVC"

        # NEVER --allowerasing here. mesa-va-drivers-freeworld Provides mesa-va-drivers, so
        # the solver substitutes it; granting erasure instead lets it delete every dependent,
        # which in testing removed 280 packages including glx-utils, egl-utils and gtk3-devel.
        if rpm -q mesa-va-drivers >/dev/null 2>&1; then
            sudo dnf swap -y -q mesa-va-drivers mesa-va-drivers-freeworld \
                || warn "mesa-va-drivers-freeworld swap failed"
        else
            sudo dnf install -y -q mesa-va-drivers-freeworld \
                || warn "mesa-va-drivers-freeworld install failed"
        fi
        sudo dnf install -y -q mesa-vdpau-drivers-freeworld >/dev/null 2>&1 \
            || echo "  no mesa-vdpau-drivers-freeworld for this release; VDPAU is legacy, supersilvia uses VA-API"

        # The VA-API driver is per vendor, and all of them come from RPM Fusion. Install every
        # one: each is inert on hardware it does not drive, libva picks by PCI ID at run time,
        # and it is what lets one setup work on AMD, Intel and NVIDIA. Mirrors distrobox.ini.
        sudo dnf install -y -q intel-media-driver || warn "intel-media-driver (Intel Gen8+ iHD) install failed"
        sudo dnf install -y -q libva-intel-driver >/dev/null 2>&1 \
            || echo "  no libva-intel-driver (pre-Gen8 i965 fallback) for this release; skipping"
        sudo dnf install -y -q libva-nvidia-driver >/dev/null 2>&1 \
            || echo "  no libva-nvidia-driver; NVIDIA decodes through NVDEC without it"

        # ffmpeg-libs both Provides and Conflicts with Fedora's libavcodec-free /
        # libswresample-free / libavutil-free. Nothing can substitute for those, so this swap
        # cannot resolve without --allowerasing — and because ffmpeg-libs provides what the
        # dependents need, the erasure stops at the -free packages themselves.
        sudo dnf swap -y -q libavcodec-free ffmpeg-libs --allowerasing \
            || sudo dnf swap -y -q ffmpeg-free ffmpeg --allowerasing \
            || warn "full ffmpeg swap failed"
        sudo dnf install -y -q ffmpeg ffmpeg-devel || warn "ffmpeg / ffmpeg-devel install failed"
        ok "VA-API freeworld + ffmpeg"
    elif have apt-get; then
        PKGS=(
            build-essential pkg-config clang
            libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libwayland-dev libssl-dev
            mesa-vulkan-drivers vulkan-tools libgl1-mesa-dri xvfb
            libgtk-3-dev libasound2-dev libfontconfig1-dev
        )
        read -r -p "  Install apt packages for native egui + Vulkan? [Y/n] " yn
        if [[ "${yn:-Y}" =~ ^[Yy]$ ]]; then
            sudo apt-get update -qq && sudo apt-get install -y -qq "${PKGS[@]}" && ok "apt packages"
        else warn "skipped — native build/tests may fail until these are installed"; fi
    else
        warn "unknown package manager; install the Fedora/Debian equivalents listed in this script"
    fi
elif [[ "$PLATFORM" == mac ]]; then
    have xcode-select && xcode-select -p >/dev/null 2>&1 && ok "Xcode CLT present" \
        || warn "run: xcode-select --install"
fi

# ---------------------------------------------------------------------------
bold "3. cargo tools"
# ---------------------------------------------------------------------------
LOG="$(mktemp -t supersilvia-setup.XXXXXX.log)"
run_logged() {   # description, then the command. Prints the log tail and dies on failure.
    local what="$1"; shift
    if "$@" >>"$LOG" 2>&1; then ok "$what"; else
        echo; tail -n 30 "$LOG" | sed 's/^/    /'
        die "$what failed — full log: $LOG"
    fi
}
have cc || die "no C linker (cc). System deps step should have installed gcc — check above."

# cargo-binstall fetches published release binaries instead of compiling. Seconds, not minutes,
# and it sidesteps stale C deps in some tools' lockfiles.
if ! have cargo-binstall; then
    echo "  installing cargo-binstall (prebuilt) …"
    run_logged "cargo-binstall" bash -c \
        'curl -L --proto "=https" --tlsv1.2 -sSf https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash'
fi
# Installed version of a pinned tool, or "" when absent.
tool_version() {
    case "$1" in
        cargo-insta) have cargo-insta && cargo insta --version 2>/dev/null | awk '{print $2}' ;;
    esac
}

# Install an exact version. Prebuilt first; unlocked source build as fallback.
# --locked is deliberately not used: some tools ship lockfiles pinning C deps that no longer
# build with current GCC. Prebuilt binaries are the reliable path.
cargo_install_pinned() {   # bin, crate, version
    local bin="$1" crate="$2" want="$3" cur
    # Both lines below must not fail under `set -e`, and on a fresh box both would:
    # tool_version returns nonzero when the tool is absent, and the `[[ -n ]] &&` guard
    # returns nonzero when it is. Either one aborted the whole script before step 4.
    cur="$(tool_version "$bin" || true)"
    if [[ "$cur" == "$want" ]]; then ok "$bin $cur (pinned)"; return; fi
    if [[ -n "$cur" ]]; then warn "$bin $cur installed, replacing with pinned $want"; fi
    echo "  installing $crate@$want (prebuilt via binstall) …"
    if cargo binstall -y --no-confirm --force "${crate}@${want}" >>"$LOG" 2>&1; then
        ok "$bin $want"
    else
        warn "no prebuilt $bin $want; compiling from source (unlocked; minutes. log: $LOG)"
        run_logged "$bin $want" cargo install --force --version "$want" "$crate"
    fi
}
cargo_install_pinned cargo-insta cargo-insta "$PIN_CARGO_INSTA"

# ---------------------------------------------------------------------------
bold "4. silvia reference checkout"
# ---------------------------------------------------------------------------
if [[ -d ../silvia/.git ]]; then
    (cd ../silvia && git pull -q --ff-only) && ok "../silvia updated"
else
    git clone -q https://github.com/figrita/silvia ../silvia && ok "../silvia cloned"
fi

# ---------------------------------------------------------------------------
bold "5. Verify"
# ---------------------------------------------------------------------------
# scripts/doctor.sh is the single definition of "this environment is usable". It reads
# the PIN_* lines above, so setup and doctor cannot disagree about what should be here.
if [[ -x "$SCRIPT_DIR/scripts/doctor.sh" ]]; then
    "$SCRIPT_DIR/scripts/doctor.sh" || die "doctor.sh reported failures — the environment is not ready"
else
    warn "scripts/doctor.sh not found; cannot verify the environment"
fi
if [[ "$PLATFORM" == linux ]] && have xvfb-run; then
    # A software Vulkan device exists ONLY as a CI fallback for headless tests, behind
    # SUPERSILVIA_SOFTWARE_GPU=1. Never use it for real work; doctor.sh fails on it.
    printf '  %-14s %s\n' "CI fallback" "$(env -u DISPLAY -u WAYLAND_DISPLAY vulkaninfo --summary 2>/dev/null | grep -A3 'PHYSICAL_DEVICE_TYPE_CPU' | sed -n 's/^[[:space:]]*deviceName[[:space:]]*=[[:space:]]*//p' | head -1)"
fi

cat <<'EOF'

Done. Next:

  distrobox enter supersilviabox           # (immutable hosts) ALL work happens in here
  cd <your clone of supersilvia>
  ./check.sh                               # the gate: doctor, fmt, clippy, tests
  cargo run                                # the app; its windows open on your desktop

  Tip: `distrobox-export --bin ~/.cargo/bin/cargo-insta` etc. for the host PATH.
  CONTRIBUTING.md has the rules and the commands; DEVSETUP.md explains the box.

EOF
