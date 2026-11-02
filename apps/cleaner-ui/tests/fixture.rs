mod common;

use common::UiFixture;

#[test]
fn fixture_guard_only_accepts_existing_owned_descendants() {
    let fixture = UiFixture::create();
    fixture.assert_owned_path(&fixture.tree.join("victim.bin"));
    let sibling = fixture.root.with_file_name(format!(
        "{}-sibling",
        fixture.root.file_name().unwrap().to_string_lossy()
    ));
    std::fs::create_dir(&sibling).unwrap();
    let mut rejected = vec![
        std::path::PathBuf::from(r"C:\"),
        std::env::current_dir().unwrap(),
        fixture.root.clone(),
        fixture.root.parent().unwrap().to_owned(),
        fixture.root.join("missing"),
        sibling.clone(),
    ];
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        rejected.push(profile.into());
    }
    for path in rejected {
        assert!(
            std::panic::catch_unwind(|| fixture.assert_owned_path(&path)).is_err(),
            "must reject {}",
            path.display()
        );
    }
    std::fs::remove_dir(sibling).unwrap();
}
