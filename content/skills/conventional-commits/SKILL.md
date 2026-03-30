---
name: conventional-commits
description: Create accurate git commits using the Conventional Commits format, including commit splitting, type and scope selection, staged-change review, meaningful bodies, and breaking-change footers. Use when Codex needs to inspect repository changes, decide whether work belongs in one or multiple commits, stage the right files, write conventional commit messages, or perform the commit on the user's behalf.
---

# Conventional Commits

Turn repository changes into clear, reviewable commits that follow Conventional Commits and accurately describe what changed.

## Workflow

### 1. Inspect Before Staging

Inspect the repository state before deciding on a commit shape.

- Run `git status --short` to see tracked, modified, deleted, and untracked files.
- Run `git diff --stat` and read the actual diffs that matter, not just file names.
- Run `git diff --staged` if anything is already staged.
- Run `git log --oneline -n 10` when recent history may reveal local scope names or commit conventions.
- Notice unrelated, generated, vendored, binary, or secret-bearing files before staging anything.

### 2. Split Mixed Changes

Prefer one atomic commit per logical change.

- Split behavior changes from refactors when a reviewer would want to understand or revert them separately.
- Split formatting-only changes from logic changes.
- Split dependency or tooling updates from product code when they are independently reviewable.
- Keep generated files with their source change only when reviewers need them together.
- Refuse to sweep unrelated user changes into the same commit unless the user explicitly asks for that.

Keep one commit only when all staged files support the same intent, the subject can describe the whole diff honestly, and a revert would likely happen as one unit.

### 3. Stage Deliberately

Stage only what belongs to the current commit.

- Prefer explicit paths or patch staging over `git add .` when the worktree is mixed.
- Re-check `git diff --staged` before committing.
- Unstage accidental files immediately instead of writing a vague message that hides them.

### 4. Choose Type and Scope Precisely

Choose the narrowest accurate Conventional Commit type.

- `feat`: add or expand user-facing behavior or a new capability.
- `fix`: correct broken behavior, wrong output, crashes, or regressions.
- `refactor`: restructure code without intended behavior change.
- `perf`: improve measurable performance characteristics.
- `docs`: change documentation only.
- `test`: add or update tests without changing production behavior.
- `build`: change dependencies, packaging, bundling, or build tooling.
- `ci`: change CI or automation pipelines.
- `chore`: perform maintenance work that does not fit the other types.
- `revert`: revert an earlier commit.

Choose a scope only when it improves clarity.

- In a single-project repository, prefer the smallest stable subsystem, package, feature area, or app boundary that explains the change, such as `auth`, `chat`, `api`, `docs`, or `build`.
- In a monorepo, if the commit is isolated to one subproject, default the scope to that subproject's path relative to repository root, such as `apps/web`, `packages/ui`, or `services/topics`.
- When a nested subproject is the real ownership boundary, keep the full root-relative path for that boundary rather than collapsing it to the leaf directory name.
- Preserve path separators in monorepo scopes when the repository does not show a different established convention.
- Inside one subproject, narrow the scope below the root-relative path only when the repository already does that consistently and the narrower scope materially improves clarity.
- If the change spans multiple unrelated subprojects, split it into multiple commits. If one combined commit is unavoidable, omit the scope unless the repository already uses a shared umbrella scope for that kind of change.
- Omit the scope when it would be vague, arbitrary, or misleading.

Use these defaults unless recent repository history shows a stronger local convention:

- Single-project repo: `fix(chat): stop duplicate optimistic messages`
- Monorepo subproject: `feat(apps/web): add chat composer attachment preview`
- Monorepo shared package: `refactor(packages/ui): split message list item variants`
- Repo-wide root change: `ci: cache bun install in GitHub Actions`

Follow repository-specific commit conventions when they already exist. If the repository consistently shortens root-relative project paths, use that local convention instead of forcing the generic defaults above.

### 5. Write the Subject, Body, and Footers

Write the subject in one of these forms:

- `<type>: <summary>`
- `<type>(<scope>): <summary>`

Write the subject line with these rules:

- Keep it imperative and specific.
- Target 72 characters or fewer when feasible.
- Prefer lower-case summaries unless proper nouns or established acronyms require capitalization.
- Avoid filler such as `update`, `misc fixes`, `changes`, or `wip`.
- Describe the actual change, not the file operation.

Add a body when the subject alone would hide important context.

- Format the body as a flat bullet list that uses `- ` for every item.
- Keep one bullet per line.
- Keep bullets contiguous with no empty lines between them.
- Never emit the literal characters `\n` inside the commit message.
- Explain why the change exists.
- Call out major implementation decisions, constraints, or tradeoffs.
- Mention migrations, follow-up work, or operator impact when relevant.

Use this shape when a body is needed:

```text
<type>: <summary>

- explain why the change exists
- call out important implementation details or tradeoffs
- mention migrations, follow-up work, or operator impact when relevant
```

Add footers only when they are real and useful.

- Add `BREAKING CHANGE:` when behavior, APIs, configuration, schemas, or contracts change incompatibly.
- Add issue references only when the user asked for them or the repository already uses them.
- Never invent ticket numbers, references, or breaking-change claims.

Comprehensive commits are precise, not bloated. Add enough context to make the commit self-explanatory without turning the message into a design document.

### 6. Verify Before Committing

Run proportionate verification before creating the commit when feasible.

- Run the smallest relevant lint, test, build, or typecheck commands that cover the change.
- Report clearly when verification was skipped, unavailable, or failed.
- Re-read `git diff --staged` after any last-minute fixups.

If hooks fail, fix the issue when it is in scope. Otherwise surface the failure clearly instead of forcing the commit through without permission.

### 7. Commit Carefully

Create the commit only when the user asked for a commit or clearly expects one as part of the task.

- Use `git commit` with a reviewed subject and add a body when needed.
- Inspect the resulting commit with `git show --stat --summary HEAD` after committing.
- Avoid empty commits unless the user explicitly requests one.
- Avoid history rewriting, amending, squashing, or force-pushing unless the user explicitly requests it.

If the user asks only for a commit message, provide the proposed subject, body, and footers without creating the commit.

## Quality Bar

Reject low-signal commit messages. Replace vague summaries with concrete ones.

Prefer:

- `feat(chat): add optimistic echo for pending messages`
- `fix(auth): stop refresh retry loop after 401 failures`
- `refactor(topics): split D1 query builder from resolver`

Avoid:

- `feat: updates`
- `fix: stuff`
- `chore: misc changes`

## Default Behavior

Assume the user wants high-signal, review-friendly commits.

- Preserve atomicity over convenience.
- Preserve accuracy over optimism.
- Preserve repository history quality over getting a commit created quickly.
