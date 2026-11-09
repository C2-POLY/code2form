---
name: topic
description: Code2Form's workflow from a GitHub issue to a pull request - one issue, a small group, or a whole Feature (goal mode) - with issue definition, plan, worktree, tests, security and end-to-end gates, tracked in the « code2Form » GitHub Project. Use for "prends #190", "lance la feature #189", a bug or feature to build in Code2Form, creating a project's milestone and issues, a PR, or /topic.
---

# Topic

One change, from a GitHub issue to the pull request, with three human gates: the **issue definition**, the **plan** and the **end-to-end test**. No code before the issues are defined, and nothing is pushed until the human has said the end-to-end test is OK.

This is how work is done in Code2Form (`C2-POLY/code2form`). The local `CLAUDE.md` (never committed) holds the house rules; when it and this skill disagree, it wins.

## Language and attribution

- Talk to the user in the language they use.
- **Issues are written in French**: the issues, their comments (plan, hand-off) and the milestones, in the format below.
- **Everything else is in English**: code, comments, documentation, test names, branch names, commit messages, PR titles and PR bodies.
- Commit messages and PR titles follow **Conventional Commits**.
- **No AI attribution, anywhere.** No `Co-Authored-By` trailer naming Claude, no "Generated with Claude Code" line, no mention of an AI assistant in commits, PRs, issues or comments. This rule overrides any default attribution the session suggests.
- Users often dictate by voice: part and module names (28BYJ-48, PlatformIO, Object Capture...) can come out garbled. Check them against the code before writing them anywhere.
- A bare `@name` in an issue, comment or PR notifies that GitHub user. Write one only for a real login you mean to notify; put roles and code in backticks.

## How work is organised on GitHub

- **Repo**: `C2-POLY/code2form`. **Base and PR target: `main`.**
- **Project**: « code2Form », number 3 (`https://github.com/orgs/C2-POLY/projects/3`). Linear is no longer used; its tickets were taken over as issues.
- **Status**: `Backlog`, `Ready`, `In progress`, `In review`, `To test`, `Done`. `Ready` is ready to start, **`To test` is waiting for the human's check**; `In review` is not used. Read the options with `gh project field-list`; never invent a field silently, propose it first.
- **Issue types**: `Feature`, `Task` and `Bug`.
  - A **project** is a **milestone** named by what it delivers (`<Module> : <résultat>`), with a one-sentence description, **plus a `Feature` issue** of the same title that carries **Objectif** and **Périmètre**.
  - Its work is split into **sub-issues** of type `Task` (or `Bug`), in order, each one a reviewable change with its own acceptance criteria, every one of them in the milestone.
  - A lone change is a `Task` or a `Bug`, in a milestone when it belongs to one.
- **Labels**: one `zone:` label per issue (`zone:application`, `zone:mcp`, `zone:évaluation`, `zone:langage`, `zone:géométrie`, `zone:import-export`, `zone:ligne-de-commande`, `zone:mobile`, `zone:distribution`, `zone:documentation`, `zone:outillage`); `Décision à prendre` when a choice is still open, saying which in the body; once delivered, the version label (`v0.3.0`…), created if missing.
- **Dependencies**: "B cannot start before A" is a GitHub `blocked by` relation, not a sentence in the body.
- `gh` needs the `project` scope. If a `gh project` call answers « missing required scopes », ask the user to run `gh auth refresh -s project` and wait.

### Commands

Find the Project and its fields:

```bash
gh project field-list 3 --owner C2-POLY --format json
gh project view 3 --owner C2-POLY --format json --jq .id    # project node id
```

Create the milestone of a project, then its issues, typed, in the Project, optionally under the Feature:

```bash
gh api repos/C2-POLY/code2form/milestones -f title="<Module> : <résultat>" -f description="<one sentence>"
gh issue create -R C2-POLY/code2form --type Task --title "<title>" --body-file <file> \
  --project code2Form --milestone "<milestone title>" [--parent <feature number>] --label zone:<zone>
```

