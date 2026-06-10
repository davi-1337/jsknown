use anyhow::Result;
use reqwest::Client;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::sync::{Mutex, Semaphore};

#[derive(Clone)]
pub struct Fetcher {
    client: Client,
    limiter: Arc<RateLimiter>,
    semaphore: Arc<Semaphore>,
}

impl Fetcher {
    pub fn new(rate_per_second: u32, rate_per_minute: u32, concurrency: usize) -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .danger_accept_invalid_certs(true)
                .user_agent("Mozilla/5.0 jsknown")
                .build()?,
            limiter: Arc::new(RateLimiter::new(rate_per_second, rate_per_minute)),
            semaphore: Arc::new(Semaphore::new(concurrency.max(1))),
        })
    }

    pub async fn get(
        &self,
        url: &str,
        headers: &BTreeMap<String, String>,
    ) -> Result<Option<String>> {
        let _permit = self.semaphore.acquire().await?;
        self.limiter.wait().await;
        let mut request = self.client.get(url).header("accept", "*/*");
        for (key, value) in headers {
            request = request.header(key, value);
        }
        let response = request.send().await?;
        if !response.status().is_success() {
            return Ok(None);
        }
        Ok(Some(response.text().await?))
    }
}

struct RateLimiter {
    per_second: u32,
    per_minute: u32,
    state: Mutex<RateState>,
}

#[derive(Default)]
struct RateState {
    second_start: Option<std::time::Instant>,
    second_count: u32,
    minute_start: Option<std::time::Instant>,
    minute_count: u32,
}

impl RateState {
    fn check_second(&mut self, now: std::time::Instant, limit: u32) -> Option<Duration> {
        check_window(
            now,
            &mut self.second_start,
            &mut self.second_count,
            limit,
            Duration::from_secs(1),
        )
    }

    fn check_minute(&mut self, now: std::time::Instant, limit: u32) -> Option<Duration> {
        check_window(
            now,
            &mut self.minute_start,
            &mut self.minute_count,
            limit,
            Duration::from_secs(60),
        )
    }
}

impl RateLimiter {
    fn new(per_second: u32, per_minute: u32) -> Self {
        Self {
            per_second,
            per_minute,
            state: Mutex::new(RateState::default()),
        }
    }

    async fn wait(&self) {
        loop {
            let delay = {
                let mut state = self.state.lock().await;
                let now = std::time::Instant::now();
                state
                    .check_second(now, self.per_second)
                    .or_else(|| state.check_minute(now, self.per_minute))
            };
            if let Some(delay) = delay {
                tokio::time::sleep(delay).await;
            } else {
                return;
            }
        }
    }
}

fn check_window(
    now: std::time::Instant,
    start: &mut Option<std::time::Instant>,
    count: &mut u32,
    limit: u32,
    window: Duration,
) -> Option<Duration> {
    if limit == 0 {
        return None;
    }
    let current_start = start.get_or_insert(now);
    if now.duration_since(*current_start) >= window {
        *current_start = now;
        *count = 0;
    }
    if *count >= limit {
        return Some(window.saturating_sub(now.duration_since(*current_start)));
    }
    *count += 1;
    None
}
