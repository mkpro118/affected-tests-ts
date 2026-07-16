# affected-tests-ts

`affected-tests-ts` selects the TypeScript test files affected by a Git diff.

It builds a dependency graph from local TypeScript and TSX imports, traces backward
from changed files to dependent tests, and prints the test paths that should run.
When the tool cannot prove a precise subset safely, it fails closed and selects
the full discovered test suite.

## Install

Install from Git:

```sh
cargo install --git https://github.com/mkpro118/affected-tests-ts
```

Then run:

```sh
affected-tests-ts --version
```

## Advisory: AI-Assisted Development

This project was developed with AI assistance. The assistance included debugging,
implementation, testing, documentation, and review support. This note is a
provenance disclosure, not a quality claim: AI assistance does not make the code
better or worse by itself.

Human review remains required. Changes should be understood, tested, and reviewed
to the same standard as any other contribution before being used as a required CI
gate. The affected-test model is intentionally conservative, but it still depends
on codebase-specific import patterns, configuration, and test conventions.

This project does not treat AI output as autonomous authority. Maintainers and
contributors remain responsible for correctness, security, licensing, and the
decision to accept or reject generated suggestions.

## Quick Start

Run affected tests for committed branch changes:

```sh
affected-tests-ts
```

By default, this is equivalent to:

```sh
affected-tests-ts --base origin/main --head HEAD --format shell
```

Include staged, unstaged, and untracked worktree changes:

```sh
affected-tests-ts --worktree
```

Print JSON with reason chains:

```sh
affected-tests-ts --base origin/main --head HEAD --format json --explain
```

Print a human-readable explanation of the current selection:

```sh
affected-tests-ts --base origin/main --head HEAD --explain
```

Inspect the dependency graph:

```sh
affected-tests-ts graph --format json
```

Ask what would be affected if one file changed:

```sh
affected-tests-ts explain src/utils/date.ts
```

Note: `explain <path>` treats `<path>` as the changed file. To explain why a
test was selected by the current Git diff, use selection mode with `--explain`
and filter the returned reason chains.

## Command Behavior

The default command:

1. Reads changed files from Git using `base...head`.
2. Discovers source and test files under the current working directory.
3. Parses TypeScript and TSX imports with Oxc.
4. Builds a local dependency graph.
5. Traces reverse dependents from changed files to tests.
6. Prints selected test paths.

The current working directory is the TypeScript project root. In a monorepo,
run the command from the package or app directory whose tests you want to select.

## Output Formats

`--format shell`

Prints newline-delimited runnable test paths. This is the default and is intended
for piping into test runners, unless `--explain` is used without an explicit
format. Status text is not printed to stdout.

`--format plain`

Prints a human-readable status, paths, and reason chains when `--explain` is
enabled.

`--format json`

Prints the strict JSON contract. Use `--explain` to include reason chains for
partial selections.

`--format docker`

Prints static Docker-style progress lines for CI logs.

`--format tui`

Uses the interactive dashboard when stdout is a terminal. In non-interactive
contexts it falls back to plain output.

## CI Usage

In CI, run `affected-tests-ts` from the TypeScript project root after checkout
and dependency installation. For monorepos, this is usually the app/package
directory, not necessarily the repository root.

The safest CI pattern is:

```sh
tests="$(affected-tests-ts --base origin/main --head HEAD)"

if [ -z "$tests" ]; then
  echo "No affected tests"
  exit 0
fi

bun test $tests
```

For pull request workflows, make sure the base ref exists locally. Many CI
providers use shallow checkouts, so fetch the target branch before running:

```sh
git fetch origin main
affected-tests-ts --base origin/main --head HEAD
```

If your CI checks out a synthetic merge commit, set `--base` and `--head` to the
revisions your pipeline considers authoritative. The tool uses a three-dot Git
range (`base...head`) for committed changes.

Use `--format docker` when you want readable step logs:

```sh
affected-tests-ts --base origin/main --head HEAD --format docker
```

Use JSON when a CI script needs to inspect status and reasons:

```sh
affected-tests-ts --base origin/main --head HEAD --format json --explain > affected-tests.json
```

Shell format is designed for command substitution and piping. It prints only test
paths to stdout. If the tool fails closed and selects the full suite, stdout still
contains runnable test paths; the reason is written to stderr.

