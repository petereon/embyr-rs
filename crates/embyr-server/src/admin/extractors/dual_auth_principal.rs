// SCAFFOLD: true
//! DualAuthPrincipal extractor — used only by `get_project` handler (ADR-010, B-02).
//!
//! Either a session-authenticated user (with account-scoped SessionContext)
//! or an operator (with EMBYR_ADMIN_KEY Bearer) can access the dual-auth route.

/// Either a session user or an operator.
///
/// # RED scaffold
/// Placeholder — real enum wired in B-02 implementation.
pub enum AuthPrincipal {
    User,     // SessionContext placeholder
    Operator, // unit — operator access is unscoped
}
