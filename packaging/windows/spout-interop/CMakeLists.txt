# SPDX-License-Identifier: AGPL-3.0-or-later
#
# spout_peer_sdk.exe, the Spout peer built on the Spout2 SDK's own spoutDX class. Only the
# DirectX side of the SDK is compiled: SpoutDX.cpp and the six SpoutGL files it calls into,
# the list of the SDK's own SpoutDX_static target. The SDK's top-level CMake, which builds the
# OpenGL library, is not used.
#
#   cmake -S packaging/windows/spout-interop -B build/spout-interop -A x64 -DSPOUT2_DIR=<Spout2>
#   cmake --build build/spout-interop --config Release
#
# SPOUT2_DIR is a checkout of https://github.com/leadedge/Spout2 at the tag
# .github/workflows/spout.yml names. The C runtime is linked in, so the .exe runs on a machine
# without the Visual C++ Redistributable.

cmake_minimum_required(VERSION 3.20)
project(spout_peer_sdk LANGUAGES CXX)

set(SPOUT2_DIR "" CACHE PATH "A checkout of https://github.com/leadedge/Spout2")
set(sdk ${SPOUT2_DIR}/SPOUTSDK)
if(NOT EXISTS ${sdk}/SpoutDirectX/SpoutDX/SpoutDX.h)
  message(FATAL_ERROR "SPOUT2_DIR is \"${SPOUT2_DIR}\", which holds no SPOUTSDK/SpoutDirectX/SpoutDX/SpoutDX.h")
endif()

set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")

add_executable(spout_peer_sdk
  spout_peer.cpp
  ${sdk}/SpoutDirectX/SpoutDX/SpoutDX.cpp
  ${sdk}/SpoutGL/SpoutCopy.cpp
  ${sdk}/SpoutGL/SpoutDirectX.cpp
  ${sdk}/SpoutGL/SpoutFrameCount.cpp
  ${sdk}/SpoutGL/SpoutSenderNames.cpp
  ${sdk}/SpoutGL/SpoutSharedMemory.cpp
  ${sdk}/SpoutGL/SpoutUtils.cpp)
# With SpoutGL on the path, SpoutDX.h takes its includes from there.
target_include_directories(spout_peer_sdk PRIVATE ${sdk}/SpoutDirectX/SpoutDX ${sdk}/SpoutGL)
target_compile_features(spout_peer_sdk PRIVATE cxx_std_17)
if(MSVC)
  target_compile_options(spout_peer_sdk PRIVATE /EHsc /utf-8)
endif()
target_link_libraries(spout_peer_sdk PRIVATE
  d3d11 dxgi winmm version shell32 advapi32 comctl32 psapi user32 gdi32 ole32)
