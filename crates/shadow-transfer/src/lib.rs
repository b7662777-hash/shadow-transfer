//! Shadow Transfer engine: authenticated sessions, resumable streaming transfers
//! and BLAKE3 verification.

mod error;
mod path;
mod progress;
mod receiver;
mod sender;
mod session;
mod source;
mod state;
pub mod util;

pub use error::TransferError;
pub use path::{verify_wire_path, PathError};
pub use progress::{Direction, ProgressEvent, ProgressTracker, SpeedMeter, TransferPhase};
pub use receiver::{
    start, ApprovalKind, ApprovalNeeded, ReceiverConfig, ReceiverEvent, ReceiverHandle,
};
pub use sender::{
    pair_with, send_sources, ChunkPolicy, FileOutcome, PairSummary, SendSummary, SenderConfig,
};
pub use source::{collect_sources, collect_sources_blocking, SourceFile};

pub use state::FinalizedFile;

/// Generates a random transfer identifier (16 bytes, hex).
pub fn generate_transfer_id() -> Result<String, TransferError> {
    session::new_transfer_id()
}
