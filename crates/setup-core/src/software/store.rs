//! Transactional software journals and installation receipts.
//!
//! Ordinary observation never initializes or recovers metadata. Writers recover
//! SQLite under a separate, directory-bound lock before reading kernel records.
//! Published launch markers remain files; they are not database records.

mod vfs;

use std::sync::Arc;

use cap_std::fs::Dir;
use rusqlite::{Connection, OpenFlags, OptionalExtension, config::DbConfig, limits::Limit, params};
use serde::{Serialize, de::DeserializeOwned};

use super::{
    records::{self, Identity, io, refuse},
    writer::CONTROL,
};
use crate::Result;

const APPLICATION: i64 = 0x4153_5450;
const VERSION: i64 = 2;
const RECORDS: &str = "CREATE TABLE records (kind TEXT NOT NULL CHECK(kind IN ('installation','preparation','removal','switch','receipt')), command TEXT NOT NULL, version TEXT NOT NULL, payload BLOB NOT NULL, PRIMARY KEY(kind,command,version)) STRICT, WITHOUT ROWID";
const OWNER: &str = "CREATE TABLE owner (singleton INTEGER PRIMARY KEY CHECK(singleton=1), root TEXT NOT NULL) STRICT";
const OPERATIONS: &str = "CREATE TABLE operations (id TEXT PRIMARY KEY, digest TEXT NOT NULL, active INTEGER CHECK(active=1), intent BLOB NOT NULL, outcome BLOB, CHECK((active IS NULL)=(outcome IS NOT NULL))) STRICT, WITHOUT ROWID";
const ONE_ACTIVE: &str =
    "CREATE UNIQUE INDEX one_active ON operations(active) WHERE active IS NOT NULL";
const INTENT_LIMIT: usize = 32 * 1024;

#[derive(Debug)]
pub(super) enum Key {
    Installation(String),
    Preparation(String),
    Removal(String),
    Switch(String),
    Receipt { command: String, version: String },
}

impl Key {
    fn parts(&self) -> Result<(&str, &str, &str)> {
        let (kind, command, version) = match self {
            Self::Installation(command) => ("installation", command, ""),
            Self::Preparation(command) => ("preparation", command, ""),
            Self::Removal(command) => ("removal", command, ""),
            Self::Switch(command) => ("switch", command, ""),
            Self::Receipt { command, version } => ("receipt", command, version.as_str()),
        };
        if !records::leaf(command) || (kind == "receipt" && !records::leaf(version)) {
            return Err(refuse());
        }
        Ok((kind, command, version))
    }
}

fn sql<T>(result: rusqlite::Result<T>) -> Result<T> {
    result.map_err(|error| refuse().with_source(error))
}

/// Old loose journals and their interrupted temporary writes remain evidence.
/// Never silently adopt, migrate or discard them as part of a new operation.
fn refuse_loose_records(root: &Dir) -> Result<()> {
    for entry in io(root.entries())? {
        if io(entry)?
            .file_name()
            .to_string_lossy()
            .starts_with(".nddev-software-")
        {
            return Err(refuse());
        }
    }
    Ok(())
}

pub(crate) fn validate_control(root: &Dir) -> Result<()> {
    let Some(control) = records::present(root, CONTROL)? else {
        return Ok(());
    };
    for entry in io(control.entries())? {
        let name = io(entry)?.file_name();
        let name = name.to_str().ok_or_else(refuse)?;
        if !matches!(
            name,
            crate::lock::LOCK_FILE_NAME | vfs::LOCK | vfs::DATABASE | vfs::JOURNAL
        ) {
            return Err(refuse());
        }
        io(vfs::inspect(&control, name))?.ok_or_else(refuse)?;
    }
    Ok(())
}

struct Store {
    // Field order ensures SQLite closes every handle before its scope drops.
    db: Connection,
    scope: Arc<vfs::Scope>,
    version: i64,
}

