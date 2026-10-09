//! Spacing calls to a rate-limited service: every caller sharing a key
//! (an API key's credential) takes the next free slot, `interval` after
//! the previous one, whichever run or agent tool call it comes from.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tokio::time::Instant;

fn next_slots() -> &'static Mutex<HashMap<String, Instant>> {
    static SLOTS: std::sync::OnceLock<Mutex<HashMap<String, Instant>>> = std::sync::OnceLock::new();
    SLOTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Waits until this caller's turn: at least `interval` after the turn
/// before it for the same `key`. Turns are handed out in arrival order.
pub async fn wait_turn(key: &str, interval: Duration) {
    let slot = {
        let mut slots = next_slots().lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let slot = slots.get(key).copied().filter(|t| *t > now).unwrap_or(now);
        slots.insert(key.to_string(), slot + interval);
        slot
    };
    tokio::time::sleep_until(slot).await;
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
            wait_turn(key, interval).await;
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
        wait_turn("test-a", Duration::from_secs(60)).await;
        wait_turn("test-b", Duration::from_secs(60)).await;
        assert_eq!(Instant::now() - start, Duration::ZERO);
        tokio::time::sleep(Duration::from_secs(61)).await;
        let before = Instant::now();
        wait_turn("test-a", Duration::from_secs(60)).await;
        assert_eq!(Instant::now() - before, Duration::ZERO);
    }

    #[test]
    fn per_minute_becomes_an_interval() {
        assert_eq!(interval_for(10), Some(Duration::from_secs(6)));
        assert_eq!(interval_for(0), None);
        assert_eq!(interval_for(6001), None);
    }
}
