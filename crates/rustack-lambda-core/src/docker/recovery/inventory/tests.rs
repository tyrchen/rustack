use std::time::Duration;

use rstest::rstest;
use serde_json::from_str;
use tokio::time::Instant;
use uuid::Uuid;

use super::{ImageTags, MAX_IMAGES, MAX_TAGS, check_tags};
use crate::docker::{DirectoryLease, DockerError};

fn lease() -> DirectoryLease {
    DirectoryLease::testing(Uuid::new_v4(), Uuid::new_v4())
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
fn row(tag: String) -> ImageTags {
    ImageTags { tags: vec![tag] }
}

#[rstest]
#[case("")]
#[case("docker.io/")]
#[case("index.docker.io/")]
#[test]
fn test_should_refuse_owner_prefix_at_any_depth_or_spelling(#[case] registry: &str) {
    let directory = lease();
    let prefix = format!(
        "{registry}rustack-internal/lambda-{}/",
        directory.owner_id().simple()
    );
    for suffix in ["run:pin", "run/deep/path:pin", "not a reference", ""] {
        assert!(matches!(
            check_tags(
                vec![row(format!("{prefix}{suffix}"))],
                &directory,
                deadline()
            ),
            Err(DockerError::Quarantined)
        ));
    }
}

#[test]
fn test_should_preserve_other_owner_registry_and_lookalike_names() {
    let directory = lease();
    let owner = directory.owner_id().simple();
    let tags = vec![
        format!(
            "rustack-internal/lambda-{}/run:pin",
            Uuid::new_v4().simple()
        ),
        format!("rustack-internal/lambda-{owner}-other/run:pin"),
        format!("registry.example/rustack-internal/lambda-{owner}/run:pin"),
        "operator/function:latest".into(),
        "<none>:<none>".into(),
    ];
    assert!(check_tags(vec![ImageTags { tags }], &directory, deadline()).is_ok());
}

#[rstest]
#[case(r#"[{"RepoTags":null}]"#)]
#[case(r#"[{"RepoTags":[]}]"#)]
#[case(r#"[{"RepoTags":["<none>:<none>"]}]"#)]
#[test]
fn test_should_accept_dangling_or_empty_tag_lists(#[case] source: &str) {
    let rows = from_str::<Vec<ImageTags>>(source).unwrap();
    assert!(check_tags(rows, &lease(), deadline()).is_ok());
}

#[test]
fn test_should_reject_missing_required_tag_field() {
    assert!(from_str::<Vec<ImageTags>>(r"[{}]").is_err());
}

#[rstest]
#[case("https://registry.example/image")]
#[case("bad reference")]
#[case("worker:v1\n")]
#[case("")]
#[test]
fn test_should_reject_malformed_foreign_metadata(#[case] tag: &str) {
    assert!(matches!(
        check_tags(vec![row(tag.into())], &lease(), deadline()),
        Err(DockerError::Protocol)
    ));
}

#[test]
fn test_should_bound_rows_total_tags_and_external_tag_bytes() {
    let directory = lease();
    let rows = || {
        (0..MAX_IMAGES)
            .map(|_| ImageTags { tags: vec![] })
            .collect::<Vec<_>>()
    };
    assert!(check_tags(rows(), &directory, deadline()).is_ok());
    let mut extra = rows();
    extra.push(ImageTags { tags: vec![] });
    assert!(matches!(
        check_tags(extra, &directory, deadline()),
        Err(DockerError::TooLarge)
    ));
    let tags = vec!["worker:v1".into(); MAX_TAGS];
    assert!(
        check_tags(
            vec![ImageTags { tags: tags.clone() }],
            &directory,
            deadline()
        )
        .is_ok()
    );
    assert!(matches!(
        check_tags(
            vec![ImageTags { tags }, row("worker:v2".into())],
            &directory,
            deadline()
        ),
        Err(DockerError::TooLarge)
    ));
    assert!(matches!(
        check_tags(vec![row("a".repeat(1025))], &directory, deadline()),
        Err(DockerError::TooLarge)
    ));
}

#[test]
fn test_should_reject_late_classification_and_redact_inventory_debug() {
    let rows = vec![row("operator/private:secret".into())];
    assert!(!format!("{rows:?}").contains("secret"));
    assert!(matches!(
        check_tags(rows, &lease(), Instant::now()),
        Err(DockerError::Deadline)
    ));
}
