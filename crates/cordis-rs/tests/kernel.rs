//! M0 acceptance tests for the microkernel.
//!
//! These cover the three things the roadmap gates M0 on: the **disposer audit** (every effect
//! recovered on unload), correct **cascade** unload, and the **reactive injection** paths
//! (`Pending` → `Active` on provide, and provider replacement restarting dependents).

use async_trait::async_trait;
use cordis_rs::fiber::FiberState;
use cordis_rs::{Context, Plugin, Result, ServiceKey};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const GREETER: ServiceKey = ServiceKey::new("greeter");
const MISSING: ServiceKey = ServiceKey::new("missing");

#[derive(Default)]
struct Greeter {
    calls: AtomicUsize,
}

/// Provides a tree-wide `greeter` service (see `Context::provide_shared`).
struct GreeterPlugin;

#[async_trait]
impl Plugin for GreeterPlugin {
    fn name(&self) -> &str {
        "greeter"
    }

    async fn apply(&self, ctx: &Arc<Context>) -> Result<()> {
        ctx.provide_shared(GREETER, Greeter::default()).await;
        Ok(())
    }
}

/// Records the order in which its two effects are unwound.
struct LifoPlugin {
    recorder: Arc<Mutex<Vec<&'static str>>>,
}

#[async_trait]
impl Plugin for LifoPlugin {
    fn name(&self) -> &str {
        "lifo"
    }

    async fn apply(&self, ctx: &Arc<Context>) -> Result<()> {
        let first = self.recorder.clone();
        ctx.effect(move || first.lock().expect("recorder").push("first"));

        let second = self.recorder.clone();
        ctx.effect(move || second.lock().expect("recorder").push("second"));
        Ok(())
    }
}

/// Consumes `greeter`; counts how many times it has been activated.
struct DependentPlugin {
    activations: Arc<AtomicUsize>,
}

#[async_trait]
impl Plugin for DependentPlugin {
    fn name(&self) -> &str {
        "dependent"
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![GREETER]
    }