impl Store {
    fn open(root: &Dir, writable: bool) -> Result<Option<Self>> {
        refuse_loose_records(root)?;
        if writable {
            match root.create_dir(CONTROL) {
                Ok(()) => records::sync(root)?,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(refuse().with_source(error)),
            }
        }
        validate_control(root)?;
        let Some(control) = records::present(root, CONTROL)? else {
            return Ok(None);
        };
        if !writable && io(vfs::inspect(&control, vfs::DATABASE))?.is_none() {
            if io(vfs::inspect(&control, vfs::JOURNAL))?.is_some() {
                return Err(refuse());
            }
            return Ok(None);
        }
        let scope = vfs::Scope::open(root, control, writable)?;
        validate_control(root)?;
        let flags = if writable {
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
        } else {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        };
        let db = sql(Connection::open_with_flags_and_vfs(
            scope.path(),
            flags | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            vfs::NAME,
        ))?;
        sql(db.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false))?;
        sql(db.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true))?;
        // Apply limits before SQLite reads an untrusted schema. The row bound
        // leaves room above the largest inventory record and its SQL key.
        for (category, limit) in [
            (Limit::SQLITE_LIMIT_LENGTH, 17 * 1024 * 1024),
            (Limit::SQLITE_LIMIT_SQL_LENGTH, 64 * 1024),
            (Limit::SQLITE_LIMIT_COLUMN, 16),
            (Limit::SQLITE_LIMIT_EXPR_DEPTH, 100),
            (Limit::SQLITE_LIMIT_ATTACHED, 0),
            (Limit::SQLITE_LIMIT_TRIGGER_DEPTH, 0),
            (Limit::SQLITE_LIMIT_WORKER_THREADS, 0),
        ] {
            sql(db.set_limit(category, limit))?;
        }
        sql(db.execute_batch(
            "PRAGMA mmap_size=0; PRAGMA temp_store=MEMORY; PRAGMA cache_size=-2048;",
        ))?;
        let mut store = Self {
            db,
            scope,
            version: 0,
        };
        if !store.read_schema(root, writable)? {
            return Ok(None);
        }
        store.scope.validate()?;
        Ok(Some(store))
    }

    fn read_schema(&mut self, root: &Dir, writable: bool) -> Result<bool> {
        let application: i64 = sql(self
            .db
            .query_row("PRAGMA application_id", [], |row| row.get(0)))?;
        let version: i64 = sql(self
            .db
            .query_row("PRAGMA user_version", [], |row| row.get(0)))?;
        let mut statement = sql(self
            .db
            .prepare("SELECT sql FROM sqlite_schema ORDER BY name"))?;
        let schemas = sql(statement.query_map([], |row| row.get::<_, String>(0)))?
            .collect::<rusqlite::Result<Vec<_>>>();
        let schemas = sql(schemas)?;
        drop(statement);
        let owner = serde_json::to_string(&Identity::of(root)?).map_err(|_| refuse())?;
        if application == 0 && version == 0 && schemas.is_empty() {
            if !writable {
                return Ok(false);
            }
            self.configure_writer()?;
            sql(self.db.execute_batch("BEGIN IMMEDIATE"))?;
            sql(self.db.execute_batch(OWNER))?;
            sql(self.db.execute_batch(RECORDS))?;
            sql(self.db.execute_batch(OPERATIONS))?;
            sql(self.db.execute_batch(ONE_ACTIVE))?;
            sql(self.db.execute("INSERT INTO owner VALUES (1,?1)", [&owner]))?;
            sql(self.db.pragma_update(None, "application_id", APPLICATION))?;
            sql(self.db.pragma_update(None, "user_version", VERSION))?;
            sql(self.db.execute_batch("COMMIT"))?;
            self.version = VERSION;
        } else {
            let legacy = version == 1 && schemas == [OWNER, RECORDS];
            if application != APPLICATION
                || !(legacy
                    || (version == VERSION && schemas == [ONE_ACTIVE, OPERATIONS, OWNER, RECORDS]))
            {
                return Err(refuse());
            }
            let recorded: String =
                sql(self
                    .db
                    .query_row("SELECT root FROM owner WHERE singleton=1", [], |row| {
                        row.get(0)
                    }))?;
            if recorded != owner {
                return Err(refuse());
            }
            if writable {
                self.configure_writer()?;
                if legacy {
                    // A read never migrates. This exact version-one schema and
                    // physical owner were validated before the writer upgrades.
                    sql(self.db.execute_batch("BEGIN IMMEDIATE"))?;
                    sql(self.db.execute_batch(OPERATIONS))?;
                    sql(self.db.execute_batch(ONE_ACTIVE))?;
                    sql(self.db.pragma_update(None, "user_version", VERSION))?;
                    sql(self.db.execute_batch("COMMIT"))?;
                }
            }
            self.version = if writable { VERSION } else { version };
        }
        Ok(true)
    }

    fn configure_writer(&self) -> Result<()> {
        let mode: String = sql(self
            .db
            .query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0)))?;
        if mode != "delete" {
            return Err(refuse());
        }
        sql(self
            .db
            .execute_batch("PRAGMA synchronous=EXTRA; PRAGMA max_page_count=65536;"))
    }
}

