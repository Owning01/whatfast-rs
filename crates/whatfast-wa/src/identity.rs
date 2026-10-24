//! The phone's identity data: the agenda names and the LID↔phone map.
//!
//! whatsapp-rust keeps both in its own session database, the same way
//! whatsmeow did: `whatsmeow_contacts` carries the full name the user saved
//! in the phone (the agenda) beside the push name, and `whatsmeow_lid_map` /
//! `lid_pn_mapping` carry the hidden-identity ↔ phone-number pairs the
//! server assigns. The sidecar reads them through a second, read-only
//! connection: the fork owns the writes, and SQLite's WAL allows the two
//! readers to share the file.

use std::path::Path;
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
    conn: Mutex<Connection>,
}

impl Identity {
    /// Opens the identity tables. A missing or unreadable database answers
    /// through an empty in-memory stand-in: every lookup then knows nothing,
    /// and the titles fall back to the push names.
    pub fn open(data_dir: &Path) -> Self {
        let conn = Connection::open_with_flags(
            data_dir.join("session.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .unwrap_or_else(|_| {
            let memory = Connection::open_in_memory().expect("an in-memory database");
            memory
                .execute_batch(
                    "CREATE TABLE whatsmeow_contacts(our_jid TEXT, their_jid TEXT, first_name TEXT,
                         full_name TEXT, push_name TEXT, business_name TEXT, redacted_phone TEXT);
                     CREATE TABLE whatsmeow_lid_map(lid TEXT, pn TEXT);
                     CREATE TABLE lid_pn_mapping(lid TEXT, phone_number TEXT);",
                )
                .expect("the empty identity tables");
            memory
        });
        Self {
            conn: Mutex::new(conn),
        }
    }

    /// The phone-number jid of one lid jid, when the map knows it.
    pub fn pn_of(&self, lid: &str) -> Option<String> {
        let digits = digits_of(lid)?;
        let conn = self.conn.lock().expect("identity lock");
        query_pair(&conn, &digits).map(|pn| format!("{pn}@s.whatsapp.net"))
    }

    /// The lid jid of one phone-number jid, when the map knows it.
    pub fn lid_of(&self, pn: &str) -> Option<String> {
        let digits = digits_of(pn)?;
        let conn = self.conn.lock().expect("identity lock");
        conn.query_row(
            "SELECT lid FROM whatsmeow_lid_map WHERE pn=?1
             UNION SELECT lid FROM lid_pn_mapping WHERE phone_number=?1 LIMIT 1",
            [&digits],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .map(|lid| format!("{lid}@lid"))
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

    /// The full contact list of the phone, agenda names included.
    pub fn phone_contacts(&self) -> Vec<PhoneContact> {
        let Ok(conn) = self.conn.lock() else {
            return Vec::new();
        };
        let Ok(mut statement) = conn.prepare(
            "SELECT their_jid, full_name, first_name, push_name, business_name
             FROM whatsmeow_contacts",
        ) else {
            return Vec::new();
        };
        statement
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
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    }
}

/// The user part of one jid, when it is a plain number.
fn digits_of(jid: &str) -> Option<String> {
    let user = jid.split('@').next()?;
    if !user.is_empty() && user.chars().all(|c| c.is_ascii_digit()) {
        Some(user.to_owned())
    } else {
        None
    }
}

/// The phone number of one lid, from whichever mapping table has it.
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
    use super::digits_of;

    #[test]
    fn the_user_part_is_digits_or_nothing() {
        assert_eq!(
            digits_of("5493816041868@s.whatsapp.net").as_deref(),
            Some("5493816041868")
        );
        assert_eq!(
            digits_of("162040542929138@lid").as_deref(),
            Some("162040542929138")
        );
        // The server is not digits_of's business: the callers pick the user
        // servers before asking.
        assert_eq!(digits_of("120363@g.us").as_deref(), Some("120363"));
        assert_eq!(digits_of("g.us").as_deref(), None);
    }
}
