use anyhow::Result;
use chrono::{NaiveDate, Utc};
use rusqlite::Connection;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct DailySummary {
    pub total_requests: i64,
    pub total_prompt_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cost: f64,
    pub error_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RequestRow {
    pub id: i64,
    pub ts: i64,
    pub group_alias: String,
    pub provider_id: String,
    pub model_id: String,
    pub prompt_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: f64,
    pub latency_ms: i64,
    pub status: String,
    pub error_type: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DailyUsage {
    pub date: String,
    pub total_requests: i64,
    pub total_prompt_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cost: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupUsage {
    pub group_alias: String,
    pub total_requests: i64,
    pub total_prompt_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cost: f64,
}

pub fn get_daily_summary(
    conn: &Connection,
    provider_id: &str,
    date: NaiveDate,
) -> Result<DailySummary> {
    let reset_hour = 0u32;
    let start_dt = date.and_hms_opt(reset_hour, 0, 0).unwrap().and_utc();
    let end_dt = date
        .succ_opt()
        .unwrap()
        .and_hms_opt(reset_hour, 0, 0)
        .unwrap()
        .and_utc();
    let start_ts = start_dt.timestamp();
    let end_ts = end_dt.timestamp();

    let mut stmt = conn.prepare(
        "SELECT 
            COUNT(*) as total_requests,
            COALESCE(SUM(prompt_tokens), 0) as total_prompt_tokens,
            COALESCE(SUM(output_tokens), 0) as total_output_tokens,
            COALESCE(SUM(cost_usd), 0.0) as total_cost,
            COALESCE(SUM(CASE WHEN status != 'success' THEN 1 ELSE 0 END), 0) as error_count
         FROM requests
         WHERE provider_id = ?1 AND ts >= ?2 AND ts < ?3",
    )?;

    let summary = stmt.query_row(
        [provider_id, &start_ts.to_string(), &end_ts.to_string()],
        |row| {
            Ok(DailySummary {
                total_requests: row.get(0)?,
                total_prompt_tokens: row.get(1)?,
                total_output_tokens: row.get(2)?,
                total_cost: row.get(3)?,
                error_count: row.get(4)?,
            })
        },
    )?;

    Ok(summary)
}

pub fn get_recent_requests(conn: &Connection, limit: usize) -> Result<Vec<RequestRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, ts, group_alias, provider_id, model_id, prompt_tokens, output_tokens, cost_usd, latency_ms, status, error_type
         FROM requests
         ORDER BY ts DESC, id DESC
         LIMIT ?1",
    )?;

    let rows = stmt.query_map([&(limit as i64)], |row| {
        Ok(RequestRow {
            id: row.get(0)?,
            ts: row.get(1)?,
            group_alias: row.get(2)?,
            provider_id: row.get(3)?,
            model_id: row.get(4)?,
            prompt_tokens: row.get(5)?,
            output_tokens: row.get(6)?,
            cost_usd: row.get(7)?,
            latency_ms: row.get(8)?,
            status: row.get(9)?,
            error_type: row.get(10)?,
        })
    })?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("Failed to collect recent requests: {}", e))
}

pub fn get_usage_by_day(
    conn: &Connection,
    provider_id: &str,
    days: u32,
) -> Result<Vec<DailyUsage>> {
    let now = Utc::now();
    let reset_hour = 0u32;
    let start_date = (now - chrono::Duration::days(days as i64)).date_naive();
    let start_ts = start_date
        .and_hms_opt(reset_hour, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp();

    let mut stmt = conn.prepare(
        "SELECT 
            DATE(ts, 'unixepoch') as date,
            COUNT(*) as total_requests,
            COALESCE(SUM(prompt_tokens), 0) as total_prompt_tokens,
            COALESCE(SUM(output_tokens), 0) as total_output_tokens,
            COALESCE(SUM(cost_usd), 0.0) as total_cost
         FROM requests
         WHERE provider_id = ?1 AND ts >= ?2
         GROUP BY DATE(ts, 'unixepoch')
         ORDER BY date ASC",
    )?;

    let rows = stmt.query_map([provider_id, &start_ts.to_string()], |row| {
        Ok(DailyUsage {
            date: row.get(0)?,
            total_requests: row.get(1)?,
            total_prompt_tokens: row.get(2)?,
            total_output_tokens: row.get(3)?,
            total_cost: row.get(4)?,
        })
    })?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("Failed to collect daily usage: {}", e))
}

