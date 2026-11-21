use std::future::Future;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{ensure, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::super::{Plan, HISTORY_URL, MAX_BYTES};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    LinuxKernelClock,
    UnsupportedClock,
    OfflineFixture,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClockReading {
    pub utc: DateTime<Utc>,
    pub monotonic_ns: u64,
    pub synchronized: bool,
    pub offset_ms: Option<i64>,
    pub max_error_ms: Option<u64>,
}

impl ClockReading {
    pub fn healthy(&self) -> bool {
        self.synchronized
            && self.offset_ms.is_some_and(|n| n.unsigned_abs() <= 1000)
            && self.max_error_ms.is_some_and(|n| n <= 1000)
    }

    pub fn follows(&self, previous: &Self) -> bool {
        if !self.healthy() || !previous.healthy() || self.monotonic_ns < previous.monotonic_ns {
            return false;
        }
        let Some(wall_ns) = (self.utc - previous.utc).num_nanoseconds() else {
            return false;
        };
        let mono_ns = self.monotonic_ns - previous.monotonic_ns;
        wall_ns >= 0 && (i128::from(wall_ns) - i128::from(mono_ns)).abs() <= 250_000_000
    }

    pub fn before_deadline(&self, deadline: DateTime<Utc>) -> bool {
        if !self.healthy() {
            return false;
        }
        let uncertainty = self.offset_ms.unwrap().unsigned_abs() + self.max_error_ms.unwrap();
        self.utc
            .checked_add_signed(chrono::Duration::milliseconds(uncertainty as i64))
            .is_some_and(|latest| latest <= deadline)
    }
}

pub trait Clock {
    fn kind(&self) -> EvidenceKind;
    fn read(&mut self) -> Result<ClockReading>;
    fn sleep(&mut self, duration: Duration) -> impl Future<Output = ()>;
}

pub struct SystemClock {
    start: Instant,
}

impl Default for SystemClock {
    fn default() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn interpret_ntp(
    ready: bool,
    nanos: bool,
    offset: i128,
    maxerror: i128,
) -> (bool, Option<i64>, Option<u64>) {
    if !ready || maxerror < 0 {
        return (false, None, None);
    }
    let magnitude = offset
        .unsigned_abs()
        .div_ceil(if nanos { 1_000_000 } else { 1000 });
    let signed = i128::try_from(magnitude)
        .ok()
        .map(|n| if offset < 0 { -n } else { n });
    let max_error = u128::try_from(maxerror).unwrap().div_ceil(1000);
    (
        true,
        signed.and_then(|n| i64::try_from(n).ok()),
        u64::try_from(max_error).ok(),
    )
}

#[cfg(target_os = "linux")]
fn kernel_health() -> (bool, Option<i64>, Option<u64>) {
    // SAFETY: timex is a C integer-only struct; zero initializes modes=0.
    let mut value: libc::timex = unsafe { std::mem::zeroed() };
    // SAFETY: valid output pointer, modes=0 only queries kernel state.
    let state = unsafe { libc::adjtimex(&mut value) };
    interpret_ntp(
        state == libc::TIME_OK && value.status & (libc::STA_UNSYNC | libc::STA_CLOCKERR) == 0,
        value.status & libc::STA_NANO != 0,
        i128::from(value.offset),
        i128::from(value.maxerror),
    )
}

#[cfg(not(target_os = "linux"))]
fn kernel_health() -> (bool, Option<i64>, Option<u64>) {
    (false, None, None)
}

impl Clock for SystemClock {
    fn kind(&self) -> EvidenceKind {
        if cfg!(target_os = "linux") {
            EvidenceKind::LinuxKernelClock
        } else {
            EvidenceKind::UnsupportedClock
        }
    }
    fn read(&mut self) -> Result<ClockReading> {
        let (synchronized, offset_ms, max_error_ms) = kernel_health();
        Ok(ClockReading {
            utc: Utc::now(),
            monotonic_ns: self.start.elapsed().as_nanos().try_into()?,
            synchronized,
            offset_ms,
            max_error_ms,
        })
    }
    async fn sleep(&mut self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FetchFailure {
    Timeout,
    Network,
    BodyTooLarge,
    IncompleteBody,
}

#[derive(Debug)]
pub struct Response {
    pub status: Option<u16>,
    pub server_date: Option<String>,
    pub body: Vec<u8>,
    pub failure: Option<FetchFailure>,
}

pub trait Transport {
    fn get(&mut self, plan: &Plan, timeout: Duration) -> impl Future<Output = Response>;
}

pub struct IssTransport {
    client: reqwest::Client,
}

impl IssTransport {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .https_only(true)
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(30))
                .user_agent("alor-imoexf-readonly-oi-shadow/1")
                .build()?,
        })
    }

    pub fn request(&self, plan: &Plan, timeout: Duration) -> Result<reqwest::Request> {
        ensure!(
            !timeout.is_zero() && timeout <= Duration::from_secs(30),
            "HTTP timeout bound"
        );
        Ok(self
            .client
            .get(HISTORY_URL)
            .query(&plan.request_params())
            .timeout(timeout)
            .build()?)
    }
}

