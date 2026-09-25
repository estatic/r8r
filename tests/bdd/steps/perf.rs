//! Non-functional targets (spec §8.1). Tagged @perf and opt-in: absolute
//! numbers depend on the machine, so run them on the reference hardware.

use crate::world::R8rWorld;
use cucumber::{then, when};
use std::time::{Duration, Instant};

#[then(expr = "the server accepted connections within {int} ms of being started")]
async fn ready_within(w: &mut R8rWorld, ms: u64) {
    let ready = w.server().ready_after.expect("server readiness was not measured");
    assert!(ready <= Duration::from_millis(ms), "ready after {ready:?}");
}

#[then(expr = "after {int} seconds idle the server uses at most {int} MB of resident memory")]
async fn idle_rss(w: &mut R8rWorld, secs: u64, mb: f64) {
    tokio::time::sleep(Duration::from_secs(secs)).await;
    let rss = w.server().rss_mb().expect("cannot read /proc/<pid>/status");
    assert!(rss <= mb, "RSS is {rss:.1} MB");
}

#[when(regex = r#"^I send (\d+) ([A-Z]+) requests to "([^"]*)" with concurrency (\d+)$"#)]
async fn load(w: &mut R8rWorld, total: usize, method: String, path: String, concurrency: usize) {
    let url = w.url(&path);
    let client = reqwest::Client::builder().pool_max_idle_per_host(concurrency).build().unwrap();
    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(concurrency));
    let started = Instant::now();
    let mut handles = Vec::with_capacity(total);
    for _ in 0..total {
        let permit = sem.clone().acquire_owned().await.unwrap();
        let (client, url, method) = (client.clone(), url.clone(), method.clone());
        handles.push(tokio::spawn(async move {
            let _permit = permit;
            let t = Instant::now();
            let req = client.request(reqwest::Method::from_bytes(method.as_bytes()).unwrap(), &url);
            let req = if method == "GET" { req } else { req.header("content-type", "application/json").body("{\"n\":1}") };
            let status = req.send().await.map(|r| r.status().as_u16()).unwrap_or(0);
            (status, t.elapsed())
        }));
    }
    w.load.clear();
    for h in handles {
        w.load.push(h.await.unwrap());
    }
    w.vars.insert("LOAD_SECONDS".into(), started.elapsed().as_secs_f64().to_string());
}

#[then(expr = "every load response had the status {int}")]
async fn all_status(w: &mut R8rWorld, status: u16) {
    let bad: Vec<u16> = w.load.iter().map(|(s, _)| *s).filter(|s| *s != status).collect();
    assert!(bad.is_empty(), "{} responses were not {status}, e.g. {:?}", bad.len(), &bad[..bad.len().min(5)]);
}

#[then(expr = "some load responses had the status {int}")]
async fn some_status(w: &mut R8rWorld, status: u16) {
    let seen: Vec<u16> = w.load.iter().map(|(s, _)| *s).collect();
    assert!(seen.contains(&status), "no {status} among statuses {seen:?}");
}

#[then(expr = "the p{int} latency is at most {int} ms")]
async fn percentile(w: &mut R8rWorld, p: usize, ms: u64) {
    let mut lat: Vec<Duration> = w.load.iter().map(|(_, d)| *d).collect();
    lat.sort();
    let idx = ((lat.len() * p).div_ceil(100)).saturating_sub(1).min(lat.len() - 1);
    assert!(lat[idx] <= Duration::from_millis(ms), "p{p} = {:?}", lat[idx]);
}

#[then(expr = "the throughput is at least {int} requests per second")]
async fn throughput(w: &mut R8rWorld, rps: f64) {
    let secs: f64 = w.var("LOAD_SECONDS").parse().unwrap();
    let actual = w.load.len() as f64 / secs;
    assert!(actual >= rps, "throughput {actual:.0} req/s");
}