`--worktree` is usually not needed in CI because CI workspaces should be clean.
It is useful for pre-push hooks, local validation scripts, or CI jobs that
generate source/test files before affected-test selection:

```sh
affected-tests-ts --worktree
```

Recommended CI guardrails:

- Pin the installed `affected-tests-ts` version or build it from the repository
  revision under test.
- Run from the same directory that owns `tsconfig.json` or `affected-tests.json`.
- Keep stdout as test paths when using shell format; send logs to stderr.
- Fall back to the full test command if the affected-test command exits non-zero.
- Periodically compare affected-test results with full-suite runs before making
  it a required CI gate.

## Worktree Mode

By default, only committed changes in the Git range are considered. Dirty
worktree changes are not part of `HEAD`, so they are ignored.

Use `--worktree` to include:

- committed range changes from `base...head`
- staged and unstaged tracked changes from `git diff HEAD`
- untracked files from `git ls-files --others --exclude-standard`

Example:

```sh
affected-tests-ts --worktree --format plain
```

## Fail-Closed Cases

The tool returns a full selection when precision would be unsafe. Examples:

- A global invalidator changed and could not be scoped.
- A source file was deleted or renamed in a way that needs a base graph.
- A local import cannot be resolved.
- A non-literal dynamic import is configured to fail closed.
- Dependency metadata changed in a way that cannot be classified safely.

Shell output still prints runnable test paths for a full result. A diagnostic
reason is written to stderr so scripts can keep stdout as paths only.

## Package Metadata Changes

`package.json`, `bun.lock`, `bun.lockb`, and `tsconfig.json` are global
invalidators by default.

Some package metadata changes can be scoped more precisely:

- Dependency-only `package.json` changes are mapped to local files importing the
  changed package.
- Text `bun.lock` changes are mapped through the lockfile package/dependent graph
  and then to local importers.

The tool still fails closed for riskier cases, including:

- `bun.lockb`, because it is binary and we'd rather not parse it.
- package scripts or other package-level configuration changes.
- broad runtime/tooling packages such as `react`, `next`, `typescript`,
  `@prisma/client`, and test/build tooling.
- unparseable or unreadable metadata.

## Configuration

The tool reads `affected-tests.json` when present. If it is absent, it can use
`tsconfig.json` for TypeScript include/exclude and path mapping information.

Example `affected-tests.json`:

```json
{
  "sourceIncludes": ["src/**/*.ts", "src/**/*.tsx"],
  "excludes": ["node_modules/**", "dist/**", "build/**", ".next/**"],
  "testPatterns": ["**/*.test.ts", "**/*.test.tsx"],
  "globalInvalidators": [
    "package.json",
    "bun.lock",
    "bun.lockb",
    "tsconfig.json"
  ],
  "dynamicImports": "failClosed",
  "dynamicImportIgnore": ["src/graphql-contract/**"],
  "compilerOptions": {
    "baseUrl": ".",
    "paths": {
      "@/*": ["src/*"]
    }
  }
}
```

Defaults:

- Source files: `**/*.ts`, `**/*.tsx`
- Tests: `**/*.test.ts`, `**/*.test.tsx`, `**/*.spec.ts`, `**/*.spec.tsx`,
  `**/__tests__/**/*.ts`, `**/__tests__/**/*.tsx`
- Excludes: `node_modules/**`, `dist/**`, `build/**`, `.next/**`
- Global invalidators: `package.json`, `bun.lock`, `bun.lockb`, `tsconfig.json`
- Dynamic imports: `failClosed`
- Dynamic import ignore: empty (no files exempted)

`dynamicImportIgnore` is a list of globs matched against the importing file's
root-relative path. When `dynamicImports` is `failClosed`, a file matching one of
these globs is exempt from the full-suite trigger for its own unresolvable dynamic
imports (for example `await import(absolutePath)`), while every other file stays
fail closed. Static import edges from the exempted file are still resolved
normally.

## Development

Install from a local checkout:

```sh
cargo install --path .
```

Run the binary from a checkout:

```sh
cargo run -- --base origin/main --head HEAD
```

Run the test suite:

```sh
cargo test
```

Run formatting, tests, and strict lints:

```sh
cargo fmt && cargo test && cargo clippy --all-targets --all-features
```

The integration tests create real temporary Git repositories so Git range,
rename, monorepo, and worktree behavior are exercised at the process boundary.
