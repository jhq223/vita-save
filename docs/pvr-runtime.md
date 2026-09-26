# PVR runtime modules

The four modules in `runtime/module/` are deployed together. They were built from `pvr-psp2-sys` revision `2b0a956ff7caab493ba5a3ec17be221d97cdede1`, using its `native/` CMake entry point and the official PSVSDK (Release/O3/Thumb2). The Rust bindings use the same revision; the offline compiler is pinned to `jhq223/pvr-compiler` revision `f4251dafc4c6246fae9e70de671d8c7a81adc9ea`.

| Module | SHA-256 |
| --- | --- |
| libGLESv2.suprx | cf712dbac9a89a9a83d44d992f03b8190b56378b2aafe42463cfc94785b3fe0b |
| libgpu_es4_ext.suprx | b426fa3811c9120ca779b3e7331f257e4b65c3e4aaa6ffc6c33e91882d1557a3 |
| libIMGEGL.suprx | d8c5026e32a6ddb0aae694b277325968e72ab2449e5fbd8aca32572dd2aeee8e |
| libpvrPSP2_WSEGL.suprx | 0bf3b73bfbc64825c4e64ee4afb6b44d5982d0e364b6b672553ef97adac8daaf |

Use the [driver build](https://github.com/jhq223/pvr-psp2-sys) to produce replacement modules. Update the entire set and the hashes together. Vita Save relies on the driver's ownership of asynchronous upload data and its program-switching behavior.

Native code retains the upstream PVR_PSP2 licensing and copyright notices in the driver repository. These hashes establish artifact identity; they are not Vita hardware validation.
