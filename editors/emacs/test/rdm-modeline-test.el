;;; rdm-modeline-test.el --- ERT tests for the modeline segment and rdm-after-write-hook  -*- lexical-binding: t; -*-

;;; Commentary:

;; The segment against a mocked runner (rdm-runner-mock.el), counting
;; every `rdm-run' call: rendering never runs rdm, a cold root schedules
;; one fill, each invalidation hook re-runs both reads once, and a
;; refresh started from an unrelated buffer fills each entry from its own
;; root.  `rdm-after-write-hook' is checked against the write surface's
;; logging stub.  The `real-' test creates a worktree with
;; `rdm-worktree-add' in the fixture's code repo and renders its segment
;; through the binary under test.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'seq)
(require 'rdm-core)
(require 'rdm-view)
(require 'rdm-transient)
(require 'rdm-compose)
(require 'rdm-worktree)
(require 'rdm-modeline)
(require 'rdm-test-helpers)
(require 'rdm-transient-stub)
(require 'rdm-runner-mock)

(defvar magit-post-refresh-hook)

(defun rdm-modeline-test--root ()
  "A fresh directory holding an empty `.git'; return its cache key."
  (let ((dir (make-temp-file "rdm-wt" t)))
    (make-directory (expand-file-name ".git" dir))
    (rdm-worktree--key dir)))

(defmacro rdm-modeline-test--with-mode (&rest body)
  "Run BODY with `rdm-modeline-mode' on and its hooks isolated.
Binds `scheduled' to the idle-timer fills the segment scheduled, newest
first; timers are captured, never run."
  (declare (indent 0) (debug t))
  `(let ((after-save-hook nil)
         (rdm-after-write-hook nil)
         (magit-post-refresh-hook nil)
         (mode-line-misc-info (copy-sequence mode-line-misc-info))
         (scheduled nil))
     (cl-letf (((symbol-function 'run-with-idle-timer)
                (lambda (_secs _repeat fn &rest args)
                  (push (cons fn args) scheduled)
                  nil)))
       (rdm-modeline-mode 1)
       (unwind-protect (progn ,@body)
         (rdm-modeline-mode -1)))))

(defun rdm-modeline-test--render (root &optional times)
  "Render the segment TIMES (default 1) from a buffer in ROOT; return it."
  (with-temp-buffer
    (setq default-directory root)
    (let (text)
      (dotimes (_ (or times 1))
        (setq text (substring-no-properties (rdm-modeline-segment))))
      text)))

(defun rdm-modeline-test--replies (table)
  "A reply function: `worktree current' from TABLE, keyed by directory.
TABLE maps a root to (CURRENT . SHOW): the parsed `worktree current'
reply (only `branch', `item', `path', `rdm_managed', never `status')
and the parsed `show' reply of its item."
  (lambda (argv dir)
    (let ((entry (cdr (assoc (rdm-worktree--key dir) table))))
      (pcase argv
        (`("worktree" "current" . ,_) (car entry))
        (`(,(or "phase" "task" "roadmap") "show" . ,_)
         (should (member "--no-body" argv))
         (cdr entry))
        (_ (error "Unexpected argv %S" argv))))))

(defun rdm-modeline-test--current (item root)
  "A parsed `worktree current' reply for ITEM at ROOT: no `status'."
  (list :branch item :item item :path root :rdm_managed t))

(defun rdm-modeline-test--warm (root)
  "Render ROOT cold, then run the fill it scheduled."
  (should (equal (rdm-modeline-test--render root) ""))
  (rdm-modeline--fill-cold root))

;;;; Rendering

(ert-deftest rdm-modeline/renders-item-status ()
  "A phase and a task render their `show' status; a roadmap renders rdm's
done/total phase counts (a wont-fix phase counts as done); a checkout
that is no rdm worktree renders nothing."
  (let* ((phase (rdm-modeline-test--root))
         (task (rdm-modeline-test--root))
         (roadmap (rdm-modeline-test--root))
         (plain (rdm-modeline-test--root))
         (table
          (list (cons phase (cons (rdm-modeline-test--current "r1/phase-2-two" phase)
                                  (list :stem "phase-2-two" :status "in-progress")))
                (cons task (cons (rdm-modeline-test--current "task/t1" task)
                                 (list :slug "t1" :status "needs-review")))
                (cons roadmap
                      (cons (rdm-modeline-test--current "r1" roadmap)
                            ;; rdm's own progress counts the wont-fix phase
                            ;; as done; the segment shows it, counting nothing.
                            (list :slug "r1" :total_phases 3 :done_phases 2
                                  :progress "2/3 done"
                                  :phases (vector (list :stem "a" :status "done")
                                                  (list :stem "b" :status "wont-fix")
                                                  (list :stem "c" :status "in-progress")))))
                (cons plain (cons nil nil)))))
    (rdm-modeline-test--with-mode
      (rdm-mock-with-runner (rdm-modeline-test--replies table)
        (dolist (root (list phase task roadmap plain))
          (rdm-modeline-test--warm root))
        (should (equal (rdm-modeline-test--render phase) "r1/phase-2-two · in-progress"))
        (should (equal (rdm-modeline-test--render task) "task/t1 · needs-review"))
        (should (equal (rdm-modeline-test--render roadmap) "r1 · 2/3"))
        (should (equal (rdm-modeline-test--render plain) ""))
        (should (member '("phase" "show" "phase-2-two" "--roadmap=r1" "--no-body"
                          "--project" "p" "--format" "json")
                        (rdm-mock-argvs calls)))
        (should (member '("task" "show" "t1" "--no-body" "--project" "p"
                          "--format" "json")
                        (rdm-mock-argvs calls)))
        ;; The nano wrapper is the same text.
        (with-temp-buffer
          (setq default-directory task)
          (should (equal (substring-no-properties (rdm-modeline-nano-segment))
                         " task/t1 · needs-review")))))))

(ert-deftest rdm-modeline/cold-schedules-one-fill ()
  "Rendering a cold root runs nothing, returns \"\" and schedules one
fill, however often it renders; the fill makes both reads."
  (let* ((root (rdm-modeline-test--root))
         (table (list (cons root (cons (rdm-modeline-test--current "task/t1" root)
                                       (list :status "open"))))))
    (rdm-modeline-test--with-mode
      (rdm-mock-with-runner (rdm-modeline-test--replies table)
        (should (equal (rdm-modeline-test--render root 5) ""))
        (should-not calls)
        (should-not contexts)
        (should (equal scheduled (list (list #'rdm-modeline--fill-cold root))))
        (apply (car (car scheduled)) (cdr (car scheduled)))
        (should (equal (mapcar #'car (rdm-mock-argvs calls)) '("worktree" "task")))
        (should (equal (rdm-modeline-test--render root) "task/t1 · open"))))))

(ert-deftest rdm-modeline/warm-runs-nothing ()
  "Rendering a warm root any number of times makes no runner call."
  (let* ((root (rdm-modeline-test--root))
         (table (list (cons root (cons (rdm-modeline-test--current "task/t1" root)
                                       (list :status "open"))))))
    (rdm-modeline-test--with-mode
      (rdm-mock-with-runner (rdm-modeline-test--replies table)
        (rdm-modeline-test--warm root)
        (let ((before (length calls))
              (contexts-before (length contexts)))
          (should (= before 2))
          (should (equal (rdm-modeline-test--render root 20) "task/t1 · open"))
          (should (= (length calls) before))
          (should (= (length contexts) contexts-before)))))))

(ert-deftest rdm-modeline/hooks-refresh-once ()
  "`after-save-hook', `magit-post-refresh-hook' and `rdm-after-write-hook'
each re-run both reads exactly once; rendering afterwards runs nothing
and shows the new status."
  (let* ((root (rdm-modeline-test--root))
         (status "open")
         (reply (lambda (argv _dir)
                  (pcase argv
                    (`("worktree" "current" . ,_)
                     (rdm-modeline-test--current "task/t1" root))
                    (`("task" "show" . ,_) (list :status status))
                    (_ (error "Unexpected argv %S" argv))))))
    (rdm-modeline-test--with-mode
      (rdm-mock-with-runner reply
        (rdm-modeline-test--warm root)
        (dolist (case (list (cons "after-save" (lambda () (run-hooks 'after-save-hook)))
                            (cons "magit" (lambda () (run-hooks 'magit-post-refresh-hook)))
                            (cons "rdm-write" (lambda () (run-hooks 'rdm-after-write-hook)))))
          (setq status (car case))
          (setq calls nil)
          (with-temp-buffer
            (setq default-directory root)
            (funcall (cdr case)))
          (should (equal (mapcar #'car (rdm-mock-argvs calls)) '("worktree" "task")))
          (should (equal (rdm-modeline-test--render root 10)
                         (format "task/t1 · %s" (car case))))
          (should (= (length calls) 2)))))))

(ert-deftest rdm-modeline/refresh-fills-each-root-from-itself ()
  "A write refresh run from an unrelated buffer (a plan-repo view whose
own context names another plan root and project) fills each cached
entry from its own root: every read runs in that root with the global
context, and each entry keeps its own item."
  (let* ((a (rdm-modeline-test--root))
         (b (rdm-modeline-test--root))
         (plan (file-name-as-directory (make-temp-file "rdm-plan" t)))
         (other (file-name-as-directory (make-temp-file "rdm-other" t)))
         (calls nil))
    (rdm-test-with-env nil
      (let ((rdm-root plan) (rdm-project "p") (rdm-bin "rdm-mock")
            (rdm-after-write-hook nil) (after-save-hook nil)
            (magit-post-refresh-hook nil)
            (mode-line-misc-info (copy-sequence mode-line-misc-info)))
        (cl-letf (((symbol-function 'run-with-idle-timer) #'ignore)
                  ((symbol-function 'rdm-run)
                   (cl-function
                    (lambda (command &key directory env callback &allow-other-keys)
                      (let ((argv (cdr command)))
                        (push (list argv (rdm-worktree--key directory) env) calls)
                        (funcall callback
                                 (pcase argv
                                   (`("info" . ,_) (list :default_branch "main"))
                                   (`("worktree" "current" . ,_)
                                    (pcase (rdm-worktree--key directory)
                                      ((pred (equal a))
                                       (rdm-modeline-test--current "task/ta" a))
                                      ((pred (equal b))
                                       (rdm-modeline-test--current "r1/phase-1-x" b))))
                                   (`("task" "show" "ta" . ,_) (list :status "open"))
                                   (`("phase" "show" "phase-1-x" . ,_)
                                    (list :status "blocked"))
                                   (_ (error "Unexpected argv %S" argv))))
                        nil)))))
          (rdm-modeline-mode 1)
          (unwind-protect
              (progn
                (rdm-modeline-test--warm a)
                (rdm-modeline-test--warm b)
                (setq calls nil)
                (with-temp-buffer
                  (setq default-directory plan)
                  (setq-local rdm-root other)
                  (setq-local rdm-project "other")
                  (run-hooks 'rdm-after-write-hook))
                (let ((reads (seq-remove (lambda (c) (equal (car (car c)) "info"))
                                         calls)))
                  (should (= (length reads) 4))
                  (dolist (c reads)
                    (should (member (nth 1 c) (list a b)))
                    (should (member (concat "RDM_ROOT=" (expand-file-name plan))
                                    (nth 2 c)))
                    (unless (equal (cadr (car c)) "current")
                      (should (member "p" (car c)))
                      (should-not (member "other" (car c)))))
                  (should (seq-find (lambda (c) (and (equal (car (car c)) "task")
                                                     (equal (nth 1 c) a)))
                                    reads))
                  (should (seq-find (lambda (c) (and (equal (car (car c)) "phase")
                                                     (equal (nth 1 c) b)))
                                    reads)))
                (should (equal (rdm-modeline-test--render a) "task/ta · open"))
                (should (equal (rdm-modeline-test--render b) "r1/phase-1-x · blocked")))
            (rdm-modeline-mode -1)))))))

;; Each case delivers replies newest first, so a stale reply lands after
;; the newer fill's replies: only the generation guard in
;; `rdm-modeline--store' can drop it.

(ert-deftest rdm-modeline/newer-fill-wins ()
  "A reply of an older fill arriving last never overwrites a newer one."
  (let* ((root (rdm-modeline-test--root))
         (status "old"))
    (rdm-modeline-test--with-mode
      (rdm-mock-with-runner
          (lambda (argv _dir)
            (pcase argv
              (`("worktree" "current" . ,_) (rdm-modeline-test--current "task/t1" root))
              (`("task" "show" . ,_) (list :status status))))
        (setq rdm-mock-defer t)
        (rdm-modeline-test--render root)
        (rdm-modeline--fill-cold root)      ; the old fill: current queued
        (rdm-mock-deliver)                  ; its show is queued, answering "old"
        (setq status "new")
        (rdm-modeline-refresh-root root)    ; the new fill: current queued
        (should (rdm-mock-deliver-newest))  ; its show is queued, answering "new"
        (should (rdm-mock-deliver-newest))  ; the new label is stored
        (should (equal (rdm-modeline-test--render root) "task/t1 · new"))
        (should (rdm-mock-deliver-newest))  ; the old fill's show, last
        (should-not rdm-mock--queue)
        (should (equal (rdm-modeline-test--render root) "task/t1 · new"))))))

(ert-deftest rdm-modeline/forget-drops-late-reply ()
  "A reply of a fill in flight when its entry is forgotten fills nothing."
  (let ((root (rdm-modeline-test--root)))
    (rdm-modeline-test--with-mode
      (rdm-mock-with-runner
          (lambda (argv _dir)
            (pcase argv
              (`("worktree" "current" . ,_) (rdm-modeline-test--current "task/t1" root))
              (`("task" "show" . ,_) (list :status "open"))))
        (setq rdm-mock-defer t)
        (rdm-modeline-test--render root)
        (rdm-modeline--fill-cold root)      ; current queued
        (let ((forgotten (gethash root rdm-modeline--cache)))
          (rdm-mock-deliver)                ; show queued
          (rdm-modeline-forget root)
          (while (rdm-mock-deliver-newest))
          (should-not (gethash root rdm-modeline--cache))
          (should-not (rdm-modeline--entry-label forgotten))
          (should-not (rdm-modeline--entry-item forgotten)))))))

(ert-deftest rdm-modeline/fill-error-renders-empty ()
  "A failing read caches an empty entry and does not refetch on render."
  (let ((root (rdm-modeline-test--root)))
    (rdm-modeline-test--with-mode
      (rdm-mock-with-runner (lambda (_argv _dir) '(:mock-error . "boom"))
        (rdm-modeline-test--warm root)
        (should (= (length calls) 1))
        (should (equal (rdm-modeline-test--render root 5) ""))
        (should (= (length calls) 1))
        (should (= (length scheduled) 1))))))

(ert-deftest rdm-modeline/mode-installs-and-removes ()
  "The mode adds the segment and its hooks, and removes them when off."
  (let ((after-save-hook nil) (rdm-after-write-hook nil)
        (magit-post-refresh-hook nil)
        (mode-line-misc-info (copy-sequence mode-line-misc-info)))
    (rdm-modeline-mode 1)
    (unwind-protect
        (progn
          (should (member rdm-modeline--mode-line-entry mode-line-misc-info))
          (should (memq #'rdm-modeline-refresh after-save-hook))
          (should (memq #'rdm-modeline-refresh magit-post-refresh-hook))
          (should (memq #'rdm-modeline-refresh-all rdm-after-write-hook)))
      (rdm-modeline-mode -1))
    (should-not (member rdm-modeline--mode-line-entry mode-line-misc-info))
    (should-not after-save-hook)
    (should-not magit-post-refresh-hook)
    (should-not rdm-after-write-hook)))

(ert-deftest rdm-worktree/open-fills-modeline ()
  "Opening a worktree with the mode on fills its entry at once."
  (let* ((root (rdm-modeline-test--root))
         (table (list (cons root (cons (rdm-modeline-test--current "task/t1" root)
                                       (list :status "open")))))
         (opened nil)
         (rdm-worktree-open-function (lambda (dir) (push dir opened))))
    (rdm-modeline-test--with-mode
      (rdm-mock-with-runner (rdm-modeline-test--replies table)
        (rdm-worktree-open root)
        (should (equal opened (list root)))
        (should (equal (mapcar #'car (rdm-mock-argvs calls)) '("worktree" "task")))
        (should (equal (rdm-modeline-test--render root) "task/t1 · open"))
        (should-not scheduled)))))

(defun rdm-modeline-test--alias (root)
  "A symbolic link to ROOT, as a directory file name (no trailing slash).
It is not ROOT's cache key, so only `rdm-worktree--key' matches them."
  (let ((link (make-temp-name
               (expand-file-name "rdm-wt-link" temporary-file-directory))))
    (make-symbolic-link (directory-file-name root) link)
    (should-not (equal link root))
    (should (equal (rdm-worktree--key link) root))
    link))

(defun rdm-modeline-test--worktree-replies (table extra)
  "The replies of TABLE, with `worktree remove'/`prune' answered by EXTRA.
EXTRA is called with the argv of a `worktree remove' or `prune'."
  (let ((base (rdm-modeline-test--replies table)))
    (lambda (argv dir)
      (pcase argv
        (`("worktree" ,(or "remove" "prune") . ,_) (funcall extra argv))
        (_ (funcall base argv dir))))))

(ert-deftest rdm-worktree/remove-forgets-modeline ()
  "Removing a worktree drops its modeline entry, even when the row's
`:path' is a symbolic link to (not the truename of) the cached root."
  (let* ((root (rdm-modeline-test--root))
         (other (rdm-modeline-test--root))
         (link (rdm-modeline-test--alias root))
         (table (list (cons root (cons (rdm-modeline-test--current "task/t1" root)
                                       (list :status "open")))
                      (cons other (cons (rdm-modeline-test--current "task/t2" other)
                                        (list :status "open"))))))
    (rdm-modeline-test--with-mode
      (rdm-mock-with-runner
          (rdm-modeline-test--worktree-replies table (lambda (_) "Removed\n"))
        (rdm-modeline-test--warm root)
        (rdm-modeline-test--warm other)
        (should (gethash root rdm-modeline--cache))
        (cl-letf (((symbol-function 'y-or-n-p) (lambda (_) t)))
          (rdm-worktree-remove (list :item "task/t1" :path link :dirty :false)))
        (should (seq-find (lambda (argv) (equal (seq-take argv 3)
                                                (list "worktree" "remove" link)))
                          (rdm-mock-argvs calls)))
        (should-not (gethash root rdm-modeline--cache))
        (should (gethash other rdm-modeline--cache))))))

(ert-deftest rdm-worktree/prune-forgets-only-removed ()
  "Pruning drops the modeline entry of a removed worktree (reported by a
symbolic link) and of one removed with its branch kept, and keeps the
entry of one rdm skipped as dirty."
  (let* ((gone (rdm-modeline-test--root))
         (gone-kept-branch (rdm-modeline-test--root))
         (kept (rdm-modeline-test--root))
         (link (rdm-modeline-test--alias gone))
         (table (list (cons gone (cons (rdm-modeline-test--current "task/t1" gone)
                                       (list :status "done")))
                      (cons gone-kept-branch
                            (cons (rdm-modeline-test--current "task/t3" gone-kept-branch)
                                  (list :status "done")))
                      (cons kept (cons (rdm-modeline-test--current "task/t2" kept)
                                       (list :status "done")))))
         (prune
          (lambda (argv)
            (let ((dry (member "--dry-run" argv)))
              (list :results
                    (vector (list :item "task/t1" :path link
                                  :action (if dry "would-remove" "removed"))
                            (list :item "task/t3" :path gone-kept-branch
                                  :action (if dry "would-remove" "removed-branch-kept"))
                            (list :item "task/t2" :path kept
                                  :action (if dry "would-remove" "skipped-dirty")))
                    :removed (if dry 0 2))))))
    (rdm-modeline-test--with-mode
      (rdm-mock-with-runner (rdm-modeline-test--worktree-replies table prune)
        (rdm-modeline-test--warm gone)
        (rdm-modeline-test--warm gone-kept-branch)
        (rdm-modeline-test--warm kept)
        (should (gethash gone-kept-branch rdm-modeline--cache))
        (cl-letf (((symbol-function 'y-or-n-p) (lambda (_) t)))
          (should (rdm-worktree-prune)))
        (should-not (gethash gone rdm-modeline--cache))
        (should-not (gethash gone-kept-branch rdm-modeline--cache))
        (should (equal (rdm-modeline-test--render kept) "task/t2 · done"))))))

;;;; rdm-after-write-hook

(defmacro rdm-modeline-test--counting-writes (&rest body)
  "Run BODY with `rdm-after-write-hook' counting its runs in `writes'."
  (declare (indent 0) (debug t))
  `(let* ((writes 0)
          (rdm-after-write-hook (list (lambda () (cl-incf writes)))))
     ,@body))

(defun rdm-modeline-test--submit (buf text)
  "Replace compose buffer BUF's text with TEXT and submit it."
  (with-current-buffer buf
    (erase-buffer)
    (insert text)
    (rdm-compose-submit)))

(ert-deftest rdm-after-write-hook/once-per-write ()
  "Status, tags, a compose-submitted body, and a compose-submitted create
of each kind each run the hook exactly once."
  (rdm-transient-test--with-stub nil
    (rdm-modeline-test--counting-writes
      (cl-letf (((symbol-function 'y-or-n-p) (lambda (&rest _) t)))
        (rdm-set-status rdm-transient-test--phase "in-progress")
        (should (= writes 1))
        (rdm-set-status rdm-transient-test--task "done")
        (should (= writes 2)))
      (rdm-edit-tags rdm-transient-test--task '("ui") nil)
      (should (= writes 3))
      (rdm-modeline-test--submit (rdm-edit-body rdm-transient-test--task) "New body")
      (should (= writes 4))
      (rdm-modeline-test--submit (rdm-create-task "t2" "T2" nil) "body")
      (should (= writes 5))
      (rdm-modeline-test--submit (rdm-create-roadmap "r2" "R2" nil) "body")
      (should (= writes 6))
      (let ((answers '("p9" "P9" "" "")))
        (cl-letf (((symbol-function 'read-string) (lambda (&rest _) (pop answers))))
          (rdm-modeline-test--submit
           (rdm-create-phase '(:kind roadmap :roadmap "r1")) "body")))
      (should (= writes 7)))))

(ert-deftest rdm-after-write-hook/not-on-decline-failure-or-noop ()
  "A declined confirmation, a refused write and a tag edit naming no
tags do not run the hook."
  (rdm-transient-test--with-stub nil
    (rdm-modeline-test--counting-writes
      (cl-letf (((symbol-function 'y-or-n-p) (lambda (&rest _) nil)))
        (should-not (rdm-set-status rdm-transient-test--task "done")))
      (should-not (rdm-edit-tags rdm-transient-test--task nil nil))
      (should (= writes 0))))
  (rdm-transient-test--with-stub (list :refuse "error: refused")
    (rdm-modeline-test--counting-writes
      (should-error (rdm-set-status rdm-transient-test--task "in-progress")
                    :type 'user-error)
      (should-error (rdm-update-body rdm-transient-test--task "x")
                    :type 'user-error)
      (should (= writes 0)))))

;;;; Against the real binary

(ert-deftest rdm-modeline/real-worktree-segment ()
  "A worktree `rdm-worktree-add' creates for a task in the fixture's code
repo renders the task and its status in the segment."
  (rdm-test-with-real
    (skip-unless code)
    (let ((rdm-root root) (rdm-project project) (rdm-bin bin)
          (rdm-session "emacs-real-modeline-test")
          (default-directory (file-name-as-directory code))
          (opened nil))
      (rdm-modeline-test--with-mode
        (let* ((rdm-worktree-open-function (lambda (dir) (push dir opened)))
               (path (rdm-worktree-add '(:kind task :slug "fixture-task-open")))
               (deadline (+ (float-time) 20))
               text)
          (should (equal opened (list path)))
          (should (file-directory-p path))
          (should (string-prefix-p (file-truename (expand-file-name
                                                   ".." (file-name-as-directory code)))
                                   (file-truename path)))
          (while (and (string-empty-p (setq text (rdm-modeline-test--render path)))
                      (< (float-time) deadline))
            (accept-process-output nil 0.05))
          (should (equal text "task/fixture-task-open · open"))
          (should-not scheduled))))))

(provide 'rdm-modeline-test)
;;; rdm-modeline-test.el ends here