pub(super) fn read<T: DeserializeOwned>(root: &Dir, key: &Key, limit: usize) -> Result<Option<T>> {
    let (kind, command, version) = key.parts()?;
    let Some(store) = Store::open(root, false)? else {
        return Ok(None);
    };
    let bytes: Option<Vec<u8>> = sql(store.db.query_row(
        "SELECT payload FROM records WHERE kind=?1 AND command=?2 AND version=?3 AND length(payload)<=?4",
        params![kind, command, version, i64::try_from(limit).map_err(|_| refuse())?], |row| row.get(0),
    ).optional())?;
    // An oversized stored row is inconsistent, never equivalent to absence.
    if bytes.is_none() && store.contains(key)? {
        return Err(refuse());
    }
    store.scope.validate()?;
    bytes
        .map(|bytes| serde_json::from_slice(&bytes).map_err(|_| refuse()))
        .transpose()
}

impl Store {
    fn contains(&self, key: &Key) -> Result<bool> {
        let (kind, command, version) = key.parts()?;
        self.scope.validate()?;
        let found = sql(self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM records WHERE kind=?1 AND command=?2 AND version=?3)",
            params![kind, command, version],
            |row| row.get(0),
        ))?;
        self.scope.validate()?;
        Ok(found)
    }
}

pub(super) fn exists(root: &Dir, key: &Key) -> Result<bool> {
    key.parts()?;
    Store::open(root, false)?.map_or(Ok(false), |store| store.contains(key))
}

pub(super) fn write<T: Serialize>(root: &Dir, key: &Key, record: &T, limit: usize) -> Result<()> {
    let (kind, command, version) = key.parts()?;
    let bytes = serde_json::to_vec(record).map_err(|_| refuse())?;
    if bytes.len() > limit {
        return Err(refuse());
    }
    let store = Store::open(root, true)?.ok_or_else(refuse)?;
    sql(store.db.execute("INSERT INTO records VALUES (?1,?2,?3,?4) ON CONFLICT(kind,command,version) DO UPDATE SET payload=excluded.payload",
        params![kind, command, version, bytes]))?;
    store.scope.validate()
}

pub(super) fn remove(root: &Dir, key: &Key) -> Result<()> {
    let (kind, command, version) = key.parts()?;
    let store = Store::open(root, true)?.ok_or_else(refuse)?;
    sql(store.db.execute(
        "DELETE FROM records WHERE kind=?1 AND command=?2 AND version=?3",
        params![kind, command, version],
    ))?;
    store.scope.validate()
}

pub(super) fn pending(root: &Dir) -> Result<bool> {
    pending_for(root, None)
}

pub(super) fn pending_for(root: &Dir, binding: Option<&super::operation::Binding>) -> Result<bool> {
    let Some(store) = Store::open(root, false)? else {
        return Ok(binding.is_some());
    };
    let pending: bool = sql(store.db.query_row(
        "SELECT EXISTS(SELECT 1 FROM records WHERE kind != 'receipt')",
        [],
        |row| row.get(0),
    ))?;
    store.scope.validate()?;
    Ok(pending || store.active()?.as_ref() != binding)
}