Change type, parent or Project afterwards: `gh issue edit <n> --type Bug`, `--parent <n>`, `--add-project code2Form`.

Set a Project field (Status included):

```bash
item=$(gh project item-list 3 --owner C2-POLY --format json --limit 600 \
  --jq '.items[] | select(.content.number==<n>) | .id')
gh project item-edit --project-id <project id> --id "$item" \
  --field-id <Status field id> --single-select-option-id <option id>
```

Record a dependency (`issueId` is blocked by `blockingIssueId`; both are node ids from `gh issue view <n> --json id`):

```bash
gh api graphql -f query='mutation($a:ID!,$b:ID!){addBlockedBy(input:{issueId:$a,blockingIssueId:$b}){clientMutationId}}' -f a=<blocked> -f b=<blocking>
```

Read a Feature with its sub-issues, in order:

```bash
gh api graphql -f query='query($n:Int!){repository(owner:"C2-POLY",name:"code2form"){issue(number:$n){title body subIssues(first:50){nodes{number title state issueType{name} blockedBy(first:5){nodes{number}}}}}}}' -F n=<number>
```

In zsh, a loop over pairs does not split `$pair`: write `for a b in 191 190 192 191; do …; done`.

## Scope of a run

A run delivers one **batch**: a single issue, a small group of related issues, or a whole Feature in **goal mode** (the user hands over the Feature and its open sub-issues are drained one after the other, in their sub-issue order, `blocked by` relations respected).

- **One project = one branch and one PR**, not one per issue: the whole milestone ships together unless the user says otherwise. A commit per sub-issue is welcome on that branch.
- Every issue of the batch is listed in the commit footer and the PR body.
- If the user slips in an unrelated topic along the way, note it and handle it afterwards, in its own batch.

## Progress

The steps always carry these short names: Issues, Analysis, Plan, Worktree, Build, Checks, Security, End-to-end, PR, Report.

At the start of each step, mark a chapter with `mcp__ccd_session__mark_chapter`, with the exact title `N/10 Step` (for example `6/10 Checks`), and nothing else in the title. If a step is blocked, do not change chapter: say in one sentence what blocks, with the output. If you go back (the end-to-end test fails, you fix it in Build), mark the chapter of the step you go back to again. Without that tool, write `N/10 Step` as the first line of the message at the start of each step. In goal mode, say which issue you are on in the message, not in the title.

## Model per type of task

Pick the model by the kind of work, not by the batch: one batch can mix cheap and risky work. Values are the `model` and `effort` of the Agent tool.

| Type of task | Model |
|---|---|
| Read and map the code, search GitHub for existing issues | Haiku, low |
| Small, well-bounded work: copy, translations (`tr!`), fixtures, docs | Haiku, low |
| Write the tests from the issue's acceptance criteria | Sonnet, medium |
| Everyday code: a studio panel, a CLI command, a library part, an MCP tool's plumbing | Sonnet, medium |
| Risky code: geometry and evaluation the reference (2021.01) defines, the agent's prompts and tools and its guard (what it may write), flashing a board, the iOS bridge (UniFFI), file formats others read (`.c2fproj`, `.c2fc`) | Opus, high |
| A bug whose cause is unknown | Opus, high |
| Security review of the diff (step 7) | Opus, high |
| Analyze a test failure (step 6) | Sonnet, medium |

**Stays in the main session**: everything that needs the user's answer, which is the issue definition (step 1), the plan (step 3), the end-to-end test (step 8), the commit and the PR (step 9).

The plan (step 3) says, for each block of work, which model it will use, so the user can change it before the build starts.

Delegation rules:

- **A sub-agent does not see the conversation**: its brief gives the goal, the files, what is done, what is excluded and what it must return.
- **A sub-agent asks the user nothing.** It works from its brief and returns.
- **A sub-agent writes nothing to GitHub** (no issue, comment, label or Project edit): it returns, and the main session writes.
- **A sub-agent's result is a hint, not a proof**: reread the files, lines and test output it cites before using them.
- When unsure between two rows, take the cheaper one and move up if the work demands it.

