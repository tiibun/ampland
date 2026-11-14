const WORKFLOW: &str = include_str!("../.github/workflows/build-binaries.yml");
const RELEASE_WORKFLOW: &str = include_str!("../.github/workflows/release-manifest-update.yml");

#[test]
fn release_publication_happens_after_matrix_builds_finish() {
    let (build_section, release_section) = WORKFLOW
        .split_once("\n  release:\n")
        .expect("workflow should define a dedicated release job");

    assert!(
        !build_section.contains("softprops/action-gh-release@"),
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
        release_section.contains("softprops/action-gh-release@"),
        "release job should publish the GitHub release"
    );
}

#[test]
fn release_workflow_dispatches_build_on_release_tag() {
    // Tags pushed with GITHUB_TOKEN do not trigger `on: push`, so the release
    // workflow has to start the build explicitly.
    let (triggers, _) = WORKFLOW
        .split_once("\njobs:\n")
        .expect("workflow should define jobs");
    assert!(
        triggers.contains("workflow_dispatch:"),
        "build workflow should accept workflow_dispatch"
    );
    assert!(
        RELEASE_WORKFLOW.contains("actions: write"),
        "release workflow needs actions: write to dispatch a workflow"
    );
    assert!(
        RELEASE_WORKFLOW.contains(r#"gh workflow run build-binaries.yml --ref "v${VERSION}""#),
        "release workflow should dispatch the build on the release tag"
    );
}
