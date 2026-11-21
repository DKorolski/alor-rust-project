//! Bounded copy of exactly the files read by qualification. No HTTP or writes.
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    plan: Plan,
    files: BTreeMap<String, Vec<u8>>,
}

fn key(root: &Path, path: &Path) -> Result<String> {
    let relative = path.strip_prefix(root)?;
    ensure!(
        relative
            .components()
            .all(|p| matches!(p, std::path::Component::Normal(_))),
        "invalid OI snapshot path"
    );
    Ok(relative
        .to_str()
        .context("non-UTF8 snapshot path")?
        .to_string())
}

impl Snapshot {
    /// All original qualification checks run before a copy can be returned.
    pub fn capture(root: &Path, plan: &Plan) -> Result<Option<Self>> {
        let files = RefCell::new(BTreeMap::new());
        let q = load_with_reader(root, plan, false, &|path| {
            let bytes = read_bounded(path)?;
            let name = key(root, path)?;
            let mut saved = files.borrow_mut();
            if let Some(old) = saved.get(&name) {
                ensure!(old == &bytes, "OI changed during capture");
            }
            saved.insert(name, bytes.clone());
            ensure!(
                saved.len() <= 8 && saved.values().map(Vec::len).sum::<usize>() <= 3 * MAX_BYTES,
                "OI bundle bound"
            );
            Ok(bytes)
        })?;
        if q.is_none() {
            return Ok(None);
        }
        let result = Self {
            plan: plan.clone(),
            files: files.into_inner(),
        };
        result.verify(plan)?;
        Ok(Some(result))
    }

    /// Revalidates the frozen bytes; never revisits an external store.
    pub fn verify(&self, expected: &Plan) -> Result<Qualification> {
        ensure!(
            self.plan == *expected
                && self.files.len() <= 8
                && self.files.values().all(|b| b.len() <= MAX_BYTES)
                && self.files.values().map(Vec::len).sum::<usize>() <= 3 * MAX_BYTES,
            "OI snapshot identity/bound"
        );
        let root = Path::new("/frozen-oi");
        let used = RefCell::new(BTreeSet::new());
        let q = load_with_reader(root, expected, false, &|path| {
            let name = key(root, path)?;
            used.borrow_mut().insert(name.clone());
            self.files
                .get(&name)
                .cloned()
                .with_context(|| format!("missing frozen OI file: {name}"))
        })?
        .context("snapshot qualification missing")?;
        ensure!(
            used.into_inner() == self.files.keys().cloned().collect(),
            "unbound OI snapshot files"
        );
        Ok(q)
    }
}
