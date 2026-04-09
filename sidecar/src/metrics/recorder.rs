use anyhow::Result;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestEvent {
    pub ts: i64,
    pub group_alias: String,
    pub provider_id: String,
    pub model_id: String,
    pub prompt_tokens: i64,
    pub output_tokens: i64,
    pub latency_ms: i64,
    pub status: String,
    pub error_type: Option<String>,
    pub input_cost_per_1m: Option<f64>,
    pub output_cost_per_1m: Option<f64>,
}

impl RequestEvent {
    pub fn calculate_cost(&self) -> f64 {
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

#[derive(Debug, Clone)]
pub struct MetricsRecorder {
    sender: mpsc::Sender<RequestEvent>,
}

impl MetricsRecorder {
    pub fn new(conn: Connection) -> (Self, tokio::task::JoinHandle<()>) {
        let (tx, mut rx) = mpsc::channel::<RequestEvent>(1024);

        let handle = tokio::spawn(async move {
            let mut conn = conn;
            while let Some(event) = rx.recv().await {
                let cost = event.calculate_cost();
                let _ = insert_request(&mut conn, &event, cost);
            }
        });

        (Self { sender: tx }, handle)
    }

    pub async fn record_request(&self, event: RequestEvent) -> Result<()> {
        self.sender
            .send(event)
            .await
            .map_err(|_| anyhow::anyhow!("Metrics recorder channel closed"))
    }

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
