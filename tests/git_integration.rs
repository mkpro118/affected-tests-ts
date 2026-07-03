//! Binary-level Git integration tests for real fixture repositories.

use std::env;
use std::fs;
use std::path;
use std::process;
use std::sync::atomic;

const BLUEPRINT_CONFIG: &str = include_str!("../fixtures/repos/blueprint/affected-tests.json");
const BLUEPRINT_SOURCE: &str = include_str!("../fixtures/repos/blueprint/src/value.ts");
const BLUEPRINT_TEST: &str = include_str!("../fixtures/repos/blueprint/tests/value.test.ts");
const MONOREPO_TSCONFIG: &str = r#"{
  "include": ["./src", "./tests"],
  "exclude": ["./build"]
}
"#;
const RANGE_ARGS: &[&str] = &["--base", "origin/main", "--head", "HEAD"];
const JSON_EXPLAIN_ARGS: &[&str] = &[
    "--base",
    "origin/main",
    "--head",
    "HEAD",
    "--format",
    "json",
    "--explain",
];
const PLAIN_EXPLAIN_ARGS: &[&str] = &[
    "--base",
    "origin/main",
    "--head",
    "HEAD",
    "--format",
    "plain",
    "--explain",
];
const DOCKER_EXPLAIN_ARGS: &[&str] = &[
    "--base",
    "origin/main",
    "--head",
    "HEAD",
    "--format",
    "docker",
    "--explain",
];
const TUI_EXPLAIN_ARGS: &[&str] = &[
    "--base",
    "origin/main",
    "--head",
    "HEAD",
    "--format",
    "tui",
    "--explain",
];
const GRAPH_JSON_ARGS: &[&str] = &["graph", "--format", "json"];
const EXPLAIN_WITH_GLOBAL_RANGE_ARGS: &[&str] = &[
    "explain",
    "src/value.ts",
    "--base",
    "origin/main",
    "--head",
    "HEAD",
];
const VERSION_ARGS: &[&str] = &["--version"];

static NEXT_REPOSITORY_ID: atomic::AtomicUsize = atomic::AtomicUsize::new(0);

