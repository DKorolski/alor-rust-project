use anyhow::Result;
use redis::AsyncCommands;

pub async fn redis_flushdb(redis_url: &str) -> Result<()> {
    let client = redis::Client::open(redis_url)?;
    let mut conn = client.get_multiplexed_async_connection().await?;
    redis::cmd("FLUSHDB")
        .query_async::<_, ()>(&mut conn)
        .await?;
    Ok(())
}

pub async fn xadd_json(redis_url: &str, stream: &str, payload: &str) -> Result<String> {
    let client = redis::Client::open(redis_url)?;
    let mut conn = client.get_multiplexed_async_connection().await?;
    let id: redis::Value = redis::cmd("XADD")
        .arg(stream)
        .arg("*")
        .arg("payload")
        .arg(payload)
        .query_async(&mut conn)
        .await?;
    Ok(match id {
        redis::Value::Data(bytes) => String::from_utf8_lossy(&bytes).to_string(),
        _ => "".to_string(),
    })
}

pub async fn xlen(redis_url: &str, stream: &str) -> Result<i64> {
    let client = redis::Client::open(redis_url)?;
    let mut conn = client.get_multiplexed_async_connection().await?;
    let len: i64 = conn.xlen(stream).await?;
    Ok(len)
}

pub fn extract_payload(fields: &redis::Value) -> Option<String> {
    let fields = match fields {
        redis::Value::Bulk(values) => values,
        _ => return None,
    };
    for chunk in fields.chunks(2) {
        if let [key, value] = chunk {
            if let redis::Value::Data(key) = key {
                if key == b"payload" {
                    if let redis::Value::Data(value) = value {
                        return Some(String::from_utf8_lossy(value).to_string());
                    }
                }
            }
        }
    }
    None
}
