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

(defun rdm-git-test--node (name kind ref &optional status children)
  "A parsed `rdm tree' node NAME of KIND with REF, STATUS and CHILDREN."
  (append (list :name name :kind kind :ref ref
                :review_ref (if (equal kind "task") ref (concat kind "/" ref)))
          (when status (list :status status))
          (list :children (or children []))))

(defun rdm-git-test--tree ()
  "A canned parsed `rdm tree --active' reply: rdm has already kept only
the active items, so every phase and task in it is offered."
  (list :name "p" :kind "project"
        :children
        (vector
         (rdm-git-test--node
          "r0" "roadmap" "r0" nil
          (vector (rdm-git-test--node "phase-1-a" "phase" "r0/phase-1-a" "reviewed")))
         (rdm-git-test--node
          "r1" "roadmap" "r1" nil
          (vector (rdm-git-test--node "phase-2-two" "phase" "r1/phase-2-two"
                                      "in-progress")))
         (rdm-git-test--node "t1" "task" "task/t1" "needs-review"))))

(defun rdm-git-test--wt (kind ref &rest fields)
  "A parsed `worktree current' reply for the KIND item REF, with FIELDS."
  (append (list :item ref :kind kind :ref ref) fields))

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
                      (`("tree" "--active" . ,_) (rdm-git-test--tree))
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
                  (rdm-git-test--wt "task" "task/t1" :slug "t1"))
                 (concat "feat: x\n\nBody text.\n\nDone: task/t1\n\n"
                         rdm-git-test--comments))))

(ert-deftest rdm-git/appends-to-trailer-block ()
  "An existing trailer block gets the line appended, with no blank line."
  (should (equal (rdm-git-test--insert-into
                  (concat "feat: x\n\nBody.\n\nRefs: #1\nCo-authored-by: A <a@b>\n\n"
                          rdm-git-test--comments)
                  (rdm-git-test--wt "phase" "r1/phase-2-two" :roadmap "r1" :stem "phase-2-two"))
                 (concat "feat: x\n\nBody.\n\nRefs: #1\nCo-authored-by: A <a@b>\n"
                         "Done: r1/phase-2-two\n\n" rdm-git-test--comments))))

(ert-deftest rdm-git/subject-is-not-a-trailer ()
  "A conventional subject alone is followed by a blank line."
  (should (equal (rdm-git-test--insert-into "feat: x\n" (rdm-git-test--wt "task" "task/t1" :slug "t1"))
                 "feat: x\n\nDone: task/t1\n")))

(ert-deftest rdm-git/empty-message-keeps-subject-line ()
  "On a fresh commit buffer the subject typed at point stays on line 1."
  (with-temp-buffer
    (rdm-git-test-commit-mode)
    (insert "\n" rdm-git-test--comments)
    (goto-char (point-min))
    (rdm-git-test--mocked (rdm-git-test--wt "phase" "r1/phase-2-two" :roadmap "r1" :stem "phase-2-two") "main"
      (rdm-insert-done-line))
    (insert "fix: subject")
    (let ((lines (split-string (buffer-string) "\n")))
      (should (equal (nth 0 lines) "fix: subject"))
      (should (equal (nth 1 lines) ""))
      (should (equal (nth 2 lines) "Done: r1/phase-2-two"))
      (should (string-prefix-p "#" (seq-find (lambda (l) (string-prefix-p "#" l))
                                             (nthcdr 3 lines))))
      (should (< (string-search "Done:" (buffer-string))
                 (string-search "# Please" (buffer-string)))))))

(ert-deftest rdm-git/preselect ()
  "`worktree current' drives the picker's default and order."
  (let ((all '("r0/phase-1-a" "r1/phase-2-two" "task/t1")))
    (dolist (case `(((:kind "phase" :roadmap "r1" :stem "phase-2-two"
                            :ref "r1/phase-2-two")
                     ,all "r1/phase-2-two")
                    ((:kind "task" :slug "t1" :ref "task/t1") ,all "task/t1")
                    ((:kind "roadmap" :roadmap "r1" :ref "r1")
                     ("r1/phase-2-two" "r0/phase-1-a" "task/t1")
                     "r1/phase-2-two")
                    (nil ,all nil)))
      (with-temp-buffer
        (rdm-git-test-commit-mode)
        (rdm-git-test--mocked (car case) "main"
          (rdm-insert-done-line)
          (should (equal (car offered) (nth 1 case)))
          (should (equal (cdr offered) (nth 2 case))))))))

(defun rdm-git-test--register (ref item)
  "Register candidate REF for ITEM, an item plist; return REF."
  (substring-no-properties (rdm-item-register ref item)))

(defun rdm-git-test--roadmap (roadmap)
  "A `worktree current' reply for the ROADMAP worktree."
  (rdm-git-test--wt "roadmap" roadmap :roadmap roadmap))

(ert-deftest rdm-git/preselect-roadmap-order ()
  "A bare roadmap with several active phases: no default, order kept.
The roadmap's candidates are the ones whose item names it."
  (let* ((rdm-item--lookup (make-hash-table :test #'equal))
         (all (list (rdm-git-test--register
                     "r1/phase-2-two" '(:kind phase :roadmap "r1" :stem "phase-2-two"))
                    (rdm-git-test--register
                     "r0/phase-1-a" '(:kind phase :roadmap "r0" :stem "phase-1-a"))
                    (rdm-git-test--register
                     "r1/phase-3-three" '(:kind phase :roadmap "r1" :stem "phase-3-three"))
                    (rdm-git-test--register "task/t1" '(:kind task :slug "t1")))))
    (should (equal (rdm-git-preselect all (rdm-git-test--roadmap "r1"))
                   '(("r1/phase-2-two" "r1/phase-3-three" "r0/phase-1-a" "task/t1"))))
    (should (equal (rdm-git-preselect all (rdm-git-test--roadmap "r0"))
                   (cons (cons "r0/phase-1-a" (remove "r0/phase-1-a" all)) "r0/phase-1-a")))
    ;; A roadmap whose name prefixes another's owns none of its phases.
    (should (equal (rdm-git-preselect all (rdm-git-test--roadmap "r")) (list all))))
  (let ((md nil))
    (with-temp-buffer
      (rdm-git-test-commit-mode)
      (rdm-git-test--mocked (rdm-git-test--roadmap "r1") "main"
        (cl-letf (((symbol-function 'completing-read)
                   (lambda (_p table &rest _)
                     (setq md (completion-metadata "" table nil))
                     (car (all-completions "" table)))))
          (rdm-insert-done-line))))
    (should (eq (completion-metadata-get md 'display-sort-function) #'identity))
    (should (eq (completion-metadata-get md 'cycle-sort-function) #'identity))))

(ert-deftest rdm-git/branch-warning ()
  "Off the default branch the line is inserted and a warning shown."
  (with-temp-buffer
    (rdm-git-test-commit-mode)
    (insert "feat: x\n")
    (rdm-git-test--mocked (rdm-git-test--wt "task" "task/t1" :slug "t1") "feature"
      (rdm-insert-done-line)
      (should (= (length warnings) 1))
      (should (string-match-p "post-merge" (car warnings))))
    (should (string-match-p "^Done: task/t1$" (buffer-string))))
  (with-temp-buffer
    (rdm-git-test-commit-mode)
    (rdm-git-test--mocked (rdm-git-test--wt "task" "task/t1" :slug "t1") "main"
      (rdm-insert-done-line)
      (should-not warnings))))

(ert-deftest rdm-git/candidates-come-from-tree-active ()
  "One `rdm tree --active' call supplies the candidates: each is its
node's `ref', and its item carries the node's refs and status."
  (let (argvs)
    (rdm-git-test--mocked nil "main"
      (cl-letf* ((orig (symbol-function 'rdm-run-sync))
                 ((symbol-function 'rdm-run-sync)
                  (lambda (command &rest keys)
                    (push (cdr command) argvs)
                    (apply orig command keys))))
        (let ((cands (rdm-git-candidates)))
          (should (equal (mapcar #'substring-no-properties cands)
                         '("r0/phase-1-a" "r1/phase-2-two" "task/t1")))
          (should (equal (seq-take (car argvs) 2) '("tree" "--active")))
          (should (= (length argvs) 1))
          (let ((phase (rdm-item-from "r1/phase-2-two")))
            (should (equal (list (plist-get phase :kind) (plist-get phase :roadmap)
                                 (plist-get phase :stem) (plist-get phase :status)
                                 (plist-get phase :review-ref))
                           '(phase "r1" "phase-2-two" "in-progress"
                                   "phase/r1/phase-2-two"))))
          (let ((task (rdm-item-from "task/t1")))
            (should (equal (plist-get task :slug) "t1"))
            (should (equal (plist-get task :ref) "task/t1"))))))))

(ert-deftest rdm-git/real-candidates ()
  "Against the real binary, the Done: picker offers the seeded plan's
active items by their refs and none of its finished or unstarted ones."
  (rdm-test-with-real
    (setq process-environment
          (append (list (concat "RDM_ROOT=" root) (concat "RDM_PROJECT=" project))
                  process-environment))
    (let* ((rdm-bin bin)
           (rdm-item--lookup (make-hash-table :test #'equal))
           (cands (mapcar #'substring-no-properties (rdm-git-candidates))))
      (should (member "sample-roadmap/phase-2-seed-two" cands))
      (should (member "task/fixture-task-active" cands))
      (should-not (member "sample-roadmap/phase-1-seed-one" cands))
      (should-not (member "sample-roadmap/phase-3-seed-three" cands))
      (should-not (member "task/fixture-task-done" cands))
      (let ((phase (rdm-item-from "sample-roadmap/phase-2-seed-two")))
        (should (equal (plist-get phase :roadmap) "sample-roadmap"))
        (should (equal (plist-get phase :stem) "phase-2-seed-two"))
        (should (equal (plist-get phase :status) "in-progress"))))))

;;;; Working tree and branch, against real repositories

(defun rdm-git-test--git (dir &rest args)
  "Run git ARGS in DIR with a clean environment; return its output lines."
  (let ((default-directory (file-name-as-directory dir))
        (process-environment (cons "GIT_DIR" (rdm-test-clean-environment))))
    (apply #'process-lines "git" "-c" "user.name=t" "-c" "user.email=t@t"
           "-c" "init.defaultBranch=main" args)))

(defun rdm-git-test--repo ()
  "A temp repository with one commit on main; return its directory."
  (let ((repo (file-truename (make-temp-file "rdm-git-repo" t))))
    (rdm-git-test--git repo "init" "-q")
    (rdm-git-test--git repo "commit" "-q" "--allow-empty" "-m" "init")
    (file-name-as-directory repo)))

(ert-deftest rdm-git/current-branch-detached ()
  "A detached HEAD has no current branch."
  (let ((repo (rdm-git-test--repo)))
    (rdm-git-test--git repo "checkout" "-q" "--detach")
    (let ((default-directory repo)
          (process-environment (cons "GIT_DIR" (rdm-test-clean-environment))))
      (should-not (rdm-git-current-branch)))))

(ert-deftest rdm-git/resolves-working-tree ()
  "From a buffer in `.git/' or `.git/worktrees/NAME/', rdm runs in the
working tree and the branch warning names its real branch."
  :tags '(magit)
  (rdm-test-with-soft-dep 'magit
    (let* ((repo (rdm-git-test--repo))
           (wt (concat (directory-file-name repo) "-wt/")))
      (rdm-git-test--git repo "checkout" "-q" "-b" "feature")
      (rdm-git-test--git repo "worktree" "add" "-q" "-b" "wt-branch"
                         (directory-file-name wt))
      (dolist (case `((,(concat repo ".git/") ,repo "feature")
                      (,(concat repo ".git/worktrees/"
                                (file-name-nondirectory (directory-file-name wt))
                                "/")
                       ,wt "wt-branch")))
        (let ((dirs nil) (warnings nil)
              (rdm-item--lookup (make-hash-table :test #'equal))
              (process-environment (cons "GIT_DIR" (rdm-test-clean-environment))))
          (with-temp-buffer
            (rdm-git-test-commit-mode)
            (setq default-directory (nth 0 case))
            (cl-letf (((symbol-function 'rdm-context)
                       (lambda () (list :root "/tmp" :project "p" :rdm-bin "rdm"
                                        :default-branch "main")))
                      ((symbol-function 'display-warning)
                       (lambda (_type msg &rest _) (push msg warnings)))
                      ((symbol-function 'completing-read)
                       (lambda (_p table &rest _) (car (all-completions "" table))))
                      ((symbol-function 'rdm-run-sync)
                       (lambda (command &rest _)
                         (push default-directory dirs)
                         (pcase (cdr command)
                           (`("tree" . ,_) (rdm-git-test--tree))
                           (`("worktree" "current" . ,_) nil)
                           (`("hook" "done-line" . ,_) "Done: task/t1\n")
                           (argv (error "Unexpected argv %S" argv))))))
              (rdm-insert-done-line))
            (should (string-match-p "^Done: task/t1$" (buffer-string))))
          (should dirs)
          (dolist (d dirs)
            (should (equal (file-truename d) (file-truename (nth 1 case)))))
          (should (= (length warnings) 1))
          (should (string-match-p (concat "Committing on " (nth 2 case) ",")
                                  (car warnings))))))))

;;;; magit wiring

(defun rdm-git-test--commit-buffer (&optional evil before)
  "A `git-commit-mode' buffer after `git-commit-setup-hook', and its messages.
With EVIL, `evil-local-mode' is on in normal state first.  BEFORE, if
non-nil, is called in the buffer just before the hook runs; the result
is (BUFFER MESSAGES . BEFORE-VALUE) instead of (BUFFER . MESSAGES)."
  (let ((buf (generate-new-buffer "COMMIT_EDITMSG"))
        (repo (make-temp-file "rdm-git-repo" t))
        msg seen)
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
        (when before (setq seen (funcall before)))
        (run-hooks 'git-commit-setup-hook)))
    (if before
        (cons buf (cons (string-join msg "\n") seen))
      (cons buf (string-join msg "\n")))))

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
            ;; Each proper prefix of KEY is unbound or a prefix keymap,
            ;; never a command.  KEY itself is unbound: `lookup-key'
            ;; returns nil, or N only when the N-key prefix is unbound
            ;; (Emacs reports an unbound prefix as "too long").
            (dolist (map (list git-commit-mode-map with-editor-mode-map))
              (let ((b (lookup-key map key)))
                (should (or (null b)
                            (and (integerp b)
                                 (null (lookup-key map (seq-take key b)))))))
              (dotimes (i (1- (length key)))
                (let ((b (lookup-key map (seq-take key (1+ i)))))
                  (should-not (integerp b))
                  (should (or (null b) (keymapp b)))))))
        (kill-buffer (car res))))))

(ert-deftest rdm-git/evil-offer ()
  "Under evil, normal-state `gd' inserts, the message names it, and
`ZZ'/`ZQ' keep their with-editor commands."
  :tags '(evil magit)
  (rdm-test-with-soft-dep 'magit
    (rdm-test-with-soft-dep 'evil
      (require 'git-commit)
      (let* ((zz (lambda ()
                   (mapcar (lambda (k) (key-binding (kbd k))) '("ZZ" "ZQ"))))
             (res (rdm-git-test--commit-buffer t zz)))
        (unwind-protect
            (with-current-buffer (car res)
              (should (eq evil-state 'normal))
              (should (string-match-p "gd" (cadr res)))
              (should (eq (key-binding (kbd "gd")) #'rdm-insert-done-line))
              ;; ZZ/ZQ resolve to what they did before the setup hook ran.
              (should (cddr res))
              (should (equal (funcall zz) (cddr res)))
              (dolist (b (cddr res))
                (should-not (eq b #'rdm-insert-done-line))))
          (kill-buffer (car res)))))))

(ert-deftest rdm-git/offer-autoloaded ()
  "Installing the package is enough: the generated loaddefs register the
commit-buffer offer once git-commit loads, without requiring rdm-git."
  (let* ((dir (file-name-directory (locate-library "rdm-git")))
         (form `(let ((out (make-temp-file "rdm-loaddefs" nil ".el")))
                  (delete-file out)
                  (let ((inhibit-message t))
                    (loaddefs-generate ,dir out))
                  (load out nil t)
                  (provide 'git-commit)
                  (princ (format "%s %s"
                                 (and (memq 'rdm-git-commit-setup
                                            (bound-and-true-p git-commit-setup-hook))
                                      t)
                                 (featurep 'rdm-git)))))
         (emacs (expand-file-name invocation-name invocation-directory)))
    (with-temp-buffer
      (should (eq 0 (call-process emacs nil t nil "-Q" "--batch"
                                  "--eval" (prin1-to-string form))))
      (should (string-suffix-p "t nil" (string-trim (buffer-string)))))))

(ert-deftest rdm-git/evil-offer-autoloaded ()
  "In a fresh session with only the loaddefs, a commit buffer's `gd'
inserts a Done: line under evil, and the offer names it."
  :tags '(evil magit)
  (rdm-test-with-soft-dep 'magit
    (rdm-test-with-soft-dep 'evil
      (let* ((dir (file-name-directory (locate-library "rdm-git")))
             (repo (make-temp-file "rdm-git-repo" t))
             (form
              `(let ((out (make-temp-file "rdm-loaddefs" nil ".el"))
                     (default-directory ,(file-name-as-directory repo))
                     msg)
                 (setq load-path ',load-path)
                 (delete-file out)
                 (let ((inhibit-message t))
                   (loaddefs-generate ,dir out))
                 (load out nil t)
                 (require 'evil)
                 (require 'git-commit)
                 (with-current-buffer (generate-new-buffer "COMMIT_EDITMSG")
                   (cl-letf (((symbol-function 'message)
                              (lambda (fmt &rest args)
                                (when fmt (push (apply #'format fmt args) msg)))))
                     (text-mode)
                     (evil-local-mode 1)
                     (evil-normal-state)
                     (git-commit-mode 1)
                     (run-hooks 'git-commit-setup-hook))
                   (princ (format "\n%S" (list (featurep 'rdm-evil)
                                               (key-binding (kbd "gd"))
                                               (string-join msg "\n")))))))
             (emacs (expand-file-name invocation-name invocation-directory)))
        (let ((default-directory repo)
              (process-environment (cons "GIT_DIR" (rdm-test-clean-environment))))
          (process-lines "git" "init" "-q"))
        (with-temp-buffer
          (let ((process-environment (cons "GIT_DIR" (rdm-test-clean-environment))))
            (should (eq 0 (call-process emacs nil t nil "-Q" "--batch"
                                        "--eval" (prin1-to-string form)))))
          (goto-char (point-max))
          (backward-sexp)
          (let ((res (read (current-buffer))))
            (should (eq (nth 1 res) 'rdm-insert-done-line))
            (should (string-match-p "rdm: gd inserts" (nth 2 res)))))))))

(provide 'rdm-git-test)
;;; rdm-git-test.el ends here
