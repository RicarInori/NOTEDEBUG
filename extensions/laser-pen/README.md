# laser-pen — WASM extension example

The first third-party-shaped plugin, and the one that proves the `inkweave_abi_v1` boundary
(DEV guide §7.1, delivered at M6).

A laser pointer is deliberately the simplest useful extension: it renders without persisting
anything, and it needs **no** permissions at all. That makes it the cleanest way to validate the
sandbox and the host-function ABI before granting any extension access to files or the network.

## Status

Not buildable yet. The ABI — the host function set, memory limits and fuel metering — is an
explicit M6 work item (2–3 weeks). Today this directory fixes the *shape*:

- `manifest.toml` — the manifest schema, including the empty permission set.
- `src/lib.rs` — the exported ABI version symbol and the WGSL the brush would submit.

## Installing it

Once M6 lands:

```bash
cargo build --release --target wasm32-unknown-unknown
# package laser_pen.wasm together with manifest.toml
```

Then use the app's **控制面板 → 添加插件** entry. Per DEV guide §8.4, the host will validate the
manifest, check `abi` against the running host, confirm the (empty) permission list with the user,
and hot-load it — no host restart, and it can be disabled or uninstalled from the same panel.