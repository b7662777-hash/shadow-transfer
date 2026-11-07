use thiserror::Error;

#[derive(Debug, Error)]
pub enum TransferError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("TLS error: {0}")]
    Tls(#[from] rustls::Error),
    #[error("protocol error: {0}")]
    Protocol(#[from] shadow_protocol::ProtocolError),
    #[error("invalid metadata: {0}")]
    Validation(#[from] shadow_protocol::ValidationError),
    #[error("TLS configuration error: {0}")]
    TlsConfig(#[from] shadow_crypto::TlsConfigError),
    #[error("trust store error: {0}")]
    Trust(#[from] shadow_crypto::TrustStoreError),
    #[error("identity error: {0}")]
    Identity(#[from] shadow_crypto::IdentityError),
    #[error("operation timed out: {0}")]
    Timeout(&'static str),
    #[error("protocol violation: {0}")]
    ProtocolViolation(String),
    #[error("peer identity {0} is not trusted; pair the devices first")]
    UntrustedPeer(String),
    #[error("the receiving device does not trust this device; enter its pairing code")]
    PairingRequired,
    #[error("pairing rejected: {0}")]
    PairingRejected(String),
    #[error("pairing is not available on the receiving device")]
    PairingUnavailable,
    #[error("transfer rejected: {0}")]
    TransferRejected(String),
    #[error("integrity verification failed for {path}")]
    VerificationFailed { path: String },
    #[error("transfer cancelled")]
    Cancelled,
    #[error("unsafe path: {0}")]
    UnsafePath(String),
    #[error("no files selected")]
    EmptySelection,
    #[error("invalid source selection: {0}")]
    InvalidSource(String),
    #[error("source file changed during transfer: {0}")]
    SourceChanged(String),
    #[error("receiver error: {0}")]
    Receiver(String),
    #[error("transfer state is invalid: {0}")]
    State(String),
    #[error("secure randomness unavailable: {0}")]
    Random(String),
}

impl TransferError {
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Io(_) | Self::Timeout(_))
    }

    pub fn protocol_error(code: u16, message: impl Into<String>) -> Self {
        Self::ProtocolViolation(format!("peer error {code}: {}", message.into()))
    }
}
