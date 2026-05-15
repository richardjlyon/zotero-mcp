use serde_json::json;
use std::path::PathBuf;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zotero_mcp::core::error::Error;
use zotero_mcp::core::writer::attachments::{attach_file, AttachFileOptions, AttachmentMode};
use zotero_mcp::core::writer::client::LocalApi;

fn api(server_uri: &str) -> LocalApi {
    LocalApi::new("http://unused", 93338)
        .unwrap()
        .with_web_base(server_uri)
        .with_api_key("test-key")
}

fn write_fixture(dir: &std::path::Path, name: &str, bytes: &[u8]) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

#[tokio::test]
async fn linked_file_inside_base_dir_posts_attachments_prefix_path() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("papers");
    std::fs::create_dir_all(&sub).unwrap();
    let file_path = write_fixture(&sub, "foo.pdf", b"%PDF-1.4\n");

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/users/93338/items"))
        .and(body_partial_json(json!([{
            "itemType": "attachment",
            "parentItem": "PARENT01",
            "linkMode": "linked_file",
            "path": "attachments:papers/foo.pdf",
            "contentType": "application/pdf"
        }])))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "successful": { "0": { "key": "LFK00001", "version": 3 } }
        })))
        .mount(&server)
        .await;

    let opts = AttachFileOptions {
        mode: AttachmentMode::LinkedFile,
        linked_attachment_base_dir: Some(dir.path().to_path_buf()),
        storage_dir: PathBuf::from("/unused-for-linked-mode"),
        max_attachment_bytes: 50 * 1024 * 1024,
        filename: None,
        content_type: None,
    };
    let key = attach_file(&api(&server.uri()), "PARENT01", &file_path, &opts)
        .await
        .unwrap();
    assert_eq!(key, "LFK00001");
}

#[tokio::test]
async fn linked_file_outside_base_dir_errors_without_network() {
    let dir = tempfile::tempdir().unwrap();
    let base_dir = dir.path().join("base");
    std::fs::create_dir_all(&base_dir).unwrap();
    let outside = write_fixture(dir.path(), "elsewhere.pdf", b"%PDF-1.4\n");

    // No mocks — if the tool makes a network call, the test fails the assertion below.
    let server = MockServer::start().await;

    let opts = AttachFileOptions {
        mode: AttachmentMode::LinkedFile,
        linked_attachment_base_dir: Some(base_dir.clone()),
        storage_dir: PathBuf::from("/unused-for-linked-mode"),
        max_attachment_bytes: 50 * 1024 * 1024,
        filename: None,
        content_type: None,
    };
    let err = attach_file(&api(&server.uri()), "PARENT01", &outside, &opts)
        .await
        .unwrap_err();
    match err {
        Error::AttachmentOutsideBaseDir {
            file_path,
            base_dir: b,
        } => {
            assert_eq!(file_path, outside);
            assert_eq!(b, base_dir);
        }
        other => panic!("expected AttachmentOutsideBaseDir, got {:?}", other),
    }
}

#[tokio::test]
async fn linked_file_without_base_dir_uses_absolute_path() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = write_fixture(dir.path(), "x.pdf", b"%PDF-1.4\n");
    let abs = file_path.to_string_lossy().into_owned();

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/users/93338/items"))
        .and(body_partial_json(json!([{
            "linkMode": "linked_file",
            "path": abs,
        }])))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "successful": { "0": { "key": "ABS00001", "version": 1 } }
        })))
        .mount(&server)
        .await;

    let opts = AttachFileOptions {
        mode: AttachmentMode::LinkedFile,
        linked_attachment_base_dir: None,
        storage_dir: PathBuf::from("/unused-for-linked-mode"),
        max_attachment_bytes: 50 * 1024 * 1024,
        filename: None,
        content_type: None,
    };
    let key = attach_file(&api(&server.uri()), "PARENT01", &file_path, &opts)
        .await
        .unwrap();
    assert_eq!(key, "ABS00001");
}

#[tokio::test]
async fn attach_file_returns_not_found_for_missing_path() {
    let server = MockServer::start().await;
    let opts = AttachFileOptions {
        mode: AttachmentMode::LinkedFile,
        linked_attachment_base_dir: None,
        storage_dir: PathBuf::from("/unused-for-linked-mode"),
        max_attachment_bytes: 50 * 1024 * 1024,
        filename: None,
        content_type: None,
    };
    let err = attach_file(
        &api(&server.uri()),
        "PARENT01",
        std::path::Path::new("/nonexistent/path.pdf"),
        &opts,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, Error::AttachmentFileNotFound(_)));
}

#[tokio::test]
async fn attach_file_returns_too_large_when_over_limit() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = write_fixture(dir.path(), "big.pdf", &vec![0u8; 200]);

    let server = MockServer::start().await;
    let opts = AttachFileOptions {
        mode: AttachmentMode::LinkedFile,
        linked_attachment_base_dir: None,
        storage_dir: PathBuf::from("/unused-for-linked-mode"),
        max_attachment_bytes: 100, // tiny ceiling to force the check
        filename: None,
        content_type: None,
    };
    let err = attach_file(&api(&server.uri()), "PARENT01", &file_path, &opts)
        .await
        .unwrap_err();
    match err {
        Error::AttachmentTooLarge {
            file_path: p,
            limit,
        } => {
            assert_eq!(p, file_path);
            assert_eq!(limit, 100);
        }
        other => panic!("expected AttachmentTooLarge, got {:?}", other),
    }
}

