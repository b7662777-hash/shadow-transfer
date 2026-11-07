use std::collections::VecDeque;
use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Send,
    Receive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferPhase {
    Connecting,
    Pairing,
    Transferring,
    Verifying,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct ProgressEvent {
    pub transfer_id: String,
    pub direction: Direction,
    pub phase: TransferPhase,
    pub file_name: String,
    pub file_index: usize,
    pub file_count: usize,
    pub file_bytes: u64,
    pub file_size: u64,
    pub total_bytes_done: u64,
    pub total_bytes: u64,
    pub speed_bps: f64,
    pub avg_bps: f64,
    pub eta_secs: Option<u64>,
}

impl ProgressEvent {
    pub fn percent(&self) -> f32 {
        if self.total_bytes == 0 {
            0.0
        } else {
            (self.total_bytes_done as f64 / self.total_bytes as f64).clamp(0.0, 1.0) as f32
        }
    }
}

/// Rolling-window throughput. Never fabricated: only observed bytes count.
pub struct SpeedMeter {
    samples: VecDeque<(Instant, u64)>,
    total: u64,
    started: Instant,
    window: Duration,
}

impl SpeedMeter {
    pub fn new() -> Self {
        Self {
            samples: VecDeque::with_capacity(32),
            total: 0,
            started: Instant::now(),
            window: Duration::from_secs(3),
        }
    }

    pub fn add(&mut self, bytes: u64) {
        self.total = self.total.saturating_add(bytes);
        let now = Instant::now();
        self.samples.push_back((now, self.total));
        while let Some((time, _)) = self.samples.front() {
            if now.duration_since(*time) > self.window && self.samples.len() > 1 {
                self.samples.pop_front();
            } else {
                break;
            }
        }
    }

    pub fn speed_bps(&self) -> f64 {
        let Some((newest_t, newest_b)) = self.samples.back() else {
            return 0.0;
        };
        let Some((oldest_t, oldest_b)) = self.samples.front() else {
            return 0.0;
        };
        let elapsed = newest_t.duration_since(*oldest_t).as_secs_f64();
        if elapsed < 0.05 {
            return 0.0;
        }
        (newest_b.saturating_sub(*oldest_b)) as f64 / elapsed
    }

    pub fn avg_bps(&self) -> f64 {
        let elapsed = self.started.elapsed().as_secs_f64();
        if elapsed < 0.05 {
            0.0
        } else {
            self.total as f64 / elapsed
        }
    }

    pub fn eta_secs(&self, remaining: u64) -> Option<u64> {
        let speed = self.speed_bps();
        if speed < 1.0 {
            None
        } else {
            Some((remaining as f64 / speed).ceil() as u64)
        }
    }
}

impl Default for SpeedMeter {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ProgressTracker {
    tx: UnboundedSender<ProgressEvent>,
    transfer_id: String,
    direction: Direction,
    phase: TransferPhase,
    file_name: String,
    file_index: usize,
    file_count: usize,
    file_size: u64,
    file_done: u64,
    completed_bytes: u64,
    total_bytes: u64,
    meter: SpeedMeter,
    last_emit: Instant,
    interval: Duration,
}

impl ProgressTracker {
    pub fn new(
        tx: UnboundedSender<ProgressEvent>,
        transfer_id: impl Into<String>,
        direction: Direction,
        file_count: usize,
        total_bytes: u64,
    ) -> Self {
        Self {
            tx,
            transfer_id: transfer_id.into(),
            direction,
            phase: TransferPhase::Connecting,
            file_name: String::new(),
            file_index: 0,
            file_count,
            file_size: 0,
            file_done: 0,
            completed_bytes: 0,
            total_bytes,
            meter: SpeedMeter::new(),
            last_emit: Instant::now() - Duration::from_secs(1),
            interval: Duration::from_millis(100),
        }
    }

    pub fn set_phase(&mut self, phase: TransferPhase) {
        self.phase = phase;
        self.emit(true);
    }

    /// Resets byte counters when a retry/reconnect re-negotiates resume offsets.
    pub fn restart(&mut self) {
        self.completed_bytes = 0;
        self.file_done = 0;
        self.meter = SpeedMeter::new();
        self.phase = TransferPhase::Connecting;
        self.emit(true);
    }

    pub fn begin_file(&mut self, index: usize, name: &str, size: u64, resume_offset: u64) {
        self.file_index = index;
        self.file_name = name.to_string();
        self.file_size = size;
        self.file_done = resume_offset.min(size);
        self.emit(true);
    }

    pub fn add_bytes(&mut self, bytes: u64) {
        self.file_done = (self.file_done + bytes).min(self.file_size);
        self.meter.add(bytes);
        self.emit(false);
    }

    pub fn finish_file(&mut self) {
        self.completed_bytes = self.completed_bytes.saturating_add(self.file_size);
        self.file_done = 0;
        self.emit(true);
    }

    fn total_done(&self) -> u64 {
        (self.completed_bytes + self.file_done).min(self.total_bytes)
    }

    fn emit(&mut self, force: bool) {
        if !force
            && self.last_emit.elapsed() < self.interval
            && self.phase == TransferPhase::Transferring
        {
            return;
        }
        self.last_emit = Instant::now();
        let done = self.total_done();
        let remaining = self.total_bytes.saturating_sub(done);
        let event = ProgressEvent {
            transfer_id: self.transfer_id.clone(),
            direction: self.direction,
            phase: self.phase,
            file_name: self.file_name.clone(),
            file_index: self.file_index,
            file_count: self.file_count,
            file_bytes: self.file_done,
            file_size: self.file_size,
            total_bytes_done: done,
            total_bytes: self.total_bytes,
            speed_bps: self.meter.speed_bps(),
            avg_bps: self.meter.avg_bps(),
            eta_secs: self.meter.eta_secs(remaining),
        };
        let _ = self.tx.send(event);
    }
}
