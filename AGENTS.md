
# general notes about opto-sync and it's dependants and dependencies

keep in mind that opto-sync is a zed package (github.com/zed-pkg), so it's effectively an sdk or library used by other codebases.

so when making changes to opto-sync repos we must keep in mind consumers of the lib.
it's ok to make breaking changes, but make sure the versioning is good -

big breaking changes should have a semver major version bump, small breaking changes a minor bumb, no breaking changes or minute changes a patch etc etc.

here are a list of other repos that depend on opto-sync, these repos primarily wrap opto-sync and serve their respective gh org:

github.com/3fa-app/3fa-app-sync
github.com/athlet-o/athleto-sync
github.com/quaestor-ledger/quaestor-sync
github.com/sonus-auris/sonus-auris-sync
github.com/daedalus-fab/daedalus-sync
github.com/fiducia-cloud/fiducia-sync
github.com/zed-pkg/zed-sync (a bit meta since zed is package manager for opto-sync etc, so this one is very important to keep in mind)

- Build values, don't mutate them: functions return new values instead of filling `&mut` parameters or caller-owned state. Public in-place methods that consumers already depend on (`CausalEnvelope::upsert`/`delete`/`acknowledge_into`) stay as thin boundaries over the value primitives (`VersionVector::incremented`/`observed`/`joined`, `CausalEnvelope::acknowledged`). Deliberate exceptions on hot paths (the `merge_*` reconciliation kernel, `values_deep_equal`, `from_entries`) carry a `HOT-PATH (imperative by design)` comment with the reason. See [`docs/FUNCTIONAL-STYLE.md`](./docs/FUNCTIONAL-STYLE.md).

## Repository-local Git worktrees

- Create or use a Git worktree only when the human operator explicitly authorizes it for the current task. Concurrency or a dirty checkout is not permission by itself.
- Put every authorized worktree at `<repository-root>/tmp/worktrees/<name>`; from the repository root, use `./tmp/worktrees/<name>`. Never place worktrees beside repositories or organization directories.
- Keep `tmp`, `temp`, `tmp/worktrees`, and `temp/worktrees` ignored in the repository-root `.gitignore`. Do not commit files from those directories.
- Relocate or remove a worktree only when the operator explicitly requests it. Before removal, preserve and publish intended changes, verify its commit is represented on the target branch, and confirm there are no tracked, untracked, ignored-sensitive, or in-use files that must survive. Remove it with `git worktree remove <path>` without `--force`; never delete a worktree directory with `rm`.

<!-- BEGIN ores-agents-pointer: managed by ORESoftware/my-ai; edit there, not here -->

## Canonical agent instructions

Before doing anything else in this repository, also read:

    .ores/agents/AGENTS.md

That path is a symlink to `~/codes/oresoftware/my-ai/AGENTS.md`, whose canonical copy is
<https://github.com/ORESoftware/my-ai/blob/main/AGENTS.md>.

It exists at a fixed path *inside* the repository because some agents cannot walk up past
the repository root, so machine-wide instructions one or more directories above are
invisible to them. This pointer plus that path make the same file reachable from a working
directory anywhere in the tree.

The symlink is deliberately **not committed**: it names an absolute path that is only valid
on a machine with `~/codes/oresoftware/my-ai` checked out, so committing it would produce a
broken link for everyone else and for CI. `.ores/` is git-ignored for that reason. If
`.ores/agents/AGENTS.md` is missing on your machine, create it with:

    mkdir -p .ores/agents
    ln -sfn "$HOME/codes/oresoftware/my-ai/AGENTS.md" .ores/agents/AGENTS.md

or run `~/codes/oresoftware/my-ai/scripts/link-repo-agents.sh` once to do it for every git
repository under `~/codes`, and `--check` to verify them.

A missing `.ores/agents/AGENTS.md` is a setup gap on the reader's machine, never a reason to
skip the canonical instructions: fetch them from the URL above instead.

<!-- END ores-agents-pointer -->