impl Store {
    fn active(&self) -> Result<Option<super::operation::Binding>> {
        if self.version == 1 {
            return Ok(None);
        }
        let binding = sql(self
            .db
            .query_row(
                "SELECT id,digest FROM operations WHERE active=1",
                [],
                |row| {
                    Ok(super::operation::Binding {
                        id: row.get(0)?,
                        digest: row.get(1)?,
                    })
                },
            )
            .optional())?;
        self.scope.validate()?;
        if let Some(binding) = &binding {
            binding.validate()?;
        }
        Ok(binding)
    }
}

/// Direct kernel commands cannot take over a caller-bound active operation.
pub(super) fn check_active(root: &Dir, expected: Option<&super::operation::Binding>) -> Result<()> {
    let active = Store::open(root, false)?
        .map(|store| store.active())
        .transpose()?
        .flatten();
    if active.as_ref() != expected {
        return Err(refuse());
    }
    Ok(())
}

pub(super) fn check_journals(root: &Dir, command: &str) -> Result<()> {
    let store = Store::open(root, false)?.ok_or_else(refuse)?;
    let foreign: bool = sql(store.db.query_row(
        "SELECT EXISTS(SELECT 1 FROM records WHERE kind != 'receipt' AND (command != ?1 OR kind = 'switch'))",
        [command], |row| row.get(0),
    ))?;
    store.scope.validate()?;
    if foreign {
        return Err(refuse());
    }
    Ok(())
}

pub(super) fn operation(
    root: &Dir,
    binding: &super::operation::Binding,
) -> Result<Option<(super::operation::Record, Option<super::operation::Outcome>)>> {
    type Row = (String, Option<i64>, Vec<u8>, Option<Vec<u8>>);
    binding.validate()?;
    let Some(store) = Store::open(root, false)? else {
        return Ok(None);
    };
    if store.version == 1 {
        return Ok(None);
    }
    let row: Option<Row> = sql(store
        .db
        .query_row(
            "SELECT digest,active,intent,outcome FROM operations WHERE id=?1",
            [&binding.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional())?;
    store.scope.validate()?;
    let Some((digest, active, intent, outcome)) = row else {
        return Ok(None);
    };
    if digest != binding.digest
        || intent.len() > INTENT_LIMIT
        || outcome
            .as_ref()
            .is_some_and(|bytes| bytes.len() > INTENT_LIMIT)
        || (active == Some(1)) != outcome.is_none()
    {
        return Err(refuse());
    }
    let record: super::operation::Record = serde_json::from_slice(&intent).map_err(|_| refuse())?;
    record.validate()?;
    if record.intent.binding != *binding {
        return Err(refuse());
    }
    let outcome = outcome
        .map(|bytes| serde_json::from_slice(&bytes).map_err(|_| refuse()))
        .transpose()?;
    if let Some(outcome) = &outcome {
        record.validate_outcome(outcome)?;
    }
    Ok(Some((record, outcome)))
}

pub(super) fn admit(root: &Dir, record: &super::operation::Record) -> Result<()> {
    record.validate()?;
    let bytes = serde_json::to_vec(record).map_err(|_| refuse())?;
    if bytes.len() > INTENT_LIMIT {
        return Err(refuse());
    }
    let mut store = Store::open(root, true)?.ok_or_else(refuse)?;
    let transaction = sql(store
        .db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate))?;
    let pending: bool = sql(transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM records WHERE kind != 'receipt') OR EXISTS(SELECT 1 FROM operations WHERE active=1)",
        [], |row| row.get(0),
    ))?;
    if pending {
        return Err(refuse());
    }
    sql(transaction.execute(
        "INSERT INTO operations(id,digest,active,intent,outcome) VALUES (?1,?2,1,?3,NULL)",
        params![
            record.intent.binding.id,
            record.intent.binding.digest,
            bytes
        ],
    ))?;
    sql(transaction.commit())?;
    store.scope.validate()
}

