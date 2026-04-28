use std::collections::HashSet;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use alor_protocol::{Envelope, MessageType, SCHEMA_VERSION};
use anyhow::{bail, Context, Result};
use chrono::{FixedOffset, TimeZone};
use strategy_runtime::strategies::moex_author41_42::{
    build_ri_author41_42_combo_shadow_journal, ModelBar, RegularSessionPolicy, ShadowJournalRecord,
};
use strategy_runtime::strategy_host::BarEvent;
use tokio::signal;
use tracing::{info, warn, Level};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Clone)]
struct Cli {
    redis_url: String,
    bars_stream: String,
    consumer_group: String,
    consumer_name: String,
    symbol: String,
    timezone_offset_hours: i32,
    warmup_count: usize,
    block_ms: usize,
    count: usize,
    out_journal_jsonl: PathBuf,
    append: bool,
    write_warmup: bool,
    once: bool,
}

#[derive(Debug, Clone)]
struct RawStreamMessage {
    id: String,
    payload: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(Level::INFO.into()))
        .init();

    let cli = Cli::parse()?;
    let client = redis::Client::open(cli.redis_url.clone())?;
    ensure_group(&client, &cli.bars_stream, &cli.consumer_group).await?;

    let mut writer = open_writer(&cli.out_journal_jsonl, cli.append)?;
    let mut bars = load_warmup_bars(&client, &cli).await?;
    sort_dedup_bars(&mut bars);

    let mut seen_records = HashSet::new();
    let warmup_records =
        build_ri_author41_42_combo_shadow_journal(&bars, RegularSessionPolicy::moex_10m());
    if cli.write_warmup {
        write_new_records(&mut writer, &mut seen_records, &warmup_records)?;
    } else {
        for record in &warmup_records {
            seen_records.insert(record_key(record));
        }
    }
    writer.flush().context("flush warmup journal")?;

    info!(
        stream = cli.bars_stream,
        consumer_group = cli.consumer_group,
        consumer_name = cli.consumer_name,
        symbol = cli.symbol,
        warmup_bars = bars.len(),
        warmup_records = warmup_records.len(),
        write_warmup = cli.write_warmup,
        out = %cli.out_journal_jsonl.display(),
        "moex author41+42 shadow runner started"
    );

    if cli.once {
        return Ok(());
    }

    loop {
        tokio::select! {
            _ = signal::ctrl_c() => {
                info!("shutdown requested (SIGINT)");
                break;
            }
            messages = read_group(&client, &cli) => {
                let messages = messages?;
                if messages.is_empty() {
                    continue;
                }
                for message in messages {
                    let ack_id = message.id.clone();
                    match decode_bar(&message.payload, &cli.symbol, cli.timezone_offset_hours) {
                        Ok(Some(bar)) => {
                            bars.push(bar);
                            sort_dedup_bars(&mut bars);
                            let records = build_ri_author41_42_combo_shadow_journal(
                                &bars,
                                RegularSessionPolicy::moex_10m(),
                            );
                            let written = write_new_records(&mut writer, &mut seen_records, &records)?;
                            if written > 0 {
                                info!(
                                    message_id = ack_id,
                                    written,
                                    total_records_seen = seen_records.len(),
                                    "moex author41+42 shadow journal updated"
                                );
                            }
                        }
                        Ok(None) => {}
                        Err(error) => {
                            warn!(?error, message_id = ack_id, "shadow runner failed to decode bar");
                        }
                    }
                    xack(&client, &cli.bars_stream, &cli.consumer_group, &ack_id).await?;
                }
                writer.flush().context("flush shadow journal")?;
            }
        }
    }

    writer.flush().context("flush shadow journal on shutdown")?;
    Ok(())
}

