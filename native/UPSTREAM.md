# Mount bridge

Rust reimplementation of the mount procedure in [vita-save-keeper](https://github.com/falkenhawk/vita-save-keeper/tree/10c6684599c064b3bfc82e3cc6dfa83d471db3c4/src/vita/mount), revision `10c6684599c064b3bfc82e3cc6dfa83d471db3c4`, and [VitaShell](https://github.com/TheOfficialFloW/VitaShell).

GPL-3.0-or-later. Upstream firmware offsets are retained; implementation uses no_std Rust and namespaced exports. No C source is compiled. Unsupported firmware is rejected. These modules provide savedata mounting only.
