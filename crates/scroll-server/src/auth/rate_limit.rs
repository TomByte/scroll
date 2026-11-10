//! Limits on sign-in and sign-up attempts, counted in fixed windows in memory by each server process.
//!
//! Every attempt counts against the client's address. Only failed sign-ins count against an email, and only
//! from the address they came from, so nobody can lock a user out by failing to sign in as them from
//! elsewhere: guessing is slowed per address, and the real user's own address is never refused for it.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::Duration;

use tokio::time::Instant;

use crate::lock;

/// Above this many tracked keys, expired windows are dropped so the map cannot grow without bound.
const PRUNE_ABOVE_KEYS: usize = 10_000;

#[derive(Debug, Clone, Copy)]
pub struct Limit {
    pub attempts: u32,
    pub window: Duration,
}

#[derive(Debug, Clone, Copy)]
pub struct AttemptLimits {
    /// Every sign-in and sign-up attempt from one address.
    pub per_address: Limit,
    /// Failed sign-ins for one email from one address.
    pub failures_per_address_and_email: Limit,
}

impl Default for AttemptLimits {
    fn default() -> Self {
        let window = Duration::from_secs(600);
        Self {
            per_address: Limit {
                attempts: 60,
                window,
            },
            failures_per_address_and_email: Limit {
                attempts: 5,
                window,
            },
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("too many attempts; retry in {retry_after:?}")]
pub struct TooManyAttempts {
    pub retry_after: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    Address(IpAddr),
    AddressAndEmail(IpAddr, String),
}

struct Window {
    started: Instant,
    count: u32,
}

impl Window {
    fn refuse_if_full(&self, limit: Limit, now: Instant) -> Result<(), TooManyAttempts> {
        let resets_at = self.started + limit.window;
        if now < resets_at && self.count >= limit.attempts {
            return Err(TooManyAttempts {
                retry_after: resets_at - now,
            });
        }
        Ok(())
    }
}

pub struct Attempts {
    per_address: Limit,
    /// `None` when only addresses are limited.
    failures_per_address_and_email: Option<Limit>,
    windows: Mutex<HashMap<Key, Window>>,
}

impl Attempts {
    pub fn new(limits: AttemptLimits) -> Self {
        Self {
            per_address: limits.per_address,
            failures_per_address_and_email: Some(limits.failures_per_address_and_email),
            windows: Mutex::default(),
        }
    }

    /// For requests that carry no email: `admit_address` only.
    pub fn per_address_only(per_address: Limit) -> Self {
        Self {
            per_address,
            failures_per_address_and_email: None,
            windows: Mutex::default(),
        }
    }

    /// Refuses the attempt if this address is over its limit for this email, or `admit_address` refuses it.
    pub fn admit(&self, ip: IpAddr, email: &str) -> Result<(), TooManyAttempts> {
        if let Some(limit) = self.failures_per_address_and_email {
            let key = Key::AddressAndEmail(ip, email.to_owned());
            if let Some(window) = lock(&self.windows).get(&key) {
                window.refuse_if_full(limit, Instant::now())?;
            }
        }
        self.admit_address(ip)
    }

    /// Refuses the request if the address is over its limit; otherwise counts it.
    pub fn admit_address(&self, ip: IpAddr) -> Result<(), TooManyAttempts> {
        let now = Instant::now();
        let mut windows = lock(&self.windows);
        if windows.len() > PRUNE_ABOVE_KEYS {
            windows.retain(|key, window| now < window.started + self.limit_for(key).window);
        }
        let key = Key::Address(ip);
        if let Some(window) = windows.get(&key) {
            window.refuse_if_full(self.per_address, now)?;
        }
        count(&mut windows, key, self.per_address, now);
        Ok(())
    }

    pub fn record_failure(&self, ip: IpAddr, email: &str) {
        let Some(limit) = self.failures_per_address_and_email else {
            return;
        };
        let key = Key::AddressAndEmail(ip, email.to_owned());
        count(&mut lock(&self.windows), key, limit, Instant::now());
    }

    fn limit_for(&self, key: &Key) -> Limit {
        match key {
            Key::Address(_) => self.per_address,
            Key::AddressAndEmail(..) => self
                .failures_per_address_and_email
                .unwrap_or(self.per_address),
        }
    }
}

fn count(windows: &mut HashMap<Key, Window>, key: Key, limit: Limit, now: Instant) {
    let window = windows.entry(key).or_insert(Window {
        started: now,
        count: 0,
    });
    if now >= window.started + limit.window {
        *window = Window {
            started: now,
            count: 0,
        };
    }
    window.count += 1;
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    const LIMITS: AttemptLimits = AttemptLimits {
        per_address: Limit {
            attempts: 4,
            window: Duration::from_secs(60),
        },
        failures_per_address_and_email: Limit {
            attempts: 2,
            window: Duration::from_secs(60),
        },
    };
    const EMAIL: &str = "a@example.test";

    fn ip(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, last))
    }

    #[test]
    fn failures_for_an_email_limit_only_the_address_they_came_from() {
        let attempts = Attempts::new(LIMITS);
        for _ in 0..2 {
            attempts.admit(ip(1), EMAIL).unwrap();
            attempts.record_failure(ip(1), EMAIL);
        }

        assert!(attempts.admit(ip(1), EMAIL).is_err());
        assert!(attempts.admit(ip(1), "b@example.test").is_ok());
        assert!(attempts.admit(ip(2), EMAIL).is_ok());
    }

    #[test]
    fn successful_attempts_do_not_count_against_an_email() {
        let attempts = Attempts::new(LIMITS);
        for _ in 0..3 {
            assert!(attempts.admit(ip(1), EMAIL).is_ok());
        }
    }

    #[test]
    fn every_attempt_counts_against_its_address() {
        let attempts = Attempts::new(LIMITS);
        for _ in 0..4 {
            attempts.admit(ip(1), EMAIL).unwrap();
        }

        assert!(attempts.admit(ip(1), "b@example.test").is_err());
        assert!(attempts.admit(ip(2), EMAIL).is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn a_window_resets_after_it_ends() {
        let attempts = Attempts::new(LIMITS);
        for _ in 0..2 {
            attempts.admit(ip(1), EMAIL).unwrap();
            attempts.record_failure(ip(1), EMAIL);
        }
        let refused = attempts.admit(ip(1), EMAIL).unwrap_err();
        assert_eq!(refused.retry_after, Duration::from_secs(60));

        tokio::time::advance(Duration::from_secs(60)).await;

        assert!(attempts.admit(ip(1), EMAIL).is_ok());
    }
}