impl Cli {
    fn parse() -> Result<Self> {
        let mut cli = Self {
            redis_url: "redis://127.0.0.1/".to_string(),
            bars_stream: "md.bars.RI.10m".to_string(),
            consumer_group: "moex-author41-42-shadow".to_string(),
            consumer_name: "moex-author41-42-shadow-1".to_string(),
            symbol: "RI".to_string(),
            timezone_offset_hours: 3,
            warmup_count: 5000,
            block_ms: 5000,
            count: 10,
            out_journal_jsonl: PathBuf::from("./strategy-runtime/moex_author41_42_shadow.jsonl"),
            append: true,
            write_warmup: false,
            once: false,
        };

        let mut args = env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--redis-url" => cli.redis_url = next_string(&mut args, "--redis-url")?,
                "--bars-stream" => cli.bars_stream = next_string(&mut args, "--bars-stream")?,
                "--consumer-group" => {
                    cli.consumer_group = next_string(&mut args, "--consumer-group")?
                }
                "--consumer-name" => cli.consumer_name = next_string(&mut args, "--consumer-name")?,
                "--symbol" => cli.symbol = next_string(&mut args, "--symbol")?,
                "--timezone-offset-hours" => {
                    cli.timezone_offset_hours = next_string(&mut args, "--timezone-offset-hours")?
                        .parse()
                        .context("parse --timezone-offset-hours")?
                }
                "--warmup-count" => {
                    cli.warmup_count = next_string(&mut args, "--warmup-count")?
                        .parse()
                        .context("parse --warmup-count")?
                }
                "--block-ms" => {
                    cli.block_ms = next_string(&mut args, "--block-ms")?
                        .parse()
                        .context("parse --block-ms")?
                }
                "--count" => {
                    cli.count = next_string(&mut args, "--count")?
                        .parse()
                        .context("parse --count")?
                }
                "--out-journal-jsonl" => {
                    cli.out_journal_jsonl = next_path(&mut args, "--out-journal-jsonl")?
                }
                "--truncate" => cli.append = false,
                "--write-warmup" => cli.write_warmup = true,
                "--once" => cli.once = true,
                "--help" | "-h" => {
                    print_usage();
                    std::process::exit(0);
                }
                other => bail!("unsupported argument {other:?}; use --help"),
            }
        }
        Ok(cli)
    }
}

fn open_writer(path: &PathBuf, append: bool) -> Result<BufWriter<File>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create journal dir {}", parent.display()))?;
    }
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(path)
        .with_context(|| format!("open journal {}", path.display()))?;
    Ok(BufWriter::new(file))
}

async fn ensure_group(client: &redis::Client, stream: &str, group: &str) -> Result<()> {
    let mut conn = client.get_multiplexed_async_connection().await?;
    let result: redis::RedisResult<redis::Value> = redis::cmd("XGROUP")
        .arg("CREATE")
        .arg(stream)
        .arg(group)
        .arg("0")
        .arg("MKSTREAM")
        .query_async(&mut conn)
        .await;
    match result {
        Ok(_) => Ok(()),
        Err(error) if error.to_string().contains("BUSYGROUP") => Ok(()),
        Err(error) => Err(error.into()),
    }
}

async fn load_warmup_bars(client: &redis::Client, cli: &Cli) -> Result<Vec<ModelBar>> {
    let mut conn = client.get_multiplexed_async_connection().await?;
    let reply: redis::Value = redis::cmd("XREVRANGE")
        .arg(&cli.bars_stream)
        .arg("+")
        .arg("-")
        .arg("COUNT")
        .arg(cli.warmup_count)
        .query_async(&mut conn)
        .await?;
    let mut bars = Vec::new();
    for message in parse_range_entries(reply) {
        match decode_bar(&message.payload, &cli.symbol, cli.timezone_offset_hours) {
            Ok(Some(bar)) => bars.push(bar),
            Ok(None) => {}
            Err(error) => warn!(
                ?error,
                message_id = message.id,
                "warmup failed to decode bar"
            ),
        }
    }
    Ok(bars)
}

async fn read_group(client: &redis::Client, cli: &Cli) -> Result<Vec<RawStreamMessage>> {
    let mut conn = client.get_multiplexed_async_connection().await?;
    let reply: redis::Value = redis::cmd("XREADGROUP")
        .arg("GROUP")
        .arg(&cli.consumer_group)
        .arg(&cli.consumer_name)
        .arg("BLOCK")
        .arg(cli.block_ms)
        .arg("COUNT")
        .arg(cli.count)
        .arg("STREAMS")
        .arg(&cli.bars_stream)
        .arg(">")
        .query_async(&mut conn)
        .await?;
    Ok(parse_read_group_entries(reply))
}

async fn xack(client: &redis::Client, stream: &str, group: &str, id: &str) -> Result<()> {
    let mut conn = client.get_multiplexed_async_connection().await?;
    let _: redis::Value = redis::cmd("XACK")
        .arg(stream)
        .arg(group)
        .arg(id)
        .query_async(&mut conn)
        .await?;
    Ok(())
}

fn parse_read_group_entries(reply: redis::Value) -> Vec<RawStreamMessage> {
    let redis::Value::Bulk(streams) = reply else {
        return Vec::new();
    };
    let Some(redis::Value::Bulk(stream)) = streams.first() else {
        return Vec::new();
    };
    if stream.len() < 2 {
        return Vec::new();
    }
    let redis::Value::Bulk(entries) = &stream[1] else {
        return Vec::new();
    };
    entries.iter().filter_map(parse_entry).collect()
}