## 0. Where you are

Run every git command, test and build from **inside the repo or its worktree** (`git -C <path> …`, or `cd <path> && …` in the same command). Read the local `CLAUDE.md` and the `README.md`, then the parts of `docs/` the change touches.

## 1. Issues: find them, or define them (gate 1)

If `gh` is not authenticated, say so and stop: no code without issues.

**The user gives numbers or a Feature**: read every issue with `gh issue view <n> --json number,title,body,state,issueType,labels,milestone,comments,projectItems`. For a Feature, read its sub-issues with the query above. Their acceptance criteria are the contract. An issue without verifiable criteria is completed with the user (below) before anything else.

**The user describes the change freely**: first look for existing issues, open and closed, in the repo:

- `gh issue list -R C2-POLY/code2form --state all --search "<two or three key words>"`;
- a second search with synonyms;
- the open Features and milestones, to see whether the change belongs under one.

Then:

- **An open issue matches**: summarize it and check with the user that its criteria are still right. If it is too thin, complete it (below) and update it with `gh issue edit --body-file`.
- **A closed issue matches**: do not reopen it. If the change picks it up again, create a new issue whose context links to it.
- **Nothing matches**: define the issue with the user **before any code**. Ask the open questions: expected behaviour, edge cases, what is out of scope, which surfaces it touches (studio, CLI, MCP and its prompts, iPhone), what the user sees. Do not guess. Decide with the user whether it is a lone `Task` or `Bug`, a sub-issue of an existing Feature, or a new project (milestone and Feature with sub-issues). Draft every issue in the format below, show them, wait for the user's OK, then create them: milestone, type, parent, Project, `Status` set to `Ready` (the Feature too), `blocked by` relations, and its `zone:` label. Give the user each number and link.

### Issue format

Short and plain, in French, written for a reader who has none of the history in the user's head:

> **Titre :** `Scan : reconstruire au détail le plus fin et remettre le scan dans ses axes`
>
> **En tant qu'utilisateur** qui scanne une pièce, **je veux** un scan fin et posé dans ses axes, **afin que** ce que j'y place ne soit pas de travers dès le départ.
>
> Aujourd'hui, `Rebuild.run` (`apps/ios/App/Scan/Rebuild.swift`) reconstruit au réglage par défaut, et `Triangles.standing()` pose le scan sans le tourner : il garde l'angle de la prise de vue. Le recalage se fait en Rust (`crates/geom`), pour que l'iPhone et le studio fassent le même.
>
> **Critères d'acceptation**
>
> - [ ] Le scan est tourné selon ses axes principaux et posé sur sa face d'appui, par une fonction Rust partagée entre l'iPhone et le studio.
> - [ ] Les îlots flottants (morceaux du support, bruit) sont retirés.
> - [ ] Un test sur un scan tourné de 17° le retrouve d'aplomb à moins de 1° près.
> - [ ] Dans le studio, « Remettre d'aplomb » s'applique aussi aux scans déjà enregistrés.
>
> Hors ticket : vérifier l'échelle avec un objet de référence.
>
> **Livré** : `à venir`

The rules behind it:

- **Title**: `<Module> : <ce que ça livre>`, a result rather than a task. For a bug, the symptom seen (`Géométrie : offset(r) place ses arcs autrement que la référence, jusqu'à 0,3 % de volume d'écart`).
- **User story** first: `**En tant que** …, **je veux** …, **afin de** …`. The role is a real person: a user who models a part, who opens an existing `.scad`, who scans a part, who wires a circuit; or the agent; or a developer. Before a vowel, the elided word goes inside the bold (`**En tant qu'utilisateur**`, `**afin d'envoyer**`): Markdown does not close `**` between an apostrophe and a letter.
- **Context** when it helps, one or two short paragraphs: what happens today and why it is a problem, what was seen and when, how the issue fits with the others (`#n`), and the technical anchor (the crate, file, tool or screen it builds on, in backticks). The reference implementation is called `la référence (2021.01)`.
- **`**Critères d'acceptation**`**: 3 to 5 checkboxes, one observable behaviour each, checkable by a test, a command and its output, a measured value, or something seen in the app or in PrusaSlicer. Behaviour the reference defines is checked against it (the goldens in `tests/oracle/`, `scripts/oracle.sh`). Include the documentation to update when there is one.
- **`Hors ticket : …`**: one line, what this issue leaves out on purpose.
- **`**Livré** : \`à venir\``** at creation; it becomes the PR link at step 9.
- **A Feature** has the same user story and context, then **Objectif** and **Périmètre**; its acceptance criteria are the capability as a whole. Its sub-issues carry the detail; do not repeat their criteria in the Feature.
- No other headings, no empty sections. Dependencies go in `blocked by` relations and parents, plus links in the context.

**Definition of Ready**, checked for every issue of the batch before going on: user story (or symptom for a bug), criteria present and each one verifiable, technical anchor checked in the real code, type set, milestone and `zone:` label set, in the Project with a `Status`, `Hors ticket` line written. If one fails, say what is missing and settle it with the user. An issue too big for one reviewable change becomes a Feature with sub-issues, not a mega-issue.

If an issue is already `In progress` with another assignee, say so and ask before taking it.

## 2. Analysis

