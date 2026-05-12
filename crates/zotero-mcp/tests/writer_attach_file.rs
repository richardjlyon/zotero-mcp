use serde_json::json;
use std::path::PathBuf;
use wiremock::matchers::{body_partial_json, header, method, path};
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
