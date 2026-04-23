const WORKFLOW: &str = include_str!("../.github/workflows/build-binaries.yml");

#[test]
fn release_publication_happens_after_matrix_builds_finish() {
    let (build_section, release_section) = WORKFLOW
        .split_once("\n  release:\n")
        .expect("workflow should define a dedicated release job");

    assert!(
        !build_section.contains("softprops/action-gh-release@v2"),
        "matrix build job should only produce artifacts"
    );
    assert!(
        release_section.contains("needs: build"),
        "release job should wait for the build matrix"
    );
    assert!(
        release_section.contains("actions/download-artifact@"),
        "release job should gather artifacts from completed build jobs"
    );
    assert!(
        release_section.contains("softprops/action-gh-release@v2"),
        "release job should publish the GitHub release"
    );
}
