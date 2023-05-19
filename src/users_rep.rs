use std::{ffi::OsStr, path::Path};

use chrono::FixedOffset;
use pickledb::error::Result;
use pickledb::{PickleDb, PickleDbDumpPolicy, SerializationMethod};
use teloxide::types::ChatId;

pub struct UsersRep {
    db: PickleDb,
}

const _DEFAULT_SECS: i32 = 5 * 3600;

// FIXME: RENAME
impl UsersRep {
    pub fn new<P: AsRef<Path>>(path: P) -> UsersRep {
        let db = PickleDb::new(
            path,
            PickleDbDumpPolicy::AutoDump,
            SerializationMethod::Json,
        );

        UsersRep { db }
    }
    pub fn open<P: AsRef<Path>>(path: P) -> Result<UsersRep> {
        Ok(UsersRep {
            db: PickleDb::load(
                path,
                PickleDbDumpPolicy::AutoDump,
                SerializationMethod::Json,
            )?,
        })
    }

    pub fn open_or_create<S: AsRef<OsStr> + ?Sized>(s: &S) -> Result<UsersRep> {
        let path = Path::new(s);

        if path.exists() {
            return UsersRep::open(path);
        }
        Ok(UsersRep::new(path))
    }

    pub fn get(&self, user_id: &ChatId) -> FixedOffset {
        let secs = self
            .db
            .get::<i32>(&user_id.0.to_string())
            .expect("Unexpected behavior: user timezone is not setted");

        FixedOffset::east_opt(secs).expect(&format!(
            "Unexpected behavior: user timezone is invalid {}",
            secs
        ))
    }

    pub fn add(&mut self, user_id: &ChatId) -> Result<()> {
        self.db.set(&user_id.0.to_string(), &_DEFAULT_SECS)
    }

    pub fn rem(&mut self, user_id: &ChatId) -> Result<bool> {
        self.db.rem(&user_id.0.to_string())
    }

    pub fn exists(&self, user_id: &ChatId) -> bool {
        self.db.exists(&user_id.0.to_string())
    }

    pub fn get_all(&self) -> Vec<ChatId> {
        self.db
            .get_all()
            .iter()
            .map(|chat_id_str| ChatId(chat_id_str.parse::<i64>().unwrap()))
            .collect()
    }
}
