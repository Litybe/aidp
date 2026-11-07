CREATE TABLE chunks (
    rowid INTEGER PRIMARY KEY,
    id TEXT NOT NULL UNIQUE,
    project TEXT NOT NULL,
    source TEXT NOT NULL,
    path TEXT NOT NULL,
    line_start INTEGER NOT NULL,
    line_end INTEGER NOT NULL,
    title TEXT NOT NULL,
    text TEXT NOT NULL,
    idents TEXT NOT NULL DEFAULT '',
    hash TEXT NOT NULL,
    mtime INTEGER NOT NULL,
    meta TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX chunks_file ON chunks (project, source, path);
CREATE INDEX chunks_span ON chunks (project, path, line_start);

CREATE VIRTUAL TABLE chunks_fts USING fts5(
    title, text, idents, path,
    content='chunks', content_rowid='rowid',
    tokenize='unicode61 remove_diacritics 2',
    prefix='2 3 4'
);

CREATE TRIGGER chunks_ai AFTER INSERT ON chunks BEGIN
    INSERT INTO chunks_fts (rowid, title, text, idents, path)
    VALUES (new.rowid, new.title, new.text, new.idents, new.path);
END;
CREATE TRIGGER chunks_ad AFTER DELETE ON chunks BEGIN
    INSERT INTO chunks_fts (chunks_fts, rowid, title, text, idents, path)
    VALUES ('delete', old.rowid, old.title, old.text, old.idents, old.path);
END;
CREATE TRIGGER chunks_au AFTER UPDATE ON chunks BEGIN
    INSERT INTO chunks_fts (chunks_fts, rowid, title, text, idents, path)
    VALUES ('delete', old.rowid, old.title, old.text, old.idents, old.path);
    INSERT INTO chunks_fts (rowid, title, text, idents, path)
    VALUES (new.rowid, new.title, new.text, new.idents, new.path);
END;

CREATE TABLE files (
    project TEXT NOT NULL,
    source TEXT NOT NULL,
    path TEXT NOT NULL,
    hash TEXT NOT NULL,
    mtime INTEGER NOT NULL,
    size INTEGER NOT NULL,
    PRIMARY KEY (project, source, path)
);

CREATE TABLE memories (
    num INTEGER PRIMARY KEY AUTOINCREMENT,
    project TEXT NOT NULL,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    text TEXT NOT NULL,
    scope TEXT NOT NULL DEFAULT '{}',
    refs TEXT NOT NULL DEFAULT '[]',
    confidence REAL NOT NULL,
    evidence INTEGER NOT NULL DEFAULT 1,
    status TEXT NOT NULL,
    supersedes INTEGER,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    last_used_at TEXT,
    uses INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX memories_project ON memories (project, status);

CREATE VIRTUAL TABLE memories_fts USING fts5(
    title, text, tags,
    content='', contentless_delete=1,
    tokenize='unicode61 remove_diacritics 2',
    prefix='2 3'
);