    async fn apply(&self, ctx: &Arc<Context>) -> Result<()> {
        assert!(
            ctx.has(GREETER),
            "injected service must be resolvable in apply()"
        );
        self.activations.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// Requires a service nothing provides.
struct MissingDependencyPlugin;

#[async_trait]
impl Plugin for MissingDependencyPlugin {
    fn name(&self) -> &str {
        "missing-dependency"
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![MISSING]
    }

    async fn apply(&self, _ctx: &Arc<Context>) -> Result<()> {
        Ok(())
    }
}

/// Mounts a child plugin; used to prove cascade unload.
struct ParentPlugin;

#[async_trait]
impl Plugin for ParentPlugin {
    fn name(&self) -> &str {
        "parent"
    }

    async fn apply(&self, ctx: &Arc<Context>) -> Result<()> {
        ctx.plugin(Arc::new(GreeterPlugin)).await?;
        Ok(())
    }
}

#[tokio::test]
async fn provides_service_to_tree_and_revokes_on_unload() {
    let root = Context::root();
    let fiber = root.plugin(Arc::new(GreeterPlugin)).await.expect("mount");
    assert_eq!(fiber.state(), FiberState::Active);

    let greeter = root
        .get::<Greeter>(GREETER)
        .expect("service is tree-visible");
    assert_eq!(greeter.calls.fetch_add(1, Ordering::SeqCst), 0);

    root.unload(&fiber).await;

    assert_eq!(fiber.state(), FiberState::Disposed);
    assert_eq!(
        fiber.effect_count(),
        0,
        "disposer audit: every effect must be recovered on unload"
    );
    assert!(
        root.get::<Greeter>(GREETER).is_none(),
        "unloading must revoke the provided service"
    );
}

#[tokio::test]
async fn effects_unwind_in_lifo_order() {
    let root = Context::root();
    let recorder = Arc::new(Mutex::new(Vec::new()));

    let fiber = root
        .plugin(Arc::new(LifoPlugin {
            recorder: recorder.clone(),
        }))
        .await
        .expect("mount");
    assert_eq!(fiber.effect_count(), 2);

    root.unload(&fiber).await;

    assert_eq!(
        *recorder.lock().expect("recorder"),
        vec!["second", "first"],
        "the recovery accumulator unwinds LIFO"
    );
}

#[tokio::test]
async fn inject_waits_pending_until_a_provider_appears() {
    let root = Context::root();
    let activations = Arc::new(AtomicUsize::new(0));

    let fiber = root
        .plugin(Arc::new(DependentPlugin {
            activations: activations.clone(),
        }))
        .await
        .expect("mount");
    assert_eq!(fiber.state(), FiberState::Pending, "missing dep => Pending");
    assert_eq!(activations.load(Ordering::SeqCst), 0);

    root.provide(GREETER, Greeter::default()).await;
    assert_eq!(
        fiber.state(),
        FiberState::Active,
        "provider arrived => Active"
    );
    assert_eq!(activations.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_unavailable_dependency_keeps_the_fiber_pending() {
    let root = Context::root();
    let fiber = root
        .plugin(Arc::new(MissingDependencyPlugin))
        .await
        .expect("mount");
    assert_eq!(fiber.state(), FiberState::Pending);

    // An unrelated service appearing must not wake a fiber waiting on something else.
    root.provide(GREETER, Greeter::default()).await;
    assert_eq!(
        fiber.state(),
        FiberState::Pending,
        "only the injected key may satisfy the dependency"
    );

    root.provide(MISSING, 7u8).await;
    assert_eq!(fiber.state(), FiberState::Active);
}

#[tokio::test]
async fn provider_replacement_restarts_dependents() {
    let root = Context::root();
    let activations = Arc::new(AtomicUsize::new(0));

    let provider = root
        .plugin(Arc::new(GreeterPlugin))
        .await
        .expect("mount provider");
    let dependent = root
        .plugin(Arc::new(DependentPlugin {
            activations: activations.clone(),
        }))
        .await
        .expect("mount dependent");
    assert_eq!(dependent.state(), FiberState::Active);
    assert_eq!(activations.load(Ordering::SeqCst), 1);

    // Swap the provider for a fresh instance on the root scope.
    root.provide(GREETER, Greeter::default()).await;

    assert_eq!(
        activations.load(Ordering::SeqCst),
        2,
        "a replaced provider must restart its dependents (DEV guide 2.2 invariant 3)"
    );
    assert_eq!(dependent.state(), FiberState::Active);

    root.unload(&provider).await;
    assert_eq!(
        dependent.state(),
        FiberState::Pending,
        "removing the provider sends dependents back to Pending"
    );
}

#[tokio::test]
async fn unload_cascades_to_children() {
    let root = Context::root();
    let parent = root
        .plugin(Arc::new(ParentPlugin))
        .await
        .expect("mount parent");

    let children = parent.children();
    assert_eq!(children.len(), 1, "the parent mounted exactly one child");
    let child = children.first().expect("child handle").clone();
    assert_eq!(child.state(), FiberState::Active);
    assert!(root.get::<Greeter>(GREETER).is_some());

    root.unload(&parent).await;

    assert_eq!(parent.state(), FiberState::Disposed);
    assert_eq!(
        child.state(),
        FiberState::Disposed,
        "cascade: disposing a parent must dispose its subtree"
    );
    assert_eq!(parent.effect_count(), 0);
}

#[tokio::test]
async fn disabling_and_reenabling_cycles_effects_cleanly() {
    let root = Context::root();
    let recorder = Arc::new(Mutex::new(Vec::new()));

    let fiber = root
        .plugin(Arc::new(LifoPlugin {
            recorder: recorder.clone(),
        }))
        .await
        .expect("mount");
    assert_eq!(fiber.effect_count(), 2);

    // Disable: recover effects but keep the plugin instance mounted, then re-apply.
    // This is the runtime "hot shutdown / hot load" cycle of ADR-007 — no host restart.
    let plugin = fiber.plugin().expect("plugin handle retained");
    fiber.recover().await;
    assert_eq!(fiber.effect_count(), 0);

    let ctx = fiber.ctx().expect("ctx retained while disabled");
    plugin.apply(&ctx).await.expect("re-apply");
    assert_eq!(fiber.effect_count(), 2, "re-enabling re-registers effects");

    root.unload(&fiber).await;
    assert_eq!(fiber.effect_count(), 0);
}