/// Retire only this operation's journal in the same commit as its outcome.
pub(super) fn complete(
    root: &Dir,
    binding: &super::operation::Binding,
    journal: Option<&Key>,
    outcome: &super::operation::Outcome,
) -> Result<()> {
    let (record, previous) = operation(root, binding)?.ok_or_else(refuse)?;
    record.validate_outcome(outcome)?;
    if previous.is_some() {
        return Err(refuse());
    }
    let bytes = serde_json::to_vec(outcome).map_err(|_| refuse())?;
    if bytes.len() > INTENT_LIMIT {
        return Err(refuse());
    }
    let mut store = Store::open(root, true)?.ok_or_else(refuse)?;
    let transaction = sql(store
        .db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate))?;
    if sql(transaction.execute(
        "UPDATE operations SET active=NULL,outcome=?1 WHERE id=?2 AND digest=?3 AND active=1 AND outcome IS NULL",
        params![bytes, binding.id, binding.digest],
    ))? != 1 { return Err(refuse()); }
    if let Some(journal) = journal {
        let (kind, command, version) = journal.parts()?;
        if command != record.intent.command || !matches!(kind, "installation" | "removal") {
            return Err(refuse());
        }
        if sql(transaction.execute(
            "DELETE FROM records WHERE kind=?1 AND command=?2 AND version=?3",
            params![kind, command, version],
        ))? != 1
        {
            return Err(refuse());
        }
    }
    let remaining: bool = sql(transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM records WHERE kind != 'receipt')",
        [],
        |row| row.get(0),
    ))?;
    if remaining {
        return Err(refuse());
    }
    sql(transaction.commit())?;
    store.scope.validate()
}

/// Called under the prefix writer before kernel recovery reads any record.
pub(super) fn recover(root: &Dir) -> Result<()> {
    drop(Store::open(root, true)?);
    Ok(())
}

