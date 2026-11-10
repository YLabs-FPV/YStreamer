# libvmx

The VMX codec that the OMT output sends, from
[openmediatransport/libvmx](https://github.com/openmediatransport/libvmx) at
commit `31724bb96530325a989bc157c8e07538d6f1c4db`. MIT licensed, see
`LICENSE.txt`; `sse2neon.h` is MIT too, with its notice at the top.

Only what the 64-bit ARM build needs is here, unchanged: `vmxcodec.cpp`,
`vmxcodec_arm.cpp` and the headers they include. The x86 and AVX2 sources,
the Windows project files and the build scripts were left out.

`../build.rs` compiles it with the `cc` crate. Upstream builds with clang's
`-fdeclspec`; build.rs maps `__declspec(align(16))` to `alignas(16)` so g++
works too.
