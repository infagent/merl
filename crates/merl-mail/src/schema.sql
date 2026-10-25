CREATE TABLE IF NOT EXISTS identities(name TEXT PRIMARY KEY, generation INTEGER NOT NULL DEFAULT 0, session TEXT, socket TEXT, socket_identity TEXT);
CREATE TABLE IF NOT EXISTS messages(seq INTEGER PRIMARY KEY AUTOINCREMENT,id TEXT NOT NULL UNIQUE,sender TEXT NOT NULL REFERENCES identities(name),recipient TEXT NOT NULL REFERENCES identities(name),kind TEXT NOT NULL,body TEXT NOT NULL,reply TEXT REFERENCES messages(id),created_at INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS messages_recipient ON messages(recipient,seq);
CREATE TABLE IF NOT EXISTS handling(message TEXT PRIMARY KEY REFERENCES messages(id),read_at INTEGER,received_at INTEGER,completed_at INTEGER,completion_reply TEXT REFERENCES messages(id));
CREATE TABLE IF NOT EXISTS notifications(agent TEXT PRIMARY KEY REFERENCES identities(name),state TEXT NOT NULL,next_at INTEGER NOT NULL,attempts INTEGER NOT NULL DEFAULT 0,last_error TEXT,generation INTEGER);
CREATE TRIGGER IF NOT EXISTS immutable_mail_update BEFORE UPDATE ON messages BEGIN SELECT RAISE(ABORT,'immutable mail'); END;
CREATE TRIGGER IF NOT EXISTS immutable_mail_delete BEFORE DELETE ON messages BEGIN SELECT RAISE(ABORT,'immutable mail'); END;
PRAGMA user_version=1;
