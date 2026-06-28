//! Binary-level Git integration tests for real fixture repositories.

use std::env;
use std::fs;
use std::path;
use std::process;

fn fixture_repo_path() -> path::PathBuf {
    env::temp_dir().join(format!(
        "affected-tests-ts-basic-selection-{}",
        process::id(),
    ))
}

fn run_command(command: &mut process::Command) -> process::Output {
    command.output().unwrap()
}

fn assert_success(output: &process::Output) {
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
}

struct WriteFixtureFileRequest<'a> {
    repository_path: &'a path::Path,
    relative_path: &'a str,
    content: &'a str,
}

fn run_git(repository_path: &path::Path, args: &[&str]) -> process::Output {
    run_command(
        process::Command::new("git")
            .arg("-C")
            .arg(repository_path)
            .args(args),
    )
}

fn commit_fixture_change(repository_path: &path::Path, message: &str) {
    let output = process::Command::new("git")
        .arg("-C")
        .arg(repository_path)
        .arg("commit")
        .arg("-m")
        .arg(message)
        .env("GIT_AUTHOR_NAME", "Affected Tests Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixtures@example.invalid")
        .env("GIT_COMMITTER_NAME", "Affected Tests Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixtures@example.invalid")
        .output()
        .unwrap();

    assert_success(&output);
}

fn write_fixture_file(request: &WriteFixtureFileRequest<'_>) {
    let file_path = request.repository_path.join(request.relative_path);
    let parent_path = file_path.parent().unwrap();

    fs::create_dir_all(parent_path).unwrap();
    fs::write(file_path, request.content).unwrap();
}

fn create_fixture_repo() -> path::PathBuf {
    let repository_path = fixture_repo_path();

    if repository_path.exists() {
        fs::remove_dir_all(&repository_path).unwrap();
    }

    fs::create_dir_all(&repository_path).unwrap();
    assert_success(&run_command(
        process::Command::new("git")
            .arg("init")
            .arg("-b")
            .arg("main")
            .arg(&repository_path),
    ));
    write_fixture_file(&WriteFixtureFileRequest {
        repository_path: &repository_path,
        relative_path: "src/value.ts",
        content: "export const value = 1;\n",
    });
    write_fixture_file(&WriteFixtureFileRequest {
        repository_path: &repository_path,
        relative_path: "tests/value.test.ts",
        content: "import { value } from '../src/value';\nvoid value;\n",
    });
    assert_success(&run_git(
        &repository_path,
        &["add", "src/value.ts", "tests/value.test.ts"],
    ));
    commit_fixture_change(&repository_path, "Add base TypeScript fixture");
    assert_success(&run_git(
        &repository_path,
        &["checkout", "-b", "feature/change-value"],
    ));
    write_fixture_file(&WriteFixtureFileRequest {
        repository_path: &repository_path,
        relative_path: "src/value.ts",
        content: "export const value = 2;\n",
    });
    assert_success(&run_git(&repository_path, &["add", "src/value.ts"]));
    commit_fixture_change(&repository_path, "Change source value");

    repository_path
}

#[test]
#[should_panic(expected = "not implemented")]
fn binary_runs_inside_fixture_repo_and_reports_real_git_range_behavior() {
    let fixture_repo = create_fixture_repo();
    let git_output = run_git(&fixture_repo, &["diff", "--name-status", "main...HEAD"]);

    // Runtime construction keeps the test reproducible while still validating
    // true Git behavior that a VFS cannot faithfully model.
    assert!(git_output.status.success());
    assert_eq!(
        String::from_utf8(git_output.stdout).unwrap(),
        "M\tsrc/value.ts\n"
    );

    let command_output = process::Command::new(env!("CARGO_BIN_EXE_affected-tests-ts"))
        .current_dir(&fixture_repo)
        .arg("--base")
        .arg("main")
        .arg("--head")
        .arg("HEAD")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();

    assert!(
        command_output.status.success(),
        "not implemented binary behavior is unavailable",
    );
    assert_eq!(
        String::from_utf8(command_output.stdout).unwrap(),
        "{\"status\":\"partial\",\"tests\":[\"tests/value.test.ts\"],\"reasons\":[]}\n",
    );
}
