# PVR runtime modules

The four modules in `runtime/module/` are deployed together. They were built from `pvr-psp2-sys` revision `cc7aee48713eefd2fb83bb30366c2cbc1d290c68`, using its `native/` CMake entry point and the official PSVSDK (Release/O3/Thumb2). The Rust bindings use the same revision; the offline compiler is pinned to `jhq223/pvr-compiler` revision `f4251dafc4c6246fae9e70de671d8c7a81adc9ea`.

| Module | SHA-256 |
| --- | --- |
| libGLESv2.suprx | 17dc93732254fc72fa057b2d8ac46752c189644fd27048d88500407efb94159b |
| libgpu_es4_ext.suprx | 705b26e3c35dd7cc48ec21517292a5219a87956dc0c5e6e9affdddc286bf367d |
| libIMGEGL.suprx | c991d3d079590f59290df95b52c9f4b8db2a6bc353bf99397adcbd04c4acd547 |
| libpvrPSP2_WSEGL.suprx | b3af60f4416010d0e9636903c05ddf800f755aeb7dffed8c251ec108c0b54f5e |

Use the [driver build](https://github.com/jhq223/pvr-psp2-sys) to produce replacement modules. Update the entire set and the hashes together. Vita Save relies on the driver's ownership of asynchronous upload data and its program-switching behavior.

Native code retains the upstream PVR_PSP2 licensing and copyright notices in the driver repository. These hashes establish artifact identity; they are not Vita hardware validation.
