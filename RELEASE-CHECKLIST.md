# Flyline-multishell release checklist

Releases use the fork's independent semantic version and an immutable
`multishell-vX.Y.Z` tag. Upstream `vX.Y.Z` tags are ancestry markers only and
must never trigger this repository's release workflow.

## Prepare the release

1. Merge the intended upstream changes, if any. Update `UPSTREAM_BASE.toml`
   with the exact imported upstream commit and nearest upstream release.
2. Choose the next independent fork version. Update `Cargo.toml`,
   `Cargo.lock`, and the embedded changelog in `src/changelog.rs`.
3. Run the manual **Update settings documentation** workflow and merge its
   pull request if it changes the README.
4. Run the manual **Generate demos** workflow and review any changed demo
   assets.
5. Run the local verification suite:

   ```bash
   cargo fmt --check
   cargo clippy --all-targets --all-features --message-format=short
   cargo test --lib
   cargo test -p flycomp
   cargo build --features standalone
   cargo test --features standalone --test standalone_startup_tests
   cargo test --features standalone --test zsh_completion_tests -- --test-threads=1
   cargo test --features standalone --test zsh_integration_tests -- --test-threads=1
   docker buildx bake bash-integration-tests zsh-integration-test
   actionlint
   shellcheck install.sh
   zsh -n scripts/flyline.zsh
   ```

   Clippy warnings must remain visible and be reviewed in the workflow output.
   Do not use `--quiet` or otherwise suppress them. Existing warnings may be
   accepted for a release only after confirming this change introduces no new
   warnings; record any accepted baseline in the release notes.

6. Push the release commit to `master` and wait for CI to pass.

## Dry-run the complete release

Run the release workflow without creating a tag or GitHub release:

```bash
gh workflow run release.yml \
  --ref master \
  -f tag=multishell-vX.Y.Z \
  -f dry_run=true
```

Inspect the workflow artifacts. Every supported target must have a `.tar.gz`
archive and `.sha256`; every archive must contain the versioned loadable
library, `flyline-standalone`, `scripts/flyline.zsh`, both licenses, and
`UPSTREAM_BASE.toml`. Also inspect the SBOM and confirm all Bash and zsh
release-install tests passed. Provenance attestations are created only for the
real tagged run, because a dry run intentionally has no published subjects.

## Create the prerelease

Create and push exactly one annotated release tag:

```bash
git tag -a multishell-vX.Y.Z -m "flyline-multishell X.Y.Z"
git push origin refs/tags/multishell-vX.Y.Z
```

Never use `git push --tags`: inherited upstream tags are intentionally present
and must not be published as fork releases.

The tag workflow creates or reuses a draft, uploads assets idempotently, runs
the installation matrix, and publishes the result as a prerelease only after
validation succeeds. Confirm that the release title and notes contain the
exact upstream base from `UPSTREAM_BASE.toml`, then verify the GitHub artifact
attestations for the published archives.

## Promote or clean up

After manually installing from the prerelease on Bash and zsh, promote it:

```bash
gh release edit multishell-vX.Y.Z --prerelease=false
```

Rerun the same tag only for a transient infrastructure failure; draft creation
and uploads are idempotent while the release remains a draft. A tagged source
defect cannot be repaired by rerunning because release tags are immutable. In
that case, delete the draft without deleting or moving its tag, fix the source,
bump the fork version, and create a new tag:

```bash
gh release delete multishell-vX.Y.Z --yes
```

Delete stale workflow artifacts before the new-version dry run if their
contents no longer match the corrected commit. Never use `--cleanup-tag` or
retarget an existing release tag.