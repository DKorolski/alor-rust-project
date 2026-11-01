use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use super::*;
use crate::strategy_host::{BarEvent, DataOrigin};

pub(crate) struct TestServer {
    child: Child,
    _dir: tempfile::TempDir,
    pub(crate) port: u16,
}
impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl TestServer {
    pub(crate) async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let dir = tempfile::tempdir().unwrap();
        let child = Command::new(
            std::env::var("REDIS_SERVER_BIN").unwrap_or_else(|_| "redis-server".into()),
        )
        .args([
            "--bind",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--save",
            "",
            "--appendonly",
            "no",
            "--dir",
        ])
        .arg(dir.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("isolated redis-server required");
        let mut server = Self {
            child,
            _dir: dir,
            port,
        };
        for _ in 0..100 {
            if let Ok(mut connection) = redis::Client::open(format!("redis://127.0.0.1:{port}/"))
                .unwrap()
                .get_multiplexed_async_connection()
                .await
            {
                let info: String = redis::cmd("INFO")
                    .arg("server")
                    .query_async(&mut connection)
                    .await
                    .expect("isolated Redis identity unavailable");
                let pid = info
                    .lines()
                    .find_map(|line| line.strip_prefix("process_id:"))
                    .and_then(|pid| pid.parse::<u32>().ok());
                assert_eq!(
                    pid,
                    Some(server.child.id()),
                    "refusing to use Redis not owned by this test"
                );
                return server;
            }
            assert!(
                server.child.try_wait().unwrap().is_none(),
                "isolated Redis exited"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("isolated Redis startup timeout");
    }
    pub(crate) async fn admin(&self) -> MultiplexedConnection {
        redis::Client::open(format!("redis://127.0.0.1:{}/", self.port))
            .unwrap()
            .get_multiplexed_async_connection()
            .await
            .unwrap()
    }
}

fn identity(stream: &str) -> IngressIdentity {
    IngressIdentity {
        generation: "integration-v1".into(),
        receipt_stream: format!("receipts.{stream}.integration-v1"),
        source_stream: stream.into(),
        symbol: "RIZ6".into(),
    }
}

fn envelope() -> String {
    serde_json::to_string(&Envelope::new(
        1,
        "fixture-gateway",
        MessageType::Bar,
        BarEvent {
            symbol: "RIZ6".into(),
            close_time_utc: 1_790_000_000,
            o: 90000.,
            h: 90100.,
            l: 89900.,
            close: 90050.,
            v: 12.,
            origin: DataOrigin::Live,
        },
    ))
    .unwrap()
}

#[test]
fn source_cursor_validation_does_not_accept_latest_or_noncanonical_ids() {
    for raw in ["$", "0", "-1-0", "01-0", "1-00", "1-1-2", "1-a"] {
        assert!(stream_id(raw).is_err());
    }
    assert_eq!(stream_id("1234-5").unwrap(), (1234, 5));
}

#[test]
fn bare_bar_is_not_accepted_as_a_versioned_source_envelope() {
    let e: Envelope<serde_json::Value> = serde_json::from_str(&envelope()).unwrap();
    let raw = serde_json::to_string(&e.payload).unwrap();
    let row = redis::streams::StreamId {
        id: "1-0".into(),
        map: std::collections::HashMap::from([(
            "payload".into(),
            redis::Value::Data(raw.into_bytes()),
        )]),
    };
    let payload = bar_payload(&row).unwrap();
    assert!(payload.starts_with("invalid Redis envelope:"));
    assert!(serde_json::from_str::<BarEvent>(&payload).is_err());
}

#[tokio::test]
#[ignore = "requires local redis-server; starts an isolated process, never REDIS_URL"]
async fn ri_context_readonly_redis_collector_integration() {
    let server = TestServer::start().await;
    let mut admin = server.admin().await;
    let _: String = redis::cmd("ACL")
        .arg("SETUSER")
        .arg("shadowreader")
        .arg("on")
        .arg(">local-test-only")
        .arg("~fixture:*")
        .arg("+multi")
        .arg("+exec")
        .arg("+xrange")
        .arg("+ping")
        .query_async(&mut admin)
        .await
        .unwrap();
    let mut reader = redis::Client::open(format!(
        "redis://shadowreader:local-test-only@127.0.0.1:{}/",
        server.port
    ))
    .unwrap()
    .get_multiplexed_async_connection()
    .await
    .unwrap();
    let cfg = identity("fixture:bars");
    for n in 1..=150 {
        let _: String = redis::cmd("XADD")
            .arg(&cfg.source_stream)
            .arg(format!("{n}-0"))
            .arg("payload")
            .arg(envelope())
            .query_async(&mut admin)
            .await
            .unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("journal");
    let mut journal = RiIngressJournal::create(&root, cfg.clone()).unwrap();
    let cutoff = journal.manifest.created_at_utc;
    let mut source = RiRedisSource::new(cfg.clone()).unwrap();
    assert!(source.seal_cutoff(&mut journal, cutoff).is_err());
    let first = source.poll(&mut reader, &mut journal).await.unwrap();
    assert_eq!(first.len(), BATCH);
    assert!(first[0].input.as_ref().unwrap().received_at_utc.unwrap() >= cutoff);
    assert_ne!(first[0].input.as_ref().unwrap().received_at_utc, Some(1));
    assert!(
        source.seal_cutoff(&mut journal, cutoff).is_err(),
        "full batch is not a caught-up poll"
    );
    let second = source.poll(&mut reader, &mut journal).await.unwrap();
    assert_eq!(second.len(), 22);
    assert_eq!(journal.last_source_id(), Some("150-0"));
    let barrier = source.seal_cutoff(&mut journal, cutoff).unwrap();
    assert_eq!(barrier.last_sequence, 150);
    let original_inputs = journal.input_prefix(150).unwrap();
    drop(journal);
    let mut journal = RiIngressJournal::open(&root, &cfg).unwrap();
    assert!(source
        .poll(&mut reader, &mut journal)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(journal.input_prefix(150).unwrap(), original_inputs);
    assert_eq!(source.seal_cutoff(&mut journal, cutoff).unwrap(), barrier);

    // ACL independently forbids source mutation and broker-command publishing.
    let denied: redis::RedisResult<String> = redis::cmd("XADD")
        .arg("fixture:commands")
        .arg("*")
        .arg("payload")
        .arg("forbidden")
        .query_async(&mut reader)
        .await;
    assert!(denied.is_err());
    let length: usize = redis::cmd("XLEN")
        .arg(&cfg.source_stream)
        .query_async(&mut admin)
        .await
        .unwrap();
    let commands: usize = redis::cmd("EXISTS")
        .arg("fixture:commands")
        .query_async(&mut admin)
        .await
        .unwrap();
    assert_eq!(length, 150);
    assert_eq!(commands, 0);

    let _: String = redis::cmd("XADD")
        .arg(&cfg.source_stream)
        .arg("151-0")
        .arg("payload")
        .arg(envelope())
        .query_async(&mut admin)
        .await
        .unwrap();
    let _: usize = redis::cmd("XTRIM")
        .arg(&cfg.source_stream)
        .arg("MAXLEN")
        .arg(1)
        .query_async(&mut admin)
        .await
        .unwrap();
    assert!(source
        .poll(&mut reader, &mut journal)
        .await
        .unwrap_err()
        .to_string()
        .contains("missing/trimmed"));
    assert_eq!(journal.last_sequence(), 150);
    assert!(source.seal_cutoff(&mut journal, cutoff).is_err());

    // Malformed source input is durably audited, not skipped on the next poll.
    let cfg = identity("fixture:bad");
    let _: String = redis::cmd("XADD")
        .arg(&cfg.source_stream)
        .arg("1-0")
        .arg("payload")
        .arg("broken envelope")
        .query_async(&mut admin)
        .await
        .unwrap();
    let mut bad = RiIngressJournal::create(&dir.path().join("bad"), cfg.clone()).unwrap();
    let mut source = RiRedisSource::new(cfg).unwrap();
    assert!(source.poll(&mut reader, &mut bad).await.is_err());
    assert_eq!(bad.last_sequence(), 1);
    assert!(bad.input_prefix(1).is_err());
    assert!(source
        .seal_cutoff(&mut bad, Utc::now().timestamp())
        .is_err());
}