const HELLO_PDF: &[u8] = include_bytes!("fixtures/hello.pdf");

fn write_hello(dir: &std::path::Path) -> PathBuf {
    let p = dir.join("hello.pdf");
    std::fs::write(&p, HELLO_PDF).unwrap();
    p
}

// `attach_file(mode=imported_file)` creates the attachment row WITHOUT
// md5/mtime (those are populated by Zotero after it uploads to the
// configured remote backend; setting them at row-creation time marks the
// row as syncState=IN_SYNC and prevents Zotero from queuing the upload),
// then drops the bytes into <storage_dir>/<attach_key>/<filename>. Zotero
// desktop's sync engine picks up the file from there and pushes it to
// whichever backend the user has configured (cloud / WebDAV / none).

#[tokio::test]
async fn imported_file_creates_row_without_md5_and_writes_bytes_to_storage() {
    let src_dir = tempfile::tempdir().unwrap();
    let storage_dir = tempfile::tempdir().unwrap();
    let file_path = write_hello(src_dir.path());

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/users/93338/items"))
        .and(body_partial_json(json!([{
            "itemType": "attachment",
            "parentItem": "PARENT01",
            "linkMode": "imported_file",
            "filename": "hello.pdf",
            "contentType": "application/pdf",
        }])))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "successful": { "0": { "key": "ATT00001", "version": 1 } }
        })))
        .mount(&server)
        .await;

    let opts = AttachFileOptions {
        mode: AttachmentMode::ImportedFile,
        linked_attachment_base_dir: None,
        storage_dir: storage_dir.path().to_path_buf(),
        max_attachment_bytes: 50 * 1024 * 1024,
        filename: None,
        content_type: None,
    };
    let key = attach_file(&api(&server.uri()), "PARENT01", &file_path, &opts)
        .await
        .unwrap();
    assert_eq!(key, "ATT00001");

    // Bytes land at <storage_dir>/<key>/<filename>, byte-identical to source.
    let written = storage_dir.path().join("ATT00001").join("hello.pdf");
    assert!(written.exists(), "expected bytes at {}", written.display());
    assert_eq!(std::fs::read(&written).unwrap(), HELLO_PDF);

    // Critical: md5 and mtime MUST NOT appear in the row body. Their
    // presence makes Zotero mark the row syncState=IN_SYNC, so the desktop
    // client never queues an upload to the configured remote backend
    // (regression guard for the bug recovery hit on 2026-05-15).
    let reqs = server.received_requests().await.unwrap();
    let row_body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    let obj = row_body[0]
        .as_object()
        .expect("body is array of one object");
    assert!(
        !obj.contains_key("md5"),
        "imported_file row body must not include md5; got {row_body:#}"
    );
    assert!(
        !obj.contains_key("mtime"),
        "imported_file row body must not include mtime; got {row_body:#}"
    );
    assert!(
        !obj.contains_key("storageHash"),
        "imported_file row body must not include storageHash; got {row_body:#}"
    );
}

#[tokio::test]
async fn imported_file_filename_override_used_for_both_row_and_storage() {
    let src_dir = tempfile::tempdir().unwrap();
    let storage_dir = tempfile::tempdir().unwrap();
    let file_path = write_hello(src_dir.path());

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/users/93338/items"))
        .and(body_partial_json(json!([{
            "filename": "renamed.pdf",
        }])))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "successful": { "0": { "key": "ATT00002", "version": 1 } }
        })))
        .mount(&server)
        .await;

    let opts = AttachFileOptions {
        mode: AttachmentMode::ImportedFile,
        linked_attachment_base_dir: None,
        storage_dir: storage_dir.path().to_path_buf(),
        max_attachment_bytes: 50 * 1024 * 1024,
        filename: Some("renamed.pdf".into()),
        content_type: None,
    };
    let key = attach_file(&api(&server.uri()), "PARENT01", &file_path, &opts)
        .await
        .unwrap();
    assert_eq!(key, "ATT00002");

    // Override propagates to the on-disk name so it matches the row.
    let written = storage_dir.path().join("ATT00002").join("renamed.pdf");
    assert!(written.exists());
}

#[tokio::test]
async fn imported_file_storage_write_failure_maps_to_upload_failed() {
    let src_dir = tempfile::tempdir().unwrap();
    let file_path = write_hello(src_dir.path());

    // Construct an unmakeable storage_dir: a child of a regular file.
    // create_dir_all will fail with NotADirectory.
    let blocker_dir = tempfile::tempdir().unwrap();
    let blocker_file = blocker_dir.path().join("file-not-a-dir");
    std::fs::write(&blocker_file, b"blocker").unwrap();
    let bad_storage = blocker_file.join("storage");

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/users/93338/items"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "successful": { "0": { "key": "ATT00003", "version": 1 } }
        })))
        .mount(&server)
        .await;

    let opts = AttachFileOptions {
        mode: AttachmentMode::ImportedFile,
        linked_attachment_base_dir: None,
        storage_dir: bad_storage,
        max_attachment_bytes: 50 * 1024 * 1024,
        filename: None,
        content_type: None,
    };
    let err = attach_file(&api(&server.uri()), "PARENT01", &file_path, &opts)
        .await
        .unwrap_err();
    match err {
        Error::UploadFailed { stage, .. } => {
            assert!(
                stage == "create_storage_dir" || stage == "write_bytes",
                "expected local-write stage, got {stage}"
            );
        }
        other => panic!("expected UploadFailed, got {:?}", other),
    }
}