fn fixture_repo_path(name: &str) -> path::PathBuf {
    let repository_id = NEXT_REPOSITORY_ID.fetch_add(1, atomic::Ordering::Relaxed);

    env::temp_dir().join(format!(
        "affected-tests-ts-{name}-{}-{repository_id}",
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

#[derive(Clone, Copy)]
struct CommandRequest<'a> {
    repository_path: &'a path::Path,
    args: &'a [&'a str],
}

struct MonorepoCommandOutputs {
    shell: process::Output,
    json: process::Output,
    plain: process::Output,
    docker: process::Output,
    tui: process::Output,
    graph: process::Output,
    explain: process::Output,
    version: process::Output,
}

fn run_git(repository_path: &path::Path, args: &[&str]) -> process::Output {
    run_command(
        process::Command::new("git")
            .arg("-C")
            .arg(repository_path)
            .args(args),
    )
}

fn assert_git_success(output: &process::Output) {
    assert_success(output);
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
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
        .env("GIT_CONFIG_VALUE_0", "false")
        .output()
        .unwrap();

    assert_success(&output);
}

fn materialize_blueprint(repository_path: &path::Path) {
    write_fixture_file(&WriteFixtureFileRequest {
        repository_path,
        relative_path: "affected-tests.json",
        content: BLUEPRINT_CONFIG,
    });
    write_fixture_file(&WriteFixtureFileRequest {
        repository_path,
        relative_path: "src/value.ts",
        content: BLUEPRINT_SOURCE,
    });
    write_fixture_file(&WriteFixtureFileRequest {
        repository_path,
        relative_path: "tests/value.test.ts",
        content: BLUEPRINT_TEST,
    });
}

fn materialize_monorepo_workspace(repository_path: &path::Path) {
    write_fixture_file(&WriteFixtureFileRequest {
        repository_path,
        relative_path: "apps/web/tsconfig.json",
        content: MONOREPO_TSCONFIG,
    });
    write_fixture_file(&WriteFixtureFileRequest {
        repository_path,
        relative_path: "apps/web/src/value.ts",
        content: BLUEPRINT_SOURCE,
    });
    write_fixture_file(&WriteFixtureFileRequest {
        repository_path,
        relative_path: "apps/web/tests/value.test.ts",
        content: BLUEPRINT_TEST,
    });
}

fn write_fixture_file(request: &WriteFixtureFileRequest<'_>) {
    let file_path = request.repository_path.join(request.relative_path);
    let parent_path = file_path.parent().unwrap();

    fs::create_dir_all(parent_path).unwrap();
    fs::write(file_path, request.content).unwrap();
}

fn create_fixture_repo(name: &str) -> path::PathBuf {
    let repository_path = fixture_repo_path(name);

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
    materialize_blueprint(&repository_path);
    assert_git_success(&run_git(
        &repository_path,
        &[
            "add",
            "affected-tests.json",
            "src/value.ts",
            "tests/value.test.ts",
        ],
    ));
    commit_fixture_change(&repository_path, "Add base TypeScript fixture");
    assert_git_success(&run_git(
        &repository_path,
        &["update-ref", "refs/remotes/origin/main", "HEAD"],
    ));
    assert_git_success(&run_git(
        &repository_path,
        &["checkout", "-b", "feature/change-value"],
    ));

    repository_path
}

fn create_changed_source_repo(name: &str) -> path::PathBuf {
    let repository_path = create_fixture_repo(name);

    write_fixture_file(&WriteFixtureFileRequest {
        repository_path: &repository_path,
        relative_path: "src/value.ts",
        content: "export const value = 2;\n",
    });
    assert_git_success(&run_git(&repository_path, &["add", "src/value.ts"]));
    commit_fixture_change(&repository_path, "Change source value");

    repository_path
}

fn create_changed_monorepo_workspace(name: &str) -> path::PathBuf {
    let repository_path = fixture_repo_path(name);

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
    materialize_monorepo_workspace(&repository_path);
    assert_git_success(&run_git(
        &repository_path,
        &[
            "add",
            "apps/web/tsconfig.json",
            "apps/web/src/value.ts",
            "apps/web/tests/value.test.ts",
        ],
    ));
    commit_fixture_change(&repository_path, "Add monorepo TypeScript workspace");
    assert_git_success(&run_git(
        &repository_path,
        &["update-ref", "refs/remotes/origin/main", "HEAD"],
    ));
    assert_git_success(&run_git(
        &repository_path,
        &["checkout", "-b", "feature/change-web-value"],
    ));
    write_fixture_file(&WriteFixtureFileRequest {
        repository_path: &repository_path,
        relative_path: "apps/web/src/value.ts",
        content: "export const value = 3;\n",
    });
    assert_git_success(&run_git(
        &repository_path,
        &["add", "apps/web/src/value.ts"],
    ));
    commit_fixture_change(&repository_path, "Change web source value");

    repository_path.join("apps/web")
}

fn create_changed_subdir_package_dependency(name: &str) -> path::PathBuf {
    let repository_path = fixture_repo_path(name);

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
        relative_path: "apps/web/tsconfig.json",
        content: MONOREPO_TSCONFIG,
    });
    write_fixture_file(&WriteFixtureFileRequest {
        repository_path: &repository_path,
        relative_path: "apps/web/package.json",
        content: "{\"name\":\"web\",\"dependencies\":{\"lodash\":\"1.0.0\"}}\n",
    });
    write_fixture_file(&WriteFixtureFileRequest {
        repository_path: &repository_path,
        relative_path: "apps/web/src/uses-lodash.ts",
        content: "import _ from 'lodash';\nexport const value = _;\n",
    });
    write_fixture_file(&WriteFixtureFileRequest {
        repository_path: &repository_path,
        relative_path: "apps/web/tests/uses-lodash.test.ts",
        content: "import { value } from '../src/uses-lodash';\nvoid value;\n",
    });
    assert_git_success(&run_git(
        &repository_path,
        &[
            "add",
            "apps/web/tsconfig.json",
            "apps/web/package.json",
            "apps/web/src/uses-lodash.ts",
            "apps/web/tests/uses-lodash.test.ts",
        ],
    ));
    commit_fixture_change(&repository_path, "Add web workspace with lodash dependency");
    assert_git_success(&run_git(
        &repository_path,
        &["update-ref", "refs/remotes/origin/main", "HEAD"],
    ));
    assert_git_success(&run_git(
        &repository_path,
        &["checkout", "-b", "feature/bump-lodash"],
    ));
    write_fixture_file(&WriteFixtureFileRequest {
        repository_path: &repository_path,
        relative_path: "apps/web/package.json",
        content: "{\"name\":\"web\",\"dependencies\":{\"lodash\":\"2.0.0\"}}\n",
    });
    assert_git_success(&run_git(&repository_path, &["add", "apps/web/package.json"]));
    commit_fixture_change(&repository_path, "Bump lodash dependency");

    repository_path.join("apps/web")
}

