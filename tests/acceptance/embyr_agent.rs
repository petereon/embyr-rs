// SCAFFOLD: true
//! Integration test binary entry point for embyr-agent acceptance tests.
//!
//! Registers each US-A0N module as a submodule so `use super::` works in the
//! individual test files.  All tests carry `#[ignore]` and are scaffolded RED
//! — DELIVER unskips them one slice at a time.
//!
//! Registered in `crates/embyr-agent/Cargo.toml` as:
//!   [[test]]
//!   name = "embyr_agent"
//!   path = "../../tests/acceptance/embyr_agent.rs"

// Shared test infrastructure (AgentHandle, start_test_agent, test_tls_config)
#[path = "embyr_agent/mod.rs"]
mod agent_common;

// Re-export common items so submodules can use `super::` or `crate::agent_common::`.
// Individual test files use: `use super::agent_common::{start_test_agent, AgentHandle};`

#[path = "embyr_agent/us_a01_get_document.rs"]
mod us_a01_get_document;

#[path = "embyr_agent/us_a02_write_operations.rs"]
mod us_a02_write_operations;

#[path = "embyr_agent/us_a03_query_operations.rs"]
mod us_a03_query_operations;

#[path = "embyr_agent/us_a04_transactions.rs"]
mod us_a04_transactions;

#[path = "embyr_agent/us_a05_subscribe.rs"]
mod us_a05_subscribe;

#[path = "embyr_agent/us_a06_lifecycle.rs"]
mod us_a06_lifecycle;

#[path = "embyr_agent/us_a07_field_transforms.rs"]
mod us_a07_field_transforms;
