/// Opaque transaction identifier returned by the backend.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TransactionId(pub Vec<u8>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionOptions {
    ReadWrite,
    ReadOnly,
}
