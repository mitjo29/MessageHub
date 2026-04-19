//! Runtime: orchestration layer that drives adapters → ingestion → classification.
//!
//! See `docs/superpowers/specs/2026-04-19-plan6-channel-runtime-design.md`.

pub mod status;
pub mod events;
pub mod factory;
pub mod ingestor;
pub mod classifier_worker;
pub mod channel_task;

// Runtime + RuntimeBuilder land in Task 11.
