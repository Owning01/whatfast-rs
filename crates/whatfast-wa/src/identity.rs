//! The phone's identity data: the agenda names and the LID↔phone map.
//!
//! whatsapp-rust keeps both in its own session database, the same way
//! whatsmeow did: `whatsmeow_contacts` carries the full name the user saved
//! in the phone (the agenda) beside the push name, and `whatsmeow_lid_map` /
//! `lid_pn_mapping` carry the hidden-identity ↔ phone-number pairs the
//! server assigns. The sidecar reads them through a second, read-only
//! connection: the fork owns the writes, and SQLite's WAL allows the two
//! readers to share the file.
//!
//! The connection is opened on the first lookup, and reopened whenever the
//! database was not there yet. A sidecar that starts before the fork links
//! its device (or while the fork creates the file) must not answer "no
//! agenda" for the rest of the process: the Go read the live store on every
//! call, and this window is the port's equivalent.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::Connection;

/// One contact as the phone knows it.
pub struct PhoneContact {
    pub jid: String,
    pub full_name: String,
    pub first_name: String,
    pub push_name: String,
    pub business_name: String,
}

/// The read-only window over the fork's session database.
pub struct Identity {
    path: PathBuf,
    conn: Mutex<Option<Connection>>,
}

impl Identity {
    /// Points the identity reader at one data directory. The file is opened
    /// lazily: a missing or unreadable database answers nothing until the
    /// fork creates it, and every lookup after that sees it.
    pub fn open(data_dir: &Path) -> Self {
        let path = data_dir.join("session.db");
        let conn = open_read_only(&path);
        Self {
            path,
            conn: Mutex::new(conn),
        }
    }

    /// Runs one read through the identity connection, opening it when the
    /// earlier attempts found no database. Answers `None` while the fork has
    /// not created it.
    fn with_conn<T>(&self, work: impl FnOnce(&Connection) -> Option<T>) -> Option<T> {
        let mut guard = self.conn.lock().ok()?;
        if guard.is_none() {
            *guard = open_read_only(&self.path);
        }
        work(guard.as_ref()?)
    }

    /// The phone-number jid of one lid jid, when the map knows it.
    pub fn pn_of(&self, lid: &str) -> Option<String> {
        let digits = user_part(lid)?;
        let pn = self.with_conn(|conn| query_pair(conn, &digits))?;
        let pn = user_part(&pn)?;
        Some(format!("{pn}@s.whatsapp.net"))
    }

    /// The lid jid of one phone-number jid, when the map knows it.
    pub fn lid_of(&self, pn: &str) -> Option<String> {
        let digits = user_part(pn)?;
        let lid = self.with_conn(|conn| {
            conn.query_row(
                "SELECT lid FROM whatsmeow_lid_map WHERE pn=?1
                 UNION SELECT lid FROM lid_pn_mapping WHERE phone_number=?1 LIMIT 1",
                [&digits],
                |row| row.get::<_, String>(0),
            )
            .ok()
        })?;
        let lid = user_part(&lid)?;
        Some(format!("{lid}@lid"))
    }

    /// Every jid form of one user: the given one plus its mapped twin.
    pub fn aliases(&self, jid: &str) -> Vec<String> {
        let mut out = vec![jid.to_owned()];
        if jid.ends_with("@lid")
            && let Some(pn) = self.pn_of(jid)
        {
            out.push(pn);
        } else if jid.ends_with("@s.whatsapp.net")
            && let Some(lid) = self.lid_of(jid)
        {
            out.push(lid);
        }
        out
    }

    /// Looks up a contact's display name and phone number from the session database.
    pub fn lookup_name(&self, jid: &str) -> Option<(String, String)> {
        let aliases = self.aliases(jid);
        self.with_conn(|conn| {
            for alias in &aliases {
                let mut statement = conn
                    .prepare(
                        "SELECT full_name, push_name, business_name
                         FROM whatsmeow_contacts WHERE their_jid=?1 LIMIT 1",
                    )
                    .ok()?;
                let result = statement
                    .query_row([alias], |row| {
                        let text = |index: usize| {
                            row.get::<_, Option<String>>(index)
                                .ok()
                                .flatten()
                                .unwrap_or_default()
                        };
                        Ok((text(0), text(1), text(2)))
                    })
                    .ok();
                if let Some((full, push, biz)) = result {
                    let name = [full, biz, push]
                        .into_iter()
                        .find(|c| !c.trim().is_empty())
                        .unwrap_or_default();
                    if !name.is_empty() {
                        let phone = if alias.ends_with("@s.whatsapp.net") {
                            user_part(alias).unwrap_or_default()
                        } else {
                            query_pair(conn, &user_part(alias).unwrap_or_default())
                                .and_then(|pn| user_part(&pn))
                                .unwrap_or_default()
                        };
                        return Some((name, phone));
                    }
                }
            }
            None
        })
    }

