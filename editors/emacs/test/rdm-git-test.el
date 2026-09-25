;;; rdm-git-test.el --- ERT tests for Done: trailer insertion  -*- lexical-binding: t; -*-

;;; Commentary:

;; `rdm-insert-done-line' with a mocked runner, in a stand-in commit mode
;; so it runs under `emacs -Q'.  Tests tagged `magit' (and `evil') check
;; the `git-commit-setup-hook' offer and its keys against the real maps.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'subr-x)
(require 'rdm-core)
(require 'rdm-pick)
(require 'rdm-git)
(require 'rdm-evil)
(require 'rdm-test-helpers)

(defvar git-commit-mode-map)
(defvar git-commit-setup-hook)
(defvar with-editor-mode-map)
(defvar evil-state)
(declare-function evil-local-mode "ext:evil-core" (&optional arg))
(declare-function evil-normal-state "ext:evil-states" (&optional arg))
(declare-function git-commit-mode "ext:git-commit" (&optional arg))

(define-derived-mode rdm-git-test-commit-mode text-mode "Commit"
  "A stand-in for `git-commit-mode'."
  (setq-local comment-start "#"))

(defun rdm-git-test--flag (argv flag)
  "The value of `--FLAG=value' in ARGV, or nil."
  (let ((prefix (concat "--" flag "=")))
    (cl-some (lambda (a) (and (string-prefix-p prefix a)
                              (substring a (length prefix))))
             argv)))

(defun rdm-git-test--tree ()
  "A canned parsed `rdm tree' reply."
  (list :name "p" :kind "project"
        :children
        (vector
         (list :name "r0" :kind "roadmap"
               :children (vector (list :name "phase-1-a" :kind "phase"
                                       :status "reviewed" :children [])))
         (list :name "r1" :kind "roadmap"
               :children (vector (list :name "phase-1-one" :kind "phase"
                                       :status "done" :children [])
                                 (list :name "phase-2-two" :kind "phase"
                                       :status "in-progress" :children [])))
         (list :name "t1" :kind "task" :status "needs-review" :children [])
         (list :name "t2" :kind "task" :status "open" :children []))))

(defmacro rdm-git-test--mocked (worktree branch &rest body)
  "Run BODY with a mocked runner; `worktree current' replies WORKTREE.
The current branch is BRANCH and the default branch \"main\".  Binds
`offered' to the (CANDIDATES . DEFAULT) the picker got and `warnings'
to the warnings shown.  The picker answers its default, else its first
candidate."
  (declare (indent 2) (debug t))
  `(let ((rdm-item--lookup (make-hash-table :test #'equal))
         (offered nil) (warnings nil))
     (cl-letf (((symbol-function 'rdm-context)
                (lambda () (list :root "/tmp" :project "p" :rdm-bin "rdm"
                                 :default-branch "main")))
               ((symbol-function 'rdm-git-current-branch) (lambda () ,branch))
               ((symbol-function 'display-warning)
                (lambda (_type msg &rest _) (push msg warnings)))
               ((symbol-function 'completing-read)
                (lambda (_prompt table _pred _req _init _hist def)
                  (let ((cands (all-completions "" table)))
                    (setq offered (cons cands def))
                    (or def (car cands)))))
               ((symbol-function 'rdm-run-sync)
                (lambda (command &rest _)
                  (let ((argv (cdr command)))
                    (pcase argv
                      (`("tree" . ,_) (rdm-git-test--tree))
                      (`("worktree" "current" . ,_) ,worktree)
                      (`("hook" "done-line" . ,_)
                       (if-let* ((task (rdm-git-test--flag argv "task")))
                           (format "Done: task/%s\n" task)
                         (format "Done: %s/%s\n"
                                 (rdm-git-test--flag argv "roadmap")
                                 (rdm-git-test--flag argv "phase"))))
                      (_ (error "Unexpected argv %S" argv)))))))
       ,@body)))

(defun rdm-git-test--insert-into (text worktree)
  "Insert a Done: line into commit message TEXT; return the buffer text."
  (with-temp-buffer
    (rdm-git-test-commit-mode)
    (insert text)
    (goto-char (point-min))
    (rdm-git-test--mocked worktree "main"
      (rdm-insert-done-line))
    (buffer-string)))

(defconst rdm-git-test--comments
  "# Please enter the commit message.\n# ------------------------ >8 ------------------------\ndiff\n")

(ert-deftest rdm-git/inserts-before-comments ()
  "The exact done-line text lands after the body, before the comments."
  (should (equal (rdm-git-test--insert-into
                  (concat "feat: x\n\nBody text.\n\n" rdm-git-test--comments)
                  (list :item "task/t1"))
                 (concat "feat: x\n\nBody text.\n\nDone: task/t1\n\n"
                         rdm-git-test--comments))))

