use crate::context::Context;
use crate::service::ServiceKey;
use async_trait::async_trait;
use std::sync::Arc;

/// Failure of a plugin's `apply`, or of an operation on a disposed scope.
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    // `message` rather than `source`: thiserror treats a field named `source` as the error chain
    // and would require it to implement `std::error::Error`.
    #[error("plugin `{name}` failed to apply: {message}")]
    Apply { name: String, message: String },

    #[error("the context has been disposed")]
    Disposed,
}

pub type Result<T> = std::result::Result<T, PluginError>;

/// A unit of functionality mounted on the kernel.
///
/// Three shapes are supported by convention (mirroring Cordis):
/// * a plain plugin that only registers effects,
/// * a plugin that [`Context::provide`](crate::Context::provide)s a service for others,
/// * a plugin that is itself a service provider *and* a consumer of others (via [`Plugin::inject`]).
///
/// `apply` must be cheap: it only registers. Long-running work belongs to an effect or an event
/// listener, never to the mount path (DEV guide §8.2 / §7.2 red lines).
#[async_trait]
pub trait Plugin: Send + Sync + 'static {
    fn name(&self) -> &str;

    /// Services this plugin requires. While any is missing the fiber stays `Pending`.
    fn inject(&self) -> Vec<ServiceKey> {
        Vec::new()
    }

    /// Register this plugin's contributions.
    ///
    /// Takes `&Arc<Context>` (not `&Context`) because mounting a child plugin needs to create a
    /// new scope, which requires an owned handle.
    async fn apply(&self, ctx: &Arc<Context>) -> Result<()>;
}

/// Body of a [`FnPlugin`].
pub type FnPluginBody = Box<dyn Fn(&Arc<Context>) + Send + Sync>;

/// Convenience for plugins that cannot fail.
pub struct FnPlugin {
    name: String,
    inject: Vec<ServiceKey>,
    apply: FnPluginBody,
}

impl FnPlugin {
    pub fn new(
        name: impl Into<String>,
        apply: impl Fn(&Arc<Context>) + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            inject: Vec::new(),
            apply: Box::new(apply),
        }
    }

    pub fn with_inject(mut self, inject: Vec<ServiceKey>) -> Self {
        self.inject = inject;
        self
    }

    pub fn shared(self) -> Arc<dyn Plugin> {
        Arc::new(self)
    }
}

#[async_trait]
impl Plugin for FnPlugin {
    fn name(&self) -> &str {
        &self.name
    }

    fn inject(&self) -> Vec<ServiceKey> {
        self.inject.clone()
    }

    async fn apply(&self, ctx: &Arc<Context>) -> Result<()> {
        (self.apply)(ctx);
        Ok(())
    }
}