pub fn get_usage_by_group(conn: &Connection, days: u32) -> Result<Vec<GroupUsage>> {
    let now = Utc::now();
    let start_ts = (now - chrono::Duration::days(days as i64)).timestamp();

    let mut stmt = conn.prepare(
        "SELECT 
            group_alias,
            COUNT(*) as total_requests,
            COALESCE(SUM(prompt_tokens), 0) as total_prompt_tokens,
            COALESCE(SUM(output_tokens), 0) as total_output_tokens,
            COALESCE(SUM(cost_usd), 0.0) as total_cost
         FROM requests
         WHERE ts >= ?1
         GROUP BY group_alias
         ORDER BY total_requests DESC",
    )?;

    let rows = stmt.query_map([&start_ts.to_string()], |row| {
        Ok(GroupUsage {
            group_alias: row.get(0)?,
            total_requests: row.get(1)?,
            total_prompt_tokens: row.get(2)?,
            total_output_tokens: row.get(3)?,
            total_cost: row.get(4)?,
        })
    })?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("Failed to collect group usage: {}", e))
}

#[derive(Debug, Clone, Serialize)]
pub struct LatencyPercentiles {
    pub p50: i64,
    pub p95: i64,
}

pub fn get_latency_percentiles(
    conn: &Connection,
    provider_id: &str,
    date: NaiveDate,
) -> Result<Option<LatencyPercentiles>> {
    let reset_hour = 0u32;
    let start_dt = date.and_hms_opt(reset_hour, 0, 0).unwrap().and_utc();
    let end_dt = date
        .succ_opt()
        .unwrap()
        .and_hms_opt(reset_hour, 0, 0)
        .unwrap()
        .and_utc();
    let start_ts = start_dt.timestamp();
    let end_ts = end_dt.timestamp();

    let mut stmt = conn.prepare(
        "SELECT latency_ms FROM requests
         WHERE provider_id = ?1 AND ts >= ?2 AND ts < ?3 AND latency_ms > 0
         ORDER BY latency_ms",
    )?;

    let latencies: Result<Vec<i64>> = stmt
        .query_map(
            [provider_id, &start_ts.to_string(), &end_ts.to_string()],
            |row| row.get(0),
        )?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("Failed to collect latencies: {}", e));

    let latencies = latencies?;
    if latencies.is_empty() {
        return Ok(None);
    }

    let n = latencies.len();
    let p50_idx = (n as f64 * 0.50).ceil() as usize - 1;
    let p95_idx = (n as f64 * 0.95).ceil() as usize - 1;

    Ok(Some(LatencyPercentiles {
        p50: latencies[p50_idx.min(n - 1)],
        p95: latencies[p95_idx.min(n - 1)],
    }))
}

pub fn get_today_token_totals(
    conn: &Connection,
    quota_reset_utc_hour: u32,
) -> Result<Vec<(String, u64)>> {
    let now = Utc::now();
    let today_start = now
        .date_naive()
        .and_hms_opt(quota_reset_utc_hour, 0, 0)
        .unwrap()
        .and_utc();
    let start_ts = if today_start <= now {
        today_start.timestamp()
    } else {
        (today_start - chrono::Duration::days(1)).timestamp()
    };

    let mut stmt = conn.prepare(
        "SELECT provider_id, COALESCE(SUM(prompt_tokens + output_tokens), 0) as total_tokens
         FROM requests
         WHERE ts >= ?1
         GROUP BY provider_id",
    )?;

    let rows = stmt.query_map([&start_ts.to_string()], |row| {
        let provider_id: String = row.get(0)?;
        let total_tokens: i64 = row.get(1)?;
        Ok((provider_id, total_tokens as u64))
    })?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("Failed to collect today's token totals: {}", e))
}

