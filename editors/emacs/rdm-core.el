;;; rdm-core.el --- Context resolver, argv builder and runner for rdm  -*- lexical-binding: t; -*-

;; Author: Edward Paget <ed.paget@gmail.com>
;; URL: https://github.com/edpaget/rdm
;; Keywords: tools, convenience

;; This file is not part of GNU Emacs.

;;; Commentary:

;; The UI-free foundation of the rdm Emacs integration.
;;
;; - `rdm-context' maps the current buffer to the plan repo root, project,
;;   default branch and `rdm' binary it should act on.  Precedence is per
;;   key: the buffer-local `rdm-root' / `rdm-project' (dir-locals safe) /
;;   `rdm-bin' (not dir-locals safe: it names a program to run), then the buffer's `process-environment' (`RDM_ROOT',
;;   `RDM_PROJECT', `RDM_BIN', which mise.el populates per buffer from
;;   `.mise.toml'), then `rdm info --format json'.  The default branch comes
;;   only from `rdm info'.
;; - `rdm-argv' turns an action from `rdm--actions' plus arguments into an
;;   argv: every read carries `--format json', every write `--no-edit';
;;   a text action (`hook done-line', `review update') carries neither.
;; - `rdm-run' / `rdm-run-sync' run rdm over pipes (never a pty), always
;;   close stdin, keep stderr out of the JSON stream, parse reads as JSON and
;;   return a write's stdout as text.  A non-zero exit is a `user-error'
;;   carrying rdm's stderr.  Every process runs with `RDM_SESSION' set to
;;   `rdm-session-id', so the plan-repo writes made from one Emacs form one
;;   changeset that `rdm status' / `rdm commit' / `rdm discard' act on.
;; - `rdm-enum-values' reads the values core declares for one field of a
;;   kind from `rdm describe' (`rdm-status-values' is its `status' case),
;;   so the package holds no copy of a core enumeration.

;;; Code:

