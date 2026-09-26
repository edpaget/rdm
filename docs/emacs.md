# Emacs package

`editors/emacs/` holds an Emacs package for rdm. It lets you browse, pick,
edit, review and commit a plan repo's roadmaps, phases and tasks from Emacs.
Every operation runs the `rdm` CLI. The package never reads the plan repo's
files itself.

It needs Emacs 31.1 or newer and an `rdm` binary. Every other package it
integrates with is optional (see [Soft dependencies](#soft-dependencies)).

## Install

### straight.el

The recipe takes every `editors/emacs/*.el` except `rdm-dev.el`, which is a
development helper and not part of the package:

```elisp
(use-package rdm
  :straight (rdm :type git :host github :repo "edpaget/rdm"
                 :files ("editors/emacs/*.el"
                         (:exclude "editors/emacs/rdm-dev.el"))))
```

straight generates the package's autoloads, so every user-facing command (see
[Commands](#commands)) is available without a `require`. `(require 'rdm)`
loads only the UI-free core.

### Development: a local checkout

To run the package from your own rdm checkout, point straight at it with
`:local-repo`:

```elisp
(use-package rdm
  :straight (rdm :type git :local-repo "~/Projects/rdm"
                 :files ("editors/emacs/*.el"
                         (:exclude "editors/emacs/rdm-dev.el"))))
```

While hacking on the package, `M-x load-file` `editors/emacs/rdm-dev.el`, then
run `rdm-dev-reload` after each edit. It reloads every package file and
re-initializes changed `defvar`/`defcustom` values, which a plain `load` leaves
alone. Nothing requires or autoloads `rdm-dev.el`.

### Without a package manager

```elisp
(add-to-list 'load-path "~/Projects/rdm/editors/emacs")
(dolist (feature '(rdm rdm-pick rdm-transient rdm-worktree rdm-git
                   rdm-modeline rdm-review))
  (require feature))
```

Without generated autoloads you require the modules yourself. `rdm-view`,
`rdm-compose` and `rdm-evil` come in through the modules above.

## Environment contract

Each buffer resolves its own context: plan repo root, project, default branch
and the `rdm` binary. `rdm-doctor` shows what a buffer resolved and where each
value came from. Precedence is per key.

| Key | 1. Override | 2. Buffer environment | 3. Fallback |
|---|---|---|---|
| root | `rdm-root` | `RDM_ROOT` | `rdm info` |
| project | `rdm-project` | `RDM_PROJECT` | `rdm info` |
| binary | `rdm-bin` | `RDM_BIN` | `rdm` on `PATH` |
| default branch | — | — | `rdm info` |

- **mise.** The intended setup puts `RDM_ROOT`, `RDM_PROJECT` and (for a
  repo-local build) `RDM_BIN` in the code repo's `.mise.toml`. With
  [mise.el](https://github.com/liuyinz/mise.el)'s `global-mise-mode`, every
  buffer in that repo gets them in its `process-environment`, and so does
  every rdm process the package starts from it.
- **Directory-local variables.** `rdm-root` and `rdm-project` are safe as
  directory-local variables:

  ```elisp
  ;; .dir-locals.el
  ((nil . ((rdm-root . "~/Projects/my-plan")
           (rdm-project . "my-project"))))
  ```

  `rdm-bin` is deliberately **not** safe: it names a program to run, so a cloned
  repo's `.dir-locals.el` cannot set it silently. Set it in your init file, or
  use `RDM_BIN`.
- **`rdm info` fallback.** A key neither the override nor the environment
  supplies comes from `rdm info --format json`, run from the buffer's project
  root. That covers `rdm config set default_project …` in the plan repo and a
  global `root` in rdm's config. rdm never infers a plan root from the current
  directory. When nothing resolves, commands signal an error that names these
  options.
- **Sessions.** Every rdm process the package runs carries `RDM_SESSION`, from
  the `rdm-session` option (one generated id per Emacs instance by default).
  The writes you make from one Emacs therefore form one changeset, and
  `rdm-plan-status`, `rdm-plan-commit` and `rdm-plan-discard` act on exactly
  that changeset. See [session identity](session-identity.md).
- **Caching.** Resolved contexts are cached per project root and
  override/environment values, so a changed override or `RDM_*` variable
  resolves afresh. After changing something only `rdm info` reports (rdm's
  config, the default branch), run `rdm-context-reset`.

### `rdm-doctor`

`M-x rdm-doctor` opens `*rdm-doctor*` with a report for the current buffer:

- the binary and the rung that supplied it (`rdm-bin`, `RDM_BIN` or `PATH`);
- its version, which is `unknown` while `rdm --version` is unavailable;
- the root and project, each with its rung (`rdm-root`/`rdm-project`, the
  buffer's `RDM_ROOT`/`RDM_PROJECT`, or `rdm info`);
- the default branch;
- whether `rdm info` ran cleanly;
- whether the buffer's environment came from mise.el (`mise-mode` on in the
  buffer);
- whether the context resolves, and if not, the error the commands would
  signal.

The doctor itself never signals. It reports the failure instead.

## Commands

### Evil bindings

rdm-evil.el installs these once evil loads, following evil-collection's
conventions.

**`rdm-view-mode`** (detail buffers), starts in normal state:

| State | Key | Command | Action |
|---|---|---|---|
| normal | `RET` | `rdm-view-open-at-point` | open the item at point (a roadmap's phase row) |
| normal | `gr` | `revert-buffer` | refresh from rdm |
| normal | `q` | `quit-window` | quit |
| normal | `y` | `rdm-view-copy-identifier` | copy the item's identifier |
| normal | `gd` | `rdm-view-copy-done-line` | copy the item's `Done:` commit line |
| normal | `x` | `rdm-dispatch` | the dispatch transient |
| normal | `K` | `rdm-review-comment-at-point` | describe the review comment at point |
| normal | `]c` | `rdm-review-next-comment` | next review comment |
| normal | `[c` | `rdm-review-previous-comment` | previous review comment |
| normal | `gA` | `rdm-review-resolve` | mark the comment at point addressed or won't-fix |
| normal | `C` | `rdm-review-comment-document` | whole-document comment in your draft review |
| normal | `gS` | `rdm-review-submit` | submit your draft review with a verdict |
| visual | `c` | `rdm-review-comment-region` | comment on the selected body text (the selection is the quote) |

**`rdm-compose-mode`** (body and comment buffers) starts in insert state:

| State | Key | Command | Action |
|---|---|---|---|
| normal | `ZZ` | `rdm-compose-submit` | submit |
| normal | `ZQ` | `rdm-compose-abort` | abort |

**`rdm-git-commit-mode`** (magit commit buffers):

| State | Key | Command | Action |
|---|---|---|---|
| normal | `gd` | `rdm-insert-done-line` | insert a `Done:` trailer |

### Vanilla bindings

**`rdm-view-mode`** (a `special-mode` buffer):

| Key | Command |
|---|---|
| `RET` | `rdm-view-open-at-point` |
| `o` | `rdm-view-open-at-point-other-window` |
| `w` | `rdm-view-copy-identifier` |
| `d` | `rdm-view-copy-done-line` |
| `@` | `rdm-view-at` (show the body at a revision) |
| `g` | `revert-buffer` |
| `q` | `quit-window` |
| `x` | `rdm-dispatch` |
| `K` | `rdm-review-comment-at-point` |
| `M-n` / `M-p` | `rdm-review-next-comment` / `rdm-review-previous-comment` |
| `A` | `rdm-review-resolve` |
| `c` | `rdm-review-comment-region` (with an active region) |
| `C` | `rdm-review-comment-document` |
| `S` | `rdm-review-submit` |

**`rdm-compose-mode`**: `C-c C-c` submits, `C-c C-k` aborts.

**`rdm-git-commit-mode`**: `C-c C-x d` inserts a `Done:` trailer.

**Embark** (`rdm-item-embark-map`, for any picked item): `RET` open, `o` open
in another window, `w` copy identifier, `d` copy `Done:` line, `x`
`rdm-dispatch`.

**`rdm-dispatch`** transient: one suffix per status `rdm describe` declares for
the item's kind; `T` tags, `B` body; `Rs` show review, `Rq` requests queue,
`Ra` close as addressed, `Rd` dismiss, `Rv` revise (copy command), `Rn` start
draft review, `Rc` comment on document, `RS` submit; `+r`/`+p`/`+t` create a
roadmap, phase or task; `S`/`C`/`D` plan-repo status, commit and discard.

### Autoloaded commands

These are the commands to call with `M-x` or bind globally. Each is autoloaded.

| Command | What it does |
|---|---|
| `rdm-roadmap` | pick a roadmap and open it (prefix: drill into its phases) |
| `rdm-phase` | pick a phase of a roadmap and open it |
| `rdm-task` | pick an open or in-progress task (prefix: every task) |
| `rdm-search` | search plan data (live per input with consult) and open a hit |
| `rdm-next` | open a roadmap's next actionable phase (the current worktree's roadmap by default) |
| `rdm-open-item-other-window` | open the item at point in another window |
| `rdm-view-at` | show the item at point with its body at a revision |
| `rdm-dispatch` | the dispatch transient: status, tags, body, reviews, create, plan repo |
| `rdm-create-roadmap` / `rdm-create-phase` / `rdm-create-task` | create an item, body in a compose buffer |
| `rdm-plan-status` / `rdm-plan-commit` / `rdm-plan-discard` | this Emacs's staged changeset: status, commit, discard |
| `rdm-review-requests` | pick a review requesting changes and open its target |
| `rdm-review-show` | display a review of the item at point |
| `rdm-worktree-list` | pick one of the code repo's rdm worktrees and open it |
| `rdm-worktree-add` | create (or reuse) the worktree for an item and open it |
| `rdm-worktree-remove` / `rdm-worktree-prune` | remove one worktree, or every done one |
| `rdm-insert-done-line` | insert a `Done:` trailer into a commit message |
| `rdm-git-commit-setup` | offer the trailer in a commit buffer (on `git-commit-setup-hook` automatically) |
| `rdm-modeline-mode` | show the current worktree's item and status in the mode line |
| `rdm-modeline-segment` / `rdm-modeline-nano-segment` | the mode-line segment, for your own mode line or nano-modeline |
| `rdm-context-reset` | forget cached contexts and `rdm describe` values |
| `rdm-doctor` | report how the current buffer resolves |

Commands that only make sense inside an rdm buffer are not autoloaded. That
covers the keymap commands above (`rdm-view-*`, `rdm-compose-*`, the review
navigation, resolution and authoring commands), the `rdm-dispatch` suffixes,
and the embark `rdm-item-*` actions. They load with the buffer or transient
that offers them.

### Suggested `SPC r` map

For a general.el/evil leader setup (edmacs binds exactly this):

| Key | Command |
|---|---|
| `SPC r r` | `rdm-dispatch` |
| `SPC r R` | `rdm-roadmap` |
| `SPC r p` | `rdm-phase` |
| `SPC r t` | `rdm-task` |
| `SPC r s` | `rdm-search` |
| `SPC r n` | `rdm-next` |
| `SPC r v` | `rdm-open-item-other-window` |
| `SPC r q` | `rdm-review-requests` |
| `SPC r w` | `rdm-worktree-list` |
| `SPC r W` | `rdm-worktree-add` |
| `SPC r d` | `rdm-insert-done-line` |
| `SPC r c` | `rdm-plan-commit` |
| `SPC r ?` | `rdm-doctor` |

## Soft dependencies

| Package | What it adds |
|---|---|
| evil | the evil bindings above; detail buffers start in normal state and compose buffers in insert state |
| consult | live `rdm-search` (one `rdm search` per input) |
| embark | actions on picked items (`rdm-item-embark-map`) |
| marginalia | nothing extra is needed: pickers annotate status and tags through the completion table, with or without it |
| magit | the `Done:` offer in commit buffers and mode-line refresh on `magit-post-refresh-hook` |
| markdown-mode | Markdown fontification of bodies and a Markdown compose buffer |
| nano-modeline | `rdm-modeline-nano-segment` serves as a segment |

The package does not require any of them. It uses each one only when it is
loaded. transient is built into Emacs (since 28), so `rdm-dispatch` needs
nothing extra.

## Running the suite locally

```bash
cargo nextest run -p rdm-cli --test emacs_package
```

This byte-compiles every `editors/emacs/*.el` with warnings as errors, runs
checkdoc over the same files, and runs the ERT suite (`editors/emacs/test/`)
in a child `emacs -Q --batch` against a seeded fixture plan. The suite also
installs the recipe's file set in a temporary directory, generates its
autoloads, and runs an autoloaded command in a child `emacs -Q`.

- `RDM_EMACS` picks the Emacs to run, otherwise the first `emacs` on `PATH`.
  With neither, the test prints a skip notice and passes.
- `RDM_EMACS_INIT_DIR` enables the second pass, in which the soft-dependency
  tests (evil, magit, consult, embark, markdown-mode) run instead of skipping.
  To build that directory once (this needs network access):

  ```bash
  mkdir -p ~/.cache/rdm-emacs-init
  cp editors/emacs/test/ci-init/init.el ~/.cache/rdm-emacs-init/
  emacs --batch --init-directory ~/.cache/rdm-emacs-init \
    -l ~/.cache/rdm-emacs-init/init.el
  export RDM_EMACS_INIT_DIR=~/.cache/rdm-emacs-init
  ```

- `RDM_EMACS_REQUIRED=1` turns both skips into failures. CI sets it.

CI runs the suite on Emacs 31.1, which is required, and on an Emacs snapshot,
which is allowed to fail. See [dev harnesses](dev-harnesses.md).
