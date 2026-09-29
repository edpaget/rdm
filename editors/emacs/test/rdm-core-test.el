;;; rdm-core-test.el --- ERT tests for rdm-core  -*- lexical-binding: t; -*-

;;; Commentary:

;; Unit tests run against generated `rdm' shell stubs.  The `real-' tests
;; run against the binary under test and a seeded plan repo, which the
;; suite driver provides through RDM_TEST_BIN / RDM_TEST_ROOT /
;; RDM_TEST_PROJECT; without them they skip.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'subr-x)
(require 'rdm-core)
(require 'rdm-test-helpers)

(defconst rdm-core-test--package-dir
  (file-name-as-directory
   (expand-file-name ".." (file-name-directory
                           (or load-file-name buffer-file-name))))
  "The package source directory.")

;;;; Stubs for `rdm info'

(defun rdm-core-test--info-stub (log &optional info-root info-project)
  "A stub answering `info' with INFO-ROOT and INFO-PROJECT (omitted if nil).
Like the real rdm, it reports `RDM_ROOT' / `RDM_PROJECT' from its
environment when set.  Each call appends \"RDM_ROOT|RDM_PROJECT\" to LOG.
`worktree current' prints null."
  (rdm-test-stub
   (format "echo \"$RDM_ROOT|$RDM_PROJECT\" >> '%s'
case \"$1\" in
  info)
    root=\"${RDM_ROOT:-%s}\"; project=\"${RDM_PROJECT:-%s}\"
    printf '{'
    [ -n \"$root\" ] && printf '\"root\":\"%%s\",' \"$root\"
    [ -n \"$project\" ] && printf '\"project\":\"%%s\",' \"$project\"
    printf '\"default_branch\":\"trunk\",\"default_format\":\"json\"}\\n' ;;
  worktree) echo null ;;
  *) echo \"unexpected $*\" >&2; exit 9 ;;
esac"
           log (or info-root "") (or info-project ""))))

(defun rdm-core-test--log-lines (log)
  "The lines appended to LOG so far."
  (if (file-exists-p log)
      (with-temp-buffer
        (insert-file-contents log)
        (split-string (buffer-string) "\n" t))
    nil))

(defmacro rdm-core-test--in-sandbox (&rest body)
  "Run BODY with `default-directory' a fresh non-project temp directory.
Binds `dirs' to three existing directories and `log' to a log file."
  (declare (indent 0) (debug t))
  `(let* ((default-directory (file-name-as-directory (make-temp-file "rdm-cwd" t)))
          (dirs (list (make-temp-file "rdm-root-a" t)
                      (make-temp-file "rdm-root-b" t)
                      (make-temp-file "rdm-root-c" t)))
          (log (make-temp-file "rdm-log")))
     (ignore dirs log)
     ,@body))

;;;; Resolver precedence

(ert-deftest rdm-context/precedence-root ()
  "Root: override > env > rdm info > actionable error."
  (rdm-core-test--in-sandbox
    (pcase-let ((`(,a ,b ,c) dirs))
      (dolist (row `((:override ,a :env ,b :info ,c :want ,a)
                     (:override nil :env ,b :info ,c :want ,b)
                     (:override nil :env nil :info ,c :want ,c)
                     (:override nil :env nil :info nil :want error)))
        (let ((stub (rdm-core-test--info-stub log (plist-get row :info) "p")))
          (rdm-test-with-env
              (append (list (concat "RDM_BIN=" stub))
                      (when (plist-get row :env)
                        (list (concat "RDM_ROOT=" (plist-get row :env)))))
            (setq rdm-root (plist-get row :override))
            (if (eq (plist-get row :want) 'error)
                (should (string-match-p
                         "\\.mise\\.toml"
                         (cadr (should-error (rdm-context) :type 'user-error))))
              (should (equal (plist-get (rdm-context) :root)
                             (expand-file-name (plist-get row :want)))))))))))

(ert-deftest rdm-context/precedence-project ()
  "Project: override > env > rdm info > actionable error."
  (rdm-core-test--in-sandbox
    (dolist (row '((:override "po" :env "pe" :info "pi" :want "po")
                   (:override nil :env "pe" :info "pi" :want "pe")
                   (:override nil :env nil :info "pi" :want "pi")
                   (:override nil :env nil :info nil :want error)))
      (let ((stub (rdm-core-test--info-stub log (car dirs) (plist-get row :info))))
        (rdm-test-with-env
            (append (list (concat "RDM_BIN=" stub))
                    (when (plist-get row :env)
                      (list (concat "RDM_PROJECT=" (plist-get row :env)))))
          (setq rdm-project (plist-get row :override))
          (if (eq (plist-get row :want) 'error)
              (let ((msg (cadr (should-error (rdm-context) :type 'user-error))))
                (should (string-match-p "\\.mise\\.toml" msg))
                (should (string-match-p "rdm config set" msg)))
            (should (equal (plist-get (rdm-context) :project)
                           (plist-get row :want)))))))))

(ert-deftest rdm-context/precedence-bin ()
  "Binary: override > RDM_BIN > `executable-find' > actionable error."
  (rdm-core-test--in-sandbox
    (let* ((s1 (rdm-core-test--info-stub log (car dirs) "p"))
           (s2 (rdm-core-test--info-stub log (car dirs) "p"))
           (s3 (rdm-core-test--info-stub log (car dirs) "p")))
      (dolist (row `((:override ,s1 :env ,s2 :path ,s3 :want ,s1)
                     (:override nil :env ,s2 :path ,s3 :want ,s2)
                     (:override nil :env nil :path ,s3 :want ,s3)
                     (:override nil :env nil :path nil :want error)))
        (rdm-test-with-env
            (when (plist-get row :env)
              (list (concat "RDM_BIN=" (plist-get row :env))))
          (let ((exec-path (when (plist-get row :path)
                             (list (file-name-directory (plist-get row :path))))))
            (setq rdm-bin (plist-get row :override))
            (if (eq (plist-get row :want) 'error)
                (should-error (rdm-context) :type 'user-error)
              (should (equal (plist-get (rdm-context) :rdm-bin)
                             (plist-get row :want))))))))))

(ert-deftest rdm-context/mise-env ()
  "The mise.el case: env supplies root and project, rdm info the branch.
rdm info runs with the env-resolved RDM_ROOT / RDM_PROJECT."
  (rdm-core-test--in-sandbox
    (let ((stub (rdm-core-test--info-stub log (nth 2 dirs) "info-proj")))
      (rdm-test-with-env (list (concat "RDM_BIN=" stub)
                               (concat "RDM_ROOT=" (car dirs))
                               "RDM_PROJECT=mise-proj")
        (let ((ctx (rdm-context)))
          (should (equal (plist-get ctx :root) (expand-file-name (car dirs))))
          (should (equal (plist-get ctx :project) "mise-proj"))
          (should (equal (plist-get ctx :default-branch) "trunk"))
          (should (equal (rdm-core-test--log-lines log)
                         (list (concat (car dirs) "|mise-proj")))))))))

(ert-deftest rdm-context/override-reaches-info ()
  "rdm info receives an override-resolved root, not the env one."
  (rdm-core-test--in-sandbox
    (let ((stub (rdm-core-test--info-stub log nil "p")))
      (rdm-test-with-env (list (concat "RDM_BIN=" stub)
                               (concat "RDM_ROOT=" (nth 1 dirs)))
        (setq rdm-root (car dirs))
        (rdm-context)
        (should (equal (rdm-core-test--log-lines log)
                       (list (concat (car dirs) "|"))))))))

(ert-deftest rdm-context/missing-root-dir ()
  "A root that does not exist on disk is an actionable error."
  (rdm-core-test--in-sandbox
    (let ((stub (rdm-core-test--info-stub log "/nonexistent/rdm-plan" "p")))
      (rdm-test-with-env (list (concat "RDM_BIN=" stub))
        (should (string-match-p
                 "does not exist"
                 (cadr (should-error (rdm-context) :type 'user-error))))))))

(ert-deftest rdm-context/info-fails ()
  "A failing rdm info becomes the actionable error carrying its stderr."
  (rdm-core-test--in-sandbox
    (let ((stub (rdm-test-stub "echo 'config is broken' >&2; exit 2")))
      (rdm-test-with-env (list (concat "RDM_BIN=" stub))
        (let ((msg (cadr (should-error (rdm-context) :type 'user-error))))
          (should (string-match-p "\\.mise\\.toml" msg))
          (should (string-match-p "config is broken" msg)))))))

(ert-deftest rdm-context/cache ()
  "A second call is served from the cache; a changed override is not."
  (rdm-core-test--in-sandbox
    (let ((stub (rdm-core-test--info-stub log (car dirs) "p")))
      (rdm-test-with-env (list (concat "RDM_BIN=" stub))
        (rdm-context)
        (rdm-context)
        (should (= 1 (length (rdm-core-test--log-lines log))))
        (setq rdm-project "other")
        (should (equal (plist-get (rdm-context) :project) "other"))
        (should (= 2 (length (rdm-core-test--log-lines log))))))))

(ert-deftest rdm-context/reset ()
  "`rdm-context-reset' makes the next call run rdm info again."
  (rdm-core-test--in-sandbox
    (let ((stub (rdm-core-test--info-stub log (car dirs) "p")))
      (rdm-test-with-env (list (concat "RDM_BIN=" stub))
        (rdm-context)
        (rdm-context-reset)
        (rdm-context)
        (should (= 2 (length (rdm-core-test--log-lines log))))))))

(ert-deftest rdm-worktree-item/null ()
  "Off an item, `rdm worktree current' prints null and the result is nil."
  (rdm-core-test--in-sandbox
    (let ((stub (rdm-core-test--info-stub log (car dirs) "p")))
      (rdm-test-with-env (list (concat "RDM_BIN=" stub))
        (should (null (rdm-worktree-item)))))))

(ert-deftest rdm-context/bin-not-dir-local-safe ()
  "A directory-local `rdm-bin' names a program to run, so it is not safe;
the root and project are only passed to rdm and stay safe."
  (should-not (safe-local-variable-p 'rdm-bin "/tmp/x"))
  (should (safe-local-variable-p 'rdm-root "/tmp/plan"))
  (should (safe-local-variable-p 'rdm-project "p")))

;;;; Argv builder

(ert-deftest rdm-argv/golden ()
  "Reads get --format json, writes --no-edit, text actions neither."
  (dolist (row rdm--actions)
    (let* ((action (car row))
           (argv (car (rdm-argv action "x" :project "p")))
           (scoped (if (plist-member (cdr row) :project)
                       (plist-get (cdr row) :project)
                     t)))
      (pcase (plist-get (cdr row) :kind)
        ('read
         (should (rdm-action-read-p action))
         (should (equal (last argv 2) '("--format" "json")))
         (should-not (member "--no-edit" argv)))
        ('write
         (should-not (rdm-action-read-p action))
         (should (equal (last argv) '("--no-edit")))
         (should-not (member "--format" argv)))
        ('text
         (should-not (rdm-action-read-p action))
         (should-not (member "--no-edit" argv))
         (should-not (member "--format" argv)))
        (kind (ert-fail (format "%s has unknown kind %s" action kind))))
      (should (eq (and (member "--project" argv) t) scoped))))
  (should (equal (rdm-argv 'task-update "fixture-task-open"
                           :status "done" :project "p")
                 '(("task" "update" "fixture-task-open" "--status=done"
                    "--project" "p" "--no-edit"))))
  (should (equal (rdm-argv 'task-show "t" :project "p")
                 '(("task" "show" "t" "--project" "p" "--format" "json"))))
  (should (equal (rdm-argv 'task-create "t" :title "T" :stdin "body\n"
                           :project "p")
                 '(("task" "create" "t" "--title=T" "--project" "p"
                    "--no-edit")
                   . "body\n")))
  (should (equal (rdm-argv 'task-update "t" :body "b" :project "p")
                 '(("task" "update" "t" "--body=b" "--project" "p"
                    "--no-edit"))))
  ;; Values starting with `-' stay attached to their flag, so clap never
  ;; reads them as options.
  (should (equal (rdm-argv 'task-update "t" :title "-T" :body "- item\n"
                           :project "p")
                 '(("task" "update" "t" "--title=-T" "--body=- item\n"
                    "--project" "p" "--no-edit"))))
  ;; A `t' value is a bare flag; a nil value drops the flag entirely.
  (should (equal (rdm-argv 'task-update "t" :clear-body t :project "p")
                 '(("task" "update" "t" "--clear-body" "--project" "p"
                    "--no-edit"))))
  (should (equal (rdm-argv 'task-update "t" :status nil :title "T"
                           :project "p")
                 '(("task" "update" "t" "--title=T" "--project" "p"
                    "--no-edit"))))
  (should (equal (rdm-argv 'task-list :status nil :project "p")
                 '(("task" "list" "--project" "p" "--format" "json"))))
  (should (equal (rdm-argv 'task-list :status "all" :project "p")
                 '(("task" "list" "--status=all" "--project" "p"
                    "--format" "json"))))
  ;; `status', `commit' and `discard' print text and take no --project.
  (should (equal (rdm-argv 'status :project "p") '(("status"))))
  (should (equal (rdm-argv 'commit :message "-m msg" :project "p")
                 '(("commit" "--message=-m msg"))))
  (should (equal (rdm-argv 'discard :force t :project "p")
                 '(("discard" "--force"))))
  (should (equal (rdm-argv 'describe "task" :project "p")
                 '(("describe" "task" "--format" "json"))))
  ;; `hook done-line' prints plain text and rejects --project.
  (should (equal (rdm-argv 'done-line :task "t" :project "p")
                 '(("hook" "done-line" "--task=t"))))
  (should (equal (rdm-argv 'done-line :roadmap "r" :phase "phase-1-x")
                 '(("hook" "done-line" "--roadmap=r" "--phase=phase-1-x"))))
  ;; Reviews: reads take --format json; `review update' returns the
  ;; updated review as JSON too.
  (should (equal (rdm-argv 'review-list :on "task/t" :project "p")
                 '(("review" "list" "--on=task/t" "--project" "p"
                    "--format" "json"))))
  (should (equal (rdm-argv 'review-show "r1" :project "p")
                 '(("review" "show" "r1" "--project" "p" "--format" "json"))))
  (should (equal (rdm-argv 'review-requests :project "p")
                 '(("review" "requests" "--project" "p" "--format" "json"))))
  (should (equal (rdm-argv 'review-update "r1" :comment 2 :status "addressed"
                           :reply "-- fixed" :applied-commit "abc123"
                           :project "p")
                 '(("review" "update" "r1" "--comment=2" "--status=addressed"
                    "--reply=-- fixed" "--applied-commit=abc123"
                    "--project" "p" "--format" "json"))))
  (should (equal (rdm-argv 'review-update "r1" :state "dismissed" :no-edit t
                           :project "p")
                 '(("review" "update" "r1" "--state=dismissed" "--no-edit"
                    "--project" "p" "--format" "json"))))
  ;; Authoring: `review start' is read as JSON (and passes --no-edit
  ;; explicitly); comment and submit are writes.  A quote starting with
  ;; `-' stays joined to its flag.
  (should (equal (rdm-argv 'review-start :on "task/t" :author "me" :no-edit t
                           :project "p")
                 '(("review" "start" "--on=task/t" "--author=me" "--no-edit"
                    "--project" "p" "--format" "json"))))
  (should (equal (rdm-argv 'review-comment "r1" :quote "- item" :occurrence 2
                           :body "b" :project "p")
                 '(("review" "comment" "r1" "--quote=- item" "--occurrence=2"
                    "--body=b" "--project" "p" "--no-edit"))))
  (should (equal (rdm-argv 'review-submit "r1" :verdict "approve" :project "p")
                 '(("review" "submit" "r1" "--verdict=approve"
                    "--project" "p" "--no-edit")))))

(ert-deftest rdm-argv/body-stdin-exclusive ()
  "Both :body and :stdin, a missing project or an unknown action errors."
  (should-error (rdm-argv 'task-create "t" :body "b" :stdin "s" :project "p"))
  (should-error (rdm-argv 'task-show "t"))
  (should-error (rdm-argv 'no-such-action :project "p")))

;;;; Runner

(ert-deftest rdm-run/nonzero-exit ()
  "A non-zero exit is a `user-error' carrying stderr, sync and async."
  (let ((stub (rdm-test-stub "echo boom >&2; exit 3")))
    (should (string-match-p
             "boom"
             (cadr (should-error (rdm-run-sync (list stub "x") :parse t
                                               :timeout 10)
                                 :type 'user-error))))
    (let (got)
      (rdm-run (list stub "x") :callback (lambda (_) (setq got 'ok))
               :on-error (lambda (e) (setq got e)))
      (with-timeout (10 (error "timed out"))
        (while (null got) (accept-process-output nil 0.05)))
      (should (eq (car got) 'user-error))
      (should (string-match-p "boom" (cadr got))))))

(ert-deftest rdm-run/early-exit-large-stdin ()
  "rdm exiting before it reads a stdin body larger than the pipe buffer
still reports its exit status and stderr, not a broken pipe."
  (let ((stub (rdm-test-stub "echo 'refused: bad arguments' >&2; exit 4"))
        (body (make-string (* 1024 1024) ?x)))
    (let ((msg (cadr (should-error (rdm-run-sync (list stub "x") :stdin body
                                                 :timeout 10)
                                   :type 'user-error))))
      (should (string-match-p "refused: bad arguments" msg))
      (should (string-match-p "exit 4" msg)))))

(ert-deftest rdm-run/stderr-noise ()
  "Lots of stderr alongside valid JSON on stdout still parses."
  (let ((stub (rdm-test-stub "i=0
while [ $i -lt 2000 ]; do echo \"warning $i: plan repo is dirty\" >&2; i=$((i+1)); done
echo '{\"slug\":\"s\",\"done\":false,\"parent\":null}'")))
    (should (equal (rdm-run-sync (list stub) :parse t :timeout 10)
                   '(:slug "s" :done :false :parent nil)))))

(ert-deftest rdm-run/malformed-json ()
  "Malformed JSON is an error naming the first stdout line."
  (let ((stub (rdm-test-stub "echo 'not json'; echo 'second line'")))
    (let ((err (should-error (rdm-run-sync (list stub) :parse t :timeout 10))))
      (should (string-match-p "not json" (error-message-string err)))
      (should-not (string-match-p "second line" (error-message-string err))))))

(ert-deftest rdm-run/write-returns-text ()
  "A write's stdout comes back verbatim, unparsed; stdin reaches rdm."
  (let ((stub (rdm-test-stub "echo 'Updated task'; cat")))
    (should (equal (rdm-run-sync (list stub) :stdin "{\"a\":1}\n" :timeout 10)
                   "Updated task\n{\"a\":1}\n"))))

(ert-deftest rdm-run/closes-stdin ()
  "A stub that reads stdin to EOF finishes, with and without a body."
  (let ((stub (rdm-test-stub "cat >/dev/null; echo '{}'")))
    (should (null (rdm-run-sync (list stub) :parse t :timeout 5)))
    (should (null (rdm-run-sync (list stub) :parse t :stdin "body" :timeout 5)))))

(ert-deftest rdm-run/pipe-not-pty ()
  "rdm's stdin and stdout are pipes, not a terminal."
  (let ((stub (rdm-test-stub "if [ -t 0 ] || [ -t 1 ]; then echo '{\"tty\":true}'; \
else echo '{\"tty\":false}'; fi")))
    (should (equal (rdm-run-sync (list stub) :parse t :timeout 5)
                   '(:tty :false)))))

(ert-deftest rdm-run/timeout ()
  "`rdm-run-sync' gives up after its timeout instead of hanging."
  (let ((stub (rdm-test-stub "sleep 30")))
    (should-error (rdm-run-sync (list stub) :timeout 0.5))))

;;;; Against the real binary

(ert-deftest rdm-context/real-info ()
  "With RDM_ROOT / RDM_PROJECT in the environment, rdm info supplies the branch."
  (rdm-test-with-real
    (setq process-environment
          (append (list (concat "RDM_ROOT=" root) (concat "RDM_PROJECT=" project))
                  process-environment))
    (setq rdm-bin bin)
    (let ((ctx (rdm-context)))
      (should (equal (plist-get ctx :root) (expand-file-name root)))
      (should (equal (plist-get ctx :project) project))
      (should (equal (plist-get ctx :default-branch) "main"))
      (should (equal (plist-get ctx :rdm-bin) bin)))))

(ert-deftest rdm-context/real-no-env ()
  "With no RDM_* and no global config, even inside the plan repo, the
resolver signals the actionable error: rdm never resolves a root from cwd."
  (rdm-test-with-real
    (let ((process-environment
           (cons (concat "XDG_CONFIG_HOME=" (make-temp-file "rdm-xdg" t))
                 process-environment)))
      (setq rdm-bin bin)
      (let ((msg (cadr (should-error (rdm-context) :type 'user-error))))
        (should (string-match-p "\\.mise\\.toml" msg))
        (should (string-match-p "rdm config set" msg))))))

(ert-deftest rdm-context/real-global-config ()
  "With no RDM_*, a global config naming the plan resolves everything via rdm info."
  (rdm-test-with-real
    (let* ((xdg (make-temp-file "rdm-xdg" t))
           (process-environment (cons (concat "XDG_CONFIG_HOME=" xdg)
                                      process-environment)))
      (make-directory (expand-file-name "rdm" xdg))
      (with-temp-file (expand-file-name "rdm/config.toml" xdg)
        (insert (format "root = %S\n" root)))
      (setq rdm-bin bin)
      (let ((ctx (rdm-context)))
        (should (equal (file-truename (plist-get ctx :root)) (file-truename root)))
        (should (equal (plist-get ctx :project) project))
        (should (equal (plist-get ctx :default-branch) "main"))))))

(ert-deftest rdm-run/real-read ()
  "A real read parses to a plist, despite rdm's stderr hints."
  (rdm-test-with-real
    (setq process-environment
          (append (list (concat "RDM_ROOT=" root) (concat "RDM_PROJECT=" project))
                  process-environment))
    (setq rdm-bin bin)
    (let ((task (rdm-call-sync 'task-show "fixture-task-open")))
      (should (equal (plist-get task :slug) "fixture-task-open")))))

(ert-deftest rdm-run/real-write ()
  "A real write goes through --no-edit with stdin closed and EDITOR=false."
  (rdm-test-with-real
    (setq process-environment
          (append (list (concat "RDM_ROOT=" root) (concat "RDM_PROJECT=" project)
                        "EDITOR=false" "VISUAL=false")
                  process-environment))
    (setq rdm-bin bin)
    (let ((out (rdm-call-sync 'task-update "fixture-task-active"
                              :status "open")))
      (should (stringp out)))
    (should (equal (plist-get (rdm-call-sync 'task-show "fixture-task-active")
                              :status)
                   "open"))))

(ert-deftest rdm-run/real-write-dash-values ()
  "A title and a body starting with `-' reach the real binary intact."
  (rdm-test-with-real
    (setq process-environment
          (append (list (concat "RDM_ROOT=" root) (concat "RDM_PROJECT=" project)
                        "EDITOR=false" "VISUAL=false")
                  process-environment))
    (setq rdm-bin bin)
    (rdm-call-sync 'task-update "fixture-task-open"
                   :title "-dash title" :body "- item one\n- item two")
    (let ((task (rdm-call-sync 'task-show "fixture-task-open")))
      (should (equal (plist-get task :title) "-dash title"))
      (should (string-match-p "\\`- item one\n- item two"
                              (plist-get task :body))))))

;;;; Soft dependency proof

(ert-deftest rdm-evil/smoke ()
  "Runs only in the init-directory pass, where evil is loaded."
  :tags '(evil)
  (rdm-test-with-soft-dep 'evil
    (should (fboundp 'evil-define-key*))))

;;;; Dev reload

(ert-deftest rdm-dev/reload ()
  "`rdm-dev-reload' re-initializes changed variables and clears the cache."
  (require 'rdm-dev)
  (let ((saved-root (default-value 'rdm-root))
        ;; Reload re-creates `rdm--context-cache', so a sentinel put in
        ;; beforehand would vanish with or without a reset.  Seed the
        ;; fresh table after every file load instead: only the reset that
        ;; follows the loads can empty it.
        (seed (lambda (&rest _)
                (when (boundp 'rdm--context-cache)
                  (puthash 'sentinel t rdm--context-cache)))))
    (unwind-protect
        (progn
          (setq-default rdm-root "/sentinel")
          (setq-default rdm-sync-timeout 999)
          (advice-add 'load :after seed)
          (let ((inhibit-message t))
            (rdm-dev-reload))
          (advice-remove 'load seed)
          (should (null (default-value 'rdm-root)))
          (should (= rdm-sync-timeout 30))
          (should (= 0 (hash-table-count rdm--context-cache))))
      (advice-remove 'load seed)
      (setq-default rdm-root saved-root))))

(ert-deftest rdm-dev/not-shipped ()
  "rdm-dev gets no autoload, and loading every shipped file leaves it unloaded."
  (let ((out (make-temp-file "rdm-loaddefs" nil ".el"))
        (found nil))
    (delete-file out)
    (let ((inhibit-message t))
      (loaddefs-generate rdm-core-test--package-dir out))
    (when (file-exists-p out)
      (with-temp-buffer
        (insert-file-contents out)
        (goto-char (point-min))
        (let (form)
          (while (setq form (condition-case nil (read (current-buffer))
                              (end-of-file nil)))
            (when (and (eq (car-safe form) 'autoload)
                       (equal (cadr form) ''rdm-dev-reload))
              (setq found t))))))
    (should-not found))
  (let* ((features
          (cl-loop for f in (directory-files rdm-core-test--package-dir nil
                                             "\\`rdm-.*\\.el\\'")
                   for feat = (file-name-sans-extension f)
                   unless (equal feat "rdm-dev") collect feat))
         (form `(progn
                  (dolist (f ',features) (require (intern f)))
                  (princ (if (featurep 'rdm-dev) "loaded" "absent"))))
         (emacs (expand-file-name invocation-name invocation-directory)))
    (should features)
    (with-temp-buffer
      (should (eq 0 (call-process emacs nil t nil "-Q" "--batch"
                                  "-L" rdm-core-test--package-dir
                                  "--eval" (prin1-to-string form))))
      (should (string-suffix-p "absent" (string-trim (buffer-string)))))))

(provide 'rdm-core-test)
;;; rdm-core-test.el ends here