fn run_affected_tests(request: CommandRequest<'_>) -> process::Output {
    process::Command::new(env!("CARGO_BIN_EXE_affected-tests-ts"))
        .current_dir(request.repository_path)
        .args(request.args)
        .output()
        .unwrap()
}

fn stdout(output: &process::Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn assert_workspace_relative_output(output: &process::Output) {
    let content = stdout(output);

    assert!(content.contains("tests/value.test.ts"));
    assert!(!content.contains("apps/web/tests/value.test.ts"));
    assert!(!content.contains("apps/web/src/value.ts"));
}

fn run_workspace_command(workspace_path: &path::Path, args: &[&str]) -> process::Output {
    run_affected_tests(CommandRequest {
        repository_path: workspace_path,
        args,
    })
}

fn run_monorepo_workspace_commands(workspace_path: &path::Path) -> MonorepoCommandOutputs {
    MonorepoCommandOutputs {
        shell: run_workspace_command(workspace_path, RANGE_ARGS),
        json: run_workspace_command(workspace_path, JSON_EXPLAIN_ARGS),
        plain: run_workspace_command(workspace_path, PLAIN_EXPLAIN_ARGS),
        docker: run_workspace_command(workspace_path, DOCKER_EXPLAIN_ARGS),
        tui: run_workspace_command(workspace_path, TUI_EXPLAIN_ARGS),
        graph: run_workspace_command(workspace_path, GRAPH_JSON_ARGS),
        explain: run_workspace_command(workspace_path, EXPLAIN_WITH_GLOBAL_RANGE_ARGS),
        version: run_workspace_command(workspace_path, VERSION_ARGS),
    }
}

fn assert_monorepo_commands_succeeded(outputs: &MonorepoCommandOutputs) {
    assert_success(&outputs.shell);
    assert_success(&outputs.json);
    assert_success(&outputs.plain);
    assert_success(&outputs.docker);
    assert_success(&outputs.tui);
    assert_success(&outputs.graph);
    assert_success(&outputs.explain);
    assert_success(&outputs.version);
}

fn assert_monorepo_selection_outputs(outputs: &MonorepoCommandOutputs) {
    assert_eq!(stdout(&outputs.shell), "tests/value.test.ts\n");
    assert!(stdout(&outputs.json).contains(r#""changedFile":"src/value.ts""#));
    assert_workspace_relative_output(&outputs.json);
    assert_workspace_relative_output(&outputs.plain);
    assert_workspace_relative_output(&outputs.tui);
    assert!(stdout(&outputs.docker).contains("partial: 1 affected test"));
    assert!(!stdout(&outputs.docker).contains("apps/web/"));
}

fn assert_monorepo_graph_output(output: &process::Output) {
    let content = stdout(output);

    assert!(content.contains(r#""path":"src/value.ts""#));
    assert!(content.contains(r#""path":"tests/value.test.ts""#));
    assert!(content.contains(r#""from":"tests/value.test.ts""#));
    assert!(!content.contains(r#""nodes":[],"edges":[]"#));
}

fn assert_monorepo_metadata_outputs(outputs: &MonorepoCommandOutputs) {
    assert_eq!(stdout(&outputs.explain), "tests/value.test.ts\n");
    assert!(stdout(&outputs.version).contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn binary_runs_inside_fixture_repo_and_reports_real_git_range_behavior() {
    let fixture_repo = create_changed_source_repo("basic-selection");
    let git_output = run_git(
        &fixture_repo,
        &["diff", "--name-status", "origin/main...HEAD"],
    );

    // Runtime construction keeps the test reproducible while still validating
    // true Git behavior that a VFS cannot faithfully model.
    assert!(git_output.status.success());
    assert_eq!(
        String::from_utf8(git_output.stdout).unwrap(),
        "M\tsrc/value.ts\n"
    );

    let command_output = run_affected_tests(CommandRequest {
        repository_path: &fixture_repo,
        args: &[
            "--base",
            "origin/main",
            "--head",
            "HEAD",
            "--format",
            "json",
            "--explain",
        ],
    });

    assert!(
        command_output.status.success(),
        "binary stderr: {}",
        String::from_utf8_lossy(&command_output.stderr),
    );
    let output = stdout(&command_output);

    assert!(output.contains(r#""status":"partial""#));
    assert!(output.contains(r#""tests":["tests/value.test.ts"]"#));
    assert!(output.contains(r#""changedFile":"src/value.ts""#));
    assert!(output.contains(r#""testFile":"tests/value.test.ts""#));
}

#[test]
fn default_shell_tui_docker_graph_and_explain_commands_are_wired() {
    let fixture_repo = create_changed_source_repo("command-formats");
    let shell_output = run_affected_tests(CommandRequest {
        repository_path: &fixture_repo,
        args: &["--base", "origin/main", "--head", "HEAD"],
    });
    let tui_output = run_affected_tests(CommandRequest {
        repository_path: &fixture_repo,
        args: &["--base", "origin/main", "--head", "HEAD", "--format", "tui"],
    });
    let docker_output = run_affected_tests(CommandRequest {
        repository_path: &fixture_repo,
        args: &[
            "--base",
            "origin/main",
            "--head",
            "HEAD",
            "--format",
            "docker",
        ],
    });
    let graph_output = run_affected_tests(CommandRequest {
        repository_path: &fixture_repo,
        args: &["graph", "--format", "json"],
    });
    let explain_output = run_affected_tests(CommandRequest {
        repository_path: &fixture_repo,
        args: &["explain", "src/value.ts"],
    });

    // The blueprint files are materialized into real repositories so these
    // command forms validate process, Git, and filesystem boundaries together.
    assert_success(&shell_output);
    assert_success(&tui_output);
    assert_success(&docker_output);
    assert_success(&graph_output);
    assert_success(&explain_output);
    assert_eq!(stdout(&shell_output), "tests/value.test.ts\n");
    assert_eq!(stdout(&tui_output), "partial\ntests/value.test.ts\n");
    assert!(stdout(&docker_output).contains("=> [result"));
    assert!(stdout(&graph_output).contains(r#""nodes""#));
    assert_eq!(stdout(&explain_output), "tests/value.test.ts\n");
}

#[test]
fn binary_run_from_monorepo_workspace_emits_workspace_relative_paths() {
    let workspace_path = create_changed_monorepo_workspace("monorepo-workspace");
    let outputs = run_monorepo_workspace_commands(&workspace_path);

    // The binary is invoked from apps/web even though Git metadata lives above
    // it, matching a real TypeScript package inside a larger monorepo.
    assert_monorepo_commands_succeeded(&outputs);
    assert_monorepo_selection_outputs(&outputs);
    assert_monorepo_graph_output(&outputs.graph);
    assert_monorepo_metadata_outputs(&outputs);
}

#[test]
fn worktree_mode_includes_uncommitted_changes() {
    let fixture_repo = create_fixture_repo("worktree-dirty");
    write_fixture_file(&WriteFixtureFileRequest {
        repository_path: &fixture_repo,
        relative_path: "src/value.ts",
        content: "export const value = 42;\n",
    });

    let default_output = run_affected_tests(CommandRequest {
        repository_path: &fixture_repo,
        args: &["--base", "origin/main", "--head", "HEAD"],
    });
    let worktree_output = run_affected_tests(CommandRequest {
        repository_path: &fixture_repo,
        args: &["--base", "origin/main", "--head", "HEAD", "--worktree"],
    });

    // The default command stays revision-based. --worktree explicitly adds the
    // dirty tracked file diff against HEAD.
    assert_success(&default_output);
    assert_success(&worktree_output);
    assert_eq!(stdout(&default_output), "");
    assert_eq!(stdout(&worktree_output), "tests/value.test.ts\n");
}

#[test]
#[should_panic(expected = "subdirectory package dependency change must scope to importer tests")]
fn package_scoping_engages_when_run_from_a_subdirectory_workspace() {
    let workspace_path = create_changed_subdir_package_dependency("subdir-scope");
    let output = run_affected_tests(CommandRequest {
        repository_path: &workspace_path,
        args: &[
            "--base",
            "origin/main",
            "--head",
            "HEAD",
            "--format",
            "json",
        ],
    });

    // The binary succeeds either way; the flaw is which decision it reaches.
    assert_success(&output);
    let content = stdout(&output);

    // Desired: a dependency-only package.json change scopes to the importing
    // file's test, exactly as it would from the repository root. Because
    // `git show <rev>:package.json` is resolved at the repo root (not the
    // apps/web cwd), scoping cannot read the file, falls back to the global
    // invalidator, and reports "full" instead.
    assert!(
        content.contains(r#""status":"partial""#),
        "subdirectory package dependency change must scope to importer tests, got: {content}",
    );
}

#[test]
fn binary_handles_real_git_renames_deletes_and_global_invalidators() {
    let rename_repo = create_fixture_repo("rename");
    assert_git_success(&run_git(
        &rename_repo,
        &["mv", "src/value.ts", "src/renamed.ts"],
    ));
    write_fixture_file(&WriteFixtureFileRequest {
        repository_path: &rename_repo,
        relative_path: "tests/value.test.ts",
        content: "import { value } from '../src/renamed';\nvoid value;\n",
    });
    assert_git_success(&run_git(&rename_repo, &["add", "tests/value.test.ts"]));
    commit_fixture_change(&rename_repo, "Rename source value");

    let delete_repo = create_fixture_repo("delete");
    assert_git_success(&run_git(&delete_repo, &["rm", "src/value.ts"]));
    commit_fixture_change(&delete_repo, "Delete source value");

    let invalidator_repo = create_fixture_repo("invalidator");
    write_fixture_file(&WriteFixtureFileRequest {
        repository_path: &invalidator_repo,
        relative_path: "tsconfig.json",
        content: "{}\n",
    });
    assert_git_success(&run_git(&invalidator_repo, &["add", "tsconfig.json"]));
    commit_fixture_change(&invalidator_repo, "Change global invalidator");

    let rename_output = run_affected_tests(CommandRequest {
        repository_path: &rename_repo,
        args: &[
            "--base",
            "origin/main",
            "--head",
            "HEAD",
            "--format",
            "json",
        ],
    });
    let delete_output = run_affected_tests(CommandRequest {
        repository_path: &delete_repo,
        args: &[
            "--base",
            "origin/main",
            "--head",
            "HEAD",
            "--format",
            "json",
        ],
    });
    let invalidator_output = run_affected_tests(CommandRequest {
        repository_path: &invalidator_repo,
        args: &[
            "--base",
            "origin/main",
            "--head",
            "HEAD",
            "--format",
            "json",
        ],
    });

    // Source renames, deletes, and invalidators fail closed because the current
    // graph cannot prove base-graph safety for old source paths.
    assert_success(&rename_output);
    assert_success(&delete_output);
    assert_success(&invalidator_output);
    assert!(stdout(&rename_output).contains(r#""status":"full""#));
    assert!(stdout(&rename_output).contains("deleted source file: src/value.ts"));
    assert!(stdout(&delete_output).contains(r#""status":"full""#));
    assert!(stdout(&delete_output).contains("deleted source file"));
    assert!(stdout(&invalidator_output).contains(r#""status":"full""#));
    assert!(stdout(&invalidator_output).contains("global invalidator changed"));
}