fn parse_range_entries(reply: redis::Value) -> Vec<RawStreamMessage> {
    let redis::Value::Bulk(entries) = reply else {
        return Vec::new();
    };
    entries.iter().filter_map(parse_entry).collect()
}

fn parse_entry(entry: &redis::Value) -> Option<RawStreamMessage> {
    let redis::Value::Bulk(values) = entry else {
        return None;
    };
    if values.len() < 2 {
        return None;
    }
    let redis::Value::Data(id) = &values[0] else {
        return None;
    };
    let payload = extract_payload(&values[1])?;
    Some(RawStreamMessage {
        id: String::from_utf8_lossy(id).to_string(),
        payload,
    })
}

fn extract_payload(fields: &redis::Value) -> Option<String> {
    let redis::Value::Bulk(values) = fields else {
        return None;
    };
    for chunk in values.chunks(2) {
        if let [redis::Value::Data(key), redis::Value::Data(value)] = chunk {
            if key == b"payload" {
                return Some(String::from_utf8_lossy(value).to_string());
            }
        }
    }
    None
}

fn decode_bar(payload: &str, symbol: &str, timezone_offset_hours: i32) -> Result<Option<ModelBar>> {
    let envelope: Envelope<serde_json::Value> =
        serde_json::from_str(payload).context("parse envelope")?;
    if envelope.schema_version > SCHEMA_VERSION {
        bail!("unsupported schema {}", envelope.schema_version);
    }
    if envelope.msg_type != MessageType::Bar {
        return Ok(None);
    }
    let bar: BarEvent = serde_json::from_value(envelope.payload).context("decode bar payload")?;
    if bar.symbol != symbol {
        return Ok(None);
    }
    let offset = FixedOffset::east_opt(timezone_offset_hours.saturating_mul(3600))
        .context("invalid timezone offset")?;
    let ts_local = offset
        .timestamp_opt(bar.close_time_utc, 0)
        .single()
        .context("invalid bar close_time_utc")?
        .naive_local();
    let close = bar.close;
    Ok(Some(ModelBar {
        ts_local,
        open: if bar.o != 0.0 { bar.o } else { close },
        high: if bar.h != 0.0 { bar.h } else { close },
        low: if bar.l != 0.0 { bar.l } else { close },
        close,
        volume: bar.v,
    }))
}

fn sort_dedup_bars(bars: &mut Vec<ModelBar>) {
    bars.sort_by_key(|bar| bar.ts_local);
    bars.dedup_by_key(|bar| bar.ts_local);
}

fn write_new_records(
    writer: &mut BufWriter<File>,
    seen_records: &mut HashSet<String>,
    records: &[ShadowJournalRecord],
) -> Result<usize> {
    let mut written = 0usize;
    for record in records {
        if !seen_records.insert(record_key(record)) {
            continue;
        }
        serde_json::to_writer(&mut *writer, record).context("serialize shadow journal record")?;
        writer
            .write_all(b"\n")
            .context("write shadow journal newline")?;
        written += 1;
    }
    Ok(written)
}

fn record_key(record: &ShadowJournalRecord) -> String {
    format!(
        "{:?}|{}|{:?}|{:?}|{:?}|{:?}",
        record.component,
        record.bar_ts_local,
        record.side,
        record.scheduled_entry_ts_local,
        record.scheduled_exit_ts_local,
        record.overlap_decision
    )
}

fn next_path(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<PathBuf> {
    Ok(PathBuf::from(next_string(args, flag)?))
}

fn next_string(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
    args.next()
        .with_context(|| format!("missing value for {flag}"))
}

fn print_usage() {
    println!(
        "Usage: moex_author41_42_shadow_runner \\
  [--redis-url redis://127.0.0.1/] \\
  [--bars-stream md.bars.RI.10m] \\
  [--consumer-group moex-author41-42-shadow] \\
  [--consumer-name moex-author41-42-shadow-1] \\
  [--symbol RI] \\
  [--timezone-offset-hours 3] \\
  [--warmup-count 5000] \\
  [--block-ms 5000] \\
  [--count 10] \\
  [--out-journal-jsonl ./strategy-runtime/moex_author41_42_shadow.jsonl] \\
  [--truncate] \\
  [--write-warmup] \\
  [--once]"
    );
}
