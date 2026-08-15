//! Admin domain types and port traits for BC-1 (Tenant Management).
//!
//! This module is the inner-hexagon home for all admin identity types.
//! No IO imports allowed — enforced by deny.toml.

pub mod account;
pub mod billing;
pub mod email;
pub mod query_log;
pub mod rbac;
pub mod sdk_key;
pub mod session;

pub use account::{AccountId, AccountMember, Role, User, UserId};
pub use billing::{
    cap_exceeded, compute_cap_status, free_plan_caps, CapStatus, DimensionCapEntry,
    Subscription, SubscriptionPlan, SubscriptionStatus, UsageDimension, WebhookEventOutcome,
};
pub use email::{EmailError, EmailMessage, IEmailSender};
pub use query_log::{IQueryLogWriter, OpStatus, OperationType, QueryLogEntry};
pub use rbac::{check_rbac, RbacAction, RbacError};
pub use session::{SessionContext, SessionToken};
