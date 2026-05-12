use serde_json::json;
use std::path::PathBuf;
use wiremock::matchers::{body_partial_json, body_string_contains, header, method, path};
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
        max_attachment_bytes: 50 * 1024 * 1024,
        filename: None,
        content_type: None,
    };
    let err = attach_file(&api(&server.uri()), "PARENT01", &outside, &opts)
        .await
        .unwrap_err();
    match err {
        Error::AttachmentOutsideBaseDir { file_path, base_dir: b } => {
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
        max_attachment_bytes: 100, // tiny ceiling to force the check
        filename: None,
        content_type: None,
    };
    let err = attach_file(&api(&server.uri()), "PARENT01", &file_path, &opts)
        .await
        .unwrap_err();
    match err {
        Error::AttachmentTooLarge { file_path: p, limit } => {
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

fn md5_hex(bytes: &[u8]) -> String {
    use md5::{Digest, Md5};
    let mut h = Md5::new();
    h.update(bytes);
    let digest = h.finalize();
    let mut s = String::with_capacity(32);
    for b in digest {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

#[tokio::test]
async fn imported_file_md5_exists_short_circuits_upload() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = write_hello(dir.path());
    let md5 = md5_hex(HELLO_PDF);

    let server = MockServer::start().await;

    // Step 5.1a: create attachment item. md5/mtime are server-managed for
    // imported_file mode — they MUST NOT be in this body; sending them makes
    // Zotero treat the row as already-linked and the authorize step 412s.
    Mock::given(method("POST"))
        .and(path("/users/93338/items"))
        .and(body_partial_json(json!([{
            "itemType": "attachment",
            "parentItem": "PARENT01",
            "linkMode": "imported_file",
            "filename": "hello.pdf",
            "contentType": "application/pdf"
        }])))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "successful": { "0": { "key": "ATT00001", "version": 1 } }
        })))
        .mount(&server)
        .await;

    // Step 5.1b: authorize -> exists:1 short-circuit.
    Mock::given(method("POST"))
        .and(path("/users/93338/items/ATT00001/file"))
        .and(header("If-None-Match", "*"))
        .and(body_string_contains(format!("md5={md5}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "exists": 1 })))
        .mount(&server)
        .await;

    // No step-5.1c PUT/register mocks — they must not be invoked.

    let opts = AttachFileOptions {
        mode: AttachmentMode::ImportedFile,
        linked_attachment_base_dir: None,
        max_attachment_bytes: 50 * 1024 * 1024,
        filename: None,
        content_type: None,
    };
    let key = attach_file(&api(&server.uri()), "PARENT01", &file_path, &opts)
        .await
        .unwrap();
    assert_eq!(key, "ATT00001");
}

#[tokio::test]
async fn imported_file_full_three_step_upload_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = write_hello(dir.path());
    let md5 = md5_hex(HELLO_PDF);

    let server = MockServer::start().await;
    let s3 = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/users/93338/items"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "successful": { "0": { "key": "ATT00002", "version": 1 } }
        })))
        .mount(&server)
        .await;

    // Step 5.1b: authorize -> returns upload URL pointing at the s3 mock.
    let upload_url = format!("{}/upload", s3.uri());
    Mock::given(method("POST"))
        .and(path("/users/93338/items/ATT00002/file"))
        .and(body_string_contains(format!("md5={md5}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "url": upload_url,
            "contentType": "application/octet-stream",
            "prefix": "PFX",
            "suffix": "SFX",
            "uploadKey": "UPLOADKEY"
        })))
        .mount(&server)
        .await;

    // Step 5.1c.upload: POST prefix + file_bytes + suffix (multipart/form-data
    // framing supplied by the authorize step).
    let mut expected_body = b"PFX".to_vec();
    expected_body.extend_from_slice(HELLO_PDF);
    expected_body.extend_from_slice(b"SFX");
    Mock::given(method("POST"))
        .and(path("/upload"))
        .and(header("Content-Type", "application/octet-stream"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s3)
        .await;

    // Step 5.1c.register: POST with form body `upload=<key>` (NOT a query param).
    Mock::given(method("POST"))
        .and(path("/users/93338/items/ATT00002/file"))
        .and(body_string_contains("upload=UPLOADKEY"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let opts = AttachFileOptions {
        mode: AttachmentMode::ImportedFile,
        linked_attachment_base_dir: None,
        max_attachment_bytes: 50 * 1024 * 1024,
        filename: None,
        content_type: None,
    };
    let key = attach_file(&api(&server.uri()), "PARENT01", &file_path, &opts)
        .await
        .unwrap();
    assert_eq!(key, "ATT00002");
}

#[tokio::test]
async fn imported_file_s3_put_failure_maps_to_upload_failed_stage_s3_put() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = write_hello(dir.path());

    let server = MockServer::start().await;
    let s3 = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/users/93338/items"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "successful": { "0": { "key": "ATT00003", "version": 1 } }
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/users/93338/items/ATT00003/file"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "url": format!("{}/upload", s3.uri()),
            "contentType": "application/octet-stream",
            "prefix": "",
            "suffix": "",
            "uploadKey": "K"
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/upload"))
        .respond_with(ResponseTemplate::new(500).set_body_string("S3 boom"))
        .mount(&s3)
        .await;

    let opts = AttachFileOptions {
        mode: AttachmentMode::ImportedFile,
        linked_attachment_base_dir: None,
        max_attachment_bytes: 50 * 1024 * 1024,
        filename: None,
        content_type: None,
    };
    let err = attach_file(&api(&server.uri()), "PARENT01", &file_path, &opts)
        .await
        .unwrap_err();
    match err {
        Error::UploadFailed { stage, detail } => {
            assert_eq!(stage, "s3_put");
            assert!(detail.contains("500") || detail.contains("S3 boom"));
        }
        other => panic!("expected UploadFailed(stage=s3_put), got {:?}", other),
    }
}
