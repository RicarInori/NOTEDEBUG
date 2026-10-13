//! Newtype wrappers for trait-object services.
//!
//! The kernel's service table stores `Arc<dyn Any + Send + Sync>`. A `dyn Trait` is not itself
//! `Any`, so a provider stores one of these concrete wrappers and consumers read it back with
//! `ctx.get::<SchedulerHandle>(SCHEDULER)`. Keeping the wrapper thin preserves the capability
//! seam: consumers still program against the trait, never a concrete implementation.

use std::sync::Arc;

use crate::registry::PluginRegistry;
use crate::runtime::{BackgroundScheduler, MemoryGovernor};
use crate::stylus::StylusDriver;
use crate::sync::SyncProvider;

#[derive(Clone)]
pub struct RegistryHandle(pub Arc<dyn PluginRegistry>);

#[derive(Clone)]
pub struct SchedulerHandle(pub Arc<dyn BackgroundScheduler>);

#[derive(Clone)]
pub struct MemoryHandle(pub Arc<dyn MemoryGovernor>);

#[derive(Clone)]
pub struct StylusHandle(pub Arc<dyn StylusDriver>);

#[derive(Clone)]
pub struct SyncHandle(pub Arc<dyn SyncProvider>);