    /// The full contact list of the phone, agenda names included.
    pub fn phone_contacts(&self) -> Vec<PhoneContact> {
        self.with_conn(|conn| {
            let mut statement = conn
                .prepare(
                    "SELECT their_jid, full_name, first_name, push_name, business_name
                     FROM whatsmeow_contacts",
                )
                .ok()?;
            let rows = statement
                .query_map([], |row| {
                    let text = |index: usize| {
                        row.get::<_, Option<String>>(index)
                            .ok()
                            .flatten()
                            .unwrap_or_default()
                    };
                    Ok(PhoneContact {
                        jid: row.get::<_, String>(0).unwrap_or_default(),
                        full_name: text(1),
                        first_name: text(2),
                        push_name: text(3),
                        business_name: text(4),
                    })
                })
                .ok()?;
            Some(rows.flatten().collect())
        })
        .unwrap_or_default()
    }
}

/// The read-only connection over one session database, when it exists and
/// the fork's WAL lets a reader in.
fn open_read_only(path: &Path) -> Option<Connection> {
    Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()
}

/// The user part of one jid or of one stored mapping, when it is a plain
/// number.
///
/// The mapping tables keep the bare user part, but a fork that stores the
/// whole jid must still answer: the server suffix is dropped before the
/// digits are checked.
fn user_part(jid: &str) -> Option<String> {
    let user = jid.split('@').next()?;
    let user = user.split(':').next()?;
    if !user.is_empty() && user.chars().all(|c| c.is_ascii_digit()) {
        Some(user.to_owned())
    } else {
        None
    }
}

/// The phone number of one lid, from whichever mapping table has it.
///
/// `lid_pn_mapping` is the fork's own table and it holds the pairs
/// `whatsmeow_lid_map` never learned: the real profile has 318 of them. The
/// other direction is symmetric, and the two tables never disagree on a lid
/// both know.
fn query_pair(conn: &Connection, lid_digits: &str) -> Option<String> {
    conn.query_row(
        "SELECT pn FROM whatsmeow_lid_map WHERE lid=?1
         UNION SELECT phone_number FROM lid_pn_mapping WHERE lid=?1 LIMIT 1",
        [lid_digits],
        |row| row.get::<_, String>(0),
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::{Identity, user_part};

    /// A directory nobody else uses, holding a session.db the test owns.
    fn profile(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "whatfast-wa-identity-unit-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn create_session_db(dir: &std::path::Path) {
        let session = rusqlite::Connection::open(dir.join("session.db")).unwrap();
        session
            .execute_batch(
                "CREATE TABLE whatsmeow_contacts(our_jid TEXT, their_jid TEXT, first_name TEXT,
                     full_name TEXT, push_name TEXT, business_name TEXT, redacted_phone TEXT);
                 CREATE TABLE whatsmeow_lid_map(lid TEXT, pn TEXT);
                 CREATE TABLE lid_pn_mapping(lid TEXT, phone_number TEXT);
                 INSERT INTO lid_pn_mapping VALUES('162040542929138','5493816041868@s.whatsapp.net');",
            )
            .unwrap();
    }

    #[test]
    fn the_user_part_is_digits_or_nothing() {
        assert_eq!(
            user_part("5493816041868@s.whatsapp.net").as_deref(),
            Some("5493816041868")
        );
        assert_eq!(
            user_part("162040542929138@lid").as_deref(),
            Some("162040542929138")
        );
        // The device suffix of our own jid is not part of the user.
        assert_eq!(
            user_part("5493816041868:59@s.whatsapp.net").as_deref(),
            Some("5493816041868")
        );
        // The server is not user_part's business: the callers pick the user
        // servers before asking.
        assert_eq!(user_part("120363@g.us").as_deref(), Some("120363"));
        assert_eq!(user_part("g.us").as_deref(), None);
        assert_eq!(user_part("abcd@s.whatsapp.net").as_deref(), None);
    }

    #[test]
    fn the_identity_tables_open_themselves_when_the_fork_creates_them() {
        let dir = profile("late-db");
        let identity = Identity::open(&dir);
        // The sidecar started before the fork linked the device.
        assert_eq!(identity.pn_of("162040542929138@lid"), None);
        assert!(identity.phone_contacts().is_empty());

        create_session_db(&dir);
        // The mapping is a full jid in the fork's own table: it still reads.
        assert_eq!(
            identity.pn_of("162040542929138@lid").as_deref(),
            Some("5493816041868@s.whatsapp.net")
        );
    }
}