1. Run `git status --short`. If the working tree is not clean, show what is there and ask before going further. Never stash, discard or commit someone else's changes on your own.
2. Run `git fetch origin --prune`.
3. Read every file the issues name, then follow the code path end to end: the studio panel or the CLI command or the MCP tool, down to the crate that does the work, and the existing tests for that area. Find the patterns already used for this kind of change and follow them.
4. Check what the platforms really offer before planning on them (an iOS or macOS API's availability in the SDK's `.swiftinterface`, a crate's version), and test the idea on a real file of the user's when one exists (a scan, a design), on a copy.
5. **Bug**: reproduce it before planning, ideally as a failing test. If you cannot, say so, with what you tried.
6. Note what the issues did not anticipate: wrong assumption, missing dependency, a product decision to make. Ask instead of guessing.

## 3. Plan (gate 2)

Write a short plan, in French:
- the approach, in two or three sentences;
- what the analysis changes in the issue, if anything (a criterion that cannot hold as written, a new issue to file);
- the files to create or change;
- the tests to write, with **each acceptance criterion of each issue mapped to at least one test or one end-to-end check** (a criterion with no matching step means the plan is incomplete);
- risks: the user's files (a scan or design rewritten in place), formats others read, what the agent may write, backward compatibility;
- the model for each block of work (see "Model per type of task");
- in goal mode, the order of the sub-issues.

Post it as a comment (`gh issue comment <n> --body-file <file>`) on each issue of the batch, or on the Feature in goal mode. Show it to the user, and **stop. Wait for an explicit OK.** If the user changes the plan, edit the comment (`gh issue comment <n> --edit-last --body-file <file>`).

Once approved, claim the issues: `gh issue edit <n> --add-assignee @me`, and `Status` to `In progress` in the Project, on each one (on the Feature too in goal mode). Apply what the plan changed in the issues (a criterion rewritten, a new issue filed in `Backlog`).

## 4. Worktree

Always a worktree, so that parallel Claude sessions never change the branch under each other. Do not ask for the name.

**Branch name**: `<type>/<short-kebab-slug>`, in English, **without issue number**. `type` is a Conventional Commits type (`feat`, `fix`, `refactor`, `perf`, `test`, `docs`, `build`, `ci`, `chore`). Examples: `feat/scan-placement`, `fix/offset-arcs`. The slug names the batch, not one issue.

**Location**: a `.worktrees/` folder next to the repo, named `code2form-<slug>`, outside the repo so it never shows in its `git status`:

```bash
git worktree add --no-track -b <type>/<slug> ../.worktrees/code2form-<slug> origin/main
```

`--no-track` keeps the branch from following `main`, so a bare `git push` never goes there. Say it in one line (path and branch). Never work on `main` directly. Before each commit, check the branch with `git branch --show-current`.

If the session already runs in a worktree the app made for it, branch from `origin/main` in that one (`git switch --no-track -c <type>/<slug> origin/main`) instead of creating another.

## 5. Build

- **Bug**: write the failing test first, see it fail for the right reason, then fix.
- **Feature**: write the tests as you go, alongside the code.
- In goal mode, one sub-issue at a time, in the plan's order, with its tests, before the next one, and a commit for each (`Refs: #189, #190`).
- Stay inside the plan. If something outside it needs fixing, note it for the hand-off instead of doing it.
- User-visible strings go through `tr` / `tr!`, in English and French.
- Never weaken, skip or delete an existing test to make a run pass.

## 6. Checks

Run them in order, from inside the worktree, as the CI does:

1. `cargo test -p <crate>` for the crates the change touches;
2. `cargo clippy --workspace --all-targets -- -D warnings`;
3. `cargo test --workspace --no-fail-fast`, its output kept in a file and read (`test result` lines, `FAILED`, `panicked`): a filtered pipe can hide everything and still exit 0;
4. when `crates/mobile` or `apps/ios` changed: `scripts/ios-xcframework.sh`, then `xcodebuild -project apps/ios/Code2Form.xcodeproj -scheme Code2Form -destination 'generic/platform=iOS Simulator' build CODE_SIGNING_ALLOWED=NO ARCHS=arm64 ONLY_ACTIVE_ARCH=NO` (the framework has no x86_64 slice).

The CI also runs on Ubuntu and Windows: paths compared in tests use `Path::ends_with`, and `HOME` falls back to `USERPROFILE`.

On a failure: read the error, fix the cause, run again. **After three failed attempts on the same failure, stop.** Report what you tried and what you saw, as an issue comment and to the user, and ask. A failure that `main` already has is not the batch's fault: say so and propose a `Bug` issue instead of fixing it here.

Then walk the acceptance criteria one by one, issue by issue, and say for each whether it is met and how you know. A criterion you cannot check is not met.

## 7. Security check of the diff

Review `git diff origin/main...HEAD`:
- Run the `security-review` skill if it is available.
- Whether it ran or not, check at least these points yourself:
  - the agent's tools write only inside the folder the guard gives them (`guard::file_scope`), and a new writing tool is in `WRITING_TOOLS`;
  - nothing flashes a board or runs a build script the user has not approved (`approve_upload`, `extra_scripts` refused);
  - a file of the user's rewritten in place is replaced in one go (written aside, then renamed), and kept when the change cannot be undone;
  - paths from a file (`.c2fproj`, `.c2fc`, `import()`) cannot leave their folder;
  - downloads and outbound HTTP calls have a timeout;
  - no secret, token, API key or personal path in code, logs, fixtures or docs: the repo is public.

Fix what you find, then run step 6 again.

## 8. End-to-end test by the human (gate 3)

1. Build what the test needs **from the worktree** (`cargo build -p code2form --release`, `cargo build -p c2f-cli --release`), and start it on test data: a copy in `~/Documents/Code2Form/test-projets/`, never the user's own files. Check it runs before handing over.
2. Give the user a numbered checklist, one block per acceptance criterion:
   - where to go (panel, menu, command);
   - with which file;
   - what to do;
   - what they should see, with the values to expect.

   Include at least one negative case: cancel, a file that does not read, a refused action.
3. **Stop. Wait for the user's answer.** When they are done, read the studio's log too.
   - **OK**: go to step 9.
   - **Not OK**: ask for exactly what they saw (screen, message, log). Go back to step 5, then 6 and 7, then this step again with an updated checklist.

Never treat silence or an ambiguous answer as an OK.

## 9. Commit, push, pull request

1. Show `git status`, and stage only the files that belong to the change. Never stage `examples/.DS_Store`, a `CLAUDE.md`, logs or editor files.
2. Commit in English, Conventional Commits:
   ```
   <type>(<scope>): <imperative subject, lower case, no final period, under 72 chars>

   <why, not how>

   Refs: #189, #190
   ```
   - `type` is one of `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, `revert`. A breaking change uses `type!:` or a `BREAKING CHANGE:` footer.
   - The `Refs:` footer lists **every** issue the commit covers. It is the only footer, plus `BREAKING CHANGE:` when relevant. **No `Co-Authored-By` trailer.**
   - **Never `Closes`, `Fixes` or `Resolves`** in a commit or a PR: GitHub would close the issue at merge without the Project status or the hand-off; closing is done in step 10.
   - Get it right the first time: fixing a pushed message means rewriting history, and `git push --force-with-lease` is the user's to run.
3. Push the branch. When SSH is refused, over HTTPS: `git -c credential.helper='!gh auth git-credential' push -u https://github.com/C2-POLY/code2form.git <branch>`. Never force-push.
4. In goal mode, after each sub-issue's end-to-end OK: commit, push, and post its hand-off comment; the PR is opened once the last sub-issue is done, unless the user asks for it earlier (then as a draft, `gh pr create --draft`). Open the PR against `main`:
   ```bash
   gh pr create -R C2-POLY/code2form --base main --head <branch> --title "<same format as the commit>" --body-file <file>
   ```
   The body contains:
   - **Context**: one paragraph on the problem, then `Refs:` with **every issue of the batch** (`#n`), so each issue shows the PR in its timeline.
   - **Changes**: what changed, area by area (issue by issue in goal mode).
   - **Tests**: what was added; the checks run, and their results.
   - **End-to-end**: the checklist the human ran, marked OK.
   - **Security**: the outcome of step 7.
   - **To review closely**: risky areas, decisions taken.

   The PR title and body are in English. The body ends with the last section above: **no "Generated with Claude Code" line and no AI attribution.** Add the PR to the Project (`gh project item-add 3 --owner C2-POLY --url <pr url>`).
5. On **each** issue of the batch:
   - a comment with the hand-off: the commit or PR, each acceptance criterion with its verdict and how it was checked, anything left out;
   - in the description, replace `à venir` in the **Livré** line with the PR link.

   The issues stay `In progress` until the PR is merged. **Never set `To test` or `Done`, and never close an issue, before the PR is merged.**

## 10. Report and tidy up

Tell the user, in a few lines: the issues, the branch, the PR link, the checks run and their results, and anything left for later. If you noticed something outside the scope along the way, propose a new issue for it instead of fixing it in this PR.

Merging needs the user's explicit OK and a green CI. Once the PR is merged:

- set `Status` to `To test` on each issue of the batch, its boxes unticked: whoever checks it in the app ticks the boxes and moves it to `Done`. When the user says they have already tested it, set `Done` and close it (`gh issue close <n> --reason completed --comment "Livré dans <pr url>."`);
- the Feature follows its last sub-issue; it stays `In progress` while one is still open;
- add the version label once the release is cut.

Then remove the worktree and its local branch, after checking from the main repo folder: `git fetch --prune`, and after the squash merge `git diff origin/main <branch>` prints nothing, and `git -C <worktree> status --short` is empty. Then `git worktree remove <path>` and `git branch -D <branch>`, and delete the remote branch if GitHub kept it. When the session itself runs in that worktree, detach it on `origin/main` instead of removing it, and delete the branch. If one check fails, remove nothing: show the output and ask.

## Never, without the user's explicit approval in this conversation

- Write code before the issues are defined and the plan is approved.
- Create, edit or close an issue, a milestone, a Project field or a label the user has not seen and agreed to.
- Push to `main`; force-push; rebase a shared branch.
- Rewrite one of the user's own files outside a test copy.
- Flash a board.
- Merge the PR; close an issue or set it to `To test` or `Done` before its PR is merged.
- Skip one of the three gates.