pub(super) fn vacant(root: &Dir) -> Result<bool> {
    let Some(store) = Store::open(root, false)? else {
        return Ok(true);
    };
    let records: bool =
        sql(store
            .db
            .query_row("SELECT EXISTS(SELECT 1 FROM records)", [], |row| row.get(0)))?;
    let operations: bool = store.version != 1
        && sql(store
            .db
            .query_row("SELECT EXISTS(SELECT 1 FROM operations)", [], |row| {
                row.get(0)
            }))?;
    store.scope.validate()?;
    Ok(!records && !operations)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;
    use std::fs;

    fn legacy_migration_keeps_readers_read_only(
        path: &std::path::Path,
        root: &Dir,
        key: &Key,
        payload: &serde_json::Value,
    ) {
        let legacy = Store::open(root, true).unwrap().unwrap();
        legacy.db.execute_batch("BEGIN; DROP INDEX one_active; DROP TABLE operations; PRAGMA user_version=1; COMMIT;").unwrap();
        drop(legacy);
        let before = fs::read(path.join(CONTROL).join(vfs::DATABASE)).unwrap();
        assert_eq!(
            read::<serde_json::Value>(root, key, 1024).unwrap(),
            Some(payload.clone())
        );
        assert_eq!(
            fs::read(path.join(CONTROL).join(vfs::DATABASE)).unwrap(),
            before
        );
        recover(root).unwrap();
        let migrated = Store::open(root, false).unwrap().unwrap();
        assert_eq!(migrated.version, 2);
        drop(migrated);
        assert_eq!(
            read::<serde_json::Value>(root, key, 1024).unwrap(),
            Some(payload.clone())
        );
    }

    #[test]
    fn records_are_confined_bounded_read_only_and_serialized_by_physical_directory() {
        let path =
            std::env::temp_dir().join(format!("software-store-{}", records::unique().unwrap()));
        fs::create_dir(&path).unwrap();
        let root = records::open_root(&path).unwrap();
        let key = Key::Installation("tool".into());
        assert!(!exists(&root, &key).unwrap());
        assert!(!path.join(CONTROL).exists());
        let payload = serde_json::json!({"phase":"generated-fixture"});
        write(&root, &key, &payload, 1024).unwrap();
        assert_eq!(
            read::<serde_json::Value>(&root, &key, 1024).unwrap(),
            Some(payload.clone())
        );
        assert!(read::<serde_json::Value>(&root, &key, 1).is_err());
        assert!(pending(&root).unwrap());

        legacy_migration_keeps_readers_read_only(&path, &root, &key, &payload);

        let store = Store::open(&root, false).unwrap().unwrap();
        assert!(store.db.execute("DELETE FROM records", []).is_err());
        assert!(Store::open(&root.try_clone().unwrap(), true).is_err());
        let scope = Arc::downgrade(&store.scope);
        drop(store);
        assert!(scope.upgrade().is_none());
        assert_eq!(
            read::<serde_json::Value>(&root, &key, 1024).unwrap(),
            Some(payload.clone())
        );

        vfs::check_lock_release_with_retained_descriptor(&root);

        let control = path.join(CONTROL);
        let database = control.join(vfs::DATABASE);
        let foreign = path.join("foreign");
        fs::write(&foreign, b"preserve unrelated bytes").unwrap();
        let saved = control.join("held-database");
        fs::rename(&database, &saved).unwrap();
        // Unknown members are preserved even if they have plausible names.
        assert!(read::<serde_json::Value>(&root, &key, 1024).is_err());
        let held = path.join("held-database");
        fs::rename(saved, &held).unwrap();
        for name in [vfs::DATABASE, vfs::JOURNAL, vfs::LOCK] {
            let named = control.join(name);
            let retained = path.join("held-lock");
            if name == vfs::LOCK {
                fs::rename(&named, &retained).unwrap();
            }
            fs::hard_link(&foreign, &named).unwrap();
            assert!(recover(&root).is_err());
            assert_eq!(fs::read(&foreign).unwrap(), b"preserve unrelated bytes");
            fs::remove_file(&named).unwrap();
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(&foreign, &named).unwrap();
                assert!(recover(&root).is_err());
                assert_eq!(fs::read(&foreign).unwrap(), b"preserve unrelated bytes");
                fs::remove_file(&named).unwrap();
            }
            if name == vfs::LOCK {
                fs::rename(retained, named).unwrap();
            }
        }
        fs::rename(held, &database).unwrap();
        let journal = control.join(vfs::JOURNAL);
        fs::write(&journal, b"unrecognized journal").unwrap();
        assert!(recover(&root).is_err());
        assert_eq!(fs::read(&journal).unwrap(), b"unrecognized journal");
        fs::remove_file(journal).unwrap();
        let loose = path.join(".nddev-software-tool.transaction.json.interrupted");
        fs::write(&loose, b"preserve old evidence").unwrap();
        assert!(recover(&root).is_err());
        assert_eq!(fs::read(&loose).unwrap(), b"preserve old evidence");
        fs::remove_file(loose).unwrap();
        remove(&root, &key).unwrap();
        assert!(!pending(&root).unwrap());
        assert!(!exists(&root, &key).unwrap());

        // Registration is shared, but independent physical stores do not
        // contend or retain one another's directory handles after closing.
        std::thread::scope(|threads| {
            for index in 0..4 {
                let path = path.join(format!("parallel-{index}"));
                let payload = &payload;
                threads.spawn(move || {
                    fs::create_dir(&path).unwrap();
                    let root = records::open_root(&path).unwrap();
                    for _ in 0..4 {
                        let key = Key::Preparation("tool".into());
                        write(&root, &key, payload, 1024).unwrap();
                        assert_eq!(
                            read::<serde_json::Value>(&root, &key, 1024)
                                .unwrap()
                                .as_ref(),
                            Some(payload)
                        );
                        remove(&root, &key).unwrap();
                    }
                });
            }
        });
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }
}
