use domain::normalize_path;
use policy::{EnterprisePolicy, enterprise_wins, is_forced_forbidden};

#[test]
fn managed_exclusions_replace_local_exclusions() {
    let local = EnterprisePolicy {
        exclusions: vec!["local".into()],
        ..Default::default()
    };
    let managed = EnterprisePolicy {
        exclusions: vec!["managed".into()],
        ..Default::default()
    };
    assert_eq!(
        enterprise_wins(&local, &managed).exclusions,
        vec!["managed"]
    );
}

#[test]
fn forced_forbidden_paths_block_matching_children() {
    let policy = EnterprisePolicy {
        forced_forbidden: vec![r"C:\Users\Alice\AppData\Local\Cache".into()],
        ..Default::default()
    };
    let child = normalize_path(r"C:\Users\Alice\AppData\Local\Cache\item.tmp").unwrap();
    let sibling = normalize_path(r"C:\Users\Alice\AppData\Local\Caches\item.tmp").unwrap();

    assert!(is_forced_forbidden(&policy, &child));
    assert!(!is_forced_forbidden(&policy, &sibling));
}
