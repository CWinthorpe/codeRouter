use anyhow::Result;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// A recorded API request event, carrying all data needed to persist it to the metrics store.
///
/// The optional `input_cost_per_1m` / `output_cost_per_1m` fields allow
/// per-request cost calculation when pricing data is available from the
/// provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestEvent {
    /// Unix-epoch timestamp of the request.
    pub ts: i64,
    /// Router group alias this request was routed through.
    pub group_alias: String,
    /// Provider identifier (e.g. `"openai"`, `"anthropic"`).
    pub provider_id: String,
    /// Model identifier within the provider (e.g. `"gpt-4o"`).
    pub model_id: String,
    /// Number of tokens in the prompt / input.
    pub prompt_tokens: i64,
    /// Number of tokens in the completion / output.
    pub output_tokens: i64,
    /// Round-trip latency in milliseconds.
    pub latency_ms: i64,
    /// Request outcome — `"success"` or `"error"`.
    pub status: String,
    /// Error classification when the request failed (e.g. `"429"`).
    pub error_type: Option<String>,
    /// Cost per 1M input tokens in USD, if known.
    pub input_cost_per_1m: Option<f64>,
    /// Cost per 1M output tokens in USD, if known.
    pub output_cost_per_1m: Option<f64>,
}

impl RequestEvent {
    /// Computes the total cost in USD for this request based on per-1M-token pricing.
    ///
    /// When either price component is `None`, its contribution is treated as zero —
    /// this lets callers record requests even when pricing data is unavailable.
    pub fn calculate_cost(&self) -> f64 {
        // Price is per 1M tokens, so divide token count by 1M before multiplying.
        let input_cost = match self.input_cost_per_1m {
            Some(cost) => (self.prompt_tokens as f64 / 1_000_000.0) * cost,
            None => 0.0,
        };
        let output_cost = match self.output_cost_per_1m {
            Some(cost) => (self.output_tokens as f64 / 1_000_000.0) * cost,
            None => 0.0,
        };
        input_cost + output_cost
    }
}

/// A non-blocking metrics recorder that sends [`RequestEvent`]s through a channel
/// to a background tokio task for database insertion.
///
/// The recorder is clone-safe: the sender side can be shared across tasks.
#[derive(Debug, Clone)]
pub struct MetricsRecorder {
    sender: mpsc::Sender<RequestEvent>,
}

impl MetricsRecorder {
    /// Creates a new recorder backed by the given connection.
    ///
    /// Spawns a background tokio task that drains the channel and inserts each
    /// event into the database. The returned [`JoinHandle`] should **not** be
    /// aborted lightly — dropping the [`MetricsRecorder`] (which closes the
    /// channel) will cause the task to exit gracefully after all queued events
    /// have been processed.
    ///
    /// # Returns
    ///
    /// A tuple of `(MetricsRecorder, JoinHandle)` where the handle represents
    /// the background insertion task.
    pub fn new(conn: Connection) -> (Self, tokio::task::JoinHandle<()>) {
        // Bounded channel prevents unbounded memory growth if the DB cannot keep up.
        let (tx, mut rx) = mpsc::channel::<RequestEvent>(1024);

        let handle = tokio::spawn(async move {
            let mut conn = conn;
            // Drain events until the sender side is dropped, then exit.
            while let Some(event) = rx.recv().await {
                let cost = event.calculate_cost();
                // Errors during insertion are intentionally ignored — metrics
                // are best-effort and must not crash the proxy.
                let _ = insert_request(&mut conn, &event, cost);
            }
        });

        (Self { sender: tx }, handle)
    }

    /// Enqueues a [`RequestEvent`] for asynchronous insertion.
    ///
    /// Awaits until the channel has capacity. Returns an error only if the
    /// background task has already exited (channel closed).
    ///
    /// # Errors
    ///
    /// Returns an error if the receiver has been dropped.
    pub async fn record_request(&self, event: RequestEvent) -> Result<()> {
        self.sender
            .send(event)
            .await
            .map_err(|_| anyhow::anyhow!("Metrics recorder channel closed"))
    }

    /// Enqueues a [`RequestEvent`] without awaiting — suitable for call-sites
    /// that are not in an async context and cannot block.
    ///
    /// If the channel is full, the event is dropped and an error is returned.
    /// This prevents non-async callers from blocking when the DB falls behind.
    ///
    /// # Errors
    ///
    /// Returns an error if the channel is full or closed.
    pub fn record_request_sync(&self, event: RequestEvent) -> Result<()> {
        self.sender
            .try_send(event)
            .map_err(|e| {
                eprintln!("[metrics] dropped event: channel full");
                anyhow::anyhow!("Failed to send event to metrics recorder: {}", e)
            })?;
        Ok(())
    }
}

/// Inserts a single request row into the database.
///
/// This is the internal function used by the background recorder task. It
/// executes a parameterised `INSERT` statement.
///
/// # Errors
///
/// Returns an error if the SQL execution fails.
fn insert_request(conn: &mut Connection, event: &RequestEvent, cost: f64) -> Result<()> {
    conn.execute(
        "INSERT INTO requests (ts, group_alias, provider_id, model_id, prompt_tokens, output_tokens, cost_usd, latency_ms, status, error_type)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            event.ts,
            event.group_alias,
            event.provider_id,
            event.model_id,
            event.prompt_tokens,
            event.output_tokens,
            cost,
            event.latency_ms,
            event.status,
            event.error_type,
        ],
    )?;
    Ok(())
}