(ert-deftest rdm-git/appends-to-trailer-block ()
  "An existing trailer block gets the line appended, with no blank line."
  (should (equal (rdm-git-test--insert-into
                  (concat "feat: x\n\nBody.\n\nRefs: #1\nCo-authored-by: A <a@b>\n\n"
                          rdm-git-test--comments)
                  (list :item "r1/phase-2-two"))
                 (concat "feat: x\n\nBody.\n\nRefs: #1\nCo-authored-by: A <a@b>\n"
                         "Done: r1/phase-2-two\n\n" rdm-git-test--comments))))

(ert-deftest rdm-git/subject-is-not-a-trailer ()
  "A conventional subject alone is followed by a blank line."
  (should (equal (rdm-git-test--insert-into "feat: x\n" (list :item "task/t1"))
                 "feat: x\n\nDone: task/t1\n")))

(ert-deftest rdm-git/preselect ()
  "`worktree current' drives the picker's default and order."
  (let ((all '("r0/phase-1-a" "r1/phase-2-two" "task/t1")))
    (dolist (case `(((:item "r1/phase-2-two") ,all "r1/phase-2-two")
                    ((:item "task/t1") ,all "task/t1")
                    ((:item "r1") ("r1/phase-2-two" "r0/phase-1-a" "task/t1") nil)
                    (nil ,all nil)))
      (with-temp-buffer
        (rdm-git-test-commit-mode)
        (rdm-git-test--mocked (car case) "main"
          (rdm-insert-done-line)
          (should (equal (car offered) (nth 1 case)))
          (should (equal (cdr offered) (nth 2 case))))))))

(ert-deftest rdm-git/branch-warning ()
  "Off the default branch the line is inserted and a warning shown."
  (with-temp-buffer
    (rdm-git-test-commit-mode)
    (insert "feat: x\n")
    (rdm-git-test--mocked (list :item "task/t1") "feature"
      (rdm-insert-done-line)
      (should (= (length warnings) 1))
      (should (string-match-p "post-merge" (car warnings))))
    (should (string-match-p "^Done: task/t1$" (buffer-string))))
  (with-temp-buffer
    (rdm-git-test-commit-mode)
    (rdm-git-test--mocked (list :item "task/t1") "main"
      (rdm-insert-done-line)
      (should-not warnings))))

;;;; magit wiring

(defun rdm-git-test--commit-buffer (&optional evil)
  "A `git-commit-mode' buffer after `git-commit-setup-hook', and its messages.
With EVIL, `evil-local-mode' is on in normal state first."
  (let ((buf (generate-new-buffer "COMMIT_EDITMSG"))
        (repo (make-temp-file "rdm-git-repo" t))
        msg)
    (let ((default-directory repo)
          (process-environment (cons "GIT_DIR" (rdm-test-clean-environment))))
      (process-lines "git" "init" "-q"))
    (with-current-buffer buf
      (setq default-directory (file-name-as-directory repo))
      (cl-letf (((symbol-function 'message)
                 (lambda (fmt &rest args)
                   (when fmt (push (apply #'format fmt args) msg)))))
        (text-mode)
        (when evil (evil-local-mode 1) (evil-normal-state))
        (git-commit-mode 1)
        (run-hooks 'git-commit-setup-hook)))
    (cons buf (string-join msg "\n"))))

(ert-deftest rdm-git/magit-offer ()
  "The setup hook enables the offer and names the vanilla key, which no
magit or with-editor commit binding already owns."
  :tags '(magit)
  (rdm-test-with-soft-dep 'magit
    (require 'git-commit)
    (let* ((res (rdm-git-test--commit-buffer))
           (key (kbd rdm-git-commit-key)))
      (unwind-protect
          (with-current-buffer (car res)
            (should rdm-git-commit-mode)
            (should (string-match-p (regexp-quote rdm-git-commit-key) (cdr res)))
            (should (eq (key-binding key) #'rdm-insert-done-line))
            ;; nil, or a number: a prefix of KEY is unbound.
            (should (natnump (or (lookup-key git-commit-mode-map key) 0)))
            (should (natnump (or (lookup-key with-editor-mode-map key) 0))))
        (kill-buffer (car res))))))

(ert-deftest rdm-git/evil-offer ()
  "Under evil, normal-state `gd' inserts, the message names it, and
`ZZ'/`ZQ' keep their with-editor commands."
  :tags '(evil magit)
  (rdm-test-with-soft-dep 'magit
    (rdm-test-with-soft-dep 'evil
      (require 'git-commit)
      (let ((res (rdm-git-test--commit-buffer t)))
        (unwind-protect
            (with-current-buffer (car res)
              (should (eq evil-state 'normal))
              (should (string-match-p "gd" (cdr res)))
              (should (eq (key-binding (kbd "gd")) #'rdm-insert-done-line))
              (dolist (k '("ZZ" "ZQ"))
                (should-not (eq (key-binding (kbd k)) #'rdm-insert-done-line))))
          (kill-buffer (car res)))))))

(provide 'rdm-git-test)
;;; rdm-git-test.el ends here