(require 'cl-lib)
(require 'json)
(require 'project)
(require 'seq)
(require 'subr-x)

(defgroup rdm nil
  "Roadmaps, phases and tasks from an rdm plan repo."
  :group 'tools
  :prefix "rdm-")

(defcustom rdm-root nil
  "Plan repo root for this buffer, or nil to resolve it.
When nil, `RDM_ROOT' from the buffer's environment is used, then
`rdm info'.  Safe as a directory-local variable."
  :type '(choice (const :tag "Resolve" nil) directory)
  :safe #'string-or-null-p)

(defcustom rdm-project nil
  "The rdm project for this buffer, or nil to resolve it.
When nil, `RDM_PROJECT' from the buffer's environment is used, then
`rdm info'.  Safe as a directory-local variable."
  :type '(choice (const :tag "Resolve" nil) string)
  :safe #'string-or-null-p)

(defcustom rdm-bin nil
  "The `rdm' executable for this buffer, or nil to resolve it.
When nil, `RDM_BIN' from the buffer's environment is used, then
`executable-find'.  Not marked safe as a directory-local variable: it
names a program to run, so a cloned repo's .dir-locals.el must not
set it silently."
  :type '(choice (const :tag "Resolve" nil) file))

(defcustom rdm-sync-timeout 30
  "Seconds `rdm-run-sync' waits for rdm before giving up."
  :type 'number)

(defcustom rdm-session nil
  "The rdm session id every rdm process this package runs carries.
nil means one id generated per Emacs instance.  A string is passed to
rdm as `RDM_SESSION' unchanged; rdm itself sanitizes it (trims it, drops
characters outside A-Z, a-z, 0-9, `.', `_' and `-', and keeps the first
64 bytes), and a value with nothing usable left, or only dots, falls
through to rdm's next identity rung.  See docs/session-identity.md."
  :type '(choice (const :tag "One per Emacs instance" nil) string))

(defcustom rdm-after-write-hook nil
  "Normal hook run after every successful plan-repo write the package makes.
It runs once per status, tag or body update and once per create, after
the write succeeded and the affected detail buffers were refreshed; a
declined confirmation, a write rdm refuses and a tag edit that changes
nothing do not run it.  The current buffer is whichever buffer the
write was made from, which need not be in the item's worktree."
  :type 'hook)

(defvar rdm--context-cache (make-hash-table :test #'equal)
  "Resolved contexts, keyed by `rdm--context-key'.")

(defvar rdm--enum-cache (make-hash-table :test #'equal)
  "Field values from `rdm describe', keyed by (BINARY KIND FIELD).")

;;;; Errors

(defun rdm--nonempty (value)
  "Return VALUE when it is a non-empty string, else nil."
  (and (stringp value) (not (string-empty-p value)) value))

(defun rdm--unresolved-error (dir &optional detail)
  "Signal the actionable `user-error' for DIR, appending DETAIL if non-nil."
  (user-error
   "rdm: No plan repo root and project resolved for %s.  Set RDM_ROOT and \
RDM_PROJECT in the code repo's .mise.toml (or `rdm-root'/`rdm-project' in \
.dir-locals.el), or run `rdm config set default_project <project>' in the \
plan repo%s"
   (abbreviate-file-name dir)
   (if detail (concat ": " detail) "")))

;;;; Session

(defvar rdm--generated-session nil
  "The session id generated for this Emacs instance, once needed.")

(defun rdm-session-id ()
  "The `RDM_SESSION' value for rdm processes: `rdm-session' or a generated id.
The value is passed through as given; rdm sanitizes it."
  (or rdm-session
      rdm--generated-session
      (setq rdm--generated-session
            (format "emacs-%d-%06x" (emacs-pid) (random #x1000000)))))

;;;; Runner

(defun rdm--first-line (text)
  "The first line of TEXT, or a placeholder when TEXT is empty."
  (let ((line (car (split-string (or text "") "\n"))))
    (if (rdm--nonempty line) line "<empty output>")))

(defun rdm--finish (state)
  "Deliver the result held in STATE once stdout and stderr are both done."
  (let ((proc (plist-get state :proc))
        (errproc (plist-get state :errproc)))
    (when (and (not (plist-get state :done))
               (memq (process-status proc) '(exit signal))
               (not (process-live-p errproc)))
      (plist-put state :done t)
      (let* ((outbuf (process-buffer proc))
             (errbuf (process-buffer errproc))
             (code (process-exit-status proc))
             (stderr (if (buffer-live-p errbuf)
                         (with-current-buffer errbuf
                           (string-trim (buffer-string)))
                       ""))
             (argv (plist-get state :argv))
             result err)
        (condition-case e
            (setq result
                  (cond
                   ((or (not (eq (process-status proc) 'exit))
                        (/= code 0))
                    (user-error "rdm: `rdm %s' failed (exit %s): %s"
                                (string-join argv " ") code
                                (if (string-empty-p stderr)
                                    "<no stderr>"
                                  stderr)))
                   ((plist-get state :parse)
                    (with-current-buffer outbuf
                      (goto-char (point-min))
                      (condition-case nil
                          (json-parse-buffer :object-type 'plist
                                             :null-object nil
                                             :false-object :false)
                        (json-error
                         (error "rdm: `rdm %s' output is not JSON: %s"
                                (string-join argv " ")
                                (rdm--first-line (buffer-string)))))))
                   (t (with-current-buffer outbuf (buffer-string)))))
          (error (setq err e)))
        (when (buffer-live-p outbuf) (kill-buffer outbuf))
        (when (buffer-live-p errbuf) (kill-buffer errbuf))
        (if err
            (funcall (plist-get state :on-error) err)
          (funcall (plist-get state :callback) result))))))

(defun rdm--default-on-error (err)
  "Report ERR from an asynchronous rdm call in the echo area."
  (message "%s" (error-message-string err)))

(cl-defun rdm-run (command &key stdin parse callback on-error directory env)
  "Run COMMAND (the program followed by its arguments) asynchronously.
The process uses pipes, never a pty, and its stdin is always closed:
STDIN, when non-nil, is written first.  stderr is collected separately.

On success CALLBACK receives the parsed JSON (a plist, with JSON null
as nil and false as `:false') when PARSE is non-nil, else stdout as a
string.  A non-zero exit becomes a `user-error' carrying stderr, and
unparsable output an `error' naming its first line; either is passed
as a condition to ON-ERROR (default: `message' it).

DIRECTORY is the working directory (default `default-directory').  ENV
is a list of \"VAR=value\" strings taking precedence over the calling
buffer's `process-environment'.  `RDM_SESSION' is always
`rdm-session-id', whatever the environment says.  Returns the process."
  (let* ((session (concat "RDM_SESSION=" (rdm-session-id)))
         (default-directory (file-name-as-directory
                             (expand-file-name (or directory default-directory))))
         (process-environment (cons session (append env process-environment)))
         (outbuf (generate-new-buffer " *rdm-out*" t))
         (errbuf (generate-new-buffer " *rdm-err*" t))
         (state (list :done nil :parse parse :argv (cdr command)
                      :callback (or callback #'ignore)
                      :on-error (or on-error #'rdm--default-on-error)))
         (errproc (make-pipe-process
                   :name "rdm-stderr" :buffer errbuf :noquery t
                   :coding 'utf-8
                   :sentinel (lambda (_p _e) (rdm--finish state))))
         (proc (condition-case err
                   (make-process
                    :name "rdm" :buffer outbuf :command command
                    :connection-type 'pipe :stderr errproc :noquery t
                    :coding 'utf-8
                    :sentinel (lambda (_p _e) (rdm--finish state)))
                 (error
                  (delete-process errproc)
                  (kill-buffer outbuf)
                  (kill-buffer errbuf)
                  (signal (car err) (cdr err))))))
    (plist-put state :proc proc)
    (plist-put state :errproc errproc)
    ;; rdm may exit before reading all of STDIN (a usage error, say), so
    ;; a body larger than the pipe buffer can fail to send with EPIPE.
    ;; Swallow that: the sentinel still delivers rdm's exit status and
    ;; stderr through `rdm--finish', which is the error worth reporting.
    (condition-case nil
        (progn
          (when stdin (process-send-string proc stdin))
          (process-send-eof proc))
      (error nil))
    proc))

(cl-defun rdm-run-sync (command &rest keys &key timeout &allow-other-keys)
  "Run COMMAND like `rdm-run' and wait for its result.
KEYS are `rdm-run' keywords other than :callback and :on-error.  Waits
at most TIMEOUT seconds (default `rdm-sync-timeout'), then kills the
process and signals an error.  Errors are re-signalled."
  (let* ((done nil) result err
         (timeout (or timeout rdm-sync-timeout))
         (proc (apply #'rdm-run command
                      :callback (lambda (r) (setq result r done t))
                      :on-error (lambda (e) (setq err e done t))
                      (cl-loop for (k v) on keys by #'cddr
                               unless (eq k :timeout) append (list k v))))
         (deadline (+ (float-time) timeout)))
    (while (and (not done) (< (float-time) deadline))
      (accept-process-output nil 0.05))
    (unless done
      (when (process-live-p proc) (delete-process proc))
      (error "rdm: `rdm %s' gave no result after %s seconds"
             (string-join (cdr command) " ") timeout))
    (if err (signal (car err) (cdr err)) result)))

;;;; Argv builder

(defconst rdm--actions
  '((info             :argv ("info")             :kind read :project nil)
    (worktree-current :argv ("worktree" "current") :kind read :project nil)
    (worktree-list    :argv ("worktree" "list")  :kind read :project nil)
    (worktree-add     :argv ("worktree" "add")   :kind read)
    (worktree-remove  :argv ("worktree" "remove") :kind text)
    (worktree-prune   :argv ("worktree" "prune") :kind read)
    (describe         :argv ("describe")         :kind read :project nil)
    (status           :argv ("status")           :kind text :project nil)
    (commit           :argv ("commit")           :kind text :project nil)
    (discard          :argv ("discard")          :kind text :project nil)
    (roadmap-list     :argv ("roadmap" "list")   :kind read)
    (roadmap-show     :argv ("roadmap" "show")   :kind read)
    (phase-list       :argv ("phase" "list")     :kind read)
    (phase-show       :argv ("phase" "show")     :kind read)
    (task-list        :argv ("task" "list")      :kind read)
    (task-show        :argv ("task" "show")      :kind read)
    (search           :argv ("search")           :kind read)
    (tree             :argv ("tree")             :kind read)
    (next             :argv ("next")             :kind read)
    (roadmap-create   :argv ("roadmap" "create") :kind write)
    (roadmap-update   :argv ("roadmap" "update") :kind write)
    (phase-create     :argv ("phase" "create")   :kind write)
    (phase-update     :argv ("phase" "update")   :kind write)
    (task-create      :argv ("task" "create")    :kind write)
    (task-update      :argv ("task" "update")    :kind write)
    (review-list      :argv ("review" "list")    :kind read)
    (review-show      :argv ("review" "show")    :kind read)
    (review-requests  :argv ("review" "requests") :kind read)
    (review-update    :argv ("review" "update")  :kind text)
    (review-start     :argv ("review" "start")   :kind read)
    (review-comment   :argv ("review" "comment") :kind write)
    (review-submit    :argv ("review" "submit")  :kind write)
    (done-line        :argv ("hook" "done-line") :kind text :project nil))
  "The rdm actions: symbol, then the subcommand words, `:kind' and `:project'.
`:kind' is `read' (JSON output, `--format json'), `write' (`--no-edit',
text output) or `text' (no suffix flag, text output).  `:project' nil
marks a command that takes no `--project'.")

(defun rdm--action (action)
  "The `rdm--actions' row for ACTION, or an error."
  (or (cdr (assq action rdm--actions))
      (error "rdm: Unknown action `%s'" action)))

(defun rdm-action-read-p (action)
  "Non-nil when ACTION is a read (its output is JSON)."
  (eq (plist-get (rdm--action action) :kind) 'read))

(defun rdm-argv (action &rest args)
  "Build the rdm argv for ACTION with ARGS; return (ARGV . STDIN).
ARGS are positional strings followed by keyword/value pairs.  `:project'
names the project (required unless the action takes none), `:body' is
passed as `--body=BODY' and `:stdin' becomes STDIN (not both); any
other `:key' becomes `--key=VALUE', or a bare `--key' when VALUE is t,
and is dropped when VALUE is nil.  Values are joined to their flag so
one starting with `-' (a Markdown list, say) is never read as an
option.  Reads get `--format json', writes `--no-edit', text actions
neither.
ARGV excludes the rdm program itself."
  (let* ((row (rdm--action action))
         (kind (plist-get row :kind))
         (scoped (if (plist-member row :project) (plist-get row :project) t))
         positional flags project body stdin)
    (while args
      (let ((a (pop args)))
        (if (keywordp a)
            (let ((v (pop args)))
              (pcase a
                (:project (setq project v))
                (:body (setq body v))
                (:stdin (setq stdin v))
                (_ (cond ((eq v t)
                          (setq flags (append flags
                                              (list (concat "--" (substring (symbol-name a) 1))))))
                         (v
                          (setq flags (append flags
                                              (list (format "--%s=%s"
                                                            (substring (symbol-name a) 1)
                                                            v)))))))))
          (push a positional))))
    (when (and body stdin)
      (error "rdm: %s given both :body and :stdin" action))
    (when (and scoped (not (rdm--nonempty project)))
      (error "rdm: %s needs a :project" action))
    (cons (append (plist-get row :argv)
                  (nreverse positional)
                  flags
                  (when body (list (concat "--body=" body)))
                  (when scoped (list "--project" project))
                  (pcase kind
                    ('read '("--format" "json"))
                    ('write '("--no-edit"))))
          stdin)))

;;;; Context resolver

(defun rdm--project-dir ()
  "The current project's root, else `default-directory'."
  (expand-file-name
   (or (when-let* ((proj (project-current nil))) (project-root proj))
       default-directory)))

(defun rdm--context-key (dir)
  "The cache key for the current buffer's context in DIR."
  (list dir rdm-root rdm-project rdm-bin
        (getenv "RDM_ROOT") (getenv "RDM_PROJECT") (getenv "RDM_BIN")))

(defun rdm--resolve-bin ()
  "The rdm executable for the current buffer, or a `user-error'."
  (or (rdm--nonempty rdm-bin)
      (rdm--nonempty (getenv "RDM_BIN"))
      (executable-find "rdm")
      (user-error "rdm: No `rdm' executable found.  Install rdm, set RDM_BIN \
in the code repo's .mise.toml, or customize `rdm-bin'")))

(defun rdm--resolve (dir)
  "Resolve the context for the current buffer, running rdm info from DIR."
  (let* ((bin (rdm--resolve-bin))
         (root (or (rdm--nonempty rdm-root) (rdm--nonempty (getenv "RDM_ROOT"))))
         (project (or (rdm--nonempty rdm-project)
                      (rdm--nonempty (getenv "RDM_PROJECT"))))
         (env (append (when root (list (concat "RDM_ROOT=" root)))
                      (when project (list (concat "RDM_PROJECT=" project)))))
         (info (condition-case e
                   (rdm-run-sync (list bin "info" "--format" "json")
                                 :parse t :directory dir :env env)
                 (error (rdm--unresolved-error dir (error-message-string e))))))
    (setq root (or root (rdm--nonempty (plist-get info :root))))
    (setq project (or project (rdm--nonempty (plist-get info :project))))
    (unless (and root project)
      (rdm--unresolved-error dir))
    (unless (file-directory-p root)
      (rdm--unresolved-error dir (format "plan repo root %s does not exist"
                                         (abbreviate-file-name root))))
    (list :root (expand-file-name root)
          :project project
          :default-branch (rdm--nonempty (plist-get info :default_branch))
          :rdm-bin bin)))

(defun rdm-context ()
  "The rdm context for the current buffer, as a plist.
Keys: `:root', `:project', `:default-branch', `:rdm-bin'.  Cached per
project root and override/environment values; `rdm-context-reset'
clears the cache.  Signals a `user-error' when no root or project
resolves."
  (let* ((dir (rdm--project-dir))
         (key (rdm--context-key dir)))
    (or (gethash key rdm--context-cache)
        (puthash key (rdm--resolve dir) rdm--context-cache))))

;;;###autoload
(defun rdm-context-reset ()
  "Forget every cached rdm context and `rdm describe' value list."
  (interactive)
  (clrhash rdm--context-cache)
  (clrhash rdm--enum-cache))

;;;; Calling rdm with a resolved context

(defun rdm--call-args (action args)
  "The `rdm-run' command and keywords for ACTION with ARGS in this buffer."
  (let* ((ctx (rdm-context))
         (built (apply #'rdm-argv action
                       (append args (list :project (plist-get ctx :project))))))
    (list (cons (plist-get ctx :rdm-bin) (car built))
          :stdin (cdr built)
          :parse (rdm-action-read-p action)
          :directory (rdm--project-dir)
          :env (list (concat "RDM_ROOT=" (plist-get ctx :root))))))

(defun rdm-call (action args callback &optional on-error)
  "Run ACTION with the `rdm-argv' ARGS asynchronously in this buffer's context.
CALLBACK receives the result and ON-ERROR any error; see `rdm-run'."
  (apply #'rdm-run (append (rdm--call-args action args)
                           (list :callback callback :on-error on-error))))

(defun rdm-call-sync (action &rest args)
  "Run ACTION with ARGS in this buffer's context and return the result."
  (apply #'rdm-run-sync (rdm--call-args action args)))

(defun rdm-enum-values (kind field)
  "The values rdm declares for FIELD (a string) of KIND (a symbol), in order.
Read from the `enum_values' of FIELD in `rdm describe KIND' and cached
per rdm binary until `rdm-context-reset'.  nil for a field without
declared values, or one KIND does not have."
  (let ((key (list (plist-get (rdm-context) :rdm-bin) kind field)))
    (pcase (gethash key rdm--enum-cache 'unset)
      ('unset
       (let* ((fields (plist-get (rdm-call-sync 'describe (symbol-name kind))
                                 :fields))
              (found (seq-find (lambda (f) (equal (plist-get f :name) field))
                               fields))
              (values (append (plist-get found :enum_values) nil)))
         (puthash key values rdm--enum-cache)))
      (cached cached))))

(defun rdm-status-values (kind)
  "The status values rdm declares for KIND (`phase' or `task'), in order.
The `status' case of `rdm-enum-values'.  nil for a kind without a status."
  (rdm-enum-values kind "status"))

(defun rdm-worktree-item ()
  "The rdm item whose worktree holds `default-directory', or nil."
  (rdm-call-sync 'worktree-current))

(provide 'rdm-core)
;;; rdm-core.el ends here