/// Synchronous, single-threaded helper for inserting a [`RequestEvent`] directly
/// into the database. Intended **only** for use in unit tests where the
/// channel-based recorder is not available.
///
/// # Errors
///
/// Propagates any SQLite insert error.
pub fn record_request_sync_for_test(conn: &mut Connection, event: &RequestEvent) -> Result<()> {
    let cost = event.calculate_cost();
    insert_request(conn, event, cost)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::db;
    use chrono::Utc;

    #[test]
    fn test_cost_calculation_with_pricing() {
        let event = RequestEvent {
            ts: Utc::now().timestamp(),
            group_alias: "test-group".to_string(),
            provider_id: "test-provider".to_string(),
            model_id: "test-model".to_string(),
            prompt_tokens: 1_000_000,
            output_tokens: 500_000,
            latency_ms: 100,
            status: "success".to_string(),
            error_type: None,
            input_cost_per_1m: Some(3.0),
            output_cost_per_1m: Some(15.0),
        };
        let cost = event.calculate_cost();
        assert!((cost - 10.5).abs() < 0.001);
    }

    #[test]
    fn test_cost_calculation_without_pricing() {
        let event = RequestEvent {
            ts: Utc::now().timestamp(),
            group_alias: "test-group".to_string(),
            provider_id: "test-provider".to_string(),
            model_id: "test-model".to_string(),
            prompt_tokens: 1_000_000,
            output_tokens: 500_000,
            latency_ms: 100,
            status: "success".to_string(),
            error_type: None,
            input_cost_per_1m: None,
            output_cost_per_1m: None,
        };
        let cost = event.calculate_cost();
        assert_eq!(cost, 0.0);
    }

    #[test]
    fn test_cost_calculation_partial_pricing() {
        let event = RequestEvent {
            ts: Utc::now().timestamp(),
            group_alias: "test-group".to_string(),
            provider_id: "test-provider".to_string(),
            model_id: "test-model".to_string(),
            prompt_tokens: 1_000_000,
            output_tokens: 500_000,
            latency_ms: 100,
            status: "success".to_string(),
            error_type: None,
            input_cost_per_1m: Some(3.0),
            output_cost_per_1m: None,
        };
        let cost = event.calculate_cost();
        assert!((cost - 3.0).abs() < 0.001);
    }

    #[test]
    fn test_record_request_insert() {
        let mut conn = db::init_in_memory_db().expect("Failed to init DB");
        let event = RequestEvent {
            ts: Utc::now().timestamp(),
            group_alias: "test-group".to_string(),
            provider_id: "test-provider".to_string(),
            model_id: "test-model".to_string(),
            prompt_tokens: 100,
            output_tokens: 50,
            latency_ms: 200,
            status: "success".to_string(),
            error_type: None,
            input_cost_per_1m: Some(1.0),
            output_cost_per_1m: Some(2.0),
        };
        record_request_sync_for_test(&mut conn, &event).expect("Failed to insert request");

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM requests", [], |r| r.get(0))
            .expect("Failed to count rows");
        assert_eq!(count, 1);

        let cost: f64 = conn
            .query_row("SELECT cost_usd FROM requests LIMIT 1", [], |r| r.get(0))
            .expect("Failed to get cost");
        let expected = (100.0 / 1_000_000.0) * 1.0 + (50.0 / 1_000_000.0) * 2.0;
        assert!((cost - expected).abs() < 0.0001);
    }

    #[test]
    fn test_record_error_request() {
        let mut conn = db::init_in_memory_db().expect("Failed to init DB");
        let event = RequestEvent {
            ts: Utc::now().timestamp(),
            group_alias: "test-group".to_string(),
            provider_id: "test-provider".to_string(),
            model_id: "test-model".to_string(),
            prompt_tokens: 0,
            output_tokens: 0,
            latency_ms: 5000,
            status: "error".to_string(),
            error_type: Some("429".to_string()),
            input_cost_per_1m: None,
            output_cost_per_1m: None,
        };
        record_request_sync_for_test(&mut conn, &event).expect("Failed to insert request");

        let status: String = conn
            .query_row("SELECT status FROM requests LIMIT 1", [], |r| r.get(0))
            .expect("Failed to get status");
        assert_eq!(status, "error");

        let error_type: Option<String> = conn
            .query_row("SELECT error_type FROM requests LIMIT 1", [], |r| r.get(0))
            .expect("Failed to get error_type");
        assert_eq!(error_type, Some("429".to_string()));
    }

    #[tokio::test]
    async fn test_channel_recorder() {
        let conn = db::init_in_memory_db().expect("Failed to init DB");
        let (recorder, handle) = MetricsRecorder::new(conn);

        let event = RequestEvent {
            ts: Utc::now().timestamp(),
            group_alias: "channel-test".to_string(),
            provider_id: "channel-provider".to_string(),
            model_id: "channel-model".to_string(),
            prompt_tokens: 500,
            output_tokens: 250,
            latency_ms: 150,
            status: "success".to_string(),
            error_type: None,
            input_cost_per_1m: Some(2.0),
            output_cost_per_1m: Some(4.0),
        };

        recorder.record_request(event).await.expect("Failed to send event");

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        drop(recorder);
        handle.await.expect("Recorder task panicked");
    }
}