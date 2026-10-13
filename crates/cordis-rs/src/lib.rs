//! cordis-rs — the Inkweave microkernel.
//!
//! A Rust port of the *design* (not the code) of [Cordis]: a hierarchical service container
//! ([`Context`]) bound 1:1 to a plugin lifecycle state machine ([`Fiber`]), where every
//! registration made through the context is a revertible [`Effect`](fiber::EffectStack) that is
//! unwound LIFO on unload.
//!
//! Design decisions follow the ADRs in `docs/inkweave-dev-guide.md`:
//!
//! * **ADR-004** — the service table is an `ArcSwap<ServiceTable>`: resolution is lock-free,
//!   registration copy-on-writes the whole table. Registration happens at startup and at the
//!   rare plugin load/unload moments, while resolution is the hot path.
//! * **ADR-007** — plugin "hot load / shutdown" means *mount / unmount + enable / disable* of a
//!   `Fiber`. There is deliberately **no** code-level hot replacement.
//! * **ADR-008** — there is no privileged core: even the control panel is a plugin mounted here.
//!
//! ```no_run
//! # async fn demo() -> cordis_rs::Result<()> {
//! use std::sync::Arc;
//! use cordis_rs::Context;
//!
//! let root = Context::root();
//! // let fiber = root.plugin(Arc::new(MyPlugin)).await?;
//! // root.unload(&fiber).await;
//! # Ok(())
//! # }
//! ```
//!
//! ### Known gaps (skeleton, tracked against the milestone plan)
//!
//! * `serial` / `parallel` event modes are not implemented yet (only `emit`, `bail`, `waterfall`).
//! * Waterfall listeners are `Fn`-based; there is no per-listener cancellation token yet.
//! * Effect disposers are `Send` future factories but the `Fiber` lock is not held across
//!   `await`, so a disposer must not re-enter the same fiber synchronously.

pub mod context;
pub mod events;
pub mod fiber;
pub mod ledger;
pub mod plugin;
pub mod service;

pub use context::{Context, ContextId};
pub use events::{EventBus, EventName, Payload};
pub use fiber::{sync_disposer, Disposer, EffectStack, Fiber, FiberId, FiberState};
pub use plugin::{FnPlugin, Plugin, PluginError, Result};
pub use service::{ServiceKey, ServiceObj, ServiceTable};
