//! Build a small Zotero-shaped SQLite database for tests.

use rusqlite::Connection;
use std::path::PathBuf;

pub struct Fixture {
    pub dir: tempfile::TempDir,
}

impl Fixture {
    pub fn sqlite_path(&self) -> PathBuf {
        self.dir.path().join("zotero.sqlite")
    }
    #[allow(dead_code)]
    pub fn storage_dir(&self) -> PathBuf {
        self.dir.path().join("storage")
    }
}

pub fn build() -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("zotero.sqlite");
    let conn = Connection::open(&db_path).expect("open");
    create_schema(&conn);
    insert_minimal_data(&conn);
    drop(conn);

    let storage = dir.path().join("storage");
    std::fs::create_dir_all(storage.join("AAAA0001")).unwrap();
    std::fs::write(storage.join("AAAA0001").join("paper.pdf"), b"%PDF-1.4 fake").unwrap();
    std::fs::write(
        storage.join("AAAA0001").join(".zotero-ft-cache"),
        b"Cached extracted text of the paper containing keyword zoteroconnectortest.",
    ).unwrap();
    std::fs::create_dir_all(storage.join("BBBB0002")).unwrap();
    std::fs::write(
        storage.join("BBBB0002").join("article.html"),
        b"<html><body><article><h1>An Article</h1><p>Hello snapshot.</p></article></body></html>",
    ).unwrap();

    Fixture { dir }
}

