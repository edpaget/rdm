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
  "A status write is an update with --status and --no-edit; a status rdm
declares a note flag for (the stub: `blocked' → reason) adds that flag,
read from the minibuffer when not given."
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
  "Declining the confirmation for a terminal status rdm declares (the
stub: `done', `wont-fix') or for discard writes nothing."
  (rdm-transient-test--with-stub nil
    (cl-letf (((symbol-function 'y-or-n-p) (lambda (&rest _) nil)))
      (should-not (rdm-set-status rdm-transient-test--task "done"))
      (should-not (rdm-set-status rdm-transient-test--phase "wont-fix"))
      (should-not (rdm-plan-discard)))
    (should-not (rdm-transient-test--argvs log))))

(ert-deftest rdm-transient/status-rules-follow-describe ()
  "Which statuses confirm and which take a note follow what `rdm describe'
declares, whatever their names: here `parked' is the one terminal
status and records `--why'.  A set `rdm-confirm-statuses' overrides
the declared terminal statuses."
  (rdm-transient-test--with-stub
      (list :extra ",\"parked\"" :terminal "[\"parked\"]"
            :flags "[{\"value\":\"parked\",\"flag\":\"why\"}]")
    (let (asked)
      (cl-letf (((symbol-function 'y-or-n-p)
                 (lambda (prompt) (push prompt asked) t))
                ((symbol-function 'read-string)
                 (lambda (prompt &rest _) (push prompt asked) "on hold")))
        (rdm-set-status rdm-transient-test--phase "done")
        (rdm-set-status rdm-transient-test--phase "blocked")
        (should-not asked)
        (rdm-set-status rdm-transient-test--phase "parked")
        (should (equal (length asked) 2))
        (should (seq-find (lambda (p) (string-search "--why" p)) asked))
        (setq asked nil)
        (let ((rdm-confirm-statuses '("in-progress")))
          (rdm-set-status rdm-transient-test--phase "parked" "given")
          (should-not asked)
          (rdm-set-status rdm-transient-test--phase "in-progress")
          (should (equal (length asked) 1)))))
    (should (equal (rdm-transient-test--argvs log)
                   '(("phase" "update" "phase-2-two" "--roadmap=r1"
                      "--status=done" "--project" "p" "--no-edit")
                     ("phase" "update" "phase-2-two" "--roadmap=r1"
                      "--status=blocked" "--project" "p" "--no-edit")
                     ("phase" "update" "phase-2-two" "--roadmap=r1"
                      "--status=parked" "--why=on hold" "--project" "p"
                      "--no-edit")
                     ("phase" "update" "phase-2-two" "--roadmap=r1"
                      "--status=parked" "--why=given" "--project" "p"
                      "--no-edit")
                     ("phase" "update" "phase-2-two" "--roadmap=r1"
                      "--status=in-progress" "--project" "p" "--no-edit"))))))

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
    (should (equal (seq-remove (lambda (argv) (equal (car argv) "info"))
                               (mapcar #'cdr (rdm-transient-test--calls log)))
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
  "A tag edit is exactly one update carrying --add-tag/--remove-tag, with
no `show' re-read; rdm keeps the tags named in neither."
  (rdm-transient-test--with-stub (list :tags "[\"needs-plan-review\",\"bug\"]")
    (should (equal (rdm-edit-tags rdm-transient-test--task '("ui") '("bug"))
                   "Updated task 't1'\n"))
    (rdm-edit-tags '(:kind roadmap :roadmap "r1" :ref "r1") '("ui" "x") nil)
    (should (equal (rdm-transient-test--argvs log)
                   '(("task" "update" "t1" "--add-tag=ui" "--remove-tag=bug"
                      "--project" "p" "--no-edit")
                     ("roadmap" "update" "r1" "--add-tag=ui" "--add-tag=x"
                      "--project" "p" "--no-edit"))))))

(ert-deftest rdm-transient/tags-remove-last ()
  "Removing an item's last tag is a single update; nothing is refused."
  (rdm-transient-test--with-stub (list :tags "[\"bug\"]")
    (rdm-edit-tags rdm-transient-test--task nil '("bug"))
    (should (equal (rdm-transient-test--argvs log)
                   '(("task" "update" "t1" "--remove-tag=bug"
                      "--project" "p" "--no-edit"))))))

(ert-deftest rdm-transient/tags-nil-edit-writes-nothing ()
  "An edit with neither tags to add nor to remove issues no rdm call."
  (rdm-transient-test--with-stub nil
    (should-not (rdm-edit-tags rdm-transient-test--task nil nil))
    (should-not (rdm-transient-test--argvs log))))

(ert-deftest rdm-transient/dispatch-tags-passes-raw-text ()
  "The dispatch tag prompt reads the current tags only to offer removal
candidates, and passes the typed add text to rdm untrimmed."
  (rdm-transient-test--with-stub (list :tags "[\"bug\",\"ui\"]")
    (cl-letf (((symbol-function 'completing-read-multiple)
               (lambda (_prompt collection &rest _)
                 (should (equal collection '("bug" "ui")))
                 '("bug")))
              ((symbol-function 'read-string) (lambda (&rest _) " a , ,b ")))
      (rdm-dispatch-edit-tags rdm-transient-test--task))
    (should (equal (rdm-transient-test--argvs log)
                   '(("task" "show" "t1" "--project" "p" "--format" "json")
                     ("task" "update" "t1" "--add-tag= a , ,b "
                      "--remove-tag=bug" "--project" "p" "--no-edit"))))))

;;;; Plan repo

(ert-deftest rdm-transient/plan-repo-argv ()
  "status, commit and discard take --format json and no --project, and
return rdm's parsed reply; the echoed summary is built from its data."
  (rdm-transient-test--with-stub nil
    (let (messages)
      (cl-letf (((symbol-function 'y-or-n-p) (lambda (&rest _) t))
                ((symbol-function 'message)
                 (lambda (fmt &rest args)
                   (push (apply #'format fmt args) messages))))
        (let ((status (rdm-plan-status)))
          (should (equal (plist-get (aref (plist-get (plist-get status :changes)
                                                     :user)
                                          0)
                                    :path)
                         "projects/p/tasks/t1.md"))
          (should (string-search "modified projects/p/tasks/t1.md" (car messages)))
          (should (string-search "1 staged by other sessions" (car messages))))
        (should (equal (plist-get (rdm-plan-commit "-plan: msg") :commit)
                       "0123456789abcdef0123"))
        (should (string-search "committed 0123456789ab" (car messages)))
        (should (equal (append (plist-get (rdm-plan-discard) :restored) nil)
                       '("projects/p/tasks/t1.md")))
        (should (string-search "discarded 1 path" (car messages)))))
    (should-error (rdm-plan-commit "  ") :type 'user-error)
    (should (equal (rdm-transient-test--argvs log)
                   '(("status" "--format" "json")
                     ("commit" "--message=-plan: msg" "--format" "json")
                     ("discard" "--force" "--format" "json"))))))

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
  "A status and a tag edit round-trip through the real binary (removing
the last tag leaves none), are listed staged in this session, and commit.
The terminal statuses and the `blocked' note flag come from the real
`rdm describe', and the blocked reason reaches the phase."
  (rdm-test-with-real
    (setq process-environment
          (append (list (concat "RDM_ROOT=" root) (concat "RDM_PROJECT=" project))
                  process-environment))
    (setq rdm-bin bin)
    (let ((rdm-session "emacs-real-dispatch-test")
          (item '(:kind phase :roadmap "sample-roadmap" :stem "phase-3-seed-three"
                        :ref "sample-roadmap/phase-3-seed-three"))
          (user-paths (lambda (json)
                        (mapcar (lambda (c) (plist-get c :path))
                                (plist-get (plist-get json :changes) :user)))))
      (should (equal (rdm-terminal-values 'phase "status") '("done" "wont-fix")))
      (should (equal (rdm-value-flag 'phase "status" "blocked") "reason"))
      (cl-letf (((symbol-function 'y-or-n-p) #'rdm-transient-test--refuse-prompts)
                ((symbol-function 'read-string) #'rdm-transient-test--refuse-prompts))
        (should (member "blocked" (rdm-status-values 'phase)))
        (rdm-set-status item "blocked" "waiting on the emacs suite")
        (rdm-edit-tags item '("emacs") nil))
      (let ((shown (rdm-call-sync 'phase-show "phase-3-seed-three"
                                  :roadmap "sample-roadmap")))
        (should (equal (plist-get shown :status) "blocked"))
        (should (equal (plist-get shown :blocked_reason) "waiting on the emacs suite"))
        (should (equal (append (plist-get shown :tags) nil) '("emacs"))))
      (rdm-edit-tags item nil '("emacs"))
      (should-not (append (plist-get (rdm-call-sync 'phase-show "phase-3-seed-three"
                                                    :roadmap "sample-roadmap")
                                     :tags)
                          nil))
      (should (seq-find (lambda (p) (string-search "phase-3-seed-three" p))
                        (funcall user-paths (rdm-plan-status))))
      (should (plist-get (rdm-plan-commit "emacs: dispatch") :commit))
      (should-not (funcall user-paths (rdm-plan-status))))))

(provide 'rdm-transient-test)
;;; rdm-transient-test.el ends here
