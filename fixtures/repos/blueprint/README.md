Fixture blueprint for binary-level Git integration tests.

The integration tests materialize these files into temporary real Git
repositories, then create commits, branches, renames, deletes, and invalidator
changes with the `git` executable. This keeps the fixture assets first-class
without committing broken gitlinks or nested repository metadata.
