;;; rdm-transient-test.el --- ERT tests for rdm-dispatch  -*- lexical-binding: t; -*-

;;; Commentary:

;; The write surface: status and tag writes, the plan-repo lifecycle and
;; session scoping.  Most tests run against an `rdm' shell stub that logs
;; each invocation's `RDM_SESSION' and argv; the `real-' test drives the
;; binary under test against the seeded plan repo.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'subr-x)
(require 'rdm-core)
(require 'rdm-view)
(require 'rdm-pick)
(require 'rdm-transient)
(require 'rdm-evil)
(require 'rdm-test-helpers)
(require 'rdm-transient-stub)

(defvar evil-state)
(declare-function evil-local-mode "ext:evil-core" (&optional arg))

;;;; Status

(ert-deftest rdm-transient/status-argv ()
  "A status write is an update with --status and --no-edit; `blocked'
adds --reason, read from the minibuffer when not given."
  (rdm-transient-test--with-stub nil
    (cl-letf (((symbol-function 'y-or-n-p) (lambda (&rest _) t))
              ((symbol-function 'read-string) (lambda (&rest _) "read reason")))
      (rdm-set-status rdm-transient-test--phase "in-progress")
      (rdm-set-status rdm-transient-test--phase "blocked" "waiting on x")
      (rdm-set-status rdm-transient-test--phase "blocked")
      (rdm-set-status rdm-transient-test--task "done"))
    (should (equal (rdm-transient-test--argvs log)
                   '(("phase" "update" "phase-2-two" "--roadmap=r1"
                      "--status=in-progress" "--project" "p" "--no-edit")
                     ("phase" "update" "phase-2-two" "--roadmap=r1"
                      "--status=blocked" "--reason=waiting on x"
                      "--project" "p" "--no-edit")
                     ("phase" "update" "phase-2-two" "--roadmap=r1"
                      "--status=blocked" "--reason=read reason"
                      "--project" "p" "--no-edit")
                     ("task" "update" "t1" "--status=done" "--project" "p"
                      "--no-edit"))))
    (should-error (rdm-set-status '(:kind roadmap :roadmap "r1") "done")
                  :type 'user-error)))

(ert-deftest rdm-transient/confirm-declined-spawns-nothing ()
  "Declining the confirmation for `done', `wont-fix' or discard runs no rdm."
  (rdm-transient-test--with-stub nil
    (cl-letf (((symbol-function 'y-or-n-p) (lambda (&rest _) nil)))
      (should-not (rdm-set-status rdm-transient-test--task "done"))
      (should-not (rdm-set-status rdm-transient-test--phase "wont-fix"))
      (should-not (rdm-plan-discard)))
    (should-not (rdm-transient-test--calls log))))

(ert-deftest rdm-transient/refusal-verbatim ()
  "A write rdm refuses signals an error carrying rdm's stderr line unchanged."
  (let ((line "error: failed to update task: refusing to mark task/t1 reviewed: \
no approving review"))
    (rdm-transient-test--with-stub (list :refuse line)
      (let ((err (should-error (rdm-set-status rdm-transient-test--task "reviewed")
                               :type 'user-error)))
        (should (string-search line (error-message-string err)))))))

(ert-deftest rdm-transient/statuses-from-describe ()
  "The Status suffixes are the values `rdm describe' declares, an extra
one included, read once per kind; a roadmap has none."
  (rdm-transient-test--with-stub (list :extra ",\"parked\"")
    (cl-letf (((symbol-function 'transient-scope)
               (lambda (&rest _) rdm-transient-test--phase)))
      (let ((suffixes (rdm-dispatch--status-children nil)))
        (should (equal (mapcar (lambda (s) (plist-get (cdr s) :description))
                               suffixes)
                       '("not-started" "in-progress" "done" "blocked"
                         "wont-fix" "parked")))
        (let ((keys (mapcar (lambda (s) (plist-get (cdr s) :key)) suffixes)))
          (should (equal keys (seq-uniq keys))))))
    (rdm-dispatch--status-specs rdm-transient-test--phase)
    (should (equal (rdm-transient-test--argvs log)
                   '(("describe" "phase" "--format" "json"))))
    (should-not (rdm-dispatch--status-specs '(:kind roadmap :roadmap "r1")))))

(ert-deftest rdm-transient/refreshes-view ()
  "A successful write re-fetches the detail buffer showing the item; a
phase write also re-fetches its roadmap's view (which lists the phase),
but not a historical `--at' view of the phase nor another roadmap's view."
  (rdm-transient-test--with-stub nil
    (let ((buf (rdm-open-item rdm-transient-test--task)))
      (should (buffer-live-p buf))
      (rdm-set-status rdm-transient-test--task "in-progress")
      (should (equal (last (rdm-transient-test--argvs log) 2)
                     '(("task" "update" "t1" "--status=in-progress"
                        "--project" "p" "--no-edit")
                       ("task" "show" "t1" "--project" "p"
                        "--format" "json")))))
    (rdm-open-item '(:kind roadmap :roadmap "r1"))
    (rdm-open-item '(:kind roadmap :roadmap "r2"))
    (rdm-open-item rdm-transient-test--phase)
    (rdm-open-item rdm-transient-test--phase "abc123")
    (should (seq-find (lambda (argv) (member "--at=abc123" argv))
                      (rdm-transient-test--argvs log)))
    (let ((before (length (rdm-transient-test--argvs log))))
      (rdm-set-status rdm-transient-test--phase "in-progress")
      (let* ((after (nthcdr before (rdm-transient-test--argvs log)))
             (shows (cdr after)))
        (should (equal (car after)
                       '("phase" "update" "phase-2-two" "--roadmap=r1"
                         "--status=in-progress" "--project" "p" "--no-edit")))
        (should (member '("roadmap" "show" "r1" "--project" "p"
                          "--format" "json")
                        shows))
        (should (member '("phase" "show" "phase-2-two" "--roadmap=r1"
                          "--project" "p" "--format" "json")
                        shows))
        (should-not (seq-find (lambda (argv) (member "r2" argv)) shows))
        (should-not (seq-find (lambda (argv)
                                (seq-find (lambda (a) (string-prefix-p "--at" a))
                                          argv))
                              shows))))))

;;;; Tags

(ert-deftest rdm-transient/tags-keep-untouched ()
  "A tag edit re-reads the tags and writes the whole list back, keeping
`needs-plan-review'."
  (rdm-transient-test--with-stub (list :tags "[\"needs-plan-review\",\"bug\"]")
    (should (equal (rdm-edit-tags rdm-transient-test--task '("ui") '("bug"))
                   '("needs-plan-review" "ui")))
    (rdm-edit-tags '(:kind roadmap :roadmap "r1") '("ui") nil)
    (should (equal (rdm-transient-test--argvs log)
                   '(("task" "show" "t1" "--project" "p" "--format" "json")
                     ("task" "update" "t1" "--tags=needs-plan-review,ui"
                      "--project" "p" "--no-edit")
                     ("roadmap" "show" "r1" "--project" "p" "--format" "json")
                     ("roadmap" "update" "r1"
                      "--tags=needs-plan-review,bug,ui"
                      "--project" "p" "--no-edit"))))))

(ert-deftest rdm-transient/tags-refuse-empty ()
  "Removing the last tag is refused and writes nothing."
  (rdm-transient-test--with-stub (list :tags "[\"bug\"]")
    (should-error (rdm-edit-tags rdm-transient-test--task nil '("bug"))
                  :type 'user-error)
    (should-not (seq-find (lambda (argv) (equal (cadr argv) "update"))
                          (rdm-transient-test--argvs log)))))

(ert-deftest rdm-transient/tags-noop-on-tagless ()
  "An edit adding nothing to an item with no tags is a quiet no-op."
  (rdm-transient-test--with-stub (list :tags "[]")
    (should-not (rdm-edit-tags rdm-transient-test--task nil nil))
    (should-not (rdm-edit-tags rdm-transient-test--task '("  ") '("bug")))
    (should-not (seq-find (lambda (argv) (equal (cadr argv) "update"))
                          (rdm-transient-test--argvs log)))))

;;;; Plan repo

(ert-deftest rdm-transient/plan-repo-argv ()
  "status, commit and discard take neither --project nor --format."
  (rdm-transient-test--with-stub nil
    (cl-letf (((symbol-function 'y-or-n-p) (lambda (&rest _) t)))
      (should (string-match-p "Uncommitted" (rdm-plan-status)))
      (rdm-plan-commit "-plan: msg")
      (rdm-plan-discard))
    (should-error (rdm-plan-commit "  ") :type 'user-error)
    (should (equal (rdm-transient-test--argvs log)
                   '(("status") ("commit" "--message=-plan: msg")
                     ("discard" "--force"))))))

;;;; Session

(ert-deftest rdm-transient/session-on-every-process ()
  "Every process, `rdm info' included, carries the package's session id,
not the inherited one; a set `rdm-session' is used as given."
  (rdm-transient-test--with-stub nil
    (cl-letf (((symbol-function 'y-or-n-p) (lambda (&rest _) t)))
      (rdm-set-status rdm-transient-test--task "in-progress")
      (rdm-edit-tags rdm-transient-test--task '("ui") nil)
      (rdm-plan-commit "msg"))
    (let ((calls (rdm-transient-test--calls log))
          (id (rdm-session-id)))
      (should (member "info" (mapcar #'cadr calls)))
      (should (> (length calls) 3))
      (should (rdm--nonempty id))
      (should-not (equal id "inherited-other"))
      (dolist (c calls) (should (equal (car c) id))))
    (let ((rdm-session "my-session.1"))
      (rdm-plan-status)
      (should (equal (car (car (last (rdm-transient-test--calls log))))
                     "my-session.1")))))

(ert-deftest rdm-transient/session-passed-verbatim ()
  "A set `rdm-session' reaches rdm unchanged, even one rdm will sanitize:
the package leaves session-id rules to core."
  (rdm-transient-test--with-stub nil
    (let ((rdm-session "my session"))
      (rdm-plan-status))
    (let ((calls (rdm-transient-test--calls log)))
      (should calls)
      (dolist (c calls) (should (equal (car c) "my session"))))))

;;;; Entry points

(ert-deftest rdm-transient/entry-points ()
  "`x' in a detail buffer and in the embark map opens `rdm-dispatch'."
  (with-temp-buffer
    (rdm-view-mode)
    (should (eq (key-binding (kbd "x")) #'rdm-dispatch)))
  (should (eq (keymap-lookup rdm-item-embark-map "x") #'rdm-item-dispatch))
  (should (commandp #'rdm-dispatch)))

(ert-deftest rdm-evil/x-opens-dispatch ()
  "Under evil, `x' in a detail buffer's normal state is `rdm-dispatch'."
  :tags '(evil)
  (rdm-test-with-soft-dep 'evil
    (with-temp-buffer
      (rdm-view-mode)
      (evil-local-mode 1)
      (should (eq evil-state 'normal))
      (should (eq (key-binding (kbd "x")) #'rdm-dispatch))
      (evil-local-mode -1))))

;;;; Against the real binary

(ert-deftest rdm-transient/real-status-tags-commit ()
  "A status and a tag edit round-trip through the real binary, are listed
staged in this session, and commit."
  (rdm-test-with-real
    (setq process-environment
          (append (list (concat "RDM_ROOT=" root) (concat "RDM_PROJECT=" project))
                  process-environment))
    (setq rdm-bin bin)
    (let ((rdm-session "emacs-real-dispatch-test")
          (item '(:kind phase :roadmap "sample-roadmap" :stem "phase-3-seed-three")))
      (cl-letf (((symbol-function 'y-or-n-p) #'rdm-transient-test--refuse-prompts)
                ((symbol-function 'read-string) #'rdm-transient-test--refuse-prompts))
        (should (member "blocked" (rdm-status-values 'phase)))
        (rdm-set-status item "blocked" "waiting on the emacs suite")
        (rdm-edit-tags item '("emacs") nil))
      (let ((shown (rdm-call-sync 'phase-show "phase-3-seed-three"
                                  :roadmap "sample-roadmap")))
        (should (equal (plist-get shown :status) "blocked"))
        (should (equal (append (plist-get shown :tags) nil) '("emacs"))))
      (should (string-match-p "phase-3-seed-three" (rdm-plan-status)))
      (should (string-match-p "Committed" (rdm-plan-commit "emacs: dispatch")))
      (should (string-match-p "No uncommitted changes" (rdm-plan-status))))))

(provide 'rdm-transient-test)
;;; rdm-transient-test.el ends here
