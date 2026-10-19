# Spout between processes

supersilvia's Spout, checked against itself and against the Spout SDK's own code on a Windows
machine, by [`../../../.github/workflows/spout.yml`](../../../.github/workflows/spout.yml) on a
push to `spout` or a `spout-*` branch, or from the Actions tab.

| file | what it is |
| --- | --- |
| `spout_peer.cpp`, `CMakeLists.txt` | `spout_peer_sdk.exe`: the peer contract on the Spout2 SDK's `spoutDX` class, built from the SDK's source with MSVC |
| `build-peers.sh` | on Linux, after `../build.sh`: supersilvia's peer (`examples/spout_peer.rs`) and `tests/syphon.rs` cross-compiled into the Windows folder's `bin/` |
| `interop.sh` | on Windows, in Git Bash: the test, then every pairing of the two peers |

## The peer contract

Both peers take the same arguments:

```text
spout_peer send    <sender> <width> <height> <seconds>
spout_peer receive <sender> <width> <height> <timeout-seconds>
```

`send` publishes a test pattern under that sender name for that long. `receive` waits for the
sender, reads its frames and exits 0 on a match, 1 otherwise, printing what it saw. The
pattern, top row first, 8 bits a channel: pixel (x, y) is R = round(255·x/(w−1)),
G = round(255·y/(h−1)), B = 64, A = 255, and a receiver matches within ±2 a channel.

`spout_peer_sdk.exe` sends a `B8G8R8A8_UNORM` texture with `spoutDX::SendTexture` about 30
times a second. It receives with `spoutDX::ReceiveTexture`, copies the received texture to the
CPU itself, reads `B8G8R8A8`, `B8G8R8X8` and `R8G8B8A8` (and their sRGB forms) by their own
byte order, and passes once two reads in a row match and `GetSenderList` names the sender. Its
lines start `spout_peer_sdk send:` or `spout_peer_sdk receive:`, and it prints every DXGI
adapter, the one its device is on, and that adapter's LUID alone on a line, `adapter-luid
<16 hex digits>`, as supersilvia's peer does. A command line it cannot read exits 2. Two
environment variables steer it:

- `SPOUT_PEER_LUID=<hex>` makes its Direct3D 11 device on that adapter. A Spout texture opens
  only on its sender's adapter, and a machine without a GPU lists two Basic Render Driver
  adapters, one of them not marked software, which supersilvia never takes. Unset, the device
  is on the default adapter, as a Spout app's is, or on WARP where that fails.
- `SPOUT_PEER_FRAMECOUNT=1` or `0` turns Spout's frame counting on or off through
  `spoutFrameCount::SetFrameCount`, which writes `HKCU\Software\Leading Edge\Spout\Framecount`:
  every Spout app this user starts afterwards reads it. With it on, `receive` also requires the
  sender's frame count to have advanced.

## What a run proves

On `windows-2025`, which has no GPU and draws on WARP, with `SUPERSILVIA_SOFTWARE_GPU=1` and
`SUPERSILVIA_ADAPTER="Basic Render"`, `interop.sh` runs, each process under a time limit and
every case whatever the one before did:

1. **`tests/syphon.rs`**, the Windows half of the loopback test.
2. **supersilvia → supersilvia**: a sender and a receiver of ours, two processes.
3. **Spout SDK → Spout SDK**, the control: if this fails, the machine cannot share a texture
   between processes, and no other case says anything.
4. **supersilvia → Spout SDK**, frame counting off, then 5. on: what we publish is a sender
   every Spout app finds, lists, opens and reads the right way up, in the right channels, and
   with frame counting on, one whose count advances.
6. **Spout SDK → supersilvia**, frame counting off, then 7. on: we find, open and read a sender
   made by Spout's own code, counted or not.

The job fails when any receiver exits non-zero or any sender does not send. Each case's output
is printed in a group of its own and kept as the `spout-logs` artifact; the summary lists each
case's verdict.

Not proved: a real GPU, two GPUs (a sender on one and a receiver on the other), an app running
elevated or in another session, and Spout 2.006 apps.

The SDK is [leadedge/Spout2](https://github.com/leadedge/Spout2) at the tag **2.007.018**,
commit `6b68a5b4`, its newest release; the workflow checks out that commit, so a moved tag
moves nothing.

## By hand on Windows

The folder, with the two executables in `bin/`, is the `spout-windows` artifact of a run, or
on Linux:

```sh
SUPERSILVIA_PROFILE=release packaging/windows/build.sh
SUPERSILVIA_PROFILE=release packaging/windows/spout-interop/build-peers.sh
```

`spout_peer_sdk.exe` needs Visual Studio 2022 with its C++ workload and CMake, and the SDK:

```bat
git clone --branch 2.007.018 --depth 1 https://github.com/leadedge/Spout2
cmake -S packaging\windows\spout-interop -B build\spout-interop -A x64 -DSPOUT2_DIR=%CD%\Spout2
cmake --build build\spout-interop --config Release
```

Then one pairing, in two terminals, the receiver started within the sender's 60 seconds:

```bat
supersilvia\bin\spout_peer.exe send "spout test" 301 167 60
build\spout-interop\Release\spout_peer_sdk.exe receive "spout test" 301 167 20
```

On a machine with more than one adapter, give the SDK's peer the adapter supersilvia's printed
first: `set SPOUT_PEER_LUID=<hex>` in `cmd`, `$env:SPOUT_PEER_LUID = "<hex>"` in PowerShell.
The whole matrix runs in Git Bash:

```sh
packaging/windows/spout-interop/interop.sh supersilvia/bin build/spout-interop/Release/spout_peer_sdk.exe
```

Spout's own demo sender and receiver, from its [releases](https://github.com/leadedge/Spout2/releases),
show either peer's sender by eye: a ramp from black to red left to right, and to green top to
bottom, over a blue of 64.
