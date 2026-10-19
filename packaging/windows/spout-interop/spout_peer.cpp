// SPDX-License-Identifier: AGPL-3.0-or-later
//
// spout_peer_sdk: the Spout peer of examples/spout_peer.rs, written against the Spout2 SDK's own
// spoutDX class, so that supersilvia's sender is read and its receiver fed by Spout's reference
// code and not only by itself.
//
//   spout_peer_sdk send    <sender> <width> <height> <seconds>
//   spout_peer_sdk receive <sender> <width> <height> <timeout-seconds>
//
// The pattern, top row first, 8 bits a channel: pixel (x, y) is R = round(255 x / (w - 1)),
// G = round(255 y / (h - 1)), B = 64, A = 255. `send` fills a DXGI_FORMAT_B8G8R8A8_UNORM
// texture with it once and sends it about 30 times a second for that long, then exits 0.
// `receive` waits for the sender, copies the frame it is handed to the CPU after each
// ReceiveTexture and checks it within 2 a channel, and exits 0 once two reads in a row match
// and the sender is in Spout's list of senders; at the timeout it exits 1, printing the last
// thing it saw that did not. A command line it cannot read exits 2.
//
// Environment:
//   SPOUT_PEER_LUID        the adapter to make the Direct3D 11 device on, by its LUID in hex,
//                          (HighPart << 32 | LowPart), as the line `adapter-luid <hex>` of
//                          either peer prints it. A shared texture opens only on its sender's
//                          adapter. Unset, the device is on the default adapter, or on WARP
//                          where there is no default.
//   SPOUT_PEER_FRAMECOUNT  1 turns Spout's frame counting on for this user, 0 off, through
//                          spoutFrameCount::SetFrameCount, which writes the registry value every
//                          Spout app reads at start. With it 1, `receive` also requires the
//                          sender's frame count to have advanced. Unset, the registry's stands.
//
// Every DXGI adapter is printed first, with its flags and LUID. Every line but `adapter-luid`
// starts with "spout_peer_sdk <mode>:".

#include "SpoutDX.h"

#include <dxgi.h>

#include <cerrno>
#include <chrono>
#include <cstdarg>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <thread>
#include <vector>