impl Transport for IssTransport {
    async fn get(&mut self, plan: &Plan, timeout: Duration) -> Response {
        let request = match self.request(plan, timeout) {
            Ok(request) => request,
            Err(_) => {
                return Response {
                    status: None,
                    server_date: None,
                    body: vec![],
                    failure: Some(FetchFailure::Network),
                }
            }
        };
        let mut response = match self.client.execute(request).await {
            Ok(response) => response,
            Err(error) => {
                return Response {
                    status: None,
                    server_date: None,
                    body: vec![],
                    failure: Some(if error.is_timeout() {
                        FetchFailure::Timeout
                    } else {
                        FetchFailure::Network
                    }),
                }
            }
        };
        let mut result = Response {
            status: Some(response.status().as_u16()),
            server_date: response
                .headers()
                .get(reqwest::header::DATE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned),
            body: vec![],
            failure: None,
        };
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    let remaining = MAX_BYTES - result.body.len();
                    result
                        .body
                        .extend_from_slice(&chunk[..remaining.min(chunk.len())]);
                    if chunk.len() > remaining {
                        result.failure = Some(FetchFailure::BodyTooLarge);
                        break;
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    result.failure = Some(if error.is_timeout() {
                        FetchFailure::Timeout
                    } else {
                        FetchFailure::IncompleteBody
                    });
                    break;
                }
            }
        }
        result
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DiskUsage {
    pub bytes: u64,
    pub files: u64,
    pub free_bytes: u64,
}

pub trait Disk {
    fn usage(&mut self, root: &Path) -> Result<DiskUsage>;
}

pub struct SystemDisk;

impl Disk for SystemDisk {
    fn usage(&mut self, root: &Path) -> Result<DiskUsage> {
        fn walk(path: &Path, bytes: &mut u64, files: &mut u64, depth: u8) -> Result<()> {
            ensure!(depth <= 8, "OI store depth bound");
            let meta = std::fs::symlink_metadata(path)?;
            ensure!(!meta.file_type().is_symlink(), "symlink in OI store");
            *files = files
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("file count overflow"))?;
            ensure!(*files <= 8192, "OI store scan bound");
            if meta.is_dir() {
                for item in std::fs::read_dir(path)? {
                    walk(&item?.path(), bytes, files, depth + 1)?;
                }
            } else {
                ensure!(meta.is_file(), "non-file OI evidence");
                *bytes = bytes
                    .checked_add(meta.len())
                    .ok_or_else(|| anyhow::anyhow!("store size overflow"))?;
            }
            Ok(())
        }
        let (mut bytes, mut files) = (0, 0);
        walk(root, &mut bytes, &mut files, 0)?;
        #[cfg(unix)]
        let free_bytes = {
            use std::os::unix::ffi::OsStrExt;
            let name = std::ffi::CString::new(root.as_os_str().as_bytes())?;
            // SAFETY: initialized output and a valid NUL-terminated path, read-only syscall.
            let mut info: libc::statvfs = unsafe { std::mem::zeroed() };
            ensure!(
                unsafe { libc::statvfs(name.as_ptr(), &mut info) } == 0,
                "statvfs failed"
            );
            u64::try_from(u128::from(info.f_bavail) * u128::from(info.f_frsize))?
        };
        #[cfg(not(unix))]
        let free_bytes = 0;
        Ok(DiskUsage {
            bytes,
            files,
            free_bytes,
        })
    }
}