pub fn get_today_request_counts(
    conn: &Connection,
    quota_reset_utc_hour: u32,
) -> Result<Vec<(String, u64)>> {
    let now = Utc::now();
    let today_start = now
        .date_naive()
        .and_hms_opt(quota_reset_utc_hour, 0, 0)
        .unwrap()
        .and_utc();
    let start_ts = if today_start <= now {
        today_start.timestamp()
    } else {
        (today_start - chrono::Duration::days(1)).timestamp()
    };

    let mut stmt = conn.prepare(
        "SELECT provider_id, COUNT(*) as total_requests
         FROM requests
         WHERE ts >= ?1
         GROUP BY provider_id",
    )?;

    let rows = stmt.query_map([&start_ts.to_string()], |row| {
        let provider_id: String = row.get(0)?;
        let total_requests: i64 = row.get(1)?;
        Ok((provider_id, total_requests as u64))
    })?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("Failed to collect today's request counts: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::db;
    use crate::metrics::recorder::{record_request_sync_for_test, RequestEvent};

    fn insert_test_requests(conn: &mut Connection) {
        let today = Utc::now().date_naive();
        let yesterday = today - chrono::Duration::days(1);

        let events = vec![
            RequestEvent {
                ts: today.and_hms_opt(10, 0, 0).unwrap().and_utc().timestamp(),
                group_alias: "glm-5-router".to_string(),
                provider_id: "provider-a".to_string(),
                model_id: "glm-4.5".to_string(),
                prompt_tokens: 1000,
                output_tokens: 500,
                latency_ms: 200,
                status: "success".to_string(),
                error_type: None,
                input_cost_per_1m: Some(3.0),
                output_cost_per_1m: Some(15.0),
            },
            RequestEvent {
                ts: today.and_hms_opt(12, 0, 0).unwrap().and_utc().timestamp(),
                group_alias: "glm-5-router".to_string(),
                provider_id: "provider-a".to_string(),
                model_id: "glm-4.5".to_string(),
                prompt_tokens: 2000,
                output_tokens: 1000,
                latency_ms: 300,
                status: "success".to_string(),
                error_type: None,
                input_cost_per_1m: Some(3.0),
                output_cost_per_1m: Some(15.0),
            },
            RequestEvent {
                ts: today.and_hms_opt(14, 0, 0).unwrap().and_utc().timestamp(),
                group_alias: "fast-model".to_string(),
                provider_id: "provider-b".to_string(),
                model_id: "fast-model-v1".to_string(),
                prompt_tokens: 500,
                output_tokens: 200,
                latency_ms: 100,
                status: "error".to_string(),
                error_type: Some("429".to_string()),
                input_cost_per_1m: None,
                output_cost_per_1m: None,
            },
            RequestEvent {
                ts: yesterday
                    .and_hms_opt(10, 0, 0)
                    .unwrap()
                    .and_utc()
                    .timestamp(),
                group_alias: "glm-5-router".to_string(),
                provider_id: "provider-a".to_string(),
                model_id: "glm-4.5".to_string(),
                prompt_tokens: 800,
                output_tokens: 400,
                latency_ms: 250,
                status: "success".to_string(),
                error_type: None,
                input_cost_per_1m: Some(3.0),
                output_cost_per_1m: Some(15.0),
            },
        ];

        for event in &events {
            record_request_sync_for_test(conn, event).expect("Failed to insert test request");
        }
    }

    #[test]
    fn test_get_daily_summary() {
        let mut conn = db::init_in_memory_db().expect("Failed to init DB");
        insert_test_requests(&mut conn);

        let today = Utc::now().date_naive();
        let summary =
            get_daily_summary(&conn, "provider-a", today).expect("Failed to get daily summary");

        assert_eq!(summary.total_requests, 2);
        assert_eq!(summary.total_prompt_tokens, 3000);
        assert_eq!(summary.total_output_tokens, 1500);
        assert_eq!(summary.error_count, 0);
        assert!(summary.total_cost > 0.0);
    }

    #[test]
    fn test_get_daily_summary_no_data() {
        let conn = db::init_in_memory_db().expect("Failed to init DB");
        let today = Utc::now().date_naive();
        let summary =
            get_daily_summary(&conn, "nonexistent", today).expect("Failed to get daily summary");

        assert_eq!(summary.total_requests, 0);
        assert_eq!(summary.total_prompt_tokens, 0);
        assert_eq!(summary.total_output_tokens, 0);
        assert_eq!(summary.total_cost, 0.0);
        assert_eq!(summary.error_count, 0);
    }

    #[test]
    fn test_get_recent_requests() {
        let mut conn = db::init_in_memory_db().expect("Failed to init DB");
        insert_test_requests(&mut conn);

        let requests = get_recent_requests(&conn, 2).expect("Failed to get recent requests");
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].group_alias, "fast-model");
        assert_eq!(requests[1].group_alias, "glm-5-router");
    }

    #[test]
    fn test_get_recent_requests_all() {
        let mut conn = db::init_in_memory_db().expect("Failed to init DB");
        insert_test_requests(&mut conn);

        let requests = get_recent_requests(&conn, 100).expect("Failed to get recent requests");
        assert_eq!(requests.len(), 4);
    }

    #[test]
    fn test_get_usage_by_day() {
        let mut conn = db::init_in_memory_db().expect("Failed to init DB");
        insert_test_requests(&mut conn);

        let usage = get_usage_by_day(&conn, "provider-a", 7).expect("Failed to get usage by day");
        assert!(!usage.is_empty());

        let today_usage = usage.iter().find(|u| {
            let today = Utc::now().date_naive();
            u.date == today.to_string()
        });
        assert!(today_usage.is_some());
        assert_eq!(today_usage.unwrap().total_requests, 2);
    }

    #[test]
    fn test_get_usage_by_group() {
        let mut conn = db::init_in_memory_db().expect("Failed to init DB");
        insert_test_requests(&mut conn);

        let usage = get_usage_by_group(&conn, 7).expect("Failed to get usage by group");
        assert_eq!(usage.len(), 2);
        assert_eq!(usage[0].group_alias, "glm-5-router");
        assert_eq!(usage[0].total_requests, 3);
        assert_eq!(usage[1].group_alias, "fast-model");
        assert_eq!(usage[1].total_requests, 1);
    }
}
