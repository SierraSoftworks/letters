//! The state directory: what files cannot hold, in one embedded store.
//!
//! Everything here can be lost without losing a document. Sessions, shares and the access log go with it,
//! so access fails closed, and the document index is rebuilt by a scan.
//!
//! A [`Table`] is rows of JSON under text keys. A row is small and read whole, and JSON keeps the file
//! readable by whoever has to look into it.

use std::fmt;
use std::marker::PhantomData;
use std::path::Path;

use redb::backends::InMemoryBackend;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition, WriteTransaction};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// The store's file in the state directory.
pub const FILE: &str = "letters.redb";

#[derive(Debug)]
pub enum StoreError {
    /// The store could not be opened, read or written.
    Storage(String),
    /// A row is not what this build writes there.
    Row { table: &'static str, key: String, why: String },
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(why) => write!(f, "the state store failed: {why}"),
            Self::Row { table, key, why } => write!(f, "the row {key:?} of {table} cannot be read: {why}"),
        }
    }
}

impl std::error::Error for StoreError {}

macro_rules! storage_error {
    ($($error:ty),*) => {$(
        impl From<$error> for StoreError {
            fn from(error: $error) -> Self {
                Self::Storage(error.to_string())
            }
        }
    )*};
}
storage_error!(
    redb::DatabaseError,
    redb::TransactionError,
    redb::TableError,
    redb::StorageError,
    redb::CommitError,
    std::io::Error
);

/// Rows of `V` under text keys.
pub struct Table<V> {
    name: &'static str,
    rows: PhantomData<fn() -> V>,
}

impl<V> Table<V> {
    pub const fn new(name: &'static str) -> Self {
        Self { name, rows: PhantomData }
    }

    fn definition(&self) -> TableDefinition<'static, &'static str, &'static [u8]> {
        TableDefinition::new(self.name)
    }

    fn row(&self, key: &str, bytes: &[u8]) -> Result<V, StoreError>
    where
        V: DeserializeOwned,
    {
        let bad =
            |error: serde_json::Error| StoreError::Row { table: self.name, key: key.into(), why: error.to_string() };
        serde_json::from_slice(bytes).map_err(bad)
    }
}

pub struct Store {
    db: Database,
}

impl Store {
    /// Opens the store in `folder`, which is made if it is not there.
    pub fn open(folder: &Path) -> Result<Self, StoreError> {
        std::fs::create_dir_all(folder)?;
        Ok(Self { db: Database::create(folder.join(FILE))? })
    }

    /// A store that is gone when it is dropped, for tests.
    pub fn in_memory() -> Self {
        let db = Database::builder().create_with_backend(InMemoryBackend::new());
        Self { db: db.expect("a store in memory opens") }
    }

    pub fn get<V: DeserializeOwned>(&self, table: &Table<V>, key: &str) -> Result<Option<V>, StoreError> {
        let read = self.db.begin_read()?;
        let rows = match read.open_table(table.definition()) {
            Ok(rows) => rows,
            // Nothing was ever written to it.
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let found = rows.get(key)?;
        found.map(|bytes| table.row(key, bytes.value())).transpose()
    }

    /// Every row whose key starts with `prefix`, in key order.
    pub fn under<V: DeserializeOwned>(&self, table: &Table<V>, prefix: &str) -> Result<Vec<(String, V)>, StoreError> {
        let read = self.db.begin_read()?;
        let rows = match read.open_table(table.definition()) {
            Ok(rows) => rows,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut out = Vec::new();
        for row in rows.range(prefix..)? {
            let (key, bytes) = row?;
            if !key.value().starts_with(prefix) {
                break;
            }
            out.push((key.value().to_owned(), table.row(key.value(), bytes.value())?));
        }
        Ok(out)
    }

    pub fn put<V: Serialize>(&self, table: &Table<V>, key: &str, value: &V) -> Result<(), StoreError> {
        self.write(|batch| batch.put(table, key, value))
    }

    pub fn remove<V>(&self, table: &Table<V>, key: &str) -> Result<(), StoreError> {
        self.write(|batch| batch.remove(table, key))
    }

    /// Makes several changes that land together or not at all. Only one writer runs at a time, so what
    /// `change` reads through the batch is what it then writes over.
    pub fn write<T>(&self, change: impl FnOnce(&mut Batch) -> Result<T, StoreError>) -> Result<T, StoreError> {
        let mut batch = Batch { write: self.db.begin_write()? };
        let done = change(&mut batch)?;
        batch.write.commit()?;
        Ok(done)
    }
}

/// The changes of one [`Store::write`].
pub struct Batch {
    write: WriteTransaction,
}

impl Batch {
    pub fn get<V: DeserializeOwned>(&mut self, table: &Table<V>, key: &str) -> Result<Option<V>, StoreError> {
        let rows = self.write.open_table(table.definition())?;
        let found = rows.get(key)?;
        found.map(|bytes| table.row(key, bytes.value())).transpose()
    }

    pub fn put<V: Serialize>(&mut self, table: &Table<V>, key: &str, value: &V) -> Result<(), StoreError> {
        let bytes = serde_json::to_vec(value).expect("a row serialises");
        self.write.open_table(table.definition())?.insert(key, bytes.as_slice())?;
        Ok(())
    }

    pub fn remove<V>(&mut self, table: &Table<V>, key: &str) -> Result<(), StoreError> {
        self.write.open_table(table.definition())?.remove(key)?;
        Ok(())
    }

    /// Removes every row whose key starts with `prefix` and that `keep` does not ask to keep.
    pub fn prune<V>(
        &mut self,
        table: &Table<V>,
        prefix: &str,
        mut keep: impl FnMut(&str) -> bool,
    ) -> Result<(), StoreError> {
        let mut rows = self.write.open_table(table.definition())?;
        let mut gone = Vec::new();
        for row in rows.range(prefix..)? {
            let (key, _) = row?;
            if !key.value().starts_with(prefix) {
                break;
            }
            if !keep(key.value()) {
                gone.push(key.value().to_owned());
            }
        }
        for key in gone {
            rows.remove(key.as_str())?;
        }
        Ok(())
    }
}
