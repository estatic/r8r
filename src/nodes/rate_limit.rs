//! Spacing calls to a rate-limited service: every caller sharing a key
//! (an API key's credential) takes the next free slot, `interval` after
//! the previous one, whichever run or agent tool call it comes from.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tokio::time::Instant;

/// Per key: when the next turn is free, and the spacing in effect.
fn next_slots() -> &'static Mutex<HashMap<String, (Instant, Duration)>> {
    static SLOTS: std::sync::OnceLock<Mutex<HashMap<String, (Instant, Duration)>>> =
        std::sync::OnceLock::new();
    SLOTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// How far ahead turns may be handed out. A longer queue (many items at a
/// low limit) is refused rather than holding the key for everyone else
/// for hours; a stopped run's turns also never block past this.
pub const MAX_QUEUE: Duration = Duration::from_secs(600);

/// Keys kept before idle ones are dropped.
const MAX_KEYS: usize = 1024;

/// Waits until this caller's turn for `key`. Turns are handed out in
/// arrival order; while the key's queue is busy they are spaced by the
/// largest interval any caller asked for, so a caller with a looser limit
/// (or none) can't squeeze turns in. Errs at once when the turn would be
/// more than `MAX_QUEUE` away.
pub async fn wait_turn(key: &str, interval: Duration) -> Result<(), String> {
    let slot = {
        let mut slots = next_slots().lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        // Keys whose last turn has passed hold nothing; drop them once
        // there are many (one entry per API key in use is fine to keep).
        if slots.len() > MAX_KEYS {
            slots.retain(|_, (next, _)| *next > now);
        }
        let (slot, spacing) = match slots.get(key) {
            Some((next, kept)) if *next > now => (*next, interval.max(*kept)),
            _ => (now, interval),
        };
        if slot - now > MAX_QUEUE {
            return Err(format!(
                "the rate limit's queue for this API key is full (the next free turn is {} s away); lower the number of items or raise the limit",
                (slot - now).as_secs()
            ));
        }
        slots.insert(key.to_string(), (slot + spacing, spacing));
        slot
    };
    tokio::time::sleep_until(slot).await;
    Ok(())
}

/// The spacing for `per_minute` requests a minute (1 to 6000).
pub fn interval_for(per_minute: u64) -> Option<Duration> {
    (1..=6000).contains(&per_minute).then(|| Duration::from_secs_f64(60.0 / per_minute as f64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn callers_sharing_a_key_are_spaced_by_the_interval() {
        let start = Instant::now();
        let key = "test-spacing";
        let interval = Duration::from_secs(6);
        let handles: Vec<_> = (0..3).map(|_| tokio::spawn(async move {
            wait_turn(key, interval).await.unwrap();
            Instant::now()
        })).collect();
        let mut times = Vec::new();
        for h in handles {
            times.push(h.await.unwrap() - start);
        }
        times.sort();
        assert_eq!(times, vec![Duration::ZERO, Duration::from_secs(6), Duration::from_secs(12)]);
    }

    #[tokio::test(start_paused = true)]
    async fn other_keys_and_a_quiet_key_go_at_once() {
        let start = Instant::now();
        wait_turn("test-a", Duration::from_secs(60)).await.unwrap();
        wait_turn("test-b", Duration::from_secs(60)).await.unwrap();
        assert_eq!(Instant::now() - start, Duration::ZERO);
        tokio::time::sleep(Duration::from_secs(61)).await;
        let before = Instant::now();
        wait_turn("test-a", Duration::from_secs(60)).await.unwrap();
        assert_eq!(Instant::now() - before, Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn a_queue_longer_than_the_limit_is_refused_at_once() {
        // One a minute: turns 0..=10 minutes are handed out, the next is refused.
        let key = "test-full";
        let minute = Duration::from_secs(60);
        let mut waits = Vec::new();
        for _ in 0..11 {
            waits.push(tokio::spawn(async move { wait_turn(key, minute).await }));
        }
        tokio::task::yield_now().await;
        let start = Instant::now();
        let err = wait_turn(key, minute).await.unwrap_err();
        assert!(err.contains("queue for this API key is full"), "{err}");
        assert_eq!(Instant::now() - start, Duration::ZERO, "refused without waiting");
        for w in waits {
            w.abort();
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_caller_with_a_looser_limit_cant_tighten_the_spacing() {
        let start = Instant::now();
        let key = "test-mixed";
        let slow = Duration::from_secs(6);
        wait_turn(key, slow).await.unwrap(); // t=0, next at 6 s
        wait_turn(key, Duration::ZERO).await.unwrap(); // no limit of its own: still waits for 6 s
        assert_eq!(Instant::now() - start, Duration::from_secs(6));
        wait_turn(key, Duration::from_millis(10)).await.unwrap(); // spacing stays 6 s while busy
        assert_eq!(Instant::now() - start, Duration::from_secs(12));
    }

    #[test]
    fn per_minute_becomes_an_interval() {
        assert_eq!(interval_for(10), Some(Duration::from_secs(6)));
        assert_eq!(interval_for(0), None);
        assert_eq!(interval_for(6001), None);
    }
}