namespace {

constexpr int kTolerance = 2;
constexpr double kSendFps = 30.0;
constexpr int kMatchesNeeded = 2;

const char* g_mode = "";

void say(const char* format, ...) {
  std::printf("spout_peer_sdk %s: ", g_mode);
  va_list args;
  va_start(args, format);
  std::vprintf(format, args);
  va_end(args);
  std::printf("\n");
  std::fflush(stdout);
}

struct Args {
  std::string name;
  unsigned width = 0;
  unsigned height = 0;
  double seconds = 0.0;
};

bool parse_size(const char* text, unsigned& out) {
  char* end = nullptr;
  errno = 0;
  const unsigned long value = std::strtoul(text, &end, 10);
  if (errno != 0 || end == text || *end != '\0' || value == 0 || value > 16384) return false;
  out = static_cast<unsigned>(value);
  return true;
}

bool parse_seconds(const char* text, double& out) {
  char* end = nullptr;
  errno = 0;
  const double value = std::strtod(text, &end);
  if (errno != 0 || end == text || *end != '\0' || !(value > 0.0) || value > 86400.0) return false;
  out = value;
  return true;
}

// Hex digits, with or without 0x.
bool parse_luid(const char* text, uint64_t& out) {
  char* end = nullptr;
  errno = 0;
  const unsigned long long value = std::strtoull(text, &end, 16);
  if (errno != 0 || end == text || *end != '\0') return false;
  out = value;
  return true;
}

// The environment variable `name`, or null where it is unset or empty.
const char* env(const char* name) {
  const char* value = std::getenv(name);
  return value && *value ? value : nullptr;
}

// round(255 i / (n - 1)) in integers; 0 where there is one row or column.
unsigned ramp(unsigned i, unsigned n) {
  if (n < 2) return 0;
  return (2u * 255u * i + (n - 1)) / (2u * (n - 1));
}

struct Rgba {
  unsigned r, g, b, a;
};

Rgba expected(unsigned x, unsigned y, const Args& args) {
  return {ramp(x, args.width), ramp(y, args.height), 64, 255};
}

const char* format_name(DXGI_FORMAT format) {
  switch (format) {
    case DXGI_FORMAT_B8G8R8A8_UNORM: return "B8G8R8A8_UNORM";
    case DXGI_FORMAT_B8G8R8A8_UNORM_SRGB: return "B8G8R8A8_UNORM_SRGB";
    case DXGI_FORMAT_B8G8R8X8_UNORM: return "B8G8R8X8_UNORM";
    case DXGI_FORMAT_R8G8B8A8_UNORM: return "R8G8B8A8_UNORM";
    case DXGI_FORMAT_R8G8B8A8_UNORM_SRGB: return "R8G8B8A8_UNORM_SRGB";
    case DXGI_FORMAT_R10G10B10A2_UNORM: return "R10G10B10A2_UNORM";
    case DXGI_FORMAT_R16G16B16A16_FLOAT: return "R16G16B16A16_FLOAT";
    case DXGI_FORMAT_R16G16B16A16_UNORM: return "R16G16B16A16_UNORM";
    case DXGI_FORMAT_R32G32B32A32_FLOAT: return "R32G32B32A32_FLOAT";
    default: return "not one this peer names";
  }
}

uint64_t luid_bits(const LUID& luid) {
  return (static_cast<uint64_t>(static_cast<uint32_t>(luid.HighPart)) << 32) | luid.LowPart;
}

// One adapter's description on one line: name, PCI ids, flags and LUID.
void describe(const char* what, const DXGI_ADAPTER_DESC1& desc) {
  std::printf("spout_peer_sdk %s: %s %ls (%04x:%04x)%s%s, LUID %016llx\n", g_mode, what,
              desc.Description, desc.VendorId, desc.DeviceId,
              (desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE) ? ", software" : "",
              (desc.Flags & DXGI_ADAPTER_FLAG_REMOTE) ? ", remote" : "",
              static_cast<unsigned long long>(luid_bits(desc.AdapterLuid)));
  std::fflush(stdout);
}

// Every adapter DXGI lists, in its order; the one whose LUID is `wanted` is returned, with a
// reference the caller releases.
IDXGIAdapter1* list_adapters(const uint64_t* wanted) {
  IDXGIFactory1* factory = nullptr;
  HRESULT hr = CreateDXGIFactory1(__uuidof(IDXGIFactory1), reinterpret_cast<void**>(&factory));
  if (FAILED(hr)) {
    say("CreateDXGIFactory1 failed: HRESULT 0x%08lx", static_cast<unsigned long>(hr));
    return nullptr;
  }
  IDXGIAdapter1* found = nullptr;
  IDXGIAdapter1* adapter = nullptr;
  for (UINT i = 0; factory->EnumAdapters1(i, &adapter) != DXGI_ERROR_NOT_FOUND; ++i) {
    DXGI_ADAPTER_DESC1 desc{};
    if (SUCCEEDED(adapter->GetDesc1(&desc))) {
      char what[32];
      std::snprintf(what, sizeof what, "adapter %u:", i);
      describe(what, desc);
      if (wanted && !found && luid_bits(desc.AdapterLuid) == *wanted) {
        found = adapter;
        continue;
      }
    }
    adapter->Release();
  }
  factory->Release();
  return found;
}

// The adapter a device was made on, and its LUID on a line of its own for a script to read.
void print_adapter(ID3D11Device* device, const char* how) {
  IDXGIDevice* dxgi = nullptr;
  IDXGIAdapter* adapter = nullptr;
  IDXGIAdapter1* adapter1 = nullptr;
  DXGI_ADAPTER_DESC1 desc{};
  if (SUCCEEDED(device->QueryInterface(__uuidof(IDXGIDevice), reinterpret_cast<void**>(&dxgi))) &&
      SUCCEEDED(dxgi->GetAdapter(&adapter)) &&
      SUCCEEDED(adapter->QueryInterface(__uuidof(IDXGIAdapter1), reinterpret_cast<void**>(&adapter1))) &&
      SUCCEEDED(adapter1->GetDesc1(&desc))) {
    const unsigned level = device->GetFeatureLevel();
    char what[96];
    std::snprintf(what, sizeof what, "Direct3D 11, feature level %u.%u, %s, on", (level >> 12) & 0xf,
                  (level >> 8) & 0xf, how);
    describe(what, desc);
    std::printf("adapter-luid %016llx\n", static_cast<unsigned long long>(luid_bits(desc.AdapterLuid)));
    std::fflush(stdout);
  } else {
    say("Direct3D 11, %s, on an adapter DXGI does not describe", how);
  }
  if (adapter1) adapter1->Release();
  if (adapter) adapter->Release();
  if (dxgi) dxgi->Release();
}

HRESULT make_device(IDXGIAdapter* adapter, D3D_DRIVER_TYPE type, ID3D11Device** device) {
  const D3D_FEATURE_LEVEL levels[] = {D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0,
                                      D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_10_0};
  const UINT count = static_cast<UINT>(sizeof levels / sizeof levels[0]);
  HRESULT hr = D3D11CreateDevice(adapter, type, nullptr, D3D11_CREATE_DEVICE_BGRA_SUPPORT, levels,
                                 count, D3D11_SDK_VERSION, device, nullptr, nullptr);
  if (hr == E_INVALIDARG) {
    // A runtime without Direct3D 11.1 refuses a list that names it.
    hr = D3D11CreateDevice(adapter, type, nullptr, D3D11_CREATE_DEVICE_BGRA_SUPPORT, levels + 1,
                           count - 1, D3D11_SDK_VERSION, device, nullptr, nullptr);
  }
  return hr;
}

// A Direct3D 11 device on SPOUT_PEER_LUID's adapter; without it, on the default adapter, or on
// WARP where that fails.
ID3D11Device* create_device() {
  uint64_t luid = 0;
  const char* luid_text = env("SPOUT_PEER_LUID");
  if (luid_text && !parse_luid(luid_text, luid)) {
    say("SPOUT_PEER_LUID is \"%s\", which is not a LUID in hex", luid_text);
    return nullptr;
  }
  IDXGIAdapter1* adapter = list_adapters(luid_text ? &luid : nullptr);
  ID3D11Device* device = nullptr;
  if (luid_text) {
    if (!adapter) {
      say("no adapter has the LUID %016llx", static_cast<unsigned long long>(luid));
      return nullptr;
    }
    const HRESULT hr = make_device(adapter, D3D_DRIVER_TYPE_UNKNOWN, &device);
    adapter->Release();
    if (FAILED(hr) || !device) {
      say("no Direct3D 11 device on the adapter %016llx: HRESULT 0x%08lx",
          static_cast<unsigned long long>(luid), static_cast<unsigned long>(hr));
      return nullptr;
    }
    print_adapter(device, "the adapter SPOUT_PEER_LUID names");
    return device;
  }
  HRESULT hr = make_device(nullptr, D3D_DRIVER_TYPE_HARDWARE, &device);
  if (SUCCEEDED(hr) && device) {
    print_adapter(device, "the default adapter");
    return device;
  }
  say("no Direct3D 11 device on the default adapter: HRESULT 0x%08lx", static_cast<unsigned long>(hr));
  hr = make_device(nullptr, D3D_DRIVER_TYPE_WARP, &device);
  if (SUCCEEDED(hr) && device) {
    print_adapter(device, "WARP");
    return device;
  }
  say("no Direct3D 11 device on WARP: HRESULT 0x%08lx", static_cast<unsigned long>(hr));
  return nullptr;
}

// SPOUT_PEER_FRAMECOUNT applied to `peer`: 1 or 0 where it is set, -1 where it is not.
int apply_frame_count(spoutDX& peer) {
  const char* text = env("SPOUT_PEER_FRAMECOUNT");
  if (!text) {
    say("frame counting as the registry has it");
    return -1;
  }
  const bool on = std::strcmp(text, "0") != 0;
  peer.frame.SetFrameCount(on);
  say("frame counting %s", on ? "on" : "off");
  return on ? 1 : 0;
}

// The senders Spout lists, quoted and joined, and whether `name` is one of them.
std::string listed(spoutDX& peer, const std::string& name, bool& found) {
  std::string names;
  found = false;
  for (const std::string& each : peer.GetSenderList()) {
    names += names.empty() ? "\"" : ", \"";
    names += each + "\"";
    if (each == name) found = true;
  }
  return names.empty() ? "none" : names;
}

// --------------------------------------------------------------------------------- send

int run_send(const Args& args) {
  ID3D11Device* device = create_device();
  if (!device) return 1;

  std::vector<uint8_t> bgra(static_cast<size_t>(args.width) * args.height * 4);
  for (unsigned y = 0; y < args.height; ++y) {
    for (unsigned x = 0; x < args.width; ++x) {
      const Rgba want = expected(x, y, args);
      uint8_t* p = &bgra[(static_cast<size_t>(y) * args.width + x) * 4];
      p[0] = static_cast<uint8_t>(want.b);
      p[1] = static_cast<uint8_t>(want.g);
      p[2] = static_cast<uint8_t>(want.r);
      p[3] = static_cast<uint8_t>(want.a);
    }
  }
  D3D11_TEXTURE2D_DESC desc{};
  desc.Width = args.width;
  desc.Height = args.height;
  desc.MipLevels = 1;
  desc.ArraySize = 1;
  desc.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
  desc.SampleDesc.Count = 1;
  desc.Usage = D3D11_USAGE_DEFAULT;
  desc.BindFlags = D3D11_BIND_SHADER_RESOURCE;
  D3D11_SUBRESOURCE_DATA initial{};
  initial.pSysMem = bgra.data();
  initial.SysMemPitch = args.width * 4;
  ID3D11Texture2D* texture = nullptr;
  HRESULT hr = device->CreateTexture2D(&desc, &initial, &texture);
  if (FAILED(hr)) {
    say("could not make the %ux%u pattern texture: HRESULT 0x%08lx", args.width, args.height,
        static_cast<unsigned long>(hr));
    device->Release();
    return 1;
  }

  int code = 0;
  {
    spoutDX sender;
    apply_frame_count(sender);
    sender.OpenDirectX11(device);
    sender.SetSenderName(args.name.c_str());
    if (args.name != sender.GetName()) {
      say("the name \"%s\" is another sender's; Spout offers \"%s\"", args.name.c_str(),
          sender.GetName());
      code = 1;
    } else {
      const auto start = std::chrono::steady_clock::now();
      const auto length = std::chrono::duration<double>(args.seconds);
      long frames = 0;
      while (std::chrono::steady_clock::now() - start < length) {
        if (!sender.SendTexture(texture)) {
          say("SendTexture failed after %ld frames", frames);
          code = 1;
          break;
        }
        if (frames == 0) {
          bool found = false;
          const std::string names = listed(sender, args.name, found);
          say("sending \"%s\", %ux%u %s, for %g s; Spout lists %s", sender.GetName(),
              sender.GetWidth(), sender.GetHeight(), format_name(desc.Format), args.seconds,
              names.c_str());
        }
        ++frames;
        sender.HoldFps(kSendFps);
      }
      if (code == 0) say("sent %ld frames", frames);
    }
    sender.ReleaseSender();
  }
  texture->Release();
  device->Release();
  return code;
}

// ------------------------------------------------------------------------------ receive

// Copies `texture` to the CPU and compares it with the pattern. Empty on a match; otherwise
// what was wrong.
std::string check(ID3D11Device* device, ID3D11DeviceContext* context, ID3D11Texture2D* texture,
                  ID3D11Texture2D*& staging, const Args& args) {
  D3D11_TEXTURE2D_DESC desc{};
  texture->GetDesc(&desc);
  char line[256];
  if (desc.Width != args.width || desc.Height != args.height) {
    std::snprintf(line, sizeof line, "the sender is %ux%u, not %ux%u", desc.Width, desc.Height,
                  args.width, args.height);
    return line;
  }
  bool bgr = true;
  bool opaque = false;
  switch (desc.Format) {
    case DXGI_FORMAT_B8G8R8A8_UNORM:
    case DXGI_FORMAT_B8G8R8A8_UNORM_SRGB: bgr = true; break;
    case DXGI_FORMAT_B8G8R8X8_UNORM: bgr = true; opaque = true; break;
    case DXGI_FORMAT_R8G8B8A8_UNORM:
    case DXGI_FORMAT_R8G8B8A8_UNORM_SRGB: bgr = false; break;
    default:
      std::snprintf(line, sizeof line, "the sender's format is %d (%s), which this peer does not read",
                    static_cast<int>(desc.Format), format_name(desc.Format));
      return line;
  }

  if (!staging) {
    D3D11_TEXTURE2D_DESC readable = desc;
    readable.MipLevels = 1;
    readable.ArraySize = 1;
    readable.SampleDesc.Count = 1;
    readable.SampleDesc.Quality = 0;
    readable.Usage = D3D11_USAGE_STAGING;
    readable.BindFlags = 0;
    readable.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    readable.MiscFlags = 0;
    const HRESULT hr = device->CreateTexture2D(&readable, nullptr, &staging);
    if (FAILED(hr)) {
      std::snprintf(line, sizeof line, "no staging texture: HRESULT 0x%08lx",
                    static_cast<unsigned long>(hr));
      return line;
    }
  }
  context->CopyResource(staging, texture);
  D3D11_MAPPED_SUBRESOURCE mapped{};
  const HRESULT hr = context->Map(staging, 0, D3D11_MAP_READ, 0, &mapped);
  if (FAILED(hr)) {
    std::snprintf(line, sizeof line, "Map failed: HRESULT 0x%08lx", static_cast<unsigned long>(hr));
    return line;
  }
  std::string verdict;
  for (unsigned y = 0; y < args.height && verdict.empty(); ++y) {
    const uint8_t* row = static_cast<const uint8_t*>(mapped.pData) + static_cast<size_t>(y) * mapped.RowPitch;
    for (unsigned x = 0; x < args.width; ++x) {
      const uint8_t* p = row + static_cast<size_t>(x) * 4;
      const Rgba got = bgr ? Rgba{p[2], p[1], p[0], opaque ? 255u : p[3]}
                           : Rgba{p[0], p[1], p[2], p[3]};
      const Rgba want = expected(x, y, args);
      if (std::abs(static_cast<int>(got.r) - static_cast<int>(want.r)) > kTolerance ||
          std::abs(static_cast<int>(got.g) - static_cast<int>(want.g)) > kTolerance ||
          std::abs(static_cast<int>(got.b) - static_cast<int>(want.b)) > kTolerance ||
          std::abs(static_cast<int>(got.a) - static_cast<int>(want.a)) > kTolerance) {
        std::snprintf(line, sizeof line,
                      "pixel (%u, %u) is RGBA %u %u %u %u, the pattern's is %u %u %u %u (%ux%u %s)",
                      x, y, got.r, got.g, got.b, got.a, want.r, want.g, want.b, want.a,
                      desc.Width, desc.Height, format_name(desc.Format));
        verdict = line;
        break;
      }
    }
  }
  context->Unmap(staging, 0);
  return verdict;
}

int run_receive(const Args& args) {
  ID3D11Device* device = create_device();
  if (!device) return 1;

  bool matched = false;
  std::string last;
  {
    spoutDX receiver;
    const int counting = apply_frame_count(receiver);
    receiver.OpenDirectX11(device);
    receiver.SetReceiverName(args.name.c_str());
    ID3D11DeviceContext* context = receiver.GetDX11Context();
    ID3D11Texture2D* staging = nullptr;
    bool connected = false;
    long reads = 0;
    int streak = 0;

    say("waiting up to %g s for \"%s\"", args.seconds, args.name.c_str());
    const auto start = std::chrono::steady_clock::now();
    const auto length = std::chrono::duration<double>(args.seconds);
    while (std::chrono::steady_clock::now() - start < length) {
      if (!receiver.ReceiveTexture()) {
        if (connected) {
          say("\"%s\" closed", args.name.c_str());
          connected = false;
        }
        streak = 0;
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
        continue;
      }
      if (receiver.IsUpdated()) {
        // A new sender, or one that changed: its texture is made again, and copied again on
        // the next ReceiveTexture.
        connected = true;
        streak = 0;
        if (staging) staging->Release();
        staging = nullptr;
        bool found = false;
        const std::string names = listed(receiver, args.name, found);
        say("connected to \"%s\": %ux%u, format %d (%s), handle 0x%llx; Spout lists %s",
            receiver.GetSenderName(), receiver.GetSenderWidth(), receiver.GetSenderHeight(),
            static_cast<int>(receiver.GetSenderFormat()), format_name(receiver.GetSenderFormat()),
            static_cast<unsigned long long>(reinterpret_cast<uintptr_t>(receiver.GetSenderHandle())),
            names.c_str());
        continue;
      }
      ID3D11Texture2D* texture = receiver.GetSenderTexture();
      if (!texture) {
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
        continue;
      }
      ++reads;
      std::string verdict = check(device, context, texture, staging, args);
      if (verdict.empty() && ++streak >= kMatchesNeeded) {
        bool found = false;
        const std::string names = listed(receiver, args.name, found);
        const long frame = receiver.GetSenderFrame();
        if (!found) {
          verdict = "the pattern arrived, and Spout's list of senders, " + names +
                    ", does not name \"" + args.name + "\"";
        } else if (counting == 1 && frame <= 0) {
          verdict = "the pattern arrived, and with frame counting on the sender counted no frames";
        } else {
          const Rgba corner = expected(args.width - 1, args.height - 1, args);
          say("match: %d reads in a row within %d a channel, read %ld, the sender's frame %ld; "
              "(0, 0) is RGBA 0 0 64 255 and (%u, %u) is %u %u %u %u",
              streak, kTolerance, reads, frame, args.width - 1, args.height - 1, corner.r,
              corner.g, corner.b, corner.a);
          matched = true;
          break;
        }
      }
      if (!verdict.empty()) {
        streak = 0;
        if (verdict != last) say("not yet: %s", verdict.c_str());
        last = verdict;
      }
      std::this_thread::sleep_for(std::chrono::milliseconds(15));
    }

    if (!matched) {
      if (!last.empty()) {
        say("no match within %g s; the last read: %s", args.seconds, last.c_str());
      } else {
        unsigned width = 0, height = 0;
        HANDLE handle = nullptr;
        DWORD format = 0;
        bool found = false;
        const std::string names = listed(receiver, args.name, found);
        if (receiver.GetSenderInfo(args.name.c_str(), width, height, handle, format)) {
          say("\"%s\" has its info, %ux%u, format %lu, handle 0x%llx, and no frame of it arrived "
              "within %g s: its texture did not open on this device; Spout lists %s",
              args.name.c_str(), width, height, static_cast<unsigned long>(format),
              static_cast<unsigned long long>(reinterpret_cast<uintptr_t>(handle)), args.seconds,
              names.c_str());
        } else {
          say("no sender \"%s\" within %g s; Spout lists %s", args.name.c_str(), args.seconds,
              names.c_str());
        }
      }
    }
    if (staging) staging->Release();
    receiver.ReleaseReceiver();
  }
  device->Release();
  return matched ? 0 : 1;
}

int usage() {
  std::fprintf(stderr,
               "usage: spout_peer_sdk send <sender> <width> <height> <seconds>\n"
               "       spout_peer_sdk receive <sender> <width> <height> <timeout-seconds>\n");
  return 2;
}

}  // namespace

int main(int argc, char** argv) {
  std::setvbuf(stdout, nullptr, _IONBF, 0);
  if (argc != 6) return usage();
  const std::string mode = argv[1];
  if (mode != "send" && mode != "receive") return usage();
  g_mode = argv[1];
  Args args;
  args.name = argv[2];
  if (args.name.empty() || args.name.size() >= 256 || !parse_size(argv[3], args.width) ||
      !parse_size(argv[4], args.height) || !parse_seconds(argv[5], args.seconds)) {
    return usage();
  }
  return mode == "send" ? run_send(args) : run_receive(args);
}
