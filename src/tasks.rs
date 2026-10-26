// =====================================================================
// Todo / Tasks (stored in %APPDATA%\EmelkaNotch\todos.db)
// =====================================================================

use chrono::Local;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::notification::{push_notice, NotifState};

#[derive(Clone)]
pub(crate) struct Todo {
    pub id: u64,
    pub text: String,
    pub done: bool,
    pub due: Option<i64>, // unix seconds
    pub notified: bool,
}

#[derive(Default)]
pub(crate) struct TodoState {
    pub items: Vec<Todo>,
    pub next_id: u64,
}

fn todo_db_path() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("APPDATA")?).join("EmelkaNotch");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("todos.db"))
}

fn open_todo_db() -> rusqlite::Result<Connection> {
    let p = todo_db_path().ok_or(rusqlite::Error::InvalidPath(PathBuf::new()))?;
    let conn = Connection::open(p)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS todos (
            id       INTEGER PRIMARY KEY,
            text     TEXT NOT NULL,
            done     INTEGER NOT NULL DEFAULT 0,
            due      INTEGER,
            notified INTEGER NOT NULL DEFAULT 0
        );",
    )?;
    Ok(conn)
}

pub(crate) fn load_todos() -> TodoState {
    let mut st = TodoState::default();
    let Ok(conn) = open_todo_db() else { return st };
    let Ok(mut stmt) = conn.prepare("SELECT id, text, done, due, notified FROM todos ORDER BY id")
    else {
        return st;
    };
    let Ok(rows) = stmt.query_map([], |r| {
        Ok(Todo {
            id: r.get::<_, i64>(0)? as u64,
            text: r.get(1)?,
            done: r.get::<_, i64>(2)? != 0,
            due: r.get(3)?,
            notified: r.get::<_, i64>(4)? != 0,
        })
    }) else {
        return st;
    };
    for t in rows.flatten() {
        st.next_id = st.next_id.max(t.id);
        st.items.push(t);
    }
    st
}

pub(crate) fn save_todos(items: &[Todo]) {
    let Ok(mut conn) = open_todo_db() else { return };
    let Ok(tx) = conn.transaction() else { return };
    let _ = tx.execute("DELETE FROM todos", []);
    for t in items {
        let _ = tx.execute(
            "INSERT INTO todos (id, text, done, due, notified) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![t.id as i64, t.text, t.done as i64, t.due, t.notified as i64],
        );
    }
    let _ = tx.commit();
}

/// Checks every second; when a todo is due, sends a notification to the pill.
pub(crate) fn spawn_todo_worker(
    ctx: eframe::egui::Context,
    todos: Arc<Mutex<TodoState>>,
    notifs: Arc<Mutex<NotifState>>,
) {
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(1));
        let now = Local::now().timestamp();
        let mut fired: Vec<String> = Vec::new();
        {
            let mut st = todos.lock().unwrap();
            for t in st.items.iter_mut() {
                if !t.done && !t.notified && t.due.map_or(false, |d| d <= now) {
                    t.notified = true;
                    fired.push(t.text.clone());
                }
            }
            if !fired.is_empty() {
                save_todos(&st.items);
            }
        }
        if !fired.is_empty() {
            for text in fired {
                push_notice(&notifs, "Reminder", text);
            }
            ctx.request_repaint();
        }
    });
}
