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
const VERSION: i64 = 1;
const RECORDS: &str = "CREATE TABLE records (kind TEXT NOT NULL CHECK(kind IN ('installation','preparation','removal','switch','receipt')), command TEXT NOT NULL, version TEXT NOT NULL, payload BLOB NOT NULL, PRIMARY KEY(kind,command,version)) STRICT, WITHOUT ROWID";
const OWNER: &str = "CREATE TABLE owner (singleton INTEGER PRIMARY KEY CHECK(singleton=1), root TEXT NOT NULL) STRICT";

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
        let store = Self { db, scope };
        let application: i64 = sql(store
            .db
            .query_row("PRAGMA application_id", [], |row| row.get(0)))?;
        let version: i64 = sql(store
            .db
            .query_row("PRAGMA user_version", [], |row| row.get(0)))?;
        let mut statement = sql(store
            .db
            .prepare("SELECT sql FROM sqlite_schema ORDER BY name"))?;
        let schemas = sql(statement.query_map([], |row| row.get::<_, String>(0)))?
            .collect::<rusqlite::Result<Vec<_>>>();
        let schemas = sql(schemas)?;
        drop(statement);
        let owner = serde_json::to_string(&Identity::of(root)?).map_err(|_| refuse())?;
        if application == 0 && version == 0 && schemas.is_empty() {
            if !writable {
                return Ok(None);
            }
            store.configure_writer()?;
            sql(store.db.execute_batch("BEGIN IMMEDIATE"))?;
            sql(store.db.execute_batch(OWNER))?;
            sql(store.db.execute_batch(RECORDS))?;
            sql(store
                .db
                .execute("INSERT INTO owner VALUES (1,?1)", [&owner]))?;
            sql(store.db.pragma_update(None, "application_id", APPLICATION))?;
            sql(store.db.pragma_update(None, "user_version", VERSION))?;
            sql(store.db.execute_batch("COMMIT"))?;
        } else {
            if application != APPLICATION || version != VERSION || schemas != [OWNER, RECORDS] {
                return Err(refuse());
            }
            let recorded: String =
                sql(store
                    .db
                    .query_row("SELECT root FROM owner WHERE singleton=1", [], |row| {
                        row.get(0)
                    }))?;
            if recorded != owner {
                return Err(refuse());
            }
            if writable {
                store.configure_writer()?;
            }
        }
        store.scope.validate()?;
        Ok(Some(store))
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
    let Some(store) = Store::open(root, false)? else {
        return Ok(false);
    };
    let pending = sql(store.db.query_row(
        "SELECT EXISTS(SELECT 1 FROM records WHERE kind != 'receipt')",
        [],
        |row| row.get(0),
    ))?;
    store.scope.validate()?;
    Ok(pending)
}

/// Called under the prefix writer before kernel recovery reads any record.
pub(super) fn recover(root: &Dir) -> Result<()> {
    drop(Store::open(root, true)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;
    use std::fs;

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