fn create_schema(c: &Connection) {
    // Minimal subset of the real Zotero schema needed by reader code.
    c.execute_batch(r#"
        CREATE TABLE version (schema TEXT PRIMARY KEY, version INT NOT NULL);
        CREATE TABLE libraries (libraryID INTEGER PRIMARY KEY);
        CREATE TABLE itemTypes (itemTypeID INTEGER PRIMARY KEY, typeName TEXT NOT NULL);
        CREATE TABLE fields (fieldID INTEGER PRIMARY KEY, fieldName TEXT NOT NULL);
        CREATE TABLE fieldsCombined (fieldID INTEGER PRIMARY KEY, fieldName TEXT NOT NULL);
        CREATE TABLE creatorTypes (creatorTypeID INTEGER PRIMARY KEY, creatorType TEXT NOT NULL);
        CREATE TABLE items (
            itemID INTEGER PRIMARY KEY,
            itemTypeID INT NOT NULL,
            dateAdded TIMESTAMP NOT NULL,
            dateModified TIMESTAMP NOT NULL,
            clientDateModified TIMESTAMP NOT NULL,
            libraryID INT NOT NULL,
            key TEXT NOT NULL,
            version INT NOT NULL,
            synced INT NOT NULL DEFAULT 0
        );
        CREATE TABLE itemDataValues (valueID INTEGER PRIMARY KEY, value);
        CREATE TABLE itemData (itemID INT, fieldID INT, valueID INT, PRIMARY KEY (itemID, fieldID));
        CREATE TABLE creators (creatorID INTEGER PRIMARY KEY, firstName TEXT, lastName TEXT, fieldMode INT);
        CREATE TABLE itemCreators (itemID INT, creatorID INT, creatorTypeID INT, orderIndex INT, PRIMARY KEY (itemID, creatorID, creatorTypeID, orderIndex));
        CREATE TABLE itemAttachments (
            itemID INTEGER PRIMARY KEY,
            parentItemID INT,
            linkMode INT,
            contentType TEXT,
            path TEXT,
            syncState INT DEFAULT 0
        );
        CREATE TABLE tags (tagID INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
        CREATE TABLE itemTags (itemID INT, tagID INT, type INT, PRIMARY KEY (itemID, tagID));
        CREATE TABLE collections (
            collectionID INTEGER PRIMARY KEY,
            collectionName TEXT NOT NULL,
            parentCollectionID INT,
            libraryID INT NOT NULL,
            key TEXT NOT NULL,
            version INT NOT NULL DEFAULT 0
        );
        CREATE TABLE collectionItems (collectionID INT, itemID INT, orderIndex INT, PRIMARY KEY (collectionID, itemID));
        CREATE TABLE fulltextWords (wordID INTEGER PRIMARY KEY, word TEXT UNIQUE);
        CREATE TABLE fulltextItems (itemID INTEGER PRIMARY KEY, indexedPages INT, totalPages INT, indexedChars INT, totalChars INT, version INT, synced INT);
        CREATE TABLE fulltextItemWords (wordID INT, itemID INT, PRIMARY KEY (wordID, itemID));
        CREATE TABLE itemAnnotations (
            itemID INTEGER PRIMARY KEY,
            parentItemID INT NOT NULL,
            type INTEGER NOT NULL,
            authorName TEXT,
            text TEXT,
            comment TEXT,
            color TEXT,
            pageLabel TEXT,
            sortIndex TEXT NOT NULL,
            position TEXT NOT NULL,
            isExternal INT NOT NULL
        );
        CREATE TABLE itemNotes (itemID INTEGER PRIMARY KEY, parentItemID INT, note TEXT, title TEXT);
    "#).unwrap();
}

fn insert_minimal_data(c: &Connection) {
    c.execute("INSERT INTO version(schema, version) VALUES ('userdata', 125)", []).unwrap();
    c.execute("INSERT INTO libraries(libraryID) VALUES (1)", []).unwrap();
    c.execute("INSERT INTO itemTypes(itemTypeID, typeName) VALUES (2, 'book'), (4, 'journalArticle'), (14, 'webpage'), (3, 'attachment'), (12, 'note'), (37, 'annotation')", []).unwrap();
    c.execute("INSERT INTO fields(fieldID, fieldName) VALUES (1, 'title'), (3, 'date'), (4, 'publisher'), (52, 'DOI'), (60, 'url'), (90, 'abstractNote')", []).unwrap();
    c.execute("INSERT INTO fieldsCombined(fieldID, fieldName) VALUES (1, 'title'), (3, 'date'), (4, 'publisher'), (52, 'DOI'), (60, 'url'), (90, 'abstractNote')", []).unwrap();
    c.execute("INSERT INTO creatorTypes(creatorTypeID, creatorType) VALUES (1, 'author'), (2, 'editor')", []).unwrap();

    // Item 1: a book "What is Modern Israel?" by Yakob Rabkin
    c.execute("INSERT INTO items VALUES (1, 2, '2026-05-01 00:00:00', '2026-05-01 00:00:00', '2026-05-01 00:00:00', 1, 'JGF2UTMW', 10005, 0)", []).unwrap();
    c.execute("INSERT INTO itemDataValues VALUES (1, 'What is Modern Israel?'), (2, '2016'), (3, 'Pluto Press')", []).unwrap();
    c.execute("INSERT INTO itemData VALUES (1, 1, 1), (1, 3, 2), (1, 4, 3)", []).unwrap();
    c.execute("INSERT INTO creators VALUES (1, 'Yakob', 'Rabkin', 0)", []).unwrap();
    c.execute("INSERT INTO itemCreators VALUES (1, 1, 1, 0)", []).unwrap();

    // Item 2: a journal article with a PDF attachment that has cached full text
    c.execute("INSERT INTO items VALUES (2, 4, '2026-05-02 00:00:00', '2026-05-02 00:00:00', '2026-05-02 00:00:00', 1, 'AAAA0001', 11, 0)", []).unwrap();
    c.execute("INSERT INTO itemDataValues VALUES (10, 'A Paper on Things'), (11, '2024'), (12, '10.1234/abcd')", []).unwrap();
    c.execute("INSERT INTO itemData VALUES (2, 1, 10), (2, 3, 11), (2, 52, 12)", []).unwrap();

    // Attachment row for item 2 (item ID 3, key "AAAA0001" so storage dir matches)
    c.execute("INSERT INTO items VALUES (3, 3, '2026-05-02 00:00:00', '2026-05-02 00:00:00', '2026-05-02 00:00:00', 1, 'AAAA0001', 12, 0)", []).unwrap();
    c.execute("INSERT INTO itemAttachments VALUES (3, 2, 0, 'application/pdf', 'storage:paper.pdf', 0)", []).unwrap();

    // Item 4: a webpage item with HTML snapshot
    c.execute("INSERT INTO items VALUES (4, 14, '2026-05-03 00:00:00', '2026-05-03 00:00:00', '2026-05-03 00:00:00', 1, 'WEB00001', 5, 0)", []).unwrap();
    c.execute("INSERT INTO itemDataValues VALUES (20, 'An Article'), (21, 'https://example.com/article')", []).unwrap();
    c.execute("INSERT INTO itemData VALUES (4, 1, 20), (4, 60, 21)", []).unwrap();
    c.execute("INSERT INTO items VALUES (5, 3, '2026-05-03 00:00:00', '2026-05-03 00:00:00', '2026-05-03 00:00:00', 1, 'BBBB0002', 6, 0)", []).unwrap();
    c.execute("INSERT INTO itemAttachments VALUES (5, 4, 1, 'text/html', 'storage:article.html', 0)", []).unwrap();

    // Collection and tag
    c.execute("INSERT INTO collections VALUES (1, 'Reading List', NULL, 1, 'COL00001', 1)", []).unwrap();
    c.execute("INSERT INTO collectionItems VALUES (1, 1, 0), (1, 2, 1), (1, 4, 2)", []).unwrap();
    c.execute("INSERT INTO tags VALUES (1, 'history'), (2, 'method')", []).unwrap();
    c.execute("INSERT INTO itemTags VALUES (1, 1, 0), (2, 2, 0)", []).unwrap();

    // Full-text words for item 2
    c.execute("INSERT INTO fulltextWords VALUES (1, 'zoteroconnectortest'), (2, 'keyword'), (3, 'paper')", []).unwrap();
    c.execute("INSERT INTO fulltextItems VALUES (3, 1, 1, 50, 50, 1, 0)", []).unwrap();
    c.execute("INSERT INTO fulltextItemWords VALUES (1, 3), (2, 3), (3, 3)", []).unwrap();

    // Annotation on the PDF attachment (parentItemID = 3, attachment for item 2)
    c.execute("INSERT INTO items VALUES (6, 37, '2026-05-04 00:00:00', '2026-05-04 00:00:00', '2026-05-04 00:00:00', 1, 'ANNO0001', 1, 0)", []).unwrap();
    c.execute("INSERT INTO itemAnnotations VALUES (6, 3, 1, 'rjl', 'A highlighted passage.', 'My note on it.', '#ffff00', '12', '00012|00000', '{}', 0)", []).unwrap();
}

#[allow(dead_code)]
pub fn fixture_path_or_create() -> PathBuf {
    build().sqlite_path()
}
