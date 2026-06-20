//! Per-circuit traffic metering. A relay counts the bytes it forwards for a client's
//! circuit and, at the window boundary, emits a [`ReceiptCore`] the client and relay
//! co-sign. Per-(client,circuit) token-bucket rate limiting drops abusive traffic so
//! it is never metered (and therefore never billed).

use crate::receipt::ReceiptCore;

/// Accumulates relayed bytes for one circuit over one settlement window.
#[derive(Clone, Debug)]
pub struct CircuitMeter {
    pub client: [u8; 32],
    pub operator: [u8; 32],
    pub node_id: u64,
    pub window_start: u64,
    bytes: u64,
}

impl CircuitMeter {
    pub fn new(client: [u8; 32], operator: [u8; 32], node_id: u64, window_start: u64) -> Self {
        Self {
            client,
            operator,
            node_id,
            window_start,
            bytes: 0,
        }
    }

    pub fn record(&mut self, n: u64) {
        self.bytes = self.bytes.saturating_add(n);
    }

    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Close the window into a signable receipt core.
    pub fn into_core(self, window_end: u64, nonce: u64) -> ReceiptCore {
        ReceiptCore {
            client: self.client,
            operator: self.operator,
            node_id: self.node_id,
            bytes: self.bytes,
            window_start: self.window_start,
            window_end,
            nonce,
        }
    }
}

/// A simple token bucket for per-circuit rate limiting (bytes/sec with a burst).
#[derive(Clone, Debug)]
pub struct TokenBucket {
    capacity: u64,
    tokens: u64,
    refill_per_sec: u64,
    last_refill: u64,
}

impl TokenBucket {
    pub fn new(capacity: u64, refill_per_sec: u64, now: u64) -> Self {
        Self {
            capacity,
            tokens: capacity,
            refill_per_sec,
            last_refill: now,
        }
    }

    /// Try to admit `n` bytes at time `now`; returns false (drop) if over limit.
    pub fn admit(&mut self, n: u64, now: u64) -> bool {
        let elapsed = now.saturating_sub(self.last_refill);
        self.tokens = self
            .tokens
            .saturating_add(elapsed.saturating_mul(self.refill_per_sec))
            .min(self.capacity);
        self.last_refill = now;
        if self.tokens >= n {
            self.tokens -= n;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_accumulates_and_closes() {
        let mut m = CircuitMeter::new([1; 32], [2; 32], 7, 600);
        m.record(1_000);
        m.record(500);
        assert_eq!(m.bytes(), 1_500);
        let core = m.into_core(1_200, 1);
        assert_eq!(core.bytes, 1_500);
        assert_eq!(core.window_end, 1_200);
    }

    #[test]
    fn token_bucket_limits_then_refills() {
        let mut tb = TokenBucket::new(1_000, 100, 0);
        assert!(tb.admit(800, 0));
        assert!(!tb.admit(800, 0)); // only 200 left
        assert!(tb.admit(200, 0));
        assert!(tb.admit(500, 10)); // +1000 refilled over 10s, capped at 1000
    }
}
