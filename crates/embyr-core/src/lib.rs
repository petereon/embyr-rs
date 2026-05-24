// Pure domain logic for embyr-rs.
// Three bounded contexts: Tenant Management (BC-1), Document Storage (BC-2),
// Real-Time Delivery (BC-3).
// NO IO crates (tokio, sqlx, tonic, axum) — enforced by deny.toml.
