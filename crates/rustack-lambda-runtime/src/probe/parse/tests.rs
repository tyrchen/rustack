use super::*;

const STATUS: &str = "Uid:\t1000 1001 1000 1000\nGid:\t1000 1002 1000 \
                      1000\nCapEff:\t0000000000000000\nNoNewPrivs:\t1\nSeccomp:\t2\n";
const MOUNTS: &str = "1 0 0:1 / / ro,relatime - overlay overlay rw\n2 1 0:2 / /tmp \
                      rw,nosuid,nodev,relatime - tmpfs tmpfs rw,size=524288k,inode64\n";
#[test]
fn test_should_parse_independent_kernel_observation_fixtures() {
    let report = status(STATUS).unwrap();
    assert_eq!((report.uid, report.gid), (1001, 1002));
    assert_eq!(report.capabilities, 0);
    assert!(report.no_new_privileges);
    assert_eq!(report.seccomp, 2);
    let (root, tmp) = mounts(MOUNTS, 0o1777).unwrap();
    assert!(root);
    assert_eq!((tmp.bytes, tmp.mode), (512 * 1024 * 1024, 0o1777));
    assert!(tmp.tmpfs && tmp.nosuid && tmp.nodev);
    assert_eq!(cpu("100000 100000\n").unwrap(), (Some(100_000), 100_000));
    assert_eq!(cpu("max 100000").unwrap(), (None, 100_000));
    assert_eq!(limit("-1").unwrap(), None);
    assert_eq!(
        limit("9223372036854771712").unwrap(),
        Some(9_223_372_036_854_771_712)
    );
}
#[rstest::rstest]
#[case::empty("")]
#[case::overflow("18446744073709551616")]
#[case::negative("-2")]
#[case::fraction("1.0")]
#[case::nonascii("١")]
#[case::multiple("1 2")]
#[test]
fn test_should_reject_invalid_kernel_numbers(#[case] value: &str) {
    assert!(number(value).is_err());
}
#[rstest::rstest]
#[case::missing("Seccomp:\t2\n", "")]
#[case::duplicate("Seccomp:\t2\n", "Seccomp:\t2\nSeccomp:\t2\n")]
#[case::short_uid("1000 1001 1000 1000", "1000 1001 1000")]
#[case::long_uid("1000 1001 1000 1000", "1000 1001 1000 1000 1000")]
#[case::uid_overflow("1000 1001 1000 1000", "1000 4294967296 1000 1000")]
#[case::bad_privilege("NoNewPrivs:\t1", "NoNewPrivs:\t2")]
#[case::bad_cap("0000000000000000", "00000000000000zz")]
#[test]
fn test_should_reject_missing_duplicate_or_invalid_process_status(
    #[case] before: &str,
    #[case] after: &str,
) {
    assert!(status(&STATUS.replace(before, after)).is_err());
}
#[rstest::rstest]
#[case::missing(
    "2 1 0:2 / /tmp rw,nosuid,nodev,relatime - tmpfs tmpfs rw,size=524288k,inode64\n",
    ""
)]
#[case::size("size=524288k", "size=18446744073709551615g")]
#[case::duplicate_size("size=524288k", "size=524288k,size=1")]
#[case::missing_separator(" - tmpfs", " tmpfs")]
#[test]
fn test_should_reject_incomplete_or_overflowing_mounts(#[case] before: &str, #[case] after: &str) {
    assert!(mounts(&MOUNTS.replace(before, after), 0o1777).is_err());
}
#[test]
fn test_should_bound_lines_and_reject_duplicate_root_or_temporary_mounts() {
    assert!(status(&"x".repeat(65537)).is_err());
    assert!(status(&format!("{STATUS}{}", "x\n".repeat(513))).is_err());
    assert!(mounts(&format!("{MOUNTS}{MOUNTS}"), 0o1777).is_err());
    assert!(cpu("1 2 3").is_err());
}

#[test]
fn test_should_require_current_controller_membership_at_namespace_root() {
    assert!(membership("0::/\n", true).unwrap());
    assert!(!membership("0::/docker/foreign\n", true).unwrap());
    let v1 = "2:memory:/\n3:pids:/\n4:cpu,cpuacct:/\n5:name=systemd:/unrelated\n";
    assert!(membership(v1, false).unwrap());
    assert!(!membership(&v1.replace("memory:/", "memory:/docker/current"), false).unwrap());
    assert!(membership(&v1.replace("2:memory:/\n", ""), false).is_err());
    assert!(membership(&format!("{v1}9:memory:/\n"), false).is_err());
    assert!(membership("0::/\n0::/\n", true).is_err());
    assert!(membership("1::/\n", true).is_err());
}
