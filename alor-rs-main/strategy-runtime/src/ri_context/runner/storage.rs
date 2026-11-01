use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use super::hash;

pub(crate) trait LogRecord: DeserializeOwned + Serialize {
    fn index(&self) -> u64;
    fn previous_sha256(&self) -> &str;
}

const MAX_FILE: u64 = 16 * 1024 * 1024;
pub(crate) const MAX_TOTAL: u64 = 256 * 1024 * 1024;
pub(crate) const MAX_TRANSACTIONS: usize = 20_000;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checked<T> {
    payload: T,
    sha256: String,
}

#[derive(Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Head {
    index: u64,
    sha256: String,
}

pub(crate) struct TransactionLog<T> {
    root: PathBuf,
    _lock: File,
    genesis: String,
    hashes: Vec<String>,
    bytes: u64,
    failed: bool,
    _record: std::marker::PhantomData<T>,
}

pub(crate) fn sync_parent(path: &Path) -> Result<()> {
    File::open(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?
    .sync_all()?;
    Ok(())
}

fn lock(root: &Path) -> Result<File> {
    ensure!(
        fs::symlink_metadata(root)?.file_type().is_dir(),
        "invalid transaction directory"
    );
    let path = root.join("writer.lock");
    if path.try_exists()? {
        ensure!(
            fs::symlink_metadata(&path)?.file_type().is_file(),
            "invalid transaction lock"
        );
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    file.try_lock()
        .context("shadow runner already active or locking unsupported")?;
    Ok(file)
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(&Checked {
        payload: value,
        sha256: hash(value)?,
    })?;
    ensure!(bytes.len() as u64 <= MAX_FILE, "transaction file too large");
    Ok(bytes)
}

fn read<T: DeserializeOwned + Serialize>(path: &Path) -> Result<T> {
    ensure!(
        fs::symlink_metadata(path)?.file_type().is_file(),
        "non-regular transaction file"
    );
    let mut bytes = vec![];
    File::open(path)?
        .take(MAX_FILE + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= MAX_FILE, "oversized transaction file");
    let checked: Checked<T> = serde_json::from_slice(&bytes)?;
    ensure!(
        hash(&checked.payload)? == checked.sha256,
        "transaction checksum mismatch"
    );
    Ok(checked.payload)
}

fn publish(root: &Path, name: &str, bytes: &[u8], replace: bool) -> Result<()> {
    let tmp = root.join(format!(".transaction-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    if replace {
        fs::rename(&tmp, root.join(name))?;
    } else {
        fs::hard_link(&tmp, root.join(name))?;
        File::open(root)?.sync_all()?;
        fs::remove_file(tmp)?;
    }
    File::open(root)?.sync_all()?;
    Ok(())
}

impl<T: LogRecord> TransactionLog<T> {
    pub(crate) fn create<H: Serialize>(root: &Path, header: &H) -> Result<Self> {
        fs::create_dir(root)?;
        sync_parent(root)?;
        let writer = lock(root)?;
        let bytes = encode(header)?;
        publish(root, "manifest.json", &bytes, false)?;
        let genesis = hash(header)?;
        let head = encode(&Head {
            index: 0,
            sha256: genesis.clone(),
        })?;
        publish(root, "HEAD.json", &head, false)?;
        Ok(Self {
            root: root.into(),
            _lock: writer,
            genesis,
            hashes: vec![],
            bytes: (bytes.len() + head.len()) as u64,
            failed: false,
            _record: std::marker::PhantomData,
        })
    }

    pub(crate) fn open<H: DeserializeOwned + Serialize + PartialEq>(
        root: &Path,
        expected: &H,
    ) -> Result<(Self, Vec<T>)> {
        let writer = lock(root)?;
        let mut names = std::collections::BTreeSet::new();
        let mut bytes = 0;
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            ensure!(
                entry.file_type()?.is_file(),
                "non-regular transaction entry"
            );
            let size = entry.metadata()?.len();
            ensure!(size <= MAX_FILE, "oversized transaction entry");
            bytes += size;
            ensure!(bytes <= MAX_TOTAL, "transaction generation byte limit");
            names.insert(
                entry
                    .file_name()
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("invalid transaction filename"))?,
            );
            ensure!(
                names.len() <= MAX_TRANSACTIONS + 3,
                "too many transaction entries"
            );
        }
        for name in ["writer.lock", "manifest.json", "HEAD.json"] {
            ensure!(names.remove(name), "missing transaction metadata");
        }
        let header: H = read(&root.join("manifest.json"))?;
        ensure!(&header == expected, "runner manifest/config mismatch");
        let genesis = hash(&header)?;
        let head: Head = read(&root.join("HEAD.json"))?;
        ensure!(
            head.index as usize <= MAX_TRANSACTIONS,
            "invalid transaction head"
        );
        let mut hashes = vec![];
        let mut transactions = vec![];
        let mut previous = genesis.clone();
        for index in 1..=head.index {
            let name = format!("{index:08}.json");
            ensure!(names.remove(&name), "missing committed transaction");
            let tx: T = read(&root.join(name))?;
            ensure!(
                tx.index() == index && tx.previous_sha256() == previous,
                "transaction chain mismatch"
            );
            previous = hash(&tx)?;
            hashes.push(previous.clone());
            transactions.push(tx);
        }
        ensure!(head.sha256 == previous, "transaction head mismatch");
        ensure!(
            names.is_empty(),
            "failed/torn/uncommitted transaction tail; explicit audit required"
        );
        Ok((
            Self {
                root: root.into(),
                _lock: writer,
                genesis,
                hashes,
                bytes,
                failed: false,
                _record: std::marker::PhantomData,
            },
            transactions,
        ))
    }

    pub(crate) fn count(&self) -> u64 {
        self.hashes.len() as u64
    }
    pub(crate) fn head(&self) -> &str {
        self.hashes.last().unwrap_or(&self.genesis)
    }
    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }

    pub(crate) fn healthy(&self) -> Result<()> {
        ensure!(
            !self.failed,
            "shadow runner failed; explicit generation recovery required"
        );
        Ok(())
    }

    pub(crate) fn append(&mut self, tx: &T) -> Result<()> {
        self.healthy()?;
        ensure!(
            self.hashes.len() < MAX_TRANSACTIONS,
            "transaction count limit; explicit rotation required"
        );
        ensure!(
            tx.index() == self.count() + 1 && tx.previous_sha256() == self.head(),
            "invalid transaction append"
        );
        let bytes = encode(tx)?;
        let sha = hash(tx)?;
        let head = encode(&Head {
            index: tx.index(),
            sha256: sha.clone(),
        })?;
        let old_head_bytes = fs::metadata(self.root.join("HEAD.json"))?.len();
        let current_head: Head = read(&self.root.join("HEAD.json"))?;
        ensure!(
            current_head.index == self.count() && current_head.sha256 == self.head(),
            "transaction head changed while running"
        );
        let retained_bytes = self
            .bytes
            .checked_sub(old_head_bytes)
            .context("transaction size accounting mismatch")?;
        ensure!(
            self.bytes + bytes.len() as u64 + head.len() as u64 <= MAX_TOTAL,
            "transaction byte limit; explicit rotation required"
        );
        publish(
            &self.root,
            &format!("{:08}.json", tx.index()),
            &bytes,
            false,
        )?;
        // HEAD is the release boundary. An extra file beyond HEAD is a torn
        // operation, never automatically replayed/published after restart.
        publish(&self.root, "HEAD.json", &head, true)?;
        self.hashes.push(sha);
        self.bytes = retained_bytes + bytes.len() as u64 + head.len() as u64;
        Ok(())
    }

    pub(crate) fn read(&self, index: u64) -> Result<T> {
        self.healthy()?;
        ensure!(
            index > 0 && index <= self.count(),
            "output is not committed"
        );
        let tx: T = read(&self.root.join(format!("{index:08}.json")))?;
        ensure!(
            hash(&tx)? == self.hashes[index as usize - 1],
            "committed output changed"
        );
        Ok(tx)
    }

    pub(crate) fn fail(&mut self, error: anyhow::Error) -> anyhow::Error {
        self.failed = true;
        let marker = encode(
            &serde_json::json!({"error_sha256": hash(&format!("{error:#}")).unwrap_or_default(), "committed_transactions": self.count()}),
        );
        let persisted = marker.and_then(|bytes| publish(&self.root, "FAILED.json", &bytes, false));
        match persisted {
            Ok(()) => error.context("shadow generation marked FAILED; automatic restart forbidden"),
            Err(_) => error.context("FAILED marker not persisted; supervisor must block restart"),
        }
    }
}
